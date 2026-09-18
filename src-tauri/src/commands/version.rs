//! 组 6：公式版本（6 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `version_list` | `formulaId` | `FormulaVersion[]` |
//! | `version_get` | `formulaId`, `version` | `FormulaVersion` |
//! | `version_switch` | `formulaId`, `version` | `void` |
//! | `version_create` | `schema`, `changeLog`, `editor` | `FormulaVersion` |
//! | `version_diff` | `formulaId`, `versionA`, `versionB` | `VersionDiff` |
//! | `version_head` | `formulaId` | `string \| null` |
//!
//! ## head 语义（ADR-014）
//!
//! 源项目 `FormulaVersion` **没有 head 字段**（已知缺口）。PC 端把 head 存在
//! `user_formulas.headVersion` 列：
//!
//! - `NULL` → **最新版本即 head**（与源项目当前行为一致，退化兼容）
//! - 非 `NULL` → 用户显式切过版本
//!
//! `version_switch` 是**唯一**会写这一列的地方。

use crate::error::{CmdResult, CommandError};
use crate::state::AppState;
use civilcalc_core::schema::FormulaSchema;
use civilcalc_core::version::chain::{diff, resolve_head};
use civilcalc_core::version::{next_version_for, FormulaVersion, VersionDiff};
use tauri::State;

/// 版本链（**旧 → 新**）
#[tauri::command]
pub fn version_list(
    state: State<'_, AppState>,
    formula_id: String,
) -> CmdResult<Vec<FormulaVersion>> {
    // 存储层返回的是 createdAt DESC（新 → 旧，对齐源项目 VersionDao）；
    // 版本链的语义是"旧 → 新"，这里翻转过来给前端。
    let mut list = state.db.list_versions(&formula_id)?;
    list.reverse();
    Ok(list)
}

/// 取指定版本
#[tauri::command]
pub fn version_get(
    state: State<'_, AppState>,
    formula_id: String,
    version: String,
) -> CmdResult<FormulaVersion> {
    state.db.get_version(&formula_id, &version).map_err(Into::into)
}

/// 当前 head 版本号；从未切过则返回 `null`（表示"最新即 head"）
#[tauri::command]
pub fn version_head(state: State<'_, AppState>, formula_id: String) -> CmdResult<Option<String>> {
    state.db.get_head_version(&formula_id).map_err(Into::into)
}

/// 切换 head 到指定版本。
///
/// ⚠️ **不改写历史** —— 只更新 `user_formulas.headVersion` 一列（源项目规格的行为）。
/// 版本号不存在时返回 [`CommandError::NotFound`]（不静默写进去）。
#[tauri::command]
pub fn version_switch(
    state: State<'_, AppState>,
    formula_id: String,
    version: String,
) -> CmdResult<()> {
    // 先确认该版本确实存在（避免 head 指向不存在的版本）
    state.db.get_version(&formula_id, &version)?;
    state.db.set_head_version(&formula_id, &version)?;
    civilcalc_core::log::i(
        "Commands",
        &format!("切换 head：{formula_id} → {version}"),
    );
    Ok(())
}

/// 基于给定 schema 新建一个版本，并把它设为 head。
///
/// - 父版本 = 当前链中 `createdAt` 最大的那条（空链则无父）
/// - `changeType` 自动推断：空链 → `"create"`，否则 `"edit"`
/// - 版本号由 `next_version` 从父版本推导（`1.0.0` → `1.0.1`；位数不足时进高位）
#[tauri::command]
pub fn version_create(
    state: State<'_, AppState>,
    schema: FormulaSchema,
    change_log: String,
    editor: String,
) -> CmdResult<FormulaVersion> {
    let existing = state.db.list_versions(&schema.id)?;
    let (parent, _next) = next_version_for(&existing);
    let change_type = if parent.is_none() { "create" } else { "edit" };

    let v = FormulaVersion::create(
        &schema,
        parent.as_deref(),
        change_type,
        change_log,
        editor,
    );

    state.db.insert_version(&v)?;
    state.db.set_head_version(&schema.id, &v.version)?;

    civilcalc_core::log::i(
        "Commands",
        &format!(
            "新建版本：{} → {}（父 {:?}，{}）",
            v.formula_id, v.version, v.parent_version, v.change_type
        ),
    );
    Ok(v)
}

