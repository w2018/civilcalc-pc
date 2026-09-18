//! 计算历史行契约。
//!
//! 源：`civilcalc-android-v2/data/db/entities/HistoryEntity.kt`
//! 与 `core/backup/BackupModel.kt` 的 `HistoryRow`（两者字段完全一致）。
//!
//! ## 为什么是「裸 JSON 字符串」而不是解析后的类型
//!
//! 三处 JSON 字段（`formulaSnapshotJson` / `inputsJson` / `resultJson`）**刻意保持字符串**，
//! 不做成 `FormulaSchema` / `HashMap<String, f64>` / `EvalResult`：
//!
//! 1. **备份包保真**（红线）：`HistoryRow` 在源项目里就是三个 String。
//!    若 Rust 侧解析后再重新序列化，字段顺序、浮点格式（`1.0` vs `1`）、
//!    未知字段都会发生变化 —— 备份包就不再是"原样搬运"了。
//! 2. **容忍残缺数据**：AI 解析成功后立即记历史时，
//!    `inputsJson = "{}"`、`resultJson = ""`（**空串**，见
//!    `FormulaRepositoryImpl.kt:193`）。空串不是合法 JSON，强类型字段会直接反序列化失败。
//! 3. 与源项目 `BackupMapping` 的设计意图一致：「备份格式是一份对外契约，
//!    表结构加字段不该被动改变备份格式」。
//!
//! 因此本类型提供**容忍式**的便捷访问器（[`HistoryEntry::inputs`] /
//! [`HistoryEntry::result`] / [`HistoryEntry::formula_snapshot`]），
//! 解析失败一律退化为"无"，与源项目 `HistoryViewModel.kt:130` 的
//! `try { ... } catch (e: Exception) { null }` 行为一致。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::eval::EvalResult;
use super::formula::FormulaSchema;

/// 一条计算历史。
///
/// 同时充当：① SQLite `history` 表行；② 备份包 `tables.history[]` 行。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    /// 自增主键。**未入库时为 0**；插入后由 SQLite 分配。
    #[serde(default)]
    pub id: i64,

    pub formula_id: String,

    /// `FormulaSchema` 的 JSON（AI 解析时记的是**当时**的快照，可能与当前库中不一致）
    pub formula_snapshot_json: String,

    /// `Map<String, Double>` 的 JSON。**可能是 `"{}"` 或空串**
    #[serde(default)]
    pub inputs_json: String,

    /// `EvalResult` 的 JSON。**可能是空串**（AI 刚解析完、尚未计算）
    #[serde(default)]
    pub result_json: String,

    /// 该次调用的 AI 思考内容（源项目 BUG-27 凭据审计：此处**绝不含** API Key）
    #[serde(default)]
    pub thinking_content: Option<String>,

    #[serde(default)]
    pub created_at: i64,
}

impl HistoryEntry {
    /// 解析公式快照。失败返回 `Err`（快照损坏属真异常，不宜静默）。
    pub fn formula_snapshot(&self) -> Result<FormulaSchema, serde_json::Error> {
        serde_json::from_str(&self.formula_snapshot_json)
    }

    /// 解析输入值。空串 / `"{}"` / 解析失败 → **空 map**（对齐源项目 `catch { emptyMap() }`）。
    pub fn inputs(&self) -> HashMap<String, f64> {
        let s = self.inputs_json.trim();
        if s.is_empty() {
            return HashMap::new();
        }
        serde_json::from_str(s).unwrap_or_default()
    }

    /// 解析计算结果。空串 / `"null"` / 解析失败 → `None`
    /// （对齐源项目 `catch { null }`）。
    pub fn result(&self) -> Option<EvalResult> {
        let s = self.result_json.trim();
        if s.is_empty() || s == "null" {
            return None;
        }
        serde_json::from_str(s).ok()
    }

    /// 是否只有 AI 解析、还没算过（源项目「历史」列表里这类条目不可点开结果）
    pub fn is_parse_only(&self) -> bool {
        self.result().is_none()
    }

