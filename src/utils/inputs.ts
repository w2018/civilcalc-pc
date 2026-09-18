/**
 * 把用户原始输入（字符串，可为算式）解析成数值表。
 *
 * 用于 Excel 转换与计算书导出：这两处都需要 `Record<string, number>`，
 * 而 `store.paramValues` 是**原始字符串**（`"12*3"` 这种算式也允许）。
 *
 * 解析规则（与 `stores/formula.ts` 的 `parseParams` 一致）：
 * - 空值跳过（不进 inputs，后端按缺失处理）
 * - 算式交 `evalExpression` 求值
 * - 失败/非有限值跳过
 */
import type { FormulaSchema } from '@/types/domain'
import { evalExpression } from '@/composables/useEval'

export async function toNumericInputs(
  schema: FormulaSchema,
  paramValues: Record<string, string>,
): Promise<Record<string, number>> {
  const out: Record<string, number> = {}
  for (const v of schema.variables) {
    const raw = (paramValues[v.symbol] ?? '').trim()
    if (!raw) continue
    const value = await evalExpression(raw)
    if (value !== null && Number.isFinite(value)) out[v.symbol] = value
  }
  return out
}
