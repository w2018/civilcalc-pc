//! 组 7a：收藏（3 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `is_favorite` | `id` | `boolean` |
//! | `toggle_favorite` | `id` | `boolean`（切换后的状态） |
//! | `favorites_list` | — | `FormulaSchema[]` |
//!
//! ## 两处存储
//!
//! 收藏同时写**两张表**（对齐源项目）：
//!
//! - `favorites` 表：`(formulaId, createdAt)` —— 承载"**收藏时间序**"
//! - `user_formulas.favorite` 列：0/1 标记位 —— 承载"**这条是否被收藏**"
//!
//! 两者冗余但语义不同：前者能按收藏时间排序，后者能随公式行一起读出来
//! （省一次 JOIN）。存储层的 `set_favorite` 会同时维护两边。

use crate::error::CmdResult;
use crate::state::AppState;
use civilcalc_core::schema::FormulaSchema;
use tauri::State;

/// 是否已收藏
#[tauri::command]
pub fn is_favorite(state: State<'_, AppState>, id: String) -> CmdResult<bool> {
    state.db.is_favorite(&id).map_err(Into::into)
}

/// 切换收藏，返回**切换后**的状态。
#[tauri::command]
pub fn toggle_favorite(state: State<'_, AppState>, id: String) -> CmdResult<bool> {
    let next = !state.db.is_favorite(&id)?;
    state.db.set_favorite(&id, next)?;
    Ok(next)
}

/// 收藏列表（按**收藏时间倒序**）
#[tauri::command]
pub fn favorites_list(state: State<'_, AppState>) -> CmdResult<Vec<FormulaSchema>> {
    state.db.list_favorite_formulas().map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use civilcalc_store::Db;

    /// 收藏切换 + 列表（用内存库验证存储层语义，命令层是纯转发）
    #[test]
    fn toggle_and_list_roundtrip() {
        let db = Db::open_in_memory().unwrap();

        let mut s: civilcalc_core::schema::FormulaSchema = serde_json::from_str(
            r#"{"id":"usr:1","resultName":"r","resultSymbol":"y","expression":"a+b",
                "variables":[],"constants":{},"domain":"通用",
                "source":{"kind":"CUSTOM"},"schemaVersion":3}"#,
        )
        .unwrap();
        s.updated_at = 1000;
        db.upsert_formula(&s).unwrap();

        // 初始未收藏
        assert!(!db.is_favorite("usr:1").unwrap());

        // 切一次 → 收藏
        let next = !db.is_favorite("usr:1").unwrap();
        assert!(next);
        db.set_favorite("usr:1", next).unwrap();
        assert!(db.is_favorite("usr:1").unwrap());
        assert_eq!(db.list_favorite_formulas().unwrap().len(), 1);

        // 再切一次 → 取消
        let next = !db.is_favorite("usr:1").unwrap();
        assert!(!next);
        db.set_favorite("usr:1", next).unwrap();
        assert!(!db.is_favorite("usr:1").unwrap());
        assert!(db.list_favorite_formulas().unwrap().is_empty());
    }

    /// `favorites_list` 按收藏时间倒序（后收藏的在前）
    #[test]
    fn favorites_list_is_newest_first() {
        let db = Db::open_in_memory().unwrap();
        for id in ["usr:1", "usr:2"] {
            let s: civilcalc_core::schema::FormulaSchema = serde_json::from_str(&format!(
                r#"{{"id":"{id}","resultName":"r","resultSymbol":"y","expression":"a+b",
                    "variables":[],"constants":{{}},"domain":"通用",
                    "source":{{"kind":"CUSTOM"}},"schemaVersion":3}}"#
            ))
            .unwrap();
            db.upsert_formula(&s).unwrap();
        }

        db.set_favorite("usr:1", true).unwrap();
        // 保证 usr:2 的收藏时间 >= usr:1
        std::thread::sleep(std::time::Duration::from_millis(5));
        db.set_favorite("usr:2", true).unwrap();

        let ids: Vec<String> = db
            .list_favorite_formulas()
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, vec!["usr:2", "usr:1"], "后收藏的应在前");
    }
}
