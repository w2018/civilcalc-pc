//! 版本链管理（差异比对 + head 解析）。
//!
//! 源：`civilcalc-android-v2/core/version/VersionChainManager.kt`
//!
//! ## 6 维 diff（维度、顺序、标签逐一对齐源项目）
//!
//! | # | `field` | `label` | 比较对象 |
//! |---|---|---|---|
//! | 1 | `expression` | 主表达式 | `expression` |
//! | 2 | `altExpressions` | 备选表达式 | `altExpressions` |
//! | 3 | `variables` | 变量列表 | `variables` |
//! | 4 | `source.ref` | 来源引用 | `source.ref`（空 → `"无"`） |
//! | 5 | `source.verified` | 核验状态 | `source.verified` |
//! | 6 | `constants` | 常量 | `constants` |
//!
//! ## ⚠️ 一处刻意的渲染偏差（语义不变）
//!
//! 源项目用 Kotlin 的 `toString()` 渲染复杂值，得到的是 **data class 的调试串**：
//!
//! ```text
//! [FormulaVar(symbol=x, desc=宽度, unit=mm, ...), ...]
//! {E=206000.0, fy=360.0}
//! ```
//!
//! Rust 侧改用 **JSON**（`serde_json::to_string`）：
//!
//! ```text
//! [{"symbol":"x","desc":"宽度","unit":"mm",...}]
//! {"E":206000.0,"fy":360.0}
//! ```
//!
//! 理由：
//! 1. 这些值是**给人看的差异说明**，JSON 比 data class 调试串更易读
//! 2. Kotlin 的 `toString()` 依赖字段声明顺序与类型名，**在 Rust 里逐字复刻极其脆弱**，
//!    且一旦源项目给 data class 加字段就会漂移
//! 3. JSON 可被前端二次解析（若将来要做结构化 diff 展示）
//!
//! **被比较的维度、顺序、`label` 文案、以及 `expression` / `source.ref` /
//! `source.verified` 三个标量的渲染方式，全部与源项目一致。**
//!
//! ## head 解析（ADR-014）
//!
//! 源项目 `FormulaVersion` **没有 head 字段**。PC 端把 head 存在
//! `user_formulas.headVersion` 列，`NULL` 表示"最新版本即 head"。
//! 解析逻辑见 [`resolve_head`]。

use serde::{Deserialize, Serialize};

use crate::schema::FormulaSchema;
use crate::version::FormulaVersion;

/// 一个维度的变化。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffItem {
    /// 字段路径（如 `"source.ref"`）
    pub field: String,
    /// 中文标签（直接给用户看）
    pub label: String,
    pub old_value: String,
    pub new_value: String,
}

/// 两个版本之间的差异。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionDiff {
    pub changes: Vec<DiffItem>,
}

impl VersionDiff {
    /// 无差异
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// 差异维度数
    pub fn len(&self) -> usize {
        self.changes.len()
    }

    /// 变化的字段路径（便于测试与日志）
    pub fn fields(&self) -> Vec<&str> {
        self.changes.iter().map(|c| c.field.as_str()).collect()
    }
}

/// diff 的维度总数（源项目固定 6 维）
pub const DIFF_DIMENSION_COUNT: usize = 6;

