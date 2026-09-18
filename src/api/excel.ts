/**
 * 组 12a：Excel 公式转换 / 校验 / 迁移。
 *
 * 见 docs/08-IPC契约.md §3 组 12 与 docs/06-Excel单元格映射契约.md。
 *
 * ## 两个命令都是**纯函数**
 *
 * 不碰数据库、不碰偏好 —— `excelMigrate` 只返回迁移结果，**不落库**。
 * 落库请调 `formulaApi.formulaSave`，避免一个操作改两处状态。
 */

import { invoke } from './invoke'
import type { FormulaSchema } from '@/types/domain'
import type { ExcelMode, ExcelResult, ValidationResult } from '@/types/excel'

/**
 * 转换公式为 Excel 公式。
 *
 * @param mode `'ref'` 引用式（`=PI()*A1^2*B1`）/ `'value'` 带数值式（`=PI()*2^2*3`）
 * @param inputs 变量值。**`ref` 模式下被忽略**（引用式不含数值）
 *
 * @remarks
 * 返回值里的 `missingInputs` **必须展示** —— 后端对缺输入按 `0` 代入且不告警，
 * 漏填参数会让公式照算但错掉。
 */
export function excelConvert(
  schema: FormulaSchema,
  mode: ExcelMode,
  inputs: Record<string, number> = {},
): Promise<ExcelResult> {
  return invoke<ExcelResult>('excel_convert', { schema, mode, inputs })
}

/**
 * 校验一份 Excel 公式是否与本地转换结果等价。
 *
 * `excelFormula` 会**覆盖** `schema.excelExpression` ——
 * 因此可以「先校验再决定要不要存」，不必先把公式写进 schema。
 *
 * 后端用 3 组固定种子样本逐组比对相对误差（阈值 `1e-9`），
 * 同一 schema 每次结论完全一致（无随机误报）。
 */
export function excelValidate(
  schema: FormulaSchema,
  excelFormula: string,
): Promise<ValidationResult> {
  return invoke<ValidationResult>('excel_validate', { schema, excelFormula })
}

/**
 * 把公式的 Excel 字段迁移到当前契约版本（v3）。
 *
 * ⚠️ **只返回结果，不落库**。要保存请接 `formulaApi.formulaSave`。
 *
 * 已经是当前版本的公式**原样返回**（不覆盖手工修正过的字段），
 * 所以可以安全地对任意公式调用。
 */
export function excelMigrate(schema: FormulaSchema): Promise<FormulaSchema> {
  return invoke<FormulaSchema>('excel_migrate', { schema })
}
