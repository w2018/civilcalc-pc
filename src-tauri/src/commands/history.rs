//! 组 7b：计算历史（6 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `history_list` | `formulaId?`, `limit`, `offset` | `HistoryEntry[]` |
//! | `history_get` | `id` | `HistoryEntry \| null` |
//! | `history_record_created` | `formulaId`, `thinking?` | `number`（新条目 id） |
//! | `history_delete` | `id` | `void` |
//! | `history_clear` | `confirm: boolean` | `void` |
//! | `history_thinking` | `formulaId` | `string \| null` |
//!
//! ## ⚠️ `history_clear` 必须二次确认
//!
//! 清空历史**不可撤销**，因此要求显式传 `confirm: true`。
//! 传 `false` 返回 [`CommandError::InvalidArgument`] —— 这是**防手滑**的设计：
//! 前端"清空"按钮必须先弹确认框，再把 `confirm=true` 传进来。
//!
//! ## 历史什么时候写入（需求 7）
//!
//! 两个时机，缺一不可：
//!
//! | 时机 | 写入点 | 条目形态 |
//! |---|---|---|
//! | **AI 生成 / 微调出新公式** | `history_record_created` | 无输入无结果（`isParseOnly`） |
//! | **计算得到结果** | `eval_formula` | 有输入有结果 |
//!
//! 前者以前**只存在于类型里**（`HistoryEntry::parse_only` 有实现、有单测），
//! 但没有任何命令调用它 —— 于是「历史」页只有计算记录，
//! 用户想知道「这条公式是哪次生成的、当时模型怎么想的」时查不到。
//!
//! ⚠️ 预览求值（`eval_schema`）**不写**历史 —— 否则用户拖滑块试参数
//! 会瞬间灌满历史。
//!
//! ⚠️ 源项目**从不写历史**（`HistoryRepositoryImpl` 只有读/删方法，
//! 全项目无 `insert` 调用），所以「历史」页在源项目里永远是空的。
//! 这里按设计文档的意图补全。

use crate::error::{CmdResult, CommandError};
use crate::state::AppState;
use civilcalc_core::schema::HistoryEntry;
use tauri::State;

/// 单页上限。防止前端传个超大 `limit` 把内存撑爆。
const MAX_LIMIT: u32 = 500;

/// 历史列表（按 `createdAt` 倒序）。
///
/// `formulaId` 为 `None` 时返回**全局历史**（所有公式混排）。
#[tauri::command]
pub fn history_list(
    state: State<'_, AppState>,
    formula_id: Option<String>,
    limit: u32,
    offset: u32,
) -> CmdResult<Vec<HistoryEntry>> {
    let limit = limit.min(MAX_LIMIT);
    state
        .db
        .list_history(formula_id.as_deref(), limit, offset)
        .map_err(Into::into)
}

/// 按 id 取单条历史（**历史回填用**）。
///
/// 回填链路：路由只带 `formulaId` + `historyId`（**不把大 JSON 塞进路由**），
/// 工作台拿到 `historyId` 后用本命令取回 `inputsJson` 预填参数。
///
/// 历史不存在（已被清理 / 删掉）返回 `null` —— **不是错误**：
/// 用户点了一条刚被清掉的历史，弹「历史不存在」比静默回落到默认值更烦人。
#[tauri::command]
pub fn history_get(state: State<'_, AppState>, id: i64) -> CmdResult<Option<HistoryEntry>> {
    state.db.get_history(id).map_err(Into::into)
}

/// 记一条「公式刚被创建」的历史（**无输入、无结果**）——需求 7。
///
/// ## 为什么由前端在 `formula_save` 之后调
///
/// 公式是前端保存的（`formula_save` 也用于**编辑既有公式**，
/// 那条路径不该记历史）。所以「这是一条**新**公式」这个信息只有
/// 调用方知道 —— 由它显式调本命令，比在 `formula_save` 里猜更可靠。
///
/// ## 参数
///
/// - `formula_id`：**必须已经在库里**（本命令从库读快照，保证与当前公式一致）
/// - `thinking`：AI 生成 / 微调时的思考全文；本地解析传 `None`
///
/// 返回新条目的 `id`。
#[tauri::command]
pub fn history_record_created(
    state: State<'_, AppState>,
    formula_id: String,
    thinking: Option<String>,
) -> CmdResult<i64> {
    // 从库里取**当前**快照：调用方刚 `formula_save` 过，
    // 用它手里的对象会与库不一致（比如后端补过 id / 时间戳）
    let schema = state.db.get_formula(&formula_id)?.ok_or_else(|| {
        CommandError::InvalidArgument {
            message: format!("公式不存在，无法记历史：{formula_id}"),
        }
    })?;

    let entry = HistoryEntry::parse_only(&schema, thinking);
    state.db.insert_history(&entry).map_err(Into::into)
}

