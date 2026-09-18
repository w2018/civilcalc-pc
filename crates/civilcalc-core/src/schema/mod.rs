//! 领域类型与 Schema 校验。
//!
//! 源：`civilcalc-android-v2/core/schema/`
//!
//! | 本模块文件 | 源文件 | 任务 | 状态 |
//! |---|---|---|---|
//! | `formula.rs` | `FormulaSchema.kt` | P0-2 | ✅ 已移植 |
//! | `eval.rs` | `EvalResult.kt` | P0-2 | ✅ 已移植 |
//! | `explanation.rs` | `ExplanationText.kt` | P0-2 | ✅ 已移植（含 8 个单测） |
//! | `image_marker.rs` | `ImageMarker.kt` | P0-2 | ✅ 已移植（含 6 个单测） |
//! | `validator.rs` | `SchemaValidator.kt` | P0-3 | ✅ 已移植 |
//! | `history.rs` | `HistoryEntity.kt` + `HistoryRow` | P1-4 | ✅ 已移植（含 7 个单测） |
//!
//! ## 本模块的定位：**持久化契约的家**
//!
//! 除 `FormulaSchema` 这类领域模型外，本模块还收纳**所有落库/进备份包的行类型**
//! （[`HistoryEntry`]、[`FormulaVersion`]）。这样：
//!
//! - `civilcalc-backup` 只需依赖 `civilcalc-core` 一个 crate 即可搬运全部七类数据
//! - `src-tauri/src/db.rs` 的 `use civilcalc_core::schema::{...}` 保持单一入口
//! - 备份格式（对外契约，ADR-009）集中在一处，便于审计
//!
//! ## 关键约定
//!
//! - `FormulaSource.ref_` 加 `#[serde(rename = "ref")]`，保证序列化后与
//!   源项目 Kotlin / 前端 TS 一致（见 `docs/04-数据契约.md` §13）
//! - Kotlin `@Serializable` 的默认值 → Rust `#[serde(default)]`
//!   （Kotlin 缺字段用默认值，Rust 默认会报错，必须显式标注）
//! - **新公式必须写入 `schemaVersion = 3`**（[`CURRENT_SCHEMA_VERSION`]），
//!   否则会触发不必要的 Excel 迁移
//!
//! ## 与源项目的一处刻意偏差
//!
//! 源项目 `FunctionDoc` 定义在 `core/formula/export/ExcelFormulaConverter.kt`，
//! 但被 `FormulaSchema.excelFunctionDocs` 引用。Rust 中若把它放在 `excel/`
//! 会造成 `excel → engine → schema` 与 `schema → excel` 的循环依赖，
//! 因此本模块定义 [`FunctionDoc`]，由 `excel/` 复用。

pub mod eval;
pub mod explanation;
pub mod formula;
pub mod history;
pub mod image_marker;
pub mod validator;

pub use eval::{
    err_function_arg_count, err_function_overflow, err_stack_size, err_unknown_function,
    err_variable_missing, EvalBranch, EvalError, EvalOutput, EvalResult, EvalStep, StepResult,
    ERR_ACOS_RANGE, ERR_ASIN_RANGE, ERR_DIVIDE_BY_ZERO, ERR_LOG10_NON_POSITIVE,
    ERR_LOG_NON_POSITIVE, ERR_NOT_FINITE, ERR_SQRT_NEGATIVE,
};
pub use formula::{
    plain_desc, result_display_name, result_unit_of, AltExpression, ExplanationStep,
    FormulaExplanation, FormulaSchema, FormulaSource, FormulaVar, FunctionDoc, ResultOutput,
    SourceKind, StepTemplate, CURRENT_SCHEMA_VERSION,
};
pub use history::HistoryEntry;
pub use validator::{matches_whitelist, validate_schema, ValidationError, WHITELIST_REFS};

// 版本行类型也从此处再导出：`db.rs` / `backup` 只需 `use civilcalc_core::schema::{...}`
// 一个入口，不必同时记住 `version` 模块路径（定义仍在 `crate::version`）。
pub use crate::version::{next_version, FormulaVersion, INITIAL_VERSION};
