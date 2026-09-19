/**
 * 数值格式化 —— 见 docs/05-项目开发方案.md §2.3.4。
 *
 * ## 精度规则（按量级自适应）
 *
 * | 绝对值 | 最多小数位 |
 * |---|---|
 * | ≥ 1e6 | 0 |
 * | ≥ 1e3 | 1 |
 * | ≥ 100 | 2 |
 * | ≥ 10 | 3 |
 * | ≥ 1 | 4 |
 * | ≥ 0.01 | 5 |
 * | 其他 | 6 |
 *
 * 这实质是「保留约 4~5 位有效数字」，与工程惯例一致 ——
 * 原始数据本身精度有限，把 `1234.5678` 显示成 `1234.5678` 反而给人
 * 「精确到小数点后 4 位」的错觉。
 *
 * ## 与源项目的两处差异（都是 docs 的 PC 端决策）
 *
 * 1. **不使用千分位**。源项目 `NumberFormat` 用 `#,##0.######`，
 *    会把 `206000` 显示成 `206,000`；docs §2.3.4 明确要求不用
 *    （与导出 docx 一致）。
 * 2. **不用科学计数法表示中等量级**。源项目 `|v| ≥ 1e6` 就转科学计数法
 *    （`206000` → `2.06E5`），工程用户看到这个会愣一下。
 *    这里只在 `≥ 1e12` 或 `< 1e-6` 时才转。
 *
 * ## 绝不返回 `NaN` / `Infinity` 字符串
 *
 * 文档硬约束：「条件不满足显示"不适用"，**禁止 `NaN` 字符串**」。
 * 遇到非有限值时返回 `—`，由调用方补充说明。
 */

/** 非有限值的占位符（禁止 `NaN` 字样出现在界面上） */
export const NOT_A_NUMBER_PLACEHOLDER = '—'

/** 数值不可用时的统一文案（如条件分支不满足） */
export const NOT_APPLICABLE_TEXT = '不适用'

/**
 * 按量级取「最多小数位」。
 *
 * 导出是为了让测试与文档能引用同一份规则，避免两处各写一套。
 */
export function digitsFor(abs: number): number {
  if (!Number.isFinite(abs)) return 0
  if (abs >= 1e6) return 0
  if (abs >= 1e3) return 1
  if (abs >= 100) return 2
  if (abs >= 10) return 3
  if (abs >= 1) return 4
  if (abs >= 0.01) return 5
  return 6
}

/** 去掉尾部多余的 `0` 与孤立的小数点（`"1.2000"` → `"1.2"`） */
function trimTrailingZeros(s: string): string {
  if (!s.includes('.')) return s
  return s.replace(/\.?0+$/, '')
}

/**
 * 格式化数值（**不补零**、**不用千分位**）。
 *
 * ```ts
 * formatNumber(1 / 3)        // "0.3333"（后端已 round 到 4 位）
 * formatNumber(206000)       // "206000"
 * formatNumber(1234.5678)    // "1234.6"（≥1e3 → 最多 1 位）
 * formatNumber(0)            // "0"
 * formatNumber(NaN)          // "—"
 * ```
 */
export function formatNumber(v: number): string {
  if (!Number.isFinite(v)) return NOT_A_NUMBER_PLACEHOLDER
  if (v === 0) return '0'

  const abs = Math.abs(v)

  // 极端量级才转科学计数法（中等量级直接展开，工程用户更习惯）
  if (abs >= 1e12 || abs < 1e-6) {
    return v.toExponential(4)
  }

  return trimTrailingZeros(v.toFixed(digitsFor(abs)))
}

/**
 * 格式化数值 + 单位。
 *
 * 单位为空 / 空白时只返回数值（不留下尾随空格）。
 */
export function formatWithUnit(v: number, unit?: string | null): string {
  const n = formatNumber(v)
  const u = unit?.trim()
  return u ? `${n} ${u}` : n
}

/**
 * 数值的**全精度**字符串（用于 `title` 提示）。
 *
 * 展示层为了控制宽度会截断小数位，hover 时给用户看到真实值 ——
 * 否则用户按显示值复算会对不上。
 */
export function fullPrecision(v: number): string {
  if (!Number.isFinite(v)) return NOT_A_NUMBER_PLACEHOLDER
  return String(v)
}

/**
 * token 数 → 紧凑文案（`12345` → `12.3k`）。
 *
 * 需求：思考框与模型测试页要显示「本次消耗的 token 数（如 xxxk）」。
 * 用 `k` 而不是千分位，是因为 token 数常常五位数以上，
 * 展开写会把标题行挤爆；`k` 是这一行的通用写法。
 *
 * | 输入 | 输出 |
 * |---|---|
 * | `0` | `0` |
 * | `980` | `980` |
 * | `1000` | `1k` |
 * | `12345` | `12.3k` |
 * | `1234567` | `1.2M` |
 *
 * 负数 / 非有限值 → `0`（token 不可能是负的，出现即视为无数据）。
 */
export function formatTokens(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '0'
  if (n < 1000) return String(Math.round(n))
  if (n < 1_000_000) {
    const k = n / 1000
    // 10k 以上不再保留小数：`123.4k` 比 `123k` 更长却没多信息
    return k >= 100 ? `${Math.round(k)}k` : `${trimTrailingZeros(k.toFixed(1))}k`
  }
  const m = n / 1_000_000
  return m >= 100 ? `${Math.round(m)}M` : `${trimTrailingZeros(m.toFixed(1))}M`
}

/** 补零到两位 */
function pad2(n: number): string {
  return n < 10 ? `0${n}` : String(n)
}

/**
 * 时间戳（**Unix 毫秒**）→ 统一的 `YYYY-MM-DD HH:mm:ss`。
 *
 * ## 🔴 一律完整到秒，不做「今天 / 今年」的省略
 *
 * 曾经这里（以及好几个组件各自抄了一份的时间格式化）会按「距今多远」省略：
 * 今天只给 `14:30`、更早只给日期。问题是**看不出具体时刻** ——
 * 用户对不上「到底是哪一次操作」，而且同一份数据在不同页面显示的精度还不一样。
 * 现在全应用统一成完整年月日时分秒。
 *
 * ## ⚠️ 入参是**毫秒**
 *
 * 后端时间戳来自 `civilcalc_core::now_ms()` —— `FormulaSchema.createdAt`、
 * `FormulaVersion.createdAt`、`HistoryEntry.createdAt` 全是毫秒。
 * 曾经 `VersionView` 按「秒」处理、多乘了一次 1000，界面上直接显示成
 * `58684/11/23`。别再犯。
 */
export function formatDateTime(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return '—'

  const d = new Date(ms)
  if (Number.isNaN(d.getTime())) return '—'

  return (
    `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}` +
    ` ${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`
  )
}
