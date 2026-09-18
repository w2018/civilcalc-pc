//! 内置公式库加载与来源校验。
//!
//! 源：`civilcalc-android-v2/core/source/BuiltinFormulaLoader.kt`
//!
//! ## 这是源项目「最高优先级」的工程安全底线
//!
//! 源项目规格原文：
//!
//! > **内置公式必须逐条可溯源**：每条内置公式**必须填写 `source.ref`（规范/手册的
//! > 精确条款、章节、页码或 URL）与 `source.verified = true`**。无来源的公式
//! > **不允许入库**，宁可缺失，不可错误。
//! >
//! > **内置库是人工维护资产**：`builtin_formulas.json` 的每条记录须由人工
//! > （工程师/维护者）依据权威资料逐条核对后提交，**禁止 LLM 批量生成该文件**。
//! >
//! > 导入时跑来源校验，**校验失败直接拒绝启动**，不允许降级放行。
//!
//! ## 与源项目的一处改进（非行为变更）
//!
//! 源项目用可变全局 `lastSkipped: List<String>` 上报跳过清单；
//! Rust 版把跳过清单**随返回值一起给出**（[`BuiltinLoadResult::skipped`]），
//! 避免全局可变状态（这也是源项目自身纪律所要求的）。
//! 调用方若需要"上次结果"，自行保存返回值即可 —— 行为等价。

use crate::builtin::builtin_formulas_json;
use crate::log;
use crate::schema::{validate_schema, FormulaSchema};
use crate::CoreError;

/// 加载结果。
#[derive(Debug, Clone)]
pub struct BuiltinLoadResult {
    /// 通过校验的公式（可安全入库/入索引）
    pub valid: Vec<FormulaSchema>,
    /// 未通过校验的公式，格式 `"{id}: {失败原因}"`（结构化上报用）
    pub skipped: Vec<String>,
}

impl BuiltinLoadResult {
    /// 原始条数
    pub fn total(&self) -> usize {
        self.valid.len() + self.skipped.len()
    }

    /// 是否有条目被跳过
    pub fn has_skipped(&self) -> bool {
        !self.skipped.is_empty()
    }
}

/// 日志 tag（对齐源项目输出风格）
const LOG_TAG: &str = "BuiltinFormula";

/// 从 JSON 文本加载并校验内置公式库。
///
/// ## 失败语义
///
/// - JSON 解析失败 → `CoreError::BuiltinSource { message: "内置公式 JSON 解析失败: ..." }`
/// - **全部条目校验失败** → `CoreError::BuiltinSource { message: "内置公式全部校验失败，拒绝启动" }`
/// - 部分失败 → 返回 `Ok`，失败条目在 [`BuiltinLoadResult::skipped`] 中，并写 WARN 日志
///
/// 注意：源项目的语义是"**全部失败才拒绝启动**"，部分失败会跳过并继续。
/// 内置库的**构建期测试**（见本文件 `#[cfg(test)]`）则要求 **100% 通过**，
/// 两者配合：运行时宽容、CI 严格。
pub fn load(json: &str) -> Result<BuiltinLoadResult, CoreError> {
    let list: Vec<FormulaSchema> = serde_json::from_str(json)
        .map_err(|e| CoreError::BuiltinSource { message: format!("内置公式 JSON 解析失败: {e}") })?;

    let mut valid: Vec<FormulaSchema> = Vec::with_capacity(list.len());
    let mut skipped: Vec<String> = Vec::new();

    for schema in list {
        match validate_schema(&schema) {
            Ok(()) => valid.push(schema),
            Err(e) => skipped.push(format!("{}: {}", schema.id, e.error_detail)),
        }
    }

    // 结构化上报跳过清单（源项目：`⏭️【内置公式】跳过 N/M 条校验失败:`）
    if !skipped.is_empty() {
        log::w(
            LOG_TAG,
            &format!("跳过 {}/{} 条校验失败", skipped.len(), valid.len() + skipped.len()),
            None,
        );
        for s in &skipped {
            log::w(LOG_TAG, s, None);
        }
    }

    // 全部失败 → 拒绝启动（源项目：IllegalStateException）
    if valid.is_empty() {
        return Err(CoreError::BuiltinSource {
            message: "内置公式全部校验失败，拒绝启动".to_string(),
        });
    }

    Ok(BuiltinLoadResult { valid, skipped })
}

