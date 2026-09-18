//! Excel 公式转换与校验（契约 v3，见 `docs/06-Excel单元格映射契约.md`）。
//!
//! 源：`civilcalc-android-v2/core/formula/export/`
//!
//! | 文件 | 源文件 | 任务 |
//! |---|---|---|
//! | `number_formatter.rs` | `ExcelNumberFormatter.kt` | ✅ P3-1 |
//! | `converter.rs` | `ExcelFormulaConverter.kt` | ✅ P3-1 |
//! | `validator.rs` | `ExcelFormulaValidator.kt` | ✅ P3-2 |
//! | `migrator.rs` | `FormulaExcelMigrator.kt` | ✅ P3-3 |
//!
//! ## ⚠️ 两条不可违反的红线
//!
//! 1. **数值代入必须用单趟词法扫描** —— **严禁 `str::replace` 循环**
//!    （这是源项目 P0 缺陷「Excel 数值代入不全」的根因，已废除）
//! 2. **取模输出 `MOD(a,b)`** —— Excel 的 `%` 是后缀百分比运算符，不是取模
//!
//! ## 契约要点
//!
//! - 变量 → 第 1 行横向：第 n 个变量 → `A1` / `B1` / … / `Z1` / `AA1`（26 进制无零位）
//! - 步骤结果 → A 列纵向：第 i 步 → `A(i+2)`
//! - 等号剥离只用 `strip_assignment()`，**不得** `replace("=", "")`（会破坏 `>=`/`<=`/`==`）

pub mod converter;
pub mod migrator;
pub mod number_formatter;
pub mod validator;

pub use converter::{
    index_to_cell_ref, scan_residuals, split_output_formula, substitute_values, ExcelFormula,
    ExcelFormulaConverter, ExcelOutputFormula, ExcelSubstituteResult, ParamCell, ResidualScan,
};
pub use migrator::{
    migrate_all as migrate_excel_fields, migrate_schema as migrate_schema_excel_fields,
    MigrationResult as ExcelMigrationResult, TARGET_SCHEMA_VERSION as EXCEL_TARGET_SCHEMA_VERSION,
};
pub use number_formatter::{format as format_excel_number, parse as parse_excel_number};
pub use validator::{
    back_translate_excel_to_local, validate as validate_excel_expression, validate_steps as validate_excel_steps,
    validate_steps_with_samples as validate_excel_steps_with_samples,
    validate_with_samples as validate_excel_expression_with_samples, JavaRandom, ValidationResult,
    DEFAULT_SAMPLE_COUNT as EXCEL_VALIDATE_SAMPLE_COUNT, RELATIVE_ERROR_THRESHOLD,
};
