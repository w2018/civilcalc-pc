//! `FormulaSchema` 与配套领域类型。
//!
//! 源：`civilcalc-android-v2/core/schema/FormulaSchema.kt`
//!
//! ## 移植说明
//!
//! - **字段 1:1 映射**，含全部 27 个字段与默认值
//! - Kotlin `@Serializable` 的默认值 → Rust `#[serde(default)]`
//!   （Kotlin 缺字段用默认值，Rust 默认会报错，必须显式标注）
//! - `FormulaSource.ref_` 加 `#[serde(rename = "ref")]`：
//!   Rust 中 `ref` 是关键字，但**序列化后必须与源项目 Kotlin / 前端 TS 一致**
//!
//! ## 与源项目的一处刻意偏差
//!
//! 源项目 `FunctionDoc` 定义在 `core/formula/export/ExcelFormulaConverter.kt`，
//! 但被 `FormulaSchema.excelFunctionDocs` 引用（Kotlin 允许跨包自由引用）。
//! Rust 中若把 `FunctionDoc` 放在 `excel/` 会造成
//! `excel → engine → schema` 与 `schema → excel` 的**循环依赖**。
//! 因此本模块定义 `FunctionDoc`，由 `excel/` 复用。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 当前时间（Unix 毫秒）—— 对齐 Kotlin `System.currentTimeMillis()`
fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 公式来源类型。
///
/// 序列化为 `STANDARD` / `AI` / `CUSTOM` / `DERIVED`（对齐源项目 Kotlin 枚举名）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SourceKind {
    /// 🔵 标准：内置/规范，**必有 `ref`** 且 `verified = true`
    Standard,
    /// 🟣 AI 生成：**`verified` 恒为 false，`ref` 恒为 null**（AI 不得自填权威出处）
    Ai,
    /// ⚪ 自定义
    Custom,
    /// 🟢 派生
    Derived,
}

/// 公式来源信息。
///
/// ## 红线（源项目最高优先级约束）
///
/// - `verified = true` 时 `ref` **必须非空且命中 21 项规范白名单**
/// - `kind = Ai` 时**不得** `verified = true`，且**不得自填 `ref`**
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormulaSource {
    pub kind: SourceKind,
    /// 精确出处（规范条款 / 手册章节页码 / URL）；无法确认 = `null`
    #[serde(default, rename = "ref")]
    pub ref_: Option<String>,
    /// 内置 / 经人工核验 = true；AI 产出一律 false
    #[serde(default)]
    pub verified: bool,
    /// AI 来源时记录模型标识
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub created_by: Option<String>,
}

impl FormulaSource {
    /// 标准来源（内置/规范）
    pub fn standard(ref_: impl Into<String>) -> Self {
        Self {
            kind: SourceKind::Standard,
            ref_: Some(ref_.into()),
            verified: true,
            model: None,
            created_by: None,
        }
    }

    /// AI 来源 —— **强制 `verified = false` 且 `ref = None`**
    pub fn ai(model: Option<String>) -> Self {
        Self {
            kind: SourceKind::Ai,
            ref_: None,
            verified: false,
            model,
            created_by: None,
        }
    }

    /// 自定义来源
    pub fn custom() -> Self {
        Self {
            kind: SourceKind::Custom,
            ref_: None,
            verified: false,
            model: None,
            created_by: None,
        }
    }
}

/// 变量定义。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormulaVar {
    pub symbol: String,
    /// 中文释义，**必填**（对齐源项目校验：`desc` 不得为空）
    pub desc: String,
    /// 单位；`verified = true` 的公式**必填**（无量纲系数可留空字符串）
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub default: Option<f64>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

/// 结果的展示名兜底链：步骤 label → 符号本身。
///
/// 源：`ResultOutputs.displayName`
///
/// 结果区、Excel 逐条公式、DOCX 计算书共用同一口径 —— 三处各写一份
/// 兜底逻辑迟早会不一致。
pub fn result_display_name(schema: &FormulaSchema, symbol: &str) -> String {
    schema
        .steps_template
        .as_ref()
        .and_then(|steps| steps.iter().find(|s| s.symbol == symbol))
        .map(|s| s.label.clone())
        .unwrap_or_else(|| symbol.to_string())
}