/// 加载**编译进二进制**的内置公式库。
///
/// 源：`app/src/main/assets/builtin_formulas.json`（55 条，82 KB）
pub fn load_embedded() -> Result<BuiltinLoadResult, CoreError> {
    load(builtin_formulas_json())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 源项目 `BuiltinFormulaSourceTest.everyBuiltinPassesSchemaValidation` 的对应验收：
    /// **55 条内置公式必须 100% 通过 SchemaValidator**（源项目已把容忍度收紧为 0）
    #[test]
    fn all_embedded_builtins_pass_validation() {
        let r = load_embedded().expect("内置库应能加载");
        assert!(!r.valid.is_empty(), "内置公式列表不能为空");
        assert_eq!(
            r.skipped,
            Vec::<String>::new(),
            "有内置公式未通过校验:\n{}",
            r.skipped.join("\n")
        );
    }

    /// 源项目 `BuiltinFormulaSourceTest.everyBuiltinMustHaveVerifiedSource` 的对应验收
    #[test]
    fn all_embedded_builtins_have_verified_source() {
        let r = load_embedded().expect("内置库应能加载");
        for s in &r.valid {
            assert!(
                s.id.starts_with("builtin:"),
                "内置公式 id 必须以 builtin: 开头: {}",
                s.id
            );
            assert!(s.source.verified, "内置公式必须 verified=true: {}", s.id);
            let ref_ = s
                .source
                .ref_
                .as_ref()
                .unwrap_or_else(|| panic!("内置公式必须填写 source.ref: {}", s.id));
            assert!(!ref_.trim().is_empty(), "source.ref 不能为空: {}", s.id);
            assert!(
                crate::schema::matches_whitelist(ref_),
                "source.ref 不在白名单，需人工评审: {} -> {}",
                s.id,
                ref_
            );
            for v in &s.variables {
                assert!(
                    !v.desc.trim().is_empty(),
                    "内置公式变量必须含中文释义: {} -> {}",
                    s.id,
                    v.symbol
                );
                assert!(
                    v.unit.is_some(),
                    "内置公式变量必须填写 unit: {} -> {}",
                    s.id,
                    v.symbol
                );
            }
        }
    }

    /// 内置库规模（源项目实测 55 条）
    #[test]
    fn embedded_builtin_count_is_55() {
        let r = load_embedded().expect("内置库应能加载");
        assert_eq!(r.valid.len(), 55, "内置公式应为 55 条");
        assert_eq!(r.total(), 55);
        assert!(!r.has_skipped());
    }

    /// 源项目验收用例：「删掉某条 `ref` 后跑来源校验 → 失败」
    #[test]
    fn removing_a_ref_causes_that_formula_to_be_skipped() {
        let mut list: Vec<FormulaSchema> =
            serde_json::from_str(builtin_formulas_json()).expect("JSON 应可解析");
        assert!(!list.is_empty());

        // 篡改第一条：清空 ref（保留 verified=true → 应被校验拦下）
        let target_id = list[0].id.clone();
        list[0].source.ref_ = None;

        let tampered = serde_json::to_string(&list).expect("应可序列化");
        let r = load(&tampered).expect("仍有 54 条合法，应返回 Ok");

        assert!(r.has_skipped(), "篡改的条目应被跳过");
        assert_eq!(r.skipped.len(), 1);
        assert!(
            r.skipped[0].starts_with(&target_id),
            "跳过清单应指向被篡改的条目，实际: {}",
            r.skipped[0]
        );
        assert!(
            r.skipped[0].contains("内置公式必须填写 source.ref"),
            "失败原因应为缺少 ref，实际: {}",
            r.skipped[0]
        );
        assert_eq!(r.valid.len(), 54);
    }

    /// 篡改 `ref` 为白名单外的条款 → 同样被拦下
    #[test]
    fn non_whitelisted_ref_is_skipped() {
        let mut list: Vec<FormulaSchema> =
            serde_json::from_str(builtin_formulas_json()).expect("JSON 应可解析");
        list[0].source.ref_ = Some("GB 99999-2020".to_string());

        let tampered = serde_json::to_string(&list).expect("应可序列化");
        let r = load(&tampered).expect("应返回 Ok");
        assert_eq!(r.skipped.len(), 1);
        assert!(
            r.skipped[0].contains("不在白名单"),
            "失败原因应为白名单，实际: {}",
            r.skipped[0]
        );
    }

    /// 全部条目非法 → **拒绝启动**
    #[test]
    fn all_invalid_refuses_to_start() {
        let json = r#"[
            {
                "id": "builtin:bad1",
                "resultName": "坏公式",
                "resultSymbol": "V",
                "expression": "a+b",
                "variables": [{"symbol":"a","desc":"a","unit":"m"},{"symbol":"b","desc":"b","unit":"m"}],
                "source": {"kind": "STANDARD", "ref": null, "verified": true}
            }
        ]"#;

        let err = load(json).expect_err("全部失败应报错");
        assert_eq!(err.code(), "BUILTIN_SOURCE_ERROR");
        assert!(
            err.to_string().contains("内置公式全部校验失败，拒绝启动"),
            "实际: {err}"
        );
    }

    /// 非法 JSON → 明确报错（不 panic）
    #[test]
    fn malformed_json_reports_error() {
        let err = load("{ not json").expect_err("应报错");
        assert_eq!(err.code(), "BUILTIN_SOURCE_ERROR");
        assert!(
            err.to_string().contains("内置公式 JSON 解析失败"),
            "实际: {err}"
        );
    }

    /// 空数组 → 全部失败（0 条有效）→ 拒绝启动
    #[test]
    fn empty_list_refuses_to_start() {
        let err = load("[]").expect_err("空列表应报错");
        assert!(err.to_string().contains("拒绝启动"), "实际: {err}");
    }

    /// 部分失败时仍返回 Ok，且跳过清单格式为 `"{id}: {原因}"`
    #[test]
    fn skipped_entries_are_formatted_as_id_colon_reason() {
        let mut list: Vec<FormulaSchema> =
            serde_json::from_str(builtin_formulas_json()).expect("JSON 应可解析");
        // 让第二条的变量 desc 为空 → 命中「变量 X 缺少 desc」
        list[1].variables[0].desc = String::new();

        let tampered = serde_json::to_string(&list).expect("应可序列化");
        let r = load(&tampered).expect("应返回 Ok");

        assert_eq!(r.skipped.len(), 1);
        let entry = &r.skipped[0];
        let (id_part, reason_part) = entry.split_once(": ").expect("格式应为 id: reason");
        assert_eq!(id_part, list[1].id);
        assert!(reason_part.contains("缺少 desc"), "实际原因: {reason_part}");
    }
}
