//! 组 5：检索（3 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `search_formulas` | `query`, `limit` | `FormulaSchema[]` |
//! | `search_suggest` | `query`, `limit` | [`SearchSuggestion`]（轻量） |
//! | `search_rebuild_index` | — | `number`（索引条数） |
//!
//! ## 索引在什么时候重建
//!
//! | 时机 | 谁触发 |
//! |---|---|
//! | 应用启动（播种完内置库之后） | `setup()` |
//! | 公式增删改之后 | 命令层（`formula_save` / `formula_delete` / 导入备份后） |
//! | 用户手动 | `search_rebuild_index` |
//!
//! ⚠️ **漏掉重建会导致搜不到新公式** —— 索引是快照，不会自动跟随数据库。
//!
//! ## 为什么 `search_suggest` 要单独一个命令
//!
//! 搜索框每敲一个字都会调一次。返回完整 `FormulaSchema`（27 字段、
//! 含 4 个可能很长的 JSON 列）会让 IPC 负载白白放大几十倍。
//! `SearchSuggestion` 只有 4 个短字段。

use crate::error::CmdResult;
use crate::state::AppState;
use civilcalc_core::schema::FormulaSchema;
use civilcalc_core::search::SearchSuggestion;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

/// 默认返回条数（与源项目 `LocalFormulaIndex.search` 的 `limit = 20` 一致）
pub const DEFAULT_LIMIT: usize = 20;

/// 单次查询的返回上限（防止前端传超大 `limit` 拖垮 IPC）
pub const MAX_LIMIT: usize = 200;

/// 索引重建完成事件
pub const EVENT_INDEX_REBUILT: &str = "index://rebuilt";

/// 事件载荷
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexRebuiltPayload {
    /// 重建后的索引条数
    pub count: usize,
}

/// 把 `limit` 夹到 `1..=MAX_LIMIT`；未传时用 [`DEFAULT_LIMIT`]
fn normalize_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

/// 完整检索（含评分排序）。
///
/// 命中规则：中文 / 全拼 / 首字母缩写均可（见 `core::search`）。
#[tauri::command]
pub fn search_formulas(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> CmdResult<Vec<FormulaSchema>> {
    let limit = normalize_limit(limit);
    state.with_index(|idx| idx.search(&query, limit))
}

/// 轻量建议（搜索框实时下拉用）。
///
/// 只返回 `id` / `resultName` / `domain` / `sourceKind`，减少 IPC 负载。
#[tauri::command]
pub fn search_suggest(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> CmdResult<Vec<SearchSuggestion>> {
    let limit = normalize_limit(limit);
    state.with_index(|idx| idx.suggest(&query, limit))
}

/// 重建检索索引，并广播 `index://rebuilt`。
///
/// @returns 重建后的索引条数
#[tauri::command]
pub fn search_rebuild_index(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<usize> {
    let count = state.rebuild_index()?;

    civilcalc_core::log::i("Commands", &format!("检索索引已重建：{count} 条"));

    // 广播失败**不算命令失败** —— 索引已经重建好了，前端拿不到通知只是少一次刷新
    if let Err(e) = app.emit(EVENT_INDEX_REBUILT, IndexRebuiltPayload { count }) {
        civilcalc_core::log::w(
            "Commands",
            &format!("广播 {EVENT_INDEX_REBUILT} 失败: {e}"),
            None,
        );
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_defaults_and_clamps() {
        assert_eq!(normalize_limit(None), DEFAULT_LIMIT);
        assert_eq!(normalize_limit(Some(0)), 1, "0 应夹到 1（而不是返回空）");
        assert_eq!(normalize_limit(Some(5)), 5);
        assert_eq!(normalize_limit(Some(MAX_LIMIT)), MAX_LIMIT);
        assert_eq!(normalize_limit(Some(99999)), MAX_LIMIT);
    }

    #[test]
    fn event_name_matches_contract() {
        assert_eq!(EVENT_INDEX_REBUILT, "index://rebuilt");
    }

    #[test]
    fn payload_serde_is_camel_case() {
        let v = serde_json::to_value(IndexRebuiltPayload { count: 55 }).unwrap();
        assert_eq!(v["count"], serde_json::json!(55));
    }

    /// 索引重建的端到端语义（用内存库 + 真实内置库）
    #[test]
    fn rebuild_reflects_database_content() {
        use civilcalc_core::search::SearchIndex;
        use civilcalc_store::Db;

        let db = Db::open_in_memory().unwrap();
        let loaded = civilcalc_core::source::builtin_loader::load_embedded().unwrap();
        db.seed_builtins_if_empty(&loaded.valid).unwrap();

        let formulas = db.list_formulas().unwrap();
        let idx = SearchIndex::build(&formulas);

        assert_eq!(idx.len(), 55, "索引条数应等于库中公式数");
        // 内置库里应有「受弯」相关公式
        assert!(
            !idx.search("受弯", 10).is_empty(),
            "应能搜到受弯相关公式"
        );
        assert!(
            !idx.search("shouwan", 10).is_empty(),
            "拼音也应能搜到"
        );
    }
}
