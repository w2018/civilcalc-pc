//! 内置公式来源校验（源项目**最高优先级**的工程安全底线）。
//!
//! 源：`civilcalc-android-v2/core/source/BuiltinFormulaLoader.kt`
//!
//! ## 行为要求（必须逐条对齐）
//!
//! 1. 逐条过 [`crate::schema::validate_schema`] 校验
//! 2. 校验失败的条目记入跳过清单（结构化上报，见 [`BuiltinLoadResult::skipped`]）
//! 3. **全部失败时拒绝启动**
//!    （源项目：`IllegalStateException("内置公式全部校验失败，拒绝启动")`）
//! 4. `verified = true` 必须有 `ref` 且命中 21 项规范白名单
//!
//! ## 红线
//!
//! **严禁 AI 批量生成 `builtin_formulas.json`** —— 内置库是人工维护资产，
//! 每条须依据权威资料逐条核对，`ref` 精确到条款号。
//!
//! ## 使用位置
//!
//! 启动时（`src-tauri` 的 `setup()`）调用 [`builtin_loader::load_embedded`]：
//! 校验通过 → 播种到 `user_formulas` → 重建搜索索引。

pub mod builtin_loader;

pub use builtin_loader::{load, load_embedded, BuiltinLoadResult};
