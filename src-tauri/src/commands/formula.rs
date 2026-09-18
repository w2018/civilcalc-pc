//! 组 3：公式 CRUD 与参数草稿（8 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `formula_get` | `id` | `FormulaSchema \| null` |
//! | `formula_list` | `filter?` | `FormulaSchema[]` |
//! | `formula_save` | `schema` | `void` |
//! | `formula_delete` | `id` | `void` |
//! | `formula_exists` | `id` | `boolean` |
//! | `formula_draft_save` | `formulaId`, `paramsJson` | `void` |
//! | `formula_draft_load` | `formulaId` | `string \| null` |
//! | `formula_draft_clear_others` | `currentId` | `number`（清掉的草稿数） |
//!
//! ## 草稿存哪
//!
//! 存**偏好**（`formula_drafts` 键，值是 `{formulaId: paramsJson}` 的 JSON），
//! 不建表 —— 与源项目一致（源项目也在 DataStore 里）。
//! 草稿是"临时输入"，不该进备份包的业务数据；它随偏好一起走。
//!
//! ## ⚠️ `formula_save` 会刷新 `updatedAt`
//!
//! 存储层（`civilcalc_store::Db::upsert_formula`）**忠实照抄 `schema.updatedAt`**
//! （对齐源项目 `saveSchema`）。命令层作为"调用方"，负责在保存前把它设为当前时间 ——
//! 否则公式列表的"最近更新"排序会不对。

use crate::config::KEY_DRAFTS;
use crate::error::{CmdResult, CommandError};
use crate::state::AppState;

use civilcalc_core::schema::FormulaSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tauri::State;

/// 列表筛选条件。全为 `None` / `false` 时返回全部。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormulaFilter {
    /// 领域（如 `"结构"`）；精确匹配
    #[serde(default)]
    pub domain: Option<String>,
    /// 来源类别：`STANDARD` / `AI` / `CUSTOM` / `DERIVED`
    #[serde(default)]
    pub source_kind: Option<String>,
    /// 只要收藏的
    #[serde(default)]
    pub favorite_only: bool,
}

/// 参数草稿表（`formulaId` → 参数 JSON）
type Drafts = BTreeMap<String, String>;

/// 读草稿表（坏数据 → 空表 + 记日志，不让整个命令失败）
fn read_drafts(state: &AppState) -> Drafts {
    let raw = state
        .with_config(|c| c.get_str(KEY_DRAFTS).map(str::to_string))
        .unwrap_or(None);
    match raw {
        None => Drafts::new(),
        Some(s) if s.trim().is_empty() => Drafts::new(),
        Some(s) => match serde_json::from_str(&s) {
            Ok(m) => m,
            Err(e) => {
                civilcalc_core::log::w(
                    "Commands",
                    &format!("参数草稿表解析失败，本次按空表处理: {e}"),
                    None,
                );
                Drafts::new()
            }
        },
    }
}

/// 写草稿表
fn write_drafts(state: &AppState, drafts: &Drafts) -> CmdResult<()> {
    let json = serde_json::to_string(drafts).map_err(|e| CommandError::Parse {
        message: format!("序列化参数草稿失败: {e}"),
    })?;
    state.update_config(move |c| {
        if drafts.is_empty() {
            c.remove(KEY_DRAFTS);
        } else {
            c.set_str(KEY_DRAFTS, json);
        }
    })
}

// =============================================================================
// 命令
// =============================================================================

/// 按 id 取公式；不存在返回 `null`（**不是错误**）
#[tauri::command]
pub fn formula_get(state: State<'_, AppState>, id: String) -> CmdResult<Option<FormulaSchema>> {
    state.db.get_formula(&id).map_err(Into::into)
}

/// 列表（可按领域 / 来源 / 收藏筛选）
#[tauri::command]
pub fn formula_list(
    state: State<'_, AppState>,
    filter: Option<FormulaFilter>,
) -> CmdResult<Vec<FormulaSchema>> {
    let all = if filter.as_ref().is_some_and(|f| f.favorite_only) {
        state.db.list_favorite_formulas()?
    } else {
        state.db.list_formulas()?
    };

    let Some(f) = filter else {
        return Ok(all);
    };

    Ok(all
        .into_iter()
        .filter(|s| f.domain.as_ref().is_none_or(|d| &s.domain == d))
        .filter(|s| {
            f.source_kind.as_ref().is_none_or(|k| {
                serde_json::to_value(s.source.kind)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .is_some_and(|name| &name == k)
            })
        })
        .collect())
}

/// 保存公式（新建或覆盖）。
///
/// - 冲突更新时**保留**原 `favorite` 与 `createdAt`（存储层保证）
/// - 这里把 `updatedAt` 刷成当前时间（见模块文档）
/// - `schemaVersion` 缺省为 3（`FormulaSchema` 的 serde 默认值），
///   **不在这里强制改写** —— 显式传入旧版本号是有意为之（如导入旧数据）
///
/// ⚠️ 保存后会**重建检索索引** —— 索引是快照，不重建就搜不到新公式。
/// 重建失败**不影响保存结果**（数据已落库），只记 WARN。
#[tauri::command]
pub fn formula_save(state: State<'_, AppState>, schema: FormulaSchema) -> CmdResult<()> {
    let mut s = schema;
    s.updated_at = civilcalc_core::now_ms();
    if s.created_at <= 0 {
        s.created_at = s.updated_at;
    }
    let id = s.id.clone();
    state.db.upsert_formula(&s)?;

    if let Err(e) = state.rebuild_index() {
        civilcalc_core::log::w(
            "Commands",
            &format!("保存公式后重建检索索引失败（公式 {id}）: {e}"),
            None,
        );
    }
    Ok(())
}

