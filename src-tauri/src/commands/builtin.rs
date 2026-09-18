//! 组 2：内置公式库（3 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `builtin_formulas` | — | `FormulaSchema[]` |
//! | `builtin_formulas_json` | — | `string`（原始 JSON，前端首次渲染用） |
//! | `builtin_status` | — | [`BuiltinStatus`] |
//! | `builtin_reload` | — | [`BuiltinStatus`] |
//!
//! ## 内置库是**编译进二进制**的只读资产
//!
//! 55 条公式随版本发布（ADR-020），**不从网络或磁盘加载**。
//! `builtin_reload` 因此只是"重新解析 + 重新校验一遍"，
//! 用途是排障（确认嵌入的 JSON 仍然自洽），**不会改变数据库内容**。

use crate::error::{CmdResult, CommandError};
use crate::state::AppState;
use civilcalc_core::schema::FormulaSchema;
use civilcalc_core::source::builtin_loader;
use serde::{Deserialize, Serialize};
use tauri::State;

/// 内置库状态（"关于"页 + 排障）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinStatus {
    /// 库中总条数（= `valid` + `skipped`）
    pub total: usize,
    /// 通过来源校验的条数
    pub valid: usize,
    /// 校验被跳过的条目（格式 `"{id}: {原因}"`）
    pub skipped: Vec<String>,
    /// 数据库里当前有多少条公式（内置播种 + 用户自建）
    pub db_formula_count: usize,
}

/// 内置公式原始 JSON（55 条）。
///
/// 前端首次渲染直接用它，省一次「命令 → 结构体 → 序列化」的往返。
#[tauri::command]
pub fn builtin_formulas_json() -> String {
    civilcalc_core::builtin::builtin_formulas_json().to_string()
}

/// 内置公式（已解析 + 已过来源校验）。
#[tauri::command]
pub fn builtin_formulas() -> CmdResult<Vec<FormulaSchema>> {
    builtin_loader::load_embedded()
        .map(|r| r.valid)
        .map_err(Into::into)
}

/// 内置库状态：解析条数 + 跳过清单 + 数据库实际条数
#[tauri::command]
pub fn builtin_status(state: State<'_, AppState>) -> CmdResult<BuiltinStatus> {
    let loaded = builtin_loader::load_embedded()?;
    let db_formula_count = state.db.list_formulas()?.len();
    Ok(BuiltinStatus {
        total: loaded.valid.len() + loaded.skipped.len(),
        valid: loaded.valid.len(),
        skipped: loaded.skipped,
        db_formula_count,
    })
}

/// 重新解析并校验内置库（排障用）。
///
/// ⚠️ **不写数据库** —— 播种只在启动时做一次（`startup::init_store`）。
/// 若用户删掉了某条内置公式，这里不会把它加回来
/// （源项目行为：用户对内置公式的删除是有效的）。
#[tauri::command]
pub fn builtin_reload(state: State<'_, AppState>) -> CmdResult<BuiltinStatus> {
    civilcalc_core::log::i("Commands", "重新加载内置公式库");
    builtin_status(state)
}

/// 内置公式条数（供 `AppInfo` 用；解析失败返回 0，不 panic）
pub fn builtin_formula_count() -> usize {
    builtin_loader::load_embedded()
        .map(|r| r.valid.len() + r.skipped.len())
        .unwrap_or(0)
}

/// 便利：校验失败时给出可读错误（命令层用）
#[allow(dead_code)]
fn builtin_error(e: civilcalc_core::CoreError) -> CommandError {
    CommandError::BuiltinSource {
        message: e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_library_has_55_valid_entries() {
        let s = builtin_loader::load_embedded().expect("内置库应可加载");
        assert_eq!(s.valid.len(), 55);
        assert!(s.skipped.is_empty());
    }

    #[test]
    fn builtin_formula_count_is_55() {
        assert_eq!(builtin_formula_count(), 55);
    }

    #[test]
    fn raw_json_is_parseable_and_has_55_items() {
        let raw = civilcalc_core::builtin::builtin_formulas_json();
        let v: serde_json::Value = serde_json::from_str(raw).expect("嵌入 JSON 应可解析");
        assert_eq!(v.as_array().map(|a| a.len()), Some(55));
    }

    /// `BuiltinStatus` 的字段名是 camelCase（前端契约）
    #[test]
    fn status_serde_is_camel_case() {
        let s = BuiltinStatus {
            total: 55,
            valid: 55,
            skipped: vec![],
            db_formula_count: 55,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert!(v.get("dbFormulaCount").is_some());
        assert!(v.get("db_formula_count").is_none(), "不得泄漏 snake_case");
    }
}