/// 比较两个 schema，返回 6 维差异。1:1 对齐源项目 `VersionChainManager.diff`。
pub fn diff(old: &FormulaSchema, new: &FormulaSchema) -> VersionDiff {
    let mut changes = Vec::new();

    // 1. 主表达式（标量，直接比较）
    if old.expression != new.expression {
        changes.push(DiffItem {
            field: "expression".to_string(),
            label: "主表达式".to_string(),
            old_value: old.expression.clone(),
            new_value: new.expression.clone(),
        });
    }

    // 2. 备选表达式
    if old.alt_expressions != new.alt_expressions {
        changes.push(DiffItem {
            field: "altExpressions".to_string(),
            label: "备选表达式".to_string(),
            old_value: json_of(&old.alt_expressions),
            new_value: json_of(&new.alt_expressions),
        });
    }

    // 3. 变量列表
    if old.variables != new.variables {
        changes.push(DiffItem {
            field: "variables".to_string(),
            label: "变量列表".to_string(),
            old_value: json_of(&old.variables),
            new_value: json_of(&new.variables),
        });
    }

    // 4. 来源引用（空 → "无"，对齐源项目 `?: "无"`）
    if old.source.ref_ != new.source.ref_ {
        changes.push(DiffItem {
            field: "source.ref".to_string(),
            label: "来源引用".to_string(),
            old_value: old.source.ref_.clone().unwrap_or_else(|| "无".to_string()),
            new_value: new.source.ref_.clone().unwrap_or_else(|| "无".to_string()),
        });
    }

    // 5. 核验状态（布尔 → "true"/"false"，与 Kotlin `Boolean.toString()` 一致）
    if old.source.verified != new.source.verified {
        changes.push(DiffItem {
            field: "source.verified".to_string(),
            label: "核验状态".to_string(),
            old_value: old.source.verified.to_string(),
            new_value: new.source.verified.to_string(),
        });
    }

    // 6. 常量
    if old.constants != new.constants {
        changes.push(DiffItem {
            field: "constants".to_string(),
            label: "常量".to_string(),
            old_value: json_of(&old.constants),
            new_value: json_of(&new.constants),
        });
    }

    VersionDiff { changes }
}

/// 把可序列化的值渲染成展示串。失败时退化为 `"?"`（**不 panic**）。
fn json_of<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "?".to_string())
}

// =============================================================================
// 版本链
// =============================================================================

/// 把版本列表按 `createdAt` 升序排列（**旧 → 新**）。
///
/// 对齐源项目 `VersionDao.getVersionChain` 的 `ORDER BY createdAt ASC`。
/// 同一毫秒的用 `version` 号兜底（见下）。
///
/// ⚠️ 返回借用而非克隆，调用方按需 clone。
pub fn chain_sorted(versions: &[FormulaVersion]) -> Vec<&FormulaVersion> {
    let mut out: Vec<&FormulaVersion> = versions.iter().collect();
    out.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            // tiebreaker：毫秒相同时按版本号比（`1.0.2` < `1.0.10` 需按数字比）
            .then_with(|| compare_version(&a.version, &b.version))
    });
    out
}

/// 按语义化版本号比较（逐段数值比较，段数不同时缺位补 0）。
///
/// `"1.0.2"` < `"1.0.10"`（字符串比较会得出相反结论）。
pub fn compare_version(a: &str, b: &str) -> std::cmp::Ordering {
    let seg = |s: &str| -> Vec<i64> {
        s.split('.')
            .map(|p| p.trim().parse::<i64>().unwrap_or(0))
            .collect()
    };
    let (mut va, mut vb) = (seg(a), seg(b));
    let n = va.len().max(vb.len());
    va.resize(n, 0);
    vb.resize(n, 0);
    va.cmp(&vb)
}

/// 解析当前生效版本（head）。
///
/// ## 规则（ADR-014）
///
/// | `stored_head` | 结果 |
/// |---|---|
/// | `Some("1.0.2")` 且链中存在 | 该版本 |
/// | `Some("9.9.9")` 但链中**不存在**（数据不一致） | **退化到最新版本** + 记 WARN |
/// | `None` | **最新版本**（与源项目当前行为一致） |
/// | 空链 | `None` |
///
/// 「最新」= [`chain_sorted`] 的最后一个。
pub fn resolve_head<'a>(
    versions: &'a [FormulaVersion],
    stored_head: Option<&str>,
) -> Option<&'a FormulaVersion> {
    if versions.is_empty() {
        return None;
    }
    let chain = chain_sorted(versions);

    if let Some(want) = stored_head {
        match chain.iter().find(|v| v.version == want) {
            Some(v) => return Some(v),
            None => {
                // 数据不一致（版本被删 / head 写错）：不报错，退化到最新，但要留痕
                crate::log::w(
                    "VersionChain",
                    &format!("headVersion={want} 在版本链中不存在，退化为最新版本"),
                    None,
                );
            }
        }
    }

    chain.last().copied()
}

