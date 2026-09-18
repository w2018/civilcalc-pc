/**
 * 组 12b：二维数学排版。
 *
 * 见 docs/08-IPC契约.md §3 组 12 与 ADR-019。
 *
 * ## 为什么是三个函数
 *
 * 源项目生产代码正好只用这三个 API（`FormulaScreen.kt:1205/1211/1218`），
 * 三者**入参形态与失败语义都不同** —— 硬合成一个会让参数变成
 * 「按 mode 走不同字段」的联合体，更难用。
 */

import { invoke } from './invoke'
import type { MathLayout, MathNode, ResultPair } from '@/types/math'

/**
 * 把整条公式（含分号分段）排成二维布局树。
 *
 * 多结果公式会排成多行，每行的 `symbol` 是分号段的赋值目标。
 *
 * ⚠️ **必须检查返回值**：排版失败时 `lines` 为空、`warnings` 非空 ——
 * 此时要回落成等宽原文展示（不要显示空白）。
 * 用 {@link isUsable} 判。
 *
 * @param constants schema 自定义常量（如 `{ alpha: 1.5 }`）；
 *                  不传时只用内置的 `pi` / `e`。**传了才能让 `alpha*b` 里的
 *                  `alpha` 被识别为常量而不是变量。**
 */
export function mathLayout(
  expression: string,
  constants?: Record<string, number>,
): Promise<MathLayout> {
  return invoke<MathLayout>('math_layout', { expression, constants })
}

/**
 * 把用户写的**条件方程**（`x+y+z=6`）排成 `左侧 = 右侧`。
 *
 * ⚠️ 等式不是赋值语句（`=` 不在开头），所以后端按**第一个** `=` 切两半再排版。
 *
 * @returns 无法排版时返回 `null`（无 `=`、`=` 在首尾、任一侧解析失败）
 *          —— **此时回落原文展示**
 */
export function mathLayoutEquation(
  equation: string,
  constants?: Record<string, number>,
): Promise<MathNode[] | null> {
  return invoke<MathNode[] | null>('math_layout_equation', { equation, constants })
}

/**
 * 结果行：多结果 `(x, y, z) = (1, 2, 3)`；单结果不带括号 `V = 31.4159`。
 *
 * @param results 数值文本**由调用方格式化**（排版层不管精度口径）
 */
export function mathLayoutResultLine(results: ResultPair[]): Promise<MathNode[]> {
  return invoke<MathNode[]>('math_layout_result_line', { results })
}
