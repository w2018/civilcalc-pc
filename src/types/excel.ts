/**
 * 组 12a：Excel 公式转换 / 校验 / 迁移。
 *
 * 见 docs/08-IPC契约.md §3 组 12 与 docs/06-Excel单元格映射契约.md。
 *
 * ## 两种模式
 *
 * | 模式 | 公式形态 | 用途 |
 * |---|---|---|
 * | `ref` | `=PI()*A1^2*B1` | **贴进 Excel 复用**：变量留在第 1 行，改数值即重算 |
 * | `value` | `=PI()*2^2*3` | **写进计算书**：数值已代入，读者无需回头查参数表 |
 *
 * ## ⚠️ 三个诊断字段（**必须展示**）
 *
 * 后端 `convert` 有两处「静默」行为，会让用户拿到一份看不出来有问题的公式：
 *
 * | 情况 | 后端行为 | 风险 |
 * |---|---|---|
 * | 表达式引用了未声明的符号 | 保留原名（不丢弃、不报错） | Excel 里一个 `#NAME?` |
 * | **值模式下某变量没给数值** | 代入 `0`（**无告警**） | 少填参数 → 结果照算但**是错的** |
 *
 * 所以后端补了三个字段。`unresolvedNames` / `unresolvedRefs` 用红条，
 * `missingInputs` 建议用**黄条显著提示**（漏填参数是工程计算里最容易出事的错误）。
 */

/** 转换模式 */
export type ExcelMode = 'ref' | 'value'

/** 参数对照表的一行：变量 → 单元格 */
export interface ParamCell {
  /** 变量符号（对应 `FormulaSchema.variables[].symbol`） */
  variable: string
  /** 释义（已去 Markdown 标记） */
  desc: string
  /** 单位（可能为空串） */
  unit: string
  /** 单元格引用，如 `A1` / `AA1` */
  cellRef: string
  /** 是否必填（取自变量定义） */
  required: boolean
}

/** 一个多结果的 Excel 视图 */
export interface ExcelOutputView {
  /** 结果符号（分号段的赋值目标） */
  symbol: string
  /** 结果名称 */
  name: string
  /** 按 `mode` 选出的公式 */
  formula: string
}

/** 一个分支的 Excel 视图 */
export interface ExcelBranchView {
  /** 分支标签（重名时带序号后缀） */
  label: string
  /** 按 `mode` 选出的公式 */
  formula: string
  /** 适用条件（原样透传） */
  condition?: string | null
}

/** `excel_convert` 的返回 */
export interface ExcelResult {
  /** 请求的模式（回显） */
  mode: ExcelMode
  /** 主公式（多结果时 = **最后一段**） */
  mainFormula: string
  /** 多结果逐条（单结果时为空数组） */
  outputs: ExcelOutputView[]
  /** 分支逐条（**顺序 = `altExpressions` 顺序**，不是字典序） */
  branches: ExcelBranchView[]
  /** 参数对照表：变量 → 单元格 */
  paramMapping: ParamCell[]
  /** 用到的 Excel 函数（含中文说明） */
  usedFunctions: import('./domain').FunctionDoc[]
  /** 全量告警（编译失败 / 数值非法 …） */
  warnings: string[]

  /** 值模式下仍残留的单元格引用（数值**非法**如 `NaN` 所致） */
  unresolvedRefs: string[]
  /** 公式里残留的、未在参数表中声明的标识符 */
  unresolvedNames: string[]
  /**
   * 值模式下**没提供数值**的变量名（它们在公式里被当成 `0`）。
   *
   * ⚠️ 后端对缺失输入按 `0` 处理且**不告警** —— 漏填参数会让结果照算但错掉。
   * **请用显著样式提示用户。**
   */
  missingInputs: string[]

  /** 变量所在行（契约：第 1 行横向 `A1`/`B1`/…） */
  variableRow: number
  /** 步骤结果起始行（契约：A 列纵向，从第 2 行起） */
  stepStartRow: number
}

/** `excel_validate` 的返回：AI 公式 vs 本地转换的比对结果 */
export interface ValidationResult {
  /** 是否等价 */
  ok: boolean
  /** 不等价时的说明（等价时为 `null`） */
  message?: string | null
  /** 本地转换出的公式（基准） */
  localFormula?: string | null
  /** 被校验的公式 */
  aiFormula?: string | null
  /**
   * 本次比对用的样本输入（排障用：可据此手工复算）。
   *
   * 样本由**固定种子**生成（`0x5EED0000`，复刻 Java LCG），
   * 因此同一 schema 每次校验的样本与结论完全一致。
   */
  sampleInputs: Record<string, number>[]
}
