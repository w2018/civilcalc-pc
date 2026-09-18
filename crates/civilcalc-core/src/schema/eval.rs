//! 求值结果类型。
//!
//! 源：`civilcalc-android-v2/core/schema/EvalResult.kt`
//!
//! ## 关键约定
//!
//! - `primary` 在多输出时 = **最后一段**（与源项目一致）
//! - `outputs` 每个分号段一个输出；单输出公式为空列表
//! - 条件不满足的分支 `applicable = false`，**不得用 NaN 字符串替代**
//! - `stepResults` 携带 Excel 公式（单元格引用式）与代入数值版

use serde::{Deserialize, Serialize};

/// 求值结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalResult {
    /// 主结果。多输出时 = 最后一段
    pub primary: f64,

    /// 每个分号段一个输出（`symbol` 为该段的赋值目标，无赋值时为 `None`）。
    ///
    /// ⚠️ **单段公式也有一个元素**（`symbol = None`），不是空数组 ——
    /// `eval_multi` 对每个段都推一个 output，与段有没有赋值符号无关。
    #[serde(default)]
    pub outputs: Vec<EvalOutput>,

    #[serde(default)]
    pub branches: Vec<EvalBranch>,

    #[serde(default)]
    pub steps: Vec<EvalStep>,

    #[serde(default)]
    pub step_results: Vec<StepResult>,

    #[serde(default)]
    pub warnings: Vec<String>,
}

impl EvalResult {
    /// 仅主结果（单输出公式的便捷构造）
    pub fn primary_only(primary: f64) -> Self {
        Self {
            primary,
            outputs: Vec::new(),
            branches: Vec::new(),
            steps: Vec::new(),
            step_results: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

/// 多输出段结果：`symbol` 为该段的赋值目标（如 `X2 = …`），可空。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalOutput {
    #[serde(default)]
    pub symbol: Option<String>,
    pub value: f64,
}

/// 分步求值结果（带 Excel 公式与代入数值版）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepResult {
    pub symbol: String,
    pub label: String,
    #[serde(default)]
    pub group: Option<String>,
    pub value: f64,
    #[serde(default)]
    pub unit: String,
    pub expression: String,

    /// 完整代入公式，如 `"ξ = x / h0 = 450 / 560 = 0.804"`
    #[serde(default)]
    pub substituted_expression: String,

    /// 对应的 Excel 公式（**单元格引用式**）
    #[serde(default)]
    pub excel_formula: String,

    /// Excel 公式代入数值版（单元格引用替换为实际数值），
    /// 如 `"=9.5*25^2*0.00617"`
    #[serde(default)]
    pub excel_value_formula: String,
}

/// 分支求值结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalBranch {
    pub label: String,
    pub value: Option<f64>,
    /// 条件是否满足。`false` 时 UI 显示"不适用"**并附原因**，不得用 NaN 替代
    pub applicable: bool,
    #[serde(default)]
    pub condition: Option<String>,
}

/// 求值过程步骤（引擎逐步记录）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalStep {
    pub step: i32,
    pub description: String,
    pub formula: String,
    pub substituted_formula: String,
    pub result: Option<f64>,
    #[serde(default)]
    pub error: Option<String>,
}

/// 求值错误。
///
/// 源：`core/schema/EvalResult.kt` 的 `EvalError`
///
/// ⚠️ 错误文案是**用户可见的**，必须与源项目逐字一致。
///
/// `rename_all_fields` 目前不改变任何字段名（`symbol` / `detail` / `ms` /
/// `position` / `msg` 都是单词），加上它是为了**防止将来加多词字段时静默出境成
/// snake_case**（见 `engine/types.rs` 的 `RpnToken` 踩过的坑）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum EvalError {
    /// 定义域越界（如 `sqrt` 负数、`log(0)`、`asin` 越界）
    #[error("{symbol}: {detail}")]
    Domain { symbol: String, detail: String },

    /// 求值超时（源项目上限 2 秒）
    #[error("计算超时 ({ms} ms)")]
    Timeout { ms: i64 },

    /// 编译失败（带位置）
    #[error("{msg}")]
    Compile {
        #[serde(default)]
        position: Option<usize>,
        msg: String,
    },
}

impl EvalError {
    /// 编译错误便捷构造（无位置）
    pub fn compile(msg: impl Into<String>) -> Self {
        EvalError::Compile {
            position: None,
            msg: msg.into(),
        }
    }

    /// 编译错误便捷构造（带位置）
    pub fn compile_at(position: usize, msg: impl Into<String>) -> Self {
        EvalError::Compile {
            position: Some(position),
            msg: msg.into(),
        }
    }
}

// =============================================================================
// 源项目中的固定错误文案（必须逐字一致，供引擎实现直接引用）
// =============================================================================

/// `"除数不能为零"`
pub const ERR_DIVIDE_BY_ZERO: &str = "除数不能为零";

/// `"计算结果溢出或无效（Inf/NaN），请检查输入量级"`
pub const ERR_NOT_FINITE: &str = "计算结果溢出或无效（Inf/NaN），请检查输入量级";

/// `sqrt` 定义域
pub const ERR_SQRT_NEGATIVE: &str = "sqrt 参数必须 >= 0";

/// `log` / `ln` 定义域
pub const ERR_LOG_NON_POSITIVE: &str = "log 参数必须 > 0";

/// `log10` 定义域
pub const ERR_LOG10_NON_POSITIVE: &str = "log10 参数必须 > 0";

/// `asin` 定义域
pub const ERR_ASIN_RANGE: &str = "asin 参数必须在 [-1, 1] 内";

/// `acos` 定义域
pub const ERR_ACOS_RANGE: &str = "acos 参数必须在 [-1, 1] 内";

/// 变量未提供值（引擎内部）
pub fn err_variable_missing(name: &str) -> String {
    format!("变量 '{name}' 未提供值")
}

/// 未知函数
pub fn err_unknown_function(name: &str) -> String {
    format!("未知函数: {name}")
}

/// 函数参数不足
pub fn err_function_arg_count(name: &str) -> String {
    format!("函数 {name} 参数不足")
}

/// 函数结果溢出
pub fn err_function_overflow(name: &str) -> String {
    format!("函数 {name} 结果溢出或无效")
}

/// 求值后栈大小异常
pub fn err_stack_size(n: usize) -> String {
    format!("表达式求值后栈大小异常: {n}")
}