/// 删除公式（**幂等**：不存在也算成功）。
///
/// ⚠️ 删除后会**重建检索索引**（否则已删公式仍能被搜到）。
#[tauri::command]
pub fn formula_delete(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.db.delete_formula(&id)?;

    if let Err(e) = state.rebuild_index() {
        civilcalc_core::log::w(
            "Commands",
            &format!("删除公式后重建检索索引失败（公式 {id}）: {e}"),
            None,
        );
    }
    Ok(())
}

/// 公式是否存在
#[tauri::command]
pub fn formula_exists(state: State<'_, AppState>, id: String) -> CmdResult<bool> {
    state.db.formula_exists(&id).map_err(Into::into)
}

/// 保存某公式的参数草稿
#[tauri::command]
pub fn formula_draft_save(
    state: State<'_, AppState>,
    formula_id: String,
    params_json: String,
) -> CmdResult<()> {
    let mut drafts = read_drafts(&state);
    if params_json.trim().is_empty() {
        drafts.remove(&formula_id);
    } else {
        drafts.insert(formula_id, params_json);
    }
    write_drafts(&state, &drafts)
}

/// 读某公式的参数草稿；无草稿返回 `null`
#[tauri::command]
pub fn formula_draft_load(state: State<'_, AppState>, formula_id: String) -> CmdResult<Option<String>> {
    Ok(read_drafts(&state).get(&formula_id).cloned())
}

/// 清掉除 `currentId` 之外的所有参数草稿（"只保留当前公式的输入"）。
///
/// 返回清掉的草稿数。
#[tauri::command]
pub fn formula_draft_clear_others(
    state: State<'_, AppState>,
    current_id: String,
) -> CmdResult<usize> {
    let mut drafts = read_drafts(&state);
    let before = drafts.len();
    drafts.retain(|k, _| k == &current_id);
    let removed = before - drafts.len();
    if removed > 0 {
        write_drafts(&state, &drafts)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use civilcalc_core::schema::{FormulaSource, CURRENT_SCHEMA_VERSION};

    fn schema(id: &str, domain: &str) -> FormulaSchema {
        FormulaSchema {
            id: id.to_string(),
            result_name: "结果".into(),
            result_symbol: "y".into(),
            result_unit: None,
            result_outputs: vec![],
            expression: "a+b".into(),
            source_equations: vec![],
            alt_expressions: vec![],
            constants: Default::default(),
            variables: vec![],
            domain: domain.to_string(),
            tags: vec![],
            reference_basis: None,
            design_notes: None,
            explanation: None,
            image_ids: vec![],
            revised_from: None,
            source: FormulaSource::custom(),
            steps_template: None,
            doc_template_id: None,
            excel_expression: None,
            excel_alt_expressions: None,
            excel_steps_template: None,
            excel_function_docs: None,
            schema_version: CURRENT_SCHEMA_VERSION,
            created_at: 1000,
            updated_at: 1000,
        }
    }

    /// `FormulaFilter` 的 serde 形状（前端契约）
    #[test]
    fn filter_serde_is_camel_case() {
        let f = FormulaFilter {
            domain: Some("结构".into()),
            source_kind: Some("CUSTOM".into()),
            favorite_only: true,
        };
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v["sourceKind"], serde_json::json!("CUSTOM"));
        assert_eq!(v["favoriteOnly"], serde_json::json!(true));
        assert!(v.get("source_kind").is_none(), "不得泄漏 snake_case");
    }

    /// 筛选逻辑：领域精确匹配
    #[test]
    fn domain_filter_is_exact_match() {
        let all = [schema("a", "结构"), schema("b", "施工"), schema("c", "结构")];
        let f = FormulaFilter {
            domain: Some("结构".into()),
            ..Default::default()
        };
        let kept: Vec<&str> = all
            .iter()
            .filter(|s| f.domain.as_ref().is_none_or(|d| &s.domain == d))
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(kept, ["a", "c"]);
    }

    /// `SourceKind` 序列化成 SCREAMING_SNAKE，筛选时按同名字符串比对
    #[test]
    fn source_kind_filter_uses_serde_name() {
        let s = schema("a", "结构");
        let name = serde_json::to_value(s.source.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string));
        assert_eq!(name.as_deref(), Some("CUSTOM"));
    }

    /// 草稿表 JSON 往返
    #[test]
    fn drafts_map_roundtrip() {
        let mut d = Drafts::new();
        d.insert("usr:1".into(), r#"{"a":1}"#.into());
        d.insert("usr:2".into(), r#"{"b":2}"#.into());

        let json = serde_json::to_string(&d).unwrap();
        let back: Drafts = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
        assert_eq!(back.get("usr:1").map(String::as_str), Some(r#"{"a":1}"#));
    }

    /// `clear_others` 的保留语义
    #[test]
    fn clear_others_keeps_only_current() {
        let mut d = Drafts::new();
        d.insert("a".into(), "1".into());
        d.insert("b".into(), "2".into());
        d.insert("c".into(), "3".into());

        let before = d.len();
        d.retain(|k, _| k == "b");
        assert_eq!(before - d.len(), 2);
        assert_eq!(d.keys().collect::<Vec<_>>(), vec!["b"]);
    }
}
