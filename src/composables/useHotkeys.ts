/**
 * 键盘快捷键 —— 见 docs/05-项目开发方案.md §2.3.4。
 *
 * | 组合 | 作用 | 注册处 |
 * |---|---|---|
 * | `Ctrl+K` | 聚焦全局搜索 | `TopBar` |
 * | `Ctrl+E` | 导出计算书 | `FormulaView` |
 * | `Ctrl+S` | 保存公式 | `FormulaView` |
 * | `Ctrl+Enter` | 强制求值 | `FormulaView` |
 *
 * ## 为什么**不**做成全局注册表
 *
 * 快捷键的作用域就是它所在页面的作用域。`Ctrl+Enter` 在「模型测试」
 * 是「发送」、在「公式工作台」是「求值」—— 如果在 `App.vue` 里注册一张
 * 大表，就得在表里判当前路由，等于把路由逻辑抄进快捷键层。
 *
 * 所以：**谁用谁注册**，组件卸载自动注销。
 *
 * ## 组合键不因焦点在输入框而失效
 *
 * `Ctrl+K` 的意义就是「不管我在哪儿都能搜」；`Ctrl+S` 在 textarea 里
 * 也该保存。所以这里**不**做「焦点在输入框就跳过」的判断。
 * 只有 `Esc` 这种会与输入法/下拉冲突的键才需要调用方自己判断。
 *
 * ## 匹配用 `event.key` 而不是 `keyCode`
 *
 * `keyCode` 已废弃，且不同键盘布局下 `key` 才是用户看到的字符。
 * `key` 统一转小写再比。
 */
import { onBeforeUnmount, onMounted } from 'vue'
import { useIsActive } from './useIsActive'

/** 组合键描述：`ctrl+k` / `ctrl+shift+s` / `esc` */
export type HotkeyCombo = string

interface Parsed {
  ctrl: boolean
  shift: boolean
  alt: boolean
  key: string
}

function parse(combo: HotkeyCombo): Parsed {
  const parts = combo
    .toLowerCase()
    .split('+')
    .map((p) => p.trim())
    .filter(Boolean)
  const key = parts.filter((p) => !['ctrl', 'control', 'shift', 'alt'].includes(p)).join('+')
  return {
    ctrl: parts.includes('ctrl') || parts.includes('control'),
    shift: parts.includes('shift'),
    alt: parts.includes('alt'),
    key,
  }
}

function matches(e: KeyboardEvent, p: Parsed): boolean {
  const ctrl = e.ctrlKey || e.metaKey // macOS 上把 Cmd 也认作 Ctrl
  if (p.ctrl !== ctrl) return false
  if (p.shift !== e.shiftKey) return false
  if (p.alt !== e.altKey) return false
  return e.key.toLowerCase() === p.key
}

/**
 * 注册一组快捷键（组件卸载时自动注销）。
 *
 * @param map 组合键 → 处理函数
 * @param enabled 可选开关；返回 `false` 时整组快捷键不响应
 *   （如「正在生成中，先别响应 Ctrl+S」）
 */
export function useHotkeys(
  map: Record<HotkeyCombo, (e: KeyboardEvent) => void>,
  enabled?: () => boolean,
): void {
  const entries = Object.entries(map).map(([combo, fn]) => [parse(combo), fn] as const)

  /**
   * 🔴 只有当前可见的页面才响应。
   *
   * 被 `keep-alive` 缓存后，切走的页面**没有卸载**，它的 keydown 监听还挂着 ——
   * 不加这道判断时，两个页面的同名快捷键会**同时触发**
   * （见 `composables/useIsActive.ts`）。
   */
  const active = useIsActive()

  function onKey(e: KeyboardEvent): void {
    if (!active.value) return
    if (enabled && !enabled()) return
    for (const [p, fn] of entries) {
      if (!matches(e, p)) continue
      // 交给我们处理了就别让浏览器再走默认行为（Ctrl+S 会弹「保存网页」）
      e.preventDefault()
      fn(e)
      return
    }
  }

  onMounted(() => window.addEventListener('keydown', onKey))
  onBeforeUnmount(() => window.removeEventListener('keydown', onKey))
}