/// 比较两个版本的 6 维差异。
///
/// 顺序按 `(versionA, versionB)` —— 前端展示时 `oldValue` 来自 A、`newValue` 来自 B。
/// 两个版本号相同时返回空差异（不是错误）。
#[tauri::command]
pub fn version_diff(
    state: State<'_, AppState>,
    formula_id: String,
    version_a: String,
    version_b: String,
) -> CmdResult<VersionDiff> {
    let va = state.db.get_version(&formula_id, &version_a)?;
    let vb = state.db.get_version(&formula_id, &version_b)?;

    let sa = va.schema().map_err(|e| CommandError::Parse {
        message: format!("版本 {version_a} 的快照无法解析: {e}"),
    })?;
    let sb = vb.schema().map_err(|e| CommandError::Parse {
        message: format!("版本 {version_b} 的快照无法解析: {e}"),
    })?;

    Ok(diff(&sa, &sb))
}

/// 解析当前生效的版本（含 head 退化逻辑，供前端展示"当前版本"徽标）。
///
/// 与 `version_head` 的区别：`version_head` 返回**存储的原始值**（可能为 null），
/// 这里返回**解析后的实际生效版本**（`NULL` → 最新）。
#[tauri::command]
pub fn version_resolve_head(
    state: State<'_, AppState>,
    formula_id: String,
) -> CmdResult<Option<FormulaVersion>> {
    let list = state.db.list_versions(&formula_id)?;
    let stored = state.db.get_head_version(&formula_id)?;
    Ok(resolve_head(&list, stored.as_deref()).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use civilcalc_core::schema::{FormulaSource, CURRENT_SCHEMA_VERSION};

    fn schema(id: &str, expr: &str) -> FormulaSchema {
        FormulaSchema {
            id: id.to_string(),
            result_name: "结果".into(),
            result_symbol: "y".into(),
            result_unit: None,
            result_outputs: vec![],
            expression: expr.to_string(),
            source_equations: vec![],
            alt_expressions: vec![],
            constants: Default::default(),
            variables: vec![],
            domain: "通用".into(),
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

    /// 空链 → 首版，`changeType = "create"`
    #[test]
    fn first_version_is_create() {
        let (parent, _) = next_version_for(&[]);
        assert!(parent.is_none());
        assert_eq!(
            if parent.is_none() { "create" } else { "edit" },
            "create"
        );
    }

    /// 非空链 → 后续版本，`changeType = "edit"`
    #[test]
    fn subsequent_version_is_edit() {
        let v = FormulaVersion::create(&schema("f", "a+b"), None, "create", "", "user");
        let (parent, next) = next_version_for(std::slice::from_ref(&v));
        assert_eq!(parent.as_deref(), Some("1.0.0"));
        assert_eq!(next, "1.0.1");
        assert_eq!(
            if parent.is_none() { "create" } else { "edit" },
            "edit"
        );
    }

    /// `version_list` 的翻转语义：存储层给的是新→旧，命令层给前端的是旧→新
    #[test]
    fn list_reversal_semantics() {
        // ⚠️ `created_at` **必须显式赋值**，不能靠 `create()` 里的 `now()`：
        // 三次 `create` 落在同一毫秒时，按时间排序的键全相等，
        // 顺序就取决于稳定排序保留的原始次序 —— 测试会**偶发失败**
        // （并发跑整个 lib 测试时更容易撞上）。
        // 生产里版本由用户操作产生、不会撞毫秒，所以修的是测试而不是实现。
        let mk = |expr: &str, parent: Option<&str>, change: &str, at: i64| {
            let mut v = FormulaVersion::create(&schema("f", expr), parent, change, "", "u");
            v.created_at = at;
            v
        };

        let mut stored = [
            mk("c", Some("1.0.1"), "edit", 3_000),
            mk("b", Some("1.0.0"), "edit", 2_000),
            mk("a", None, "create", 1_000),
        ];
        // 存储层：createdAt DESC（新→旧）
        stored.sort_by_key(|v| std::cmp::Reverse(v.created_at));
        stored.reverse(); // 命令层做的事

        let versions: Vec<&str> = stored.iter().map(|v| v.version.as_str()).collect();
        assert_eq!(versions, vec!["1.0.0", "1.0.1", "1.0.2"], "应为旧→新");
    }

    /// 两个版本号相同时 diff 为空（不是错误）
    #[test]
    fn diff_of_same_version_is_empty() {
        let s = schema("f", "a+b");
        assert!(diff(&s, &s).is_empty());
    }

    /// 表达式变化能反映到 diff
    #[test]
    fn diff_detects_expression_change() {
        let a = schema("f", "a+b");
        let b = schema("f", "a*b");
        let d = diff(&a, &b);
        assert_eq!(d.fields(), ["expression"]);
        assert_eq!(d.changes[0].old_value, "a+b");
        assert_eq!(d.changes[0].new_value, "a*b");
    }
}
