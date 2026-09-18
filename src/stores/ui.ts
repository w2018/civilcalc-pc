/**
 * UI 状态（侧栏折叠、主题、背景、全局搜索框）。
 *
 * ## 为什么侧栏折叠不放偏好文件
 *
 * 偏好文件（`config.json`）存的是**用户可见的设置**，改动会触发
 * 「重置外观」等区块语义。侧栏折叠是**纯 UI 临时状态**，
 * 放 localStorage 更合适 —— 也避免每次折叠都写一次盘。
 *
 * **默认折叠、之后记住用户的选择**（见 `readCollapsed()`）——
 * 也就是说「重置软件」不会把它清回默认（它本来也不在偏好里）。
 *
 * ## 主题 / 背景**要**放偏好
 *
 * 它们属于"外观"，用户会期望它在「设置 → 外观」里出现，
 * 且要参与备份。所以走 `appearance_*` 命令。
 */

import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { appearanceApi } from '@/api'
import { assetSrc } from '@/api/asset'
import { setThemeMode } from '@/composables/useTheme'
import {
  applyBackgroundColor,
  setBackgroundUrl,
  // 别名：本 store 也有同名 action，直接用原名会撞
  setTextColor as applyTextColor,
  setTransparency as applyTransparency,
} from '@/composables/useBackground'
import { notifySaved } from '@/utils/toast'
import type { ThemeMode } from '@/types/system'

/** 侧栏折叠状态的 localStorage 键 */
const COLLAPSED_KEY = 'civilcalc.ui.navCollapsed'

/** 侧栏宽度（展开 / 折叠） */
export const NAV_WIDTH = 208
export const NAV_WIDTH_COLLAPSED = 56

