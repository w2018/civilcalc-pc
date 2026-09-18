/**
 * 自定义应用背景 + 文字色自动判定 —— 见 docs/05-项目开发方案.md §2.3.6。
 *
 * ## 三个数据属性正交
 *
 * | 属性 | 决定什么 | 取值 |
 * |---|---|---|
 * | `data-theme` | 卡片 / 边框 / 控件 / 徽章的色板 | `light` / `dark` |
 * | `data-bg` | 页面底色是否被用户覆盖 | `default` / `custom` |
 * | `data-bg-contrast` | **文字色组**（由背景亮度自动推导） | `dark-text` / `light-text` |
 *
 * 关键点：**浅色主题 + 深色背景**时文字会自动变浅色，不会出现深底黑字。
 *
 * ## 自定义背景只影响 `--c-bg`
 *
 * 卡片背景固定用 `--c-surface`，**不随自定义背景变化** ——
 * 否则用户选了一张花哨的图，卡片会跟着糊掉。
 */

import { ref } from 'vue'

/** 默认背景色（与 tokens.css 的 `--c-bg` 一致） */
export const DEFAULT_BG = '#f2f2f2'

/** WCAG 对比度阈值（正文可读性下限） */
export const CONTRAST_MIN = 4.5

/** 深色文字组（浅背景用） */
const DARK_TEXT = '#191919'
/** 浅色文字组（深背景用） */
const LIGHT_TEXT = '#ededed'

/** 背景图 URL（未设置时为 `null`）；由 `stores/ui.ts` 设置 */
const backgroundUrl = ref<string | null>(null)

/** 背景透明度 0~100（0 = 完全透明看不到图，100 = 完全不透明） */
const transparency = ref(22)

/** 自定义文字色：`AUTO` 或 `#RRGGBB` */
const textColor = ref('AUTO')

/** 最近一次对比度检查结果（低于阈值时由设置页提示用户） */
const lastContrast = ref<{ ratio: number; ok: boolean } | null>(null)