    /// 便捷构造：AI 解析成功后立即记的那条历史（无输入、无结果）。
    ///
    /// 对齐 `FormulaRepositoryImpl.kt:190`：`inputsJson = "{}"`、`resultJson = ""`。
    pub fn parse_only(schema: &FormulaSchema, thinking: Option<String>) -> Self {
        Self {
            id: 0,
            formula_id: schema.id.clone(),
            formula_snapshot_json: serde_json::to_string(schema).unwrap_or_default(),
            inputs_json: "{}".to_string(),
            result_json: String::new(),
            thinking_content: thinking.filter(|s| !s.is_empty()),
            created_at: crate::now_ms(),
        }
    }

    /// 便捷构造：**计算完成后**记的那条历史（有输入、有结果）。
    ///
    /// ## ⚠️ 只在"确认计算"时调用
    ///
    /// 源项目行为：不是每次预览求值都写历史 —— 否则用户拖滑块试参数
    /// 会瞬间灌满历史。编辑中的实时预览走 `eval_schema`（**不写历史**）。
    ///
    /// ## 空值归一
    ///
    /// - `thinking` 为 `None` 或**空白串** → 存 `None`
    ///   （对齐源项目 `thinkingContent?.ifBlank { null }`）
    pub fn from_eval(
        schema: &FormulaSchema,
        inputs: &HashMap<String, f64>,
        result: &EvalResult,
        thinking: Option<String>,
    ) -> Self {
        Self {
            id: 0,
            formula_id: schema.id.clone(),
            formula_snapshot_json: serde_json::to_string(schema).unwrap_or_default(),
            inputs_json: serde_json::to_string(inputs).unwrap_or_else(|_| "{}".to_string()),
            result_json: serde_json::to_string(result).unwrap_or_default(),
            thinking_content: thinking.filter(|s| !s.trim().is_empty()),
            created_at: crate::now_ms(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::EvalOutput;

    fn minimal_schema_json() -> String {
        // 用最小可解析的 FormulaSchema JSON（字段与 schema/formula.rs 对齐）
        // ⚠️ SourceKind 的 serde 表示是 SCREAMING_SNAKE（对齐源项目 Kotlin 枚举名）
        serde_json::json!({
            "id": "usr:1",
            "resultName": "结果",
            "resultSymbol": "y",
            "expression": "a+b",
            "variables": [],
            "constants": {},
            "domain": "通用",
            "source": { "kind": "CUSTOM" },
            "schemaVersion": 3
        })
        .to_string()
    }

    fn entry() -> HistoryEntry {
        HistoryEntry {
            id: 7,
            formula_id: "usr:1".to_string(),
            formula_snapshot_json: minimal_schema_json(),
            inputs_json: r#"{"a":1.0,"b":2.0}"#.to_string(),
            result_json: r#"{"primary":3.0,"outputs":[],"branches":[],"steps":[],"stepResults":[],"warnings":[]}"#
                .to_string(),
            thinking_content: Some("思考".to_string()),
            created_at: 1234,
        }
    }

    #[test]
    fn serde_keys_are_camel_case_matching_backup_row() {
        let json = serde_json::to_value(entry()).unwrap();
        for key in [
            "id",
            "formulaId",
            "formulaSnapshotJson",
            "inputsJson",
            "resultJson",
            "thinkingContent",
            "createdAt",
        ] {
            assert!(json.get(key).is_some(), "缺少键: {key}（须与 HistoryRow 一致）");
        }
        // 不得出现 snake_case 泄漏
        assert!(json.get("formula_id").is_none());
        assert!(json.get("created_at").is_none());
    }

    #[test]
    fn roundtrip_is_stable() {
        let e = entry();
        let s = serde_json::to_string(&e).unwrap();
        let back: HistoryEntry = serde_json::from_str(&s).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn inputs_tolerates_empty_and_invalid() {
        let mut e = entry();
        e.inputs_json = String::new();
        assert!(e.inputs().is_empty());

        e.inputs_json = "{}".to_string();
        assert!(e.inputs().is_empty());

        e.inputs_json = "not json".to_string();
        assert!(e.inputs().is_empty(), "解析失败应退化为空 map，不 panic");

        e.inputs_json = r#"{"x":2.5}"#.to_string();
        assert_eq!(e.inputs().get("x"), Some(&2.5));
    }

    #[test]
    fn result_tolerates_empty_null_and_invalid() {
        let mut e = entry();
        e.result_json = String::new();
        assert!(e.result().is_none(), "空串应视为无结果（AI 解析后未计算）");

        e.result_json = "null".to_string();
        assert!(e.result().is_none());

        e.result_json = "{ 坏".to_string();
        assert!(e.result().is_none());

        e.result_json = r#"{"primary":1.5}"#.to_string();
        assert_eq!(e.result().unwrap().primary, 1.5);
    }

    #[test]
    fn parse_only_marks_no_result() {
        let schema: FormulaSchema = serde_json::from_str(&minimal_schema_json()).unwrap();
        let e = HistoryEntry::parse_only(&schema, Some(String::new()));

        assert_eq!(e.formula_id, "usr:1");
        assert_eq!(e.inputs_json, "{}");
        assert_eq!(e.result_json, "");
        assert!(e.is_parse_only());
        // 空白思考内容归一为 None（对齐 `thinkingContent?.ifBlank { null }`）
        assert_eq!(e.thinking_content, None);
        assert!(e.formula_snapshot().is_ok());
    }

    #[test]
    fn parse_only_keeps_real_thinking() {
        let schema: FormulaSchema = serde_json::from_str(&minimal_schema_json()).unwrap();
        let e = HistoryEntry::parse_only(&schema, Some("推理过程".to_string()));
        assert_eq!(e.thinking_content.as_deref(), Some("推理过程"));
    }

    #[test]
    fn missing_optional_fields_default() {
        // 旧备份可能缺字段，须能反序列化
        let json = r#"{"formulaId":"usr:1","formulaSnapshotJson":"{}"}"#;
        let e: HistoryEntry = serde_json::from_str(json).unwrap();
        assert_eq!(e.id, 0);
        assert_eq!(e.inputs_json, "");
        assert_eq!(e.created_at, 0);
    }

    // ---------------- from_eval（计算完成后写历史） ----------------

    fn schema_and_result() -> (FormulaSchema, EvalResult, HashMap<String, f64>) {
        let schema: FormulaSchema = serde_json::from_str(&minimal_schema_json()).unwrap();
        let result = EvalResult::primary_only(3.0);
        let inputs = HashMap::from([("a".to_string(), 1.0), ("b".to_string(), 2.0)]);
        (schema, result, inputs)
    }

    #[test]
    fn from_eval_records_inputs_and_result() {
        let (schema, result, inputs) = schema_and_result();
        let e = HistoryEntry::from_eval(&schema, &inputs, &result, Some("思考".into()));

        assert_eq!(e.formula_id, "usr:1");
        assert!(!e.is_parse_only(), "有结果 → 不是 parse-only");
        assert_eq!(e.inputs().get("a"), Some(&1.0));
        assert_eq!(e.result().unwrap().primary, 3.0);
        assert_eq!(e.thinking_content.as_deref(), Some("思考"));
        assert!(e.formula_snapshot().is_ok());
    }

    /// 空白思考内容归一为 `None`（对齐 `thinkingContent?.ifBlank { null }`）
    #[test]
    fn from_eval_normalizes_blank_thinking() {
        let (schema, result, inputs) = schema_and_result();
        for blank in [None, Some(String::new()), Some("   ".to_string()), Some("\n\t".to_string())] {
            let e = HistoryEntry::from_eval(&schema, &inputs, &result, blank);
            assert_eq!(e.thinking_content, None, "空白应归一为 None");
        }
    }

    #[test]
    fn from_eval_keeps_empty_inputs_as_empty_object() {
        let (schema, result, _) = schema_and_result();
        let e = HistoryEntry::from_eval(&schema, &HashMap::new(), &result, None);
        assert_eq!(e.inputs_json, "{}", "空输入应序列化为 {{}} 而非空串");
        assert!(e.inputs().is_empty());
    }

    /// 多输出/分支/警告都要原样进 `resultJson`
    #[test]
    fn from_eval_preserves_rich_result() {
        let (schema, _, inputs) = schema_and_result();
        let mut result = EvalResult::primary_only(10.0);
        result.outputs.push(EvalOutput {
            symbol: Some("X1".into()),
            value: 4.0,
        });
        result.warnings.push("条件分支不满足".into());

        let e = HistoryEntry::from_eval(&schema, &inputs, &result, None);
        let back = e.result().unwrap();
        assert_eq!(back.outputs.len(), 1);
        assert_eq!(back.outputs[0].symbol.as_deref(), Some("X1"));
        assert_eq!(back.warnings, vec!["条件分支不满足"]);
    }
}
