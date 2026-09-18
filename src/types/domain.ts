/**
 * 领域类型 —— 与 Rust `civilcalc_core::schema` 一一对应。
 *
 * 手写（ADR-015），不引第三方生成器；由契约测试（P5-8）保证一致。
 *
 * 见 docs/08-IPC契约.md §2 与 docs/04-数据契约.md §1/§2。
 *
 * ⚠️ 所有字段名是 **camelCase** —— Rust 侧统一加了
 * `#[serde(rename_all = "camelCase")]`，且带结构变体的枚举还加了
 * `rename_all_fields`（否则多词字段会漏成 snake_case）。
 */

// =============================================================================
// 公式结构
// =============================================================================

/** 来源类别。SCREAMING_SNAKE，与源项目 Kotlin 枚举名一致 */
export type SourceKind = 'STANDARD' | 'AI' | 'CUSTOM' | 'DERIVED'

export interface FormulaSource {
  kind: SourceKind
  /**
   * 规范条款引用，如 `"GB 50010-2010 第6.2.10条"`。
   *
   * ⚠️ JSON 键名是 `ref`（Rust 字段叫 `ref_`，靠 `#[serde(rename = "ref")]` 对齐）
   */
  ref?: string | null
  verified: boolean
  /** AI 生成时用的模型名 */
  model?: string | null
  createdBy?: string | null
}

export interface FormulaVar {
  symbol: string
  /** 中文释义，**必填**（校验器会拒绝空 desc） */
  desc: string
  /** 单位；无量纲可省略。`verified` 的公式要求非空（可为空串） */
  unit?: string | null
  default?: number | null
  min?: number | null
  max?: number | null
  required: boolean
}

export interface AltExpression {
  label: string
  expression: string
  /** 分支条件，如 `"a>b"`；无条件则省略 */
  condition?: string | null
}

export interface ResultOutput {
  symbol: string
  /** 展示名 */
  name: string
  unit?: string | null
}

export interface StepTemplate {
  symbol: string
  label: string
  group?: string | null
  expression: string
  unit: string
  note?: string | null
}

export interface ExplanationStep {
  title: string
  detail: string
  expression?: string | null
}

/** AI 生成的公式详解（理解需求 / 解决方式 / 分步依据） */
export interface FormulaExplanation {
  summary: string
  solution: string
  steps: ExplanationStep[]
}

/**
 * Excel 函数说明（`excel_convert` / `schema.excelFunctionDocs` 共用）。
 *
 * ⚠️ 字段与 Rust `civilcalc_core::schema::FunctionDoc` **逐一对应**
 * （`#[serde(rename_all = "camelCase")]`）。
 * 之前这里误写成 `name` / `desc` / `params`，与后端完全对不上 —— 已修正。
 */
export interface FunctionDoc {
  /** 本引擎中的函数名，如 `sqrt` */
  name: string
  /**
   * 对应的 Excel 函数名。
   *
   * ⚠️ 不一定同名：`cbrt` → `POWER(x,1/3)`、`hypot` → `SQRT(x^2+y^2)`
   * （Excel 无对应函数，展开成等价形态）。
   */
  excelName: string
  /** 中文说明 */
  description: string
  /** 调用语法，如 `SQRT(number)` */
  syntax: string
}

/**
 * 公式的完整结构（**27 个字段**）。
 *
 * ⚠️ 新公式必须写 `schemaVersion: 3`，否则会触发多余的 Excel 字段迁移。
 */
