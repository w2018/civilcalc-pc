/**
 * 三态主题（`system` / `light` / `dark`）—— 见 docs/05-项目开发方案.md §2.3.6。
 *
 * ## 为什么是模块级单例而不是 `provide/inject`
 *
 * 主题是**全局唯一状态**，且需要在 `main.ts` 里于挂载前初始化。
 * 用模块级 `ref` 可以让任意组件直接 `useTheme()` 而无需注入链，
 * 代价是测试时需要注意状态残留（提供 `resetForTest()`）。
 *
 * ## 与背景的关系：**正交**
 *
 * - `data-theme` 决定卡片 / 边框 / 控件 / 徽章的色板
 * - `data-bg-contrast` 决定**文字色组**（见 `useBackground`）
 *
 * 所以浅色主题 + 深色背景时，文字会由背景亮度接管为浅色，不会出现深底黑字。
 *
 * ## 落盘
 *
 * 主题写在偏好键 `theme_mode`（取值 `SYSTEM` / `LIGHT` / `DARK`）。
 * 本模块**只负责应用到 DOM**；读写偏好由 `stores/ui.ts` 编排
 * （避免 composable 直接依赖 IPC，便于单测）。
 */

import { computed, ref } from 'vue'
import type { ThemeMode } from '@/types/system'

/** 当前主题模式（落盘值） */
const mode = ref<ThemeMode>('SYSTEM')

/** 系统是否为暗色（`SYSTEM` 模式下生效） */
const systemDark = ref(false)

/** 媒体查询句柄（懒创建，便于测试环境跳过） */
let media: MediaQueryList | null = null
let mediaBound = false

/** 实际是否为暗色（解析 `SYSTEM` 后的结果） */
const isDark = computed(
  () => mode.value === 'DARK' || (mode.value === 'SYSTEM' && systemDark.value),
)

/** 把当前状态写到 `<html>` 的 `data-theme` 属性上 */
function apply(): void {
  document.documentElement.dataset.theme = isDark.value ? 'dark' : 'light'
}

/**
 * 绑定系统主题变化的监听（幂等，只会绑一次）。
 *
 * 只在 `mode === 'SYSTEM'` 时响应 —— 用户手动选了亮/暗就不该被系统改回去。
 */
function bindSystemWatcher(): void {
  if (mediaBound || typeof window === 'undefined' || !window.matchMedia) return
  media = window.matchMedia('(prefers-color-scheme: dark)')
  systemDark.value = media.matches

  const onChange = (e: MediaQueryListEvent) => {
    systemDark.value = e.matches
    if (mode.value === 'SYSTEM') apply()
  }

  // 新旧 API 兼容：Safari < 14 只有 addListener
  if (typeof media.addEventListener === 'function') {
    media.addEventListener('change', onChange)
  } else {
    ;(media as unknown as { addListener: (cb: (e: MediaQueryListEvent) => void) => void })
      .addListener(onChange)
  }
  mediaBound = true
}

/**
 * 设置主题模式并立即生效。
 *
 * @param next `SYSTEM` / `LIGHT` / `DARK`
 */
export function setThemeMode(next: ThemeMode): void {
  mode.value = next
  bindSystemWatcher()
  apply()
}

/**
 * 初始化：读系统偏好 + 应用一次。
 *
 * 由 `main.ts` 在挂载前调用（避免首屏闪白/闪黑）。
 */
export function initTheme(initial: ThemeMode = 'SYSTEM'): void {
  bindSystemWatcher()
  setThemeMode(initial)
}

/** 供测试清理模块级状态 */
export function resetThemeForTest(): void {
  mode.value = 'SYSTEM'
  systemDark.value = false
  mediaBound = false
  media = null
}

/**
 * 主题轮转顺序：跟随系统 → 亮 → 暗 → 跟随系统。
 *
 * 放在这里而不是组件里，是为了让「标题栏快捷按钮」与
 * 「设置页三选一」共用同一套顺序，不会各写一份。
 */
export function nextTheme(cur: ThemeMode): ThemeMode {
  switch (cur) {
    case 'SYSTEM':
      return 'LIGHT'
    case 'LIGHT':
      return 'DARK'
    case 'DARK':
      return 'SYSTEM'
  }
}

/** 主题模式 → 图标（标题栏按钮用） */
export function themeIcon(mode: ThemeMode): string {
  switch (mode) {
    case 'DARK':
      return '🌙'
    case 'LIGHT':
      return '☀'
    case 'SYSTEM':
      return '🖥'
  }
}

/** 主题模式 → 中文标签 */
export function themeLabel(mode: ThemeMode): string {
  switch (mode) {
    case 'DARK':
      return '暗色'
    case 'LIGHT':
      return '亮色'
    case 'SYSTEM':
      return '跟随系统'
  }
}

export function useTheme() {
  return {
    /** 落盘的模式 */
    mode: computed(() => mode.value),
    /** 解析后的实际结果 */
    isDark,
    setThemeMode,
  }
}