/// 计算"下一个版本号"及其父版本，用于新建版本。
///
/// 便捷包装：把链交给它，得到 `(parent, next)`。
/// 空链 → `(None, "1.0.0")`。
pub fn next_version_for(versions: &[FormulaVersion]) -> (Option<String>, String) {
    let parent = chain_sorted(versions)
        .last()
        .map(|v| v.version.clone());
    let next = crate::version::next_version(parent.as_deref());
    (parent, next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{AltExpression, FormulaSource, FormulaVar};
    use std::collections::HashMap;

    fn base_schema() -> FormulaSchema {
        FormulaSchema {
            id: "usr:1".into(),
            result_name: "结果".into(),
            result_symbol: "y".into(),
            result_unit: Some("mm".into()),
            result_outputs: vec![],
            expression: "a+b".into(),
            source_equations: vec![],
            alt_expressions: vec![],
            constants: HashMap::new(),
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
            schema_version: crate::schema::CURRENT_SCHEMA_VERSION,
            created_at: 1000,
            updated_at: 1000,
        }
    }

    /// `FormulaVar` 没有 `Default` 派生，测试里统一走这个助手
    fn var(symbol: &str, desc: &str, unit: &str) -> FormulaVar {
        FormulaVar {
            symbol: symbol.to_string(),
            desc: desc.to_string(),
            unit: Some(unit.to_string()),
            default: None,
            min: None,
            max: None,
            required: true,
        }
    }

    fn version_of(id: &str, ver: &str, created_at: i64) -> FormulaVersion {
        let mut s = base_schema();
        s.id = id.to_string();
        FormulaVersion {
            formula_id: id.to_string(),
            version: ver.to_string(),
            parent_version: None,
            schema_json: serde_json::to_string(&s).unwrap(),
            change_type: "edit".into(),
            change_log: String::new(),
            editor: "user".into(),
            created_at,
            verified: false,
        }
    }

    // ---------------- diff ----------------

    #[test]
    fn identical_schemas_have_no_diff() {
        let a = base_schema();
        let d = diff(&a, &a.clone());
        assert!(d.is_empty());
        assert_eq!(d.len(), 0);
        assert_eq!(d.fields(), Vec::<&str>::new());
    }

    #[test]
    fn expression_change_is_dimension_1() {
        let a = base_schema();
        let mut b = a.clone();
        b.expression = "a*b".into();

        let d = diff(&a, &b);
        assert_eq!(d.len(), 1);
        assert_eq!(d.changes[0].field, "expression");
        assert_eq!(d.changes[0].label, "主表达式");
        assert_eq!(d.changes[0].old_value, "a+b");
        assert_eq!(d.changes[0].new_value, "a*b");
    }

    #[test]
    fn variables_change_is_json_rendered() {
        let a = base_schema();
        let mut b = a.clone();
        b.variables = vec![var("x", "宽度", "mm")];

        let d = diff(&a, &b);
        assert_eq!(d.changes.len(), 1);
        assert_eq!(d.changes[0].field, "variables");
        assert_eq!(d.changes[0].label, "变量列表");
        assert_eq!(d.changes[0].old_value, "[]");
        assert!(d.changes[0].new_value.contains("\"symbol\":\"x\""));
        assert!(d.changes[0].new_value.contains("宽度"));
    }

    #[test]
    fn source_ref_none_renders_as_wu() {
        let a = base_schema();
        let mut b = a.clone();
        b.source.ref_ = Some("GB 50010-2010 第6.2.10条".into());

        let d = diff(&a, &b);
        assert_eq!(d.changes.len(), 1);
        assert_eq!(d.changes[0].field, "source.ref");
        assert_eq!(d.changes[0].label, "来源引用");
        assert_eq!(d.changes[0].old_value, "无", "None 应渲染为「无」");
        assert_eq!(d.changes[0].new_value, "GB 50010-2010 第6.2.10条");
    }

    #[test]
    fn verified_change_is_true_false() {
        let mut a = base_schema();
        a.source.verified = false;
        let mut b = a.clone();
        b.source.verified = true;

        let d = diff(&a, &b);
        assert_eq!(d.changes.len(), 1);
        assert_eq!(d.changes[0].field, "source.verified");
        assert_eq!(d.changes[0].label, "核验状态");
        assert_eq!(d.changes[0].old_value, "false");
        assert_eq!(d.changes[0].new_value, "true");
    }

    #[test]
    fn constants_change_is_json_rendered() {
        let a = base_schema();
        let mut b = a.clone();
        b.constants = HashMap::from([("E".to_string(), 206000.0)]);

        let d = diff(&a, &b);
        assert_eq!(d.changes[0].field, "constants");
        assert_eq!(d.changes[0].label, "常量");
        assert_eq!(d.changes[0].old_value, "{}");
        assert!(d.changes[0].new_value.contains("206000"));
    }

    #[test]
    fn alt_expressions_change() {
        let a = base_schema();
        let mut b = a.clone();
        b.alt_expressions = vec![AltExpression {
            label: "分支".into(),
            expression: "a-b".into(),
            condition: Some("a>b".into()),
        }];

        let d = diff(&a, &b);
        assert_eq!(d.changes[0].field, "altExpressions");
        assert_eq!(d.changes[0].label, "备选表达式");
        assert!(d.changes[0].new_value.contains("a-b"));
    }

    /// 多维度同时变化时，**顺序固定为源项目的 6 维顺序**
    #[test]
    fn multiple_changes_keep_source_order() {
        let a = base_schema();
        let mut b = a.clone();
        b.expression = "a*b".into(); // 1
        b.alt_expressions = vec![AltExpression {
            label: "L".into(),
            expression: "a/b".into(),
            condition: None,
        }]; // 2
        b.variables = vec![var("x", "d", "u")]; // 3
        b.source.ref_ = Some("ref".into()); // 4
        b.source.verified = true; // 5
        b.constants = HashMap::from([("k".to_string(), 1.0)]); // 6

        let d = diff(&a, &b);
        assert_eq!(
            d.fields(),
            vec![
                "expression",
                "altExpressions",
                "variables",
                "source.ref",
                "source.verified",
                "constants"
            ]
        );
        assert_eq!(d.len(), DIFF_DIMENSION_COUNT);
    }

    /// 不在 6 维里的字段（如 `resultName` / `domain`）变化**不产生差异**
    /// （对齐源项目：只比这 6 维）
    #[test]
    fn fields_outside_six_dimensions_are_ignored() {
        let a = base_schema();
        let mut b = a.clone();
        b.result_name = "改了名字".into();
        b.domain = "结构".into();
        b.tags = vec!["新标签".into()];
        b.reference_basis = Some("依据".into());
        b.design_notes = Some("备注".into());

        assert!(diff(&a, &b).is_empty(), "6 维之外的字段不应进 diff");
    }

    #[test]
    fn diff_serde_shape_is_camel_case() {
        let a = base_schema();
        let mut b = a.clone();
        b.expression = "z".into();
        let v = serde_json::to_value(diff(&a, &b)).unwrap();
        assert!(v["changes"][0].get("oldValue").is_some());
        assert!(v["changes"][0].get("newValue").is_some());
        assert!(v["changes"][0].get("old_value").is_none(), "不得泄漏 snake_case");
    }

    // ---------------- 版本号比较 ----------------

    #[test]
    fn version_compare_is_numeric_not_lexicographic() {
        use std::cmp::Ordering;
        assert_eq!(compare_version("1.0.2", "1.0.10"), Ordering::Less);
        assert_eq!(compare_version("1.0.10", "1.0.2"), Ordering::Greater);
        assert_eq!(compare_version("1.0.0", "1.0.0"), Ordering::Equal);
        // 段数不同按缺位补 0
        assert_eq!(compare_version("1.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_version("1", "1.0.1"), Ordering::Less);
    }

    // ---------------- 链排序 ----------------

    #[test]
    fn chain_sorted_is_ascending_by_created_at() {
        let vs = vec![
            version_of("f", "1.0.2", 3000),
            version_of("f", "1.0.0", 1000),
            version_of("f", "1.0.1", 2000),
        ];
        let chain = chain_sorted(&vs);
        let nums: Vec<&str> = chain.iter().map(|v| v.version.as_str()).collect();
        assert_eq!(nums, vec!["1.0.0", "1.0.1", "1.0.2"]);
    }

    #[test]
    fn chain_sorted_tiebreaks_by_version_number() {
        // 同一毫秒 → 按版本号数值升序（而非字符串序）
        let vs = vec![
            version_of("f", "1.0.10", 5000),
            version_of("f", "1.0.2", 5000),
        ];
        let chain = chain_sorted(&vs);
        let nums: Vec<&str> = chain.iter().map(|v| v.version.as_str()).collect();
        assert_eq!(nums, vec!["1.0.2", "1.0.10"]);
    }

    // ---------------- head 解析 ----------------

    #[test]
    fn head_none_means_latest() {
        let vs = vec![
            version_of("f", "1.0.0", 1000),
            version_of("f", "1.0.1", 2000),
        ];
        let h = resolve_head(&vs, None).unwrap();
        assert_eq!(h.version, "1.0.1", "NULL 时退化为最新版本");
    }

    #[test]
    fn head_explicit_wins() {
        let vs = vec![
            version_of("f", "1.0.0", 1000),
            version_of("f", "1.0.1", 2000),
            version_of("f", "1.0.2", 3000),
        ];
        let h = resolve_head(&vs, Some("1.0.1")).unwrap();
        assert_eq!(h.version, "1.0.1", "显式 head 优先于最新");
    }

    #[test]
    fn head_pointing_to_missing_version_falls_back_to_latest() {
        let vs = vec![
            version_of("f", "1.0.0", 1000),
            version_of("f", "1.0.1", 2000),
        ];
        // 数据不一致：head 指向不存在的版本
        let h = resolve_head(&vs, Some("9.9.9")).unwrap();
        assert_eq!(h.version, "1.0.1", "应退化为最新，而不是 None");
    }

    #[test]
    fn head_on_empty_chain_is_none() {
        assert!(resolve_head(&[], None).is_none());
        assert!(resolve_head(&[], Some("1.0.0")).is_none());
    }

    #[test]
    fn head_single_version() {
        let vs = vec![version_of("f", "1.0.0", 1000)];
        assert_eq!(resolve_head(&vs, None).unwrap().version, "1.0.0");
        assert_eq!(resolve_head(&vs, Some("1.0.0")).unwrap().version, "1.0.0");
    }

    // ---------------- 下一个版本 ----------------

    #[test]
    fn next_version_for_empty_chain_is_initial() {
        let (parent, next) = next_version_for(&[]);
        assert_eq!(parent, None);
        assert_eq!(next, "1.0.0");
    }

    #[test]
    fn next_version_for_chain_uses_latest_as_parent() {
        let vs = vec![
            version_of("f", "1.0.0", 1000),
            version_of("f", "1.0.1", 2000),
        ];
        let (parent, next) = next_version_for(&vs);
        assert_eq!(parent.as_deref(), Some("1.0.1"));
        assert_eq!(next, "1.0.2");
    }

    /// 链里只有非连续版本时，以**createdAt 最大**的那条为父
    #[test]
    fn next_version_uses_latest_even_if_numbers_gap() {
        let vs = vec![
            version_of("f", "1.0.0", 1000),
            version_of("f", "3.2.9", 5000),
        ];
        let (parent, next) = next_version_for(&vs);
        assert_eq!(parent.as_deref(), Some("3.2.9"));
        assert_eq!(next, "3.2.10");
    }
}
