/**
 * 「已保存」提示（需求 8）。
 *
 * ## 为什么不能直接 `ElMessage.success('已保存')`
 *
 * 设置里有**实时保存**：拖透明度滑块、在提示词框里打字 ——
 * 这些操作会**连续触发**保存。每次弹一个 toast 的结果是：
 * 屏幕上同时堆十几个「已保存」，把内容全遮住。
 *
 * 所以这里做**首尾合并**：
 *
 * - 距上次提示超过 `MIN_GAP` → **立刻**弹（单次点按保存按钮零延迟）
 * - 太近 → 丢掉，并安排一次**尾随**提示（拖完滑块停手后仍有一次确认）
 *
 * 效果：连续操作时最多每 0.9 秒一个提示；单次操作立即反馈。
 *
 * ## 不要用它提示「草稿已保存」
 *
 * 参数草稿、模型测试草稿这类**用户没主动保存**的东西不该弹提示 ——
 * 那不是「保存设置」，是内部状态。
 *
 * ⚠️ `ElMessage` **不要显式 `import`** —— 它由 `unplugin-auto-import` 注入。
 * 手写 `import { ElMessage } from 'element-plus'` 会把 element-plus 的
 * barrel 文件拉进依赖图，实测会让产物从 239 kB 涨到 1065 kB
 * （见 `vite.config.ts` 的 `manualChunks` 说明）。
 */

/** 两次提示之间的最小间隔（毫秒） */
const MIN_GAP = 900

/** 上一次真正弹提示的时刻 */
let lastShownAt = 0
/** 尾随提示的定时器 */
let trailing: number | null = null

function show(message: string): void {
  lastShownAt = Date.now()
  ElMessage({
    message,
    type: 'success',
    duration: 1400,
    // 同文案合并：万一还是撞上了，不叠一屏
    grouping: true,
  })
}

/**
 * 提示「已保存」。
 *
 * @param message 文案（默认「已保存」）。给更具体的说法更好，
 *   如「外观已保存」「导出选项已保存」——用户能确认**是哪一项**生效了。
 */
export function notifySaved(message = '已保存'): void {
  const now = Date.now()
  const since = now - lastShownAt

  if (since >= MIN_GAP) {
    if (trailing !== null) {
      window.clearTimeout(trailing)
      trailing = null
    }
    show(message)
    return
  }

  // 太近：安排一次尾随提示（覆盖「最后一次改动」）
  if (trailing !== null) window.clearTimeout(trailing)
  trailing = window.setTimeout(() => {
    trailing = null
    show(message)
  }, MIN_GAP - since)
}

/** 供测试 / 退出前清理 */
export function resetSavedToast(): void {
  if (trailing !== null) {
    window.clearTimeout(trailing)
    trailing = null
  }
  lastShownAt = 0
}
