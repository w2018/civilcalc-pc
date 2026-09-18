//! 内置公式库（55 条，编译进二进制）。
//!
//! 源：`civilcalc-android-v2/app/src/main/assets/builtin_formulas.json`（82 KB）
//!
//! ## 组成（7 个领域）
//!
//! | 领域 | 条数 | ID 前缀 |
//! |---|---|---|
//! | 结构 | 20 | `builtin:struct_*` |
//! | 施工 | 10 | `builtin:constr_*` |
//! | 预算 | 6 | `builtin:budget_*` |
//! | 桩基 | 5 | `builtin:pile_*` |
//! | 基坑 | 6 | `builtin:excavation_*` |
//! | 测量 | 4 | `builtin:survey_*` |
//! | 数学 | 4 | `builtin:math_*` |
//!
//! ## 决策（ADR-020）
//!
//! 一期**直接复用现有 55 条**，不扩充。扩充瓶颈是专业核验而非工程：
//! 每条需要精确到条款的规范号，`verified=true` 必须有据可查。

/// 内置公式库 JSON（编译进二进制）
pub const BUILTIN_FORMULAS_JSON: &str = include_str!("builtin_formulas.json");

/// 获取内置公式库原始 JSON 文本。
///
/// 解析与校验由 [`crate::source::builtin_loader`] 负责（P0-11）。
pub fn builtin_formulas_json() -> &'static str {
    BUILTIN_FORMULAS_JSON
}
