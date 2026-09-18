/**
 * 组 4：求值与校验。
 *
 * 见 docs/08-IPC契约.md §3 组 4。
 *
 * ## ⚠️ 两个陷阱（展示结果时必读）
 *
 * 1. **结果统一保留 4 位小数**（源项目 `roundFinal`）——
 *    不要自己再 `toFixed` 成别的位数，否则与 Android 端显示不一致
 * 2. **常量名不区分大小写**：`E` 会命中内置**欧拉数** `e`。
 *    把弹性模量命名为 `E` 会静默算错，建议改用 `Es`
 */

import { invoke } from './invoke'
import type { EvalResult, FormulaSchema, StepResult } from '@/types/domain'

/**
 * **按 Schema 直接求值**（编辑中/未保存的实时预览）。
 *
 * **不写历史** —— 需要落历史请用 {@link evalFormula}。
 */
export function evalSchema(
  schema: FormulaSchema,
  inputs: Record<string, number>,
): Promise<EvalResult> {
  return invoke<EvalResult>('eval_schema', { schema, inputs })
}

/**
 * **按 ID 求值**（查库取 Schema），并**写一条计算历史**。
 *
 * 这是用户点「计算」时该调的命令。
 */
export function evalFormula(
  formulaId: string,
  inputs: Record<string, number>,
  thinkingContent?: string,
): Promise<EvalResult> {
  return invoke<EvalResult>('eval_formula', { formulaId, inputs, thinkingContent })
}

/**
 * 校验 Schema（12 类规则）。通过返回 `void`，失败抛 `validation`。
 *
 * ⚠️ 命令名带 `_cmd` 后缀（Rust 侧避免与 `civilcalc_core::schema::validate_schema` 重名）。
 */
export function validateSchemaCmd(schema: FormulaSchema): Promise<void> {
  return invoke<void>('validate_schema_cmd', { schema })
}

/**
 * **分步求值**（查库取 Schema）。
 *
 * 返回逐步骤结果（含代入公式与 Excel 公式），供分步面板与计算书使用。
 *
 * ## 与 {@link evalFormula} 是**并列**关系
 *
 * 源项目在「点计算」时同时调两者（`FormulaViewModel.kt:375-377`），
 * 前端拿两份结果。所以**两个都要调**，不是一个包含另一个。
 *
 * ## 没有分步模板时返回空数组（不是错误）
 *
 * 大多数公式没有 `stepsTemplate` —— 空数组让前端直接隐藏分步区。
 *
 * ## 任一步骤失败 → 整体失败
 *
 * 不返回半截结果：读者看到前两步而第三步缺失，会以为公式本来就只有两步。
 */
export function evalSteps(
  formulaId: string,
  inputs: Record<string, number>,
): Promise<StepResult[]> {
  return invoke<StepResult[]>('eval_steps', { formulaId, inputs })
}