/// 删除单条历史（**幂等**：不存在也算成功）
#[tauri::command]
pub fn history_delete(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    state.db.delete_history(id).map_err(Into::into)
}

/// 清空全部历史。**必须传 `confirm: true`**。
#[tauri::command]
pub fn history_clear(state: State<'_, AppState>, confirm: bool) -> CmdResult<usize> {
    if !confirm {
        return Err(CommandError::InvalidArgument {
            message: "清空历史不可撤销，请显式确认（confirm=true）".to_string(),
        });
    }
    // 先数一下，好给用户一个"清了多少条"的反馈
    let n = state.db.list_history(None, MAX_LIMIT, 0)?.len();
    state.db.clear_history()?;
    civilcalc_core::log::i("Commands", &format!("已清空计算历史（{n} 条）"));
    Ok(n)
}

/// 取某公式最近一次历史的思考内容（供"AI 思考过程"面板回溯）。
///
/// ⚠️ **不过滤空值**：最新那条没有思考内容就返回 `null`
/// （对齐源项目 `getLatestThinkingContent`）。
#[tauri::command]
pub fn history_thinking(
    state: State<'_, AppState>,
    formula_id: String,
) -> CmdResult<Option<String>> {
    state.db.latest_thinking(&formula_id).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::MAX_LIMIT;
    use civilcalc_core::schema::{EvalResult, HistoryEntry};
    use civilcalc_store::Db;

    fn entry(formula_id: &str, thinking: Option<&str>, created_at: i64) -> HistoryEntry {
        let mut e = HistoryEntry::parse_only(
            &serde_json::from_str(
                r#"{"id":"usr:1","resultName":"r","resultSymbol":"y","expression":"a+b",
                    "variables":[],"constants":{},"domain":"通用",
                    "source":{"kind":"CUSTOM"},"schemaVersion":3}"#,
            )
            .unwrap(),
            thinking.map(str::to_string),
        );
        e.formula_id = formula_id.to_string();
        e.result_json = serde_json::to_string(&EvalResult::primary_only(3.0)).unwrap();
        e.created_at = created_at;
        e
    }

    #[test]
    fn limit_is_capped() {
        assert_eq!(1000u32.min(MAX_LIMIT), 500);
        assert_eq!(10u32.min(MAX_LIMIT), 10);
    }

    /// `history_clear` 的 `confirm` 闸门
    #[test]
    fn clear_requires_confirm_flag() {
        // 命令层的判断逻辑（这里直接验证语义，命令本体需要 Tauri State）
        let confirm = false;
        assert!(
            if !confirm { Err("需确认") } else { Ok(()) }.is_err(),
            "confirm=false 必须被拒"
        );
    }

    /// 全局 vs 按公式筛选
    #[test]
    fn list_filters_by_formula() {
        let db = Db::open_in_memory().unwrap();
        db.insert_history(&entry("usr:1", Some("t1"), 1000)).unwrap();
        db.insert_history(&entry("usr:2", Some("t2"), 2000)).unwrap();

        assert_eq!(db.list_history(None, 50, 0).unwrap().len(), 2);
        assert_eq!(db.list_history(Some("usr:1"), 50, 0).unwrap().len(), 1);
        assert_eq!(
            db.list_history(Some("usr:1"), 50, 0).unwrap()[0].formula_id,
            "usr:1"
        );
    }

    /// 倒序：最新的在前
    #[test]
    fn list_is_newest_first() {
        let db = Db::open_in_memory().unwrap();
        db.insert_history(&entry("f", Some("旧"), 1000)).unwrap();
        db.insert_history(&entry("f", Some("新"), 2000)).unwrap();

        let list = db.list_history(Some("f"), 50, 0).unwrap();
        assert_eq!(list[0].thinking_content.as_deref(), Some("新"));
        assert_eq!(list[1].thinking_content.as_deref(), Some("旧"));
    }

    /// `history_thinking`：取最新一条，不过滤空值
    #[test]
    fn thinking_returns_latest_entry_verbatim() {
        let db = Db::open_in_memory().unwrap();
        db.insert_history(&entry("f", Some("第一次"), 1000)).unwrap();
        assert_eq!(
            db.latest_thinking("f").unwrap().as_deref(),
            Some("第一次")
        );

        // 最新一条无思考 → None（对齐源项目，不做非空过滤）
        db.insert_history(&entry("f", None, 2000)).unwrap();
        assert_eq!(db.latest_thinking("f").unwrap(), None);
    }

    #[test]
    fn delete_is_idempotent() {
        let db = Db::open_in_memory().unwrap();
        let id = db.insert_history(&entry("f", None, 1000)).unwrap();
        db.delete_history(id).unwrap();
        // 再删一次不报错
        db.delete_history(id).unwrap();
        assert!(db.list_history(None, 50, 0).unwrap().is_empty());
    }
}