export const useUiStore = defineStore('ui', () => {
  // ---------------------------------------------------------------- 侧栏

  const navCollapsed = ref(readCollapsed())

  /**
   * 读侧栏折叠状态。
   *
   * 🔴 **默认折叠**：没存过（首次启动）时按折叠算 —— 侧栏是导航，
   * 日常大部分时间在看内容区，展开是「需要时才做」的动作。
   * 用户手动切过一次之后就记住他的选择（`localStorage`）。
   */
  function readCollapsed(): boolean {
    try {
      const saved = localStorage.getItem(COLLAPSED_KEY)
      // `null` = 从没存过 → 默认折叠；存过就完全听用户的
      return saved === null ? true : saved === '1'
    } catch {
      // 隐私模式 / 存储被禁：读不到偏好，同样按默认折叠，不影响启动
      return true
    }
  }

  function toggleNav(): void {
    navCollapsed.value = !navCollapsed.value
    try {
      localStorage.setItem(COLLAPSED_KEY, navCollapsed.value ? '1' : '0')
    } catch {
      // 同上：写不进去也不该报错
    }
  }

  const navWidth = computed(() => (navCollapsed.value ? NAV_WIDTH_COLLAPSED : NAV_WIDTH))

  // ---------------------------------------------------------------- 主题

  const themeMode = ref<ThemeMode>('SYSTEM')

  // ---------------------------------------------------------------- 背景

  const hasBackground = ref(false)
  const transparency = ref(22)
  const textColor = ref('AUTO')
  const backgroundUrl = ref<string | null>(null)

  /** 生效的页面底色（自定义背景色的十六进制值） */
  const backgroundColor = ref<string | null>(null)

  /**
   * 从后端拉外观状态并应用。
   *
   * 由 `App.vue` 在 `onMounted` 调用 —— **不阻塞挂载**：
   * 主题已在 `main.ts` 里用默认值应用过一次，这里只是纠正。
   * 拉取失败只记日志（外观读不到不该让应用打不开）。
   */
  async function loadAppearance(): Promise<void> {
    try {
      const a = await appearanceApi.appearanceGet()
      themeMode.value = a.themeMode
      setThemeMode(a.themeMode)

      transparency.value = a.backgroundTransparency
      applyTransparency(a.backgroundTransparency)

      textColor.value = a.textColor
      applyTextColor(a.textColor)

      // 🔴 页面底色**必须**一起应用（需求 9）。
      //
      // `applyBackgroundColor` 除了设 `--c-bg-custom`，还会推导
      // `data-bg-contrast`（文字色组）。此前这里漏了它，于是：
      //
      // - 重启后自定义底色丢了，但 DOM 上的 `data-bg-contrast` 也没被设过
      // - 「重置软件」清了偏好，DOM 上却还留着旧的 `data-bg-contrast`
      //   → 文字色看起来「重置失败」
      //
      // 传空串 = 回落默认底色（`applyBackgroundColor` 内部处理）。
      backgroundColor.value = a.backgroundColor
      applyBackgroundColor(a.backgroundColor ?? '')
    } catch (e) {
      console.warn('[ui] 读外观失败，保持默认', e)
    }

    try {
      const bg = await appearanceApi.backgroundGet()
      hasBackground.value = bg.hasBackground
      backgroundUrl.value = bg.imagePath ? assetSrc(bg.imagePath) : null
      setBackgroundUrl(backgroundUrl.value)
    } catch (e) {
      console.warn('[ui] 读背景失败，保持默认', e)
    }
  }

  /**
   * 切换主题（立即生效 + 落盘）。
   *
   * 落盘失败不回滚 UI —— 用户看到的效果与预期一致更重要，
   * 下次启动回到旧值也比"点了没反应"好。
   */
  async function setTheme(next: ThemeMode): Promise<void> {
    themeMode.value = next
    setThemeMode(next)
    await saveAppearance()
  }

  /** 设置自定义页面底色（`null` 恢复默认） */
  async function setBackgroundColor(hex: string | null): Promise<void> {
    backgroundColor.value = hex
    applyBackgroundColor(hex ?? '')
    await saveAppearance()
  }

  /**
   * 设置背景图透明度（0~100）。
   *
   * 与主题一样「先应用后落盘」：落盘失败不回滚 UI ——
   * 用户看到的效果与预期一致更重要。
   */
  async function setTransparency(next: number): Promise<void> {
    const v = Math.min(100, Math.max(0, Math.round(next)))
    transparency.value = v
    applyTransparency(v)
    await saveAppearance()
  }

  /** 设置文字色（`AUTO` 或 `#RRGGBB`） */
  async function setTextColor(next: string): Promise<void> {
    textColor.value = next
    applyTextColor(next)
    await saveAppearance()
  }

  /**
   * 设置背景图：把 `path` 指向的图片**复制**进应用数据目录并应用。
   *
   * 后端负责缩放/落盘；前端只刷新 URL。
   */
  async function setBackgroundImage(path: string, nextTransparency?: number): Promise<void> {
    const t = nextTransparency ?? transparency.value
    await appearanceApi.backgroundSave(path, t)
    transparency.value = t
    applyTransparency(t)
    await refreshBackground()
  }

  /** 清除背景图（后端会**一并重置文字色与透明度**，所以这里也要同步） */
  async function clearBackground(): Promise<void> {
    await appearanceApi.backgroundClear()
    hasBackground.value = false
    backgroundUrl.value = null
    setBackgroundUrl(null)

    // 后端把文字色与透明度也回默认了 —— 前端必须跟着走，
    // 否则界面还留着旧透明度，与磁盘上的状态不一致
    const def = await appearanceApi.backgroundDefaultTransparency()
    transparency.value = def
    applyTransparency(def)
    textColor.value = 'AUTO'
    applyTextColor('AUTO')

    await refreshBackground()
  }

  /**
   * 统一落盘外观（五个字段一起，因为 `appearance_save` 是整体覆盖）。
   *
   * ## 提示在这里统一给（需求 8）
   *
   * 主题、底色、透明度、文字色四个入口都汇到这一个函数，
   * 所以「已保存」只写一处。`notifySaved` 会把连续触发**合并**
   * （拖透明度滑块时不会刷屏）—— 见 `utils/toast.ts`。
   */
  async function saveAppearance(): Promise<void> {
    try {
      await appearanceApi.appearanceSave({
        themeMode: themeMode.value,
        hasBackground: hasBackground.value,
        backgroundTransparency: transparency.value,
        textColor: textColor.value,
        backgroundColor: backgroundColor.value,
      })
      notifySaved('外观已保存')
    } catch (e) {
      console.warn('[ui] 外观落盘失败', e)
      ElMessage.error('外观保存失败，请重试')
    }
  }

  /** 重新读一次背景（设置页改完背景图后调用） */
  async function refreshBackground(): Promise<void> {
    try {
      const bg = await appearanceApi.backgroundGet()
      hasBackground.value = bg.hasBackground
      backgroundUrl.value = bg.imagePath ? assetSrc(bg.imagePath) : null
      setBackgroundUrl(backgroundUrl.value)
    } catch (e) {
      console.warn('[ui] 刷新背景失败', e)
    }
  }

  return {
    // 侧栏
    navCollapsed,
    navWidth,
    toggleNav,
    // 主题
    themeMode,
    setTheme,
    // 背景
    hasBackground,
    transparency,
    textColor,
    backgroundUrl,
    backgroundColor,
    loadAppearance,
    setBackgroundColor,
    setTransparency,
    setTextColor,
    setBackgroundImage,
    clearBackground,
    refreshBackground,
  }
})