/// 结果的单位兜底链：步骤 unit → 公式级 `resultUnit`。
pub fn result_unit_of(schema: &FormulaSchema, symbol: &str) -> String {
    schema
        .steps_template
        .as_ref()
        .and_then(|steps| steps.iter().find(|s| s.symbol == symbol))
        .map(|s| s.unit.clone())
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| schema.result_unit.clone().unwrap_or_default())
}

/// 去掉 `desc` 中的行内 Markdown 标记（`**`、`==`、`` ` ``）保留内容。
///
/// 源：`FormulaSchema.kt` 的 `FormulaVar.plainDesc()`
///
/// 供**纯文本出口**使用：参数输入标签、Excel 映射表、DOCX 计算书、搜索索引等
/// —— 这些位置不该出现 Markdown 标记。
pub fn plain_desc(var: &FormulaVar) -> String {
    var.desc
        .replace("**", "")
        .replace("==", "")
        .replace('`', "")
}

/// 备选表达式（多结果分支）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AltExpression {
    pub label: String,
    pub expression: String,
    /// 可选适用条件，由引擎判定（不满足时标"不适用"，**不用 NaN 替代**）
    #[serde(default)]
    pub condition: Option<String>,
}

/// 并列多输出的一个结果。
///
/// `symbol` 对应 `expression` 中某分号段的赋值目标符号。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultOutput {
    pub symbol: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub unit: String,
}

/// 计算公式详解（AI 分析：理解需求 / 解决方式 / 分步依据）。
///
/// 三个字段都有 serde 默认值，因此 `Default` 的语义是「**无详解内容**」——
/// [`crate::schema::FormulaExplanation::default`] 与「`explanation` 为 `None`」
/// 在业务上等价（见 `civilcalc-report` 的 `has_explanation_content`）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormulaExplanation {
    /// 对用户需求的理解与总体计算思路
    #[serde(default)]
    pub summary: String,
    /// 对应需求理解的解决方式
    #[serde(default)]
    pub solution: String,
    /// 逐项拆解（依据 + 预期结果）
    #[serde(default)]
    pub steps: Vec<ExplanationStep>,
}

/// 详解里的一个步骤。
///
/// `title` 是唯一的必填项（源项目 Kotlin data class 亦然）——
/// 没有标题的步骤无法在计算书里定位。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplanationStep {
    pub title: String,
    #[serde(default)]
    pub expression: Option<String>,
    #[serde(default)]
    pub detail: String,
}

/// 分步计算模板：每步一个有序步骤，后步可引用前步 `symbol`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepTemplate {
    pub symbol: String,
    pub label: String,
    #[serde(default)]
    pub group: Option<String>,
    pub expression: String,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// Excel 函数说明（供 Excel 面板展示"本公式用到了哪些函数"）。
///
/// 源：`core/formula/export/ExcelFormulaConverter.kt` 的 `FunctionDoc`
/// （见本模块文档中"与源项目的一处刻意偏差"）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionDoc {
    /// 本引擎中的函数名，如 `sqrt`
    pub name: String,
    /// 对应 Excel 函数名，如 `SQRT`
    pub excel_name: String,
    pub description: String,
    pub syntax: String,
}

