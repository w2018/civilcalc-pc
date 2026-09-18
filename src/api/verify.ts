/**
 * 组 12c：方程代入验算。
 *
 * 见 docs/08-IPC契约.md §3 组 12。
 */

import { invoke } from './invoke'
import type { EquationCheck } from '@/types/verify'

/**
 * 把原始方程逐条代入当前取值核对。
 *
 * @param sourceEquations `schema.sourceEquations`（用户需求里给出的原始方程）。
 *                        空白项会被跳过，序号按非空项连续编号
 * @param inputs 当前取值（用户填的系数 + 本次算出的结果量）
 *
 * @remarks
 * ⚠️ 返回项的 `holds` 是 **`boolean | null`**：
 * `null` 表示**无法核对**（不是不成立），必须把 `note` 显示出来。
 */
export function verifyEquations(
  sourceEquations: string[],
  inputs: Record<string, number>,
): Promise<EquationCheck[]> {
  return invoke<EquationCheck[]>('verify_equations', { sourceEquations, inputs })
}