/** `#RGB` / `#RRGGBB` → 0~255 三元组；解析失败返回 `null` */
export function hexToRgb(hex: string): { r: number; g: number; b: number } | null {
  const s = hex.trim().replace(/^#/, '')
  const full =
    s.length === 3
      ? s
          .split('')
          .map((c) => c + c)
          .join('')
      : s
  if (!/^[0-9a-fA-F]{6}$/.test(full)) return null
  return {
    r: Number.parseInt(full.slice(0, 2), 16),
    g: Number.parseInt(full.slice(2, 4), 16),
    b: Number.parseInt(full.slice(4, 6), 16),
  }
}

/** sRGB 分量线性化（WCAG 2.x 公式） */
function srgb(c: number): number {
  const v = c / 255
  return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4
}

/** WCAG 相对亮度（0 = 黑，1 = 白） */
export function relativeLuminance(hex: string): number | null {
  const rgb = hexToRgb(hex)
  if (!rgb) return null
  return 0.2126 * srgb(rgb.r) + 0.7152 * srgb(rgb.g) + 0.0722 * srgb(rgb.b)
}

/** WCAG 对比度（1 ~ 21）。任一颜色无法解析时返回 `null` */
export function contrastRatio(a: string, b: string): number | null {
  const la = relativeLuminance(a)
  const lb = relativeLuminance(b)
  if (la === null || lb === null) return null
  const [hi, lo] = la > lb ? [la, lb] : [lb, la]
  return (hi + 0.05) / (lo + 0.05)
}

/**
 * 按背景亮度推导文字色组。
 *
 * 阈值 `0.5` 而非 `0.18`（WCAG 分界）—— 微信风的中间灰偏多，
 * 用 0.5 能让 `#767676` 这类中灰落到「浅底深字」，与设计稿一致。
 */
export function contrastGroupOf(bgHex: string): 'dark-text' | 'light-text' {
  const lum = relativeLuminance(bgHex)
  if (lum === null) return 'dark-text'
  return lum < 0.5 ? 'light-text' : 'dark-text'
}

/**
 * 应用背景色并自动推导文字色组。
 *
 * @param hex 背景色（`#RRGGBB`）。传空串或非法值 → 回落默认背景
 */
export function applyBackgroundColor(hex: string): void {
  const root = document.documentElement
  const valid = hexToRgb(hex) !== null
  const bg = valid ? hex : DEFAULT_BG

  root.style.setProperty('--c-bg-custom', bg)
  root.dataset.bg = valid && bg.toLowerCase() !== DEFAULT_BG ? 'custom' : 'default'

  const group = contrastGroupOf(bg)
  root.dataset.bgContrast = group

  const target = group === 'light-text' ? LIGHT_TEXT : DARK_TEXT
  const ratio = contrastRatio(bg, target)
  lastContrast.value = ratio === null ? null : { ratio, ok: ratio >= CONTRAST_MIN }
}

/**
 * 设置背景图 URL（传 `null` 清除）。
 *
 * 图片由 Rust 侧复制到 `app_data_dir/background.jpg`，前端用
 * `assetSrc()` 转成 WebView 可访问的 URL。
 */
export function setBackgroundUrl(url: string | null): void {
  backgroundUrl.value = url
  const root = document.documentElement
  if (url) {
    root.style.setProperty('--bg-image', `url("${url}")`)
    root.dataset.bgImage = 'on'
  } else {
    root.style.removeProperty('--bg-image')
    root.dataset.bgImage = 'off'
  }
}

/** 设置透明度（夹到 0~100） */
export function setTransparency(v: number): void {
  transparency.value = Math.min(100, Math.max(0, Math.round(v)))
  document.documentElement.style.setProperty('--bg-transparency', String(transparency.value / 100))
  applySurfaceAlpha(transparency.value)
}

/**
 * 区块面的半透明程度**跟着「透明度」滑块走**（需求 3）。
 *
 * ## 为什么不能把区块面写死
 *
 * 开了背景图之后区块面换成半透明的 `--c-surface-block`（让图透出来）。
 * 但如果这个半透明值是**写死的**，滑块就只能影响卡片**缝隙**里的图 ——
 * 用户会觉得「拖了没反应」。
 *
 * 所以把两者绑到一起：
 *
 * | 透明度 | 图本身 | 区块面 alpha |
 * |---|---|---|
 * | 0 | 看不见 | 1.00（完全不透，图只在缝隙里） |
 * | 22（默认） | 0.22 | 0.89 |
 * | 50 | 0.50 | 0.75 |
 * | 100 | 1.00 | 0.50（图明显透进卡片） |
 *
 * 线性映射 `alpha = 1 - t/100 × 0.5`，下限 0.5 —— 再透下去正文就压不住了
 * （区块框线还在，但对比度会掉）。
 *
 * 次级面（卡片内的代码块 / 输入底）比主面再透一点点，保持层次。
 */
function applySurfaceAlpha(percent: number): void {
  const a = 1 - (percent / 100) * 0.5
  const root = document.documentElement
  root.style.setProperty('--bg-surface-alpha', a.toFixed(3))
  root.style.setProperty('--bg-surface-alpha-2', Math.max(0.45, a - 0.04).toFixed(3))
}

/**
 * 设置文字色。
 *
 * @param value `AUTO`（跟随主题）或 `#RRGGBB`
 */
export function setTextColor(value: string): void {
  textColor.value = value
  const root = document.documentElement
  if (value === 'AUTO' || !hexToRgb(value)) {
    // 交回给 data-bg-contrast / data-theme 决定
    root.style.removeProperty('--c-text-override')
    root.dataset.textCustom = 'off'
  } else {
    root.style.setProperty('--c-text-override', value)
    root.dataset.textCustom = 'on'
  }
}

/** 一次性应用完整背景状态（`stores/ui.ts` 初始化时调） */
export function applyBackgroundState(opts: {
  color?: string
  url?: string | null
  transparency?: number
  textColor?: string
}): void {
  if (opts.color !== undefined) applyBackgroundColor(opts.color)
  if (opts.url !== undefined) setBackgroundUrl(opts.url)
  if (opts.transparency !== undefined) setTransparency(opts.transparency)
  if (opts.textColor !== undefined) setTextColor(opts.textColor)
}

/** 供测试清理 */
export function resetBackgroundForTest(): void {
  backgroundUrl.value = null
  transparency.value = 22
  textColor.value = 'AUTO'
  lastContrast.value = null
  applySurfaceAlpha(22)
}

export function useBackground() {
  return {
    backgroundUrl,
    transparency,
    textColor,
    lastContrast,
    applyBackgroundColor,
    setBackgroundUrl,
    setTransparency,
    setTextColor,
  }
}