export interface FormulaSchema {
  id: string
  resultName: string
  resultSymbol: string
  resultUnit?: string | null
  resultOutputs: ResultOutput[]
  /** 主表达式。支持分号分段与赋值前缀（如 `"X1 = a+b; X2 = X1*c"`） */
  expression: string
  /** 来源方程（供验算逐条核对） */
  sourceEquations: string[]
  altExpressions: AltExpression[]
  /** 常量表，如 `{ E: 206000 }` */
  constants: Record<string, number>
  variables: FormulaVar[]
  /** 领域，如 `"结构"` / `"施工"` / `"预算"` */
  domain: string
  tags: string[]
  /** 设计依据 */
  referenceBasis?: string | null
  /** 设计说明 */
  designNotes?: string | null
  explanation?: FormulaExplanation | null
  /** 内联图 id；正文里以 `{{img:N}}` 引用 */
  imageIds: string[]
  /** 派生自哪个公式（续写微调时记录） */
  revisedFrom?: string | null
  source: FormulaSource
  stepsTemplate?: StepTemplate[] | null
  docTemplateId?: string | null
  excelExpression?: string | null
  excelAltExpressions?: AltExpression[] | null
  excelStepsTemplate?: StepTemplate[] | null
  excelFunctionDocs?: FunctionDoc[] | null
  /** ⚠️ 新公式写 3 */
  schemaVersion: number
  createdAt: number
  updatedAt: number
}

// =============================================================================
// 求值结果
// =============================================================================

export interface EvalOutput {
  /** 该分号段的赋值目标（如 `"X2"`），可空 */
  symbol?: string | null
  value: number
}

export interface StepResult {
  symbol: string
  label: string
  group?: string | null
  value: number
  unit: string
  expression: string
  /** 完整代入式，如 `"ξ = x / h0 = 450 / 560 = 0.804"` */
  substitutedExpression: string
  /** **单元格引用式** Excel 公式（如 `"=B2/B3"`） */
  excelFormula: string
  /** 带数值的 Excel 公式 */
  excelValueFormula?: string | null
}

export interface EvalBranch {
  label: string
  value?: number | null
  /** 条件是否成立。**不成立时不得用 NaN 字符串替代** */
  applicable: boolean
  /**
   * 分支条件，如 `"b²-4ac ≥ 0"`。
   *
   * ⚠️ `applicable = false` 时**必须把它当作「原因」展示出来** ——
   * 只显示「不适用」而不说为什么，用户会以为是程序坏了。
   */
  condition?: string | null
}

/**
 * 求值过程的一步（引擎逐步记录）。
 *
 * 与 [`StepResult`] 的区别：这个是**引擎内部**的逐步记录（含错误），
 * `StepResult` 是给「分步展示」用的成品（含 Excel 公式与单位）。
 */
export interface EvalStep {
  /** 步序号（从 1 开始） */
  step: number
  /** 这一步在做什么 */
  description: string
  /** 原始公式 */
  formula: string
  /** 代入数值后的公式 */
  substitutedFormula: string
  result?: number | null
  /** 该步的错误（有值时 `result` 通常为空） */
  error?: string | null
}

/**
 * 求值错误（**可判别联合**，与 Rust `EvalError` 的 `#[serde(tag = "kind")]` 一一对应）。
 *
 * 三个变体的字段**各不相同**（不是同一个 `msg`）：
 * - `domain` 带 `symbol` + `detail`
 * - `timeout` 带 `ms`
 * - `compile` 带 `position` + `msg`
 */
export type EvalError =
  | { kind: 'domain'; symbol: string; detail: string }
  | { kind: 'timeout'; ms: number }
  | { kind: 'compile'; position?: number | null; msg: string }

/** 求值错误 → 用户可读文案 */
export function evalErrorMessage(e: EvalError): string {
  switch (e.kind) {
    case 'domain':
      return `${e.symbol}: ${e.detail}`
    case 'timeout':
      return `计算超时（${e.ms} ms）`
    case 'compile':
      return e.msg
  }
}

/** 取错误关联的变量名（用于把对应参数框标红并聚焦） */
export function evalErrorSymbol(e: EvalError | null | undefined): string | null {
  return e?.kind === 'domain' ? e.symbol : null
}

export interface EvalResult {
  /** 主结果。多输出时 = **最后一段** */
  primary: number
  /** 多输出：每个分号段一个；单输出公式为空数组 */
  outputs: EvalOutput[]
  branches: EvalBranch[]
  steps: EvalStep[]
  stepResults: StepResult[]
  /** 非致命提示（如条件分支不满足） */
  warnings: string[]
}