/// 公式 Schema —— 贯穿全系统的核心数据结构。
///
/// Kotlin DTO、SQLite JSON 列、TS 类型、UI State 四层均以此为准。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormulaSchema {
    /// 稳定 ID：`builtin:xxx` / `usr:uuid`
    pub id: String,
    /// "=" 左侧中文语义
    pub result_name: String,
    /// 原符号（V）
    pub result_symbol: String,
    /// 结果单位
    #[serde(default)]
    pub result_unit: Option<String>,

    /// 并列多输出的展示清单（如方程组的 x、y）
    ///
    /// `symbol` 须与 `expression` 各分号段的赋值目标一致；数组顺序即展示顺序；
    /// 未列入的段视为中间量不展示；单结果公式留空（缺省即旧行为）。
    #[serde(default)]
    pub result_outputs: Vec<ResultOutput>,

    /// 主表达式。支持分号分段 + ASCII 赋值前缀：
    /// `X = 2*a; Y = X + b; Z = Y^2`
    pub expression: String,

    /// 用户需求里给出的原始方程 / 条件式（AI 原样抄录，一条一项，如 `"x+y+z=6"`）。
    ///
    /// 仅用于公式页回显与代入验算，**不参与计算**；需求未给方程的公式留空。
    #[serde(default)]
    pub source_equations: Vec<String>,

    #[serde(default)]
    pub alt_expressions: Vec<AltExpression>,

    #[serde(default)]
    pub constants: HashMap<String, f64>,

    #[serde(default)]
    pub variables: Vec<FormulaVar>,

    /// 适用领域 / 规范版本
    #[serde(default)]
    pub domain: String,

    #[serde(default)]
    pub tags: Vec<String>,

    #[serde(default)]
    pub reference_basis: Option<String>,

    /// AI 自归纳的公式设计要点（供续写时拼接上下文）
    #[serde(default)]
    pub design_notes: Option<String>,

    /// AI 计算公式详解
    #[serde(default)]
    pub explanation: Option<FormulaExplanation>,

    /// 本次生成所依据的附图（`LlmImageStore` 中的图片 ID）。
    ///
    /// 顺序 = 发送顺序，与详解里的 `{{img:N}}` 序号一一对应。
    #[serde(default)]
    pub image_ids: Vec<String>,

    /// 本公式由哪个公式微调而来（续写谱系）
    #[serde(default)]
    pub revised_from: Option<String>,

    pub source: FormulaSource,

    #[serde(default)]
    pub steps_template: Option<Vec<StepTemplate>>,

    #[serde(default)]
    pub doc_template_id: Option<String>,

    // ===== AI 生成的 Excel 公式（单元格引用 A1/B2...，函数映射到 Excel 原生）=====
    #[serde(default)]
    pub excel_expression: Option<String>,
    #[serde(default)]
    pub excel_alt_expressions: Option<Vec<AltExpression>>,
    #[serde(default)]
    pub excel_steps_template: Option<Vec<StepTemplate>>,
    #[serde(default)]
    pub excel_function_docs: Option<Vec<FunctionDoc>>,

    /// Schema 版本：
    /// - `1` = 旧版无 AI Excel
    /// - `2` = 含 AI 生成 Excel
    /// - **`3` = 第 1 行横向单元格契约（当前）**
    ///
    /// ⚠️ 源项目默认值是 `1`，但迁移触发条件为 `< 3`。
    /// **PC 端新建公式必须写入当前契约版本（3）**，否则会触发不必要的迁移。
    #[serde(default = "default_schema_version")]
    pub schema_version: i32,

    #[serde(default = "now_ms")]
    pub created_at: i64,
    #[serde(default = "now_ms")]
    pub updated_at: i64,
}

/// 当前 Excel 契约版本（见 `docs/06-Excel单元格映射契约.md`）
pub const CURRENT_SCHEMA_VERSION: i32 = 3;

fn default_schema_version() -> i32 {
    CURRENT_SCHEMA_VERSION
}

impl FormulaSchema {
    /// 是否为多结果公式（有并列输出清单）
    pub fn is_multi_result(&self) -> bool {
        !self.result_outputs.is_empty()
    }

    /// 是否含可导出的「详解」内容（需求理解 / 解决方式 / 分步依据任一非空）。
    ///
    /// 源：`core/report/ExportOptions.kt` 的 `FormulaSchema.hasExplanationContent()`
    pub fn has_explanation_content(&self) -> bool {
        let Some(e) = &self.explanation else {
            return false;
        };
        !e.summary.is_empty()
            || !e.solution.is_empty()
            || e.steps
                .iter()
                .any(|s| !s.title.is_empty() || !s.detail.is_empty() || s.expression.is_some())
    }
}
