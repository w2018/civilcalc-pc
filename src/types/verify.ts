/**
 * 组 12c：方程代入验算。
 *
 * 见 docs/08-IPC契约.md §3 组 12。
 *
 * ## 🔵 验算不经模型
 *
 * 全部由本地表达式引擎完成 —— 结论可以直接当依据用。
 * 这是本项目与「让 AI 判断答案对不对」的根本区别。
 *
 * ## 🔴 三种结果，不要混淆
 *
 * | `holds` | 含义 | 建议表现 |
 * |---|---|---|
 * | `true` | 两侧数值相等（按 4 位小数容差） | ✓ 绿色 |
 * | `false` | 明确不相等 | ✗ 红色 |
 * | `null` | **无法核对**（不是不成立） | 灰色 + 显示 `note` |
 *
 * `null` 的三种来源：不是完整等式、缺取值、两侧无法解析。
 * **绝不静默丢掉一条方程** —— 宁可显示「未核对」。
 */

/** 一条原始方程的代入验算结果 */
export interface EquationCheck {
  /** 序号，从 1 起（空白项不占号） */
  index: number
  /** 圈号标记（`①`/`②`/…），与原始方程的给出顺序对应 */
  mark: string
  /** 原始方程原文（已 trim） */
  source: string
  /** 代入当前取值后的左侧文本（如 `2*1-2+3`） */
  leftText: string
  /** 代入当前取值后的右侧文本 */
  rightText: string
  /** 两侧数值是否相等；`null` = 无法核对 */
  holds: boolean | null
  /** 无法核对的原因（`holds === null` 时有值） */
  note?: string | null
}

/** 是否已得出明确结论（成立或不成立） */
export function isChecked(check: EquationCheck): boolean {
  return check.holds !== null
}

/**
 * 一句话总结核对情况（列表头部用）。
 *
 * 三种状态分开数 —— 「无法核对」不能混进「不成立」，
 * 否则用户会以为自己的解错了。
 */
export function summarizeChecks(checks: EquationCheck[]): {
  total: number
  passed: number
  failed: number
  unchecked: number
} {
  let passed = 0
  let failed = 0
  let unchecked = 0
  for (const c of checks) {
    if (c.holds === true) passed++
    else if (c.holds === false) failed++
    else unchecked++
  }
  return { total: checks.length, passed, failed, unchecked }
}