// =============================================================================
// 历史 / 版本
// =============================================================================

/**
 * 一条计算历史。
 *
 * ⚠️ 三个 JSON 字段是**字符串**（与备份包同形，保证逐字节保真）。
 * 需要结构体时自行 `JSON.parse`，并注意：
 * - `inputsJson` 可能是 `"{}"` 或**空串**
 * - `resultJson` 可能是**空串**（AI 刚解析完、尚未计算）
 */
export interface HistoryEntry {
  id: number
  formulaId: string
  /** 公式快照 JSON */
  formulaSnapshotJson: string
  /** 输入值 JSON */
  inputsJson: string
  /** 结果 JSON */
  resultJson: string
  thinkingContent?: string | null
  createdAt: number
}

/** 解析 `HistoryEntry.inputsJson`（容忍空串与坏数据） */
export function parseHistoryInputs(e: HistoryEntry): Record<string, number> {
  const s = e.inputsJson?.trim()
  if (!s) return {}
  try {
    return JSON.parse(s) as Record<string, number>
  } catch {
    return {}
  }
}

/** 解析 `HistoryEntry.resultJson`（容忍空串与坏数据） */
export function parseHistoryResult(e: HistoryEntry): EvalResult | null {
  const s = e.resultJson?.trim()
  if (!s || s === 'null') return null
  try {
    return JSON.parse(s) as EvalResult
  } catch {
    return null
  }
}

/**
 * 解析 `HistoryEntry.formulaSnapshotJson`（容忍空串与坏数据）。
 *
 * 历史列表要显示**当时的**公式名 —— 公式可能已被改名或删除，
 * 用快照里的名字才反映「那一次算的是什么」。
 */
export function parseHistorySnapshot(e: HistoryEntry): FormulaSchema | null {
  const s = e.formulaSnapshotJson?.trim()
  if (!s || s === 'null') return null
  try {
    return JSON.parse(s) as FormulaSchema
  } catch {
    return null
  }
}

/** 历史条目的显示名（快照名 → 回落「已删除的公式」） */
export function historyDisplayName(e: HistoryEntry): string {
  const snap = parseHistorySnapshot(e)
  const name = snap?.resultName?.trim()
  return name && name.length > 0 ? name : '（公式已删除）'
}

export interface FormulaVersion {
  formulaId: string
  version: string
  parentVersion?: string | null
  /** 快照 JSON（字符串） */
  schemaJson: string
  /** `create` / `edit` / `refine` / `verify` / `revert` */
  changeType: string
  changeLog: string
  editor: string
  createdAt: number
  /**
   * ⚠️ **0/1 而不是 boolean**
   *
   * 源项目 `FormulaVersionRow.verified: Int`，备份包是跨端契约，不能改成布尔。
   */
  verified: number
}

export interface DiffItem {
  /** 字段路径，如 `"source.ref"` */
  field: string
  /** 中文标签，如 `"来源引用"` */
  label: string
  oldValue: string
  newValue: string
}

/**
 * 两版之间的 6 维差异。
 *
 * 维度顺序固定：
 * `expression` → `altExpressions` → `variables` → `source.ref` →
 * `source.verified` → `constants`
 */
export interface VersionDiff {
  changes: DiffItem[]
}

// =============================================================================
// 检索
// =============================================================================

/** 轻量建议（`search_suggest` 用，减少 IPC 负载） */
export interface SearchSuggestion {
  id: string
  resultName: string
  domain: string
  sourceKind: SourceKind
}

// =============================================================================
// 展示辅助
// =============================================================================

/** 变量单位后缀（无量纲返回空串） */
export function unitSuffix(unit?: string | null): string {
  return unit ? ` ${unit}` : ''
}

/** 来源类别 → 中文标签 */
export function sourceKindLabel(kind: SourceKind): string {
  switch (kind) {
    case 'STANDARD':
      return '规范'
    case 'AI':
      return 'AI 生成'
    case 'CUSTOM':
      return '自定义'
    case 'DERIVED':
      return '派生'
  }
}
