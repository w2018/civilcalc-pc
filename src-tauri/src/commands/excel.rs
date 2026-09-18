//! 组 12a：Excel 公式转换 / 校验 / 迁移（3 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `excel_convert` | `schema`, `mode`, `inputs` | `ExcelResult` |
//! | `excel_validate` | `schema`, `excelFormula` | `ValidationResult` |
//! | `excel_migrate` | `schema` | `FormulaSchema` |
//!
//! ## 两种模式（`mode`）
//!
//! | 模式 | 公式形态 | 用途 |
//! |---|---|---|
//! | `"ref"` | `=PI()*A1^2*B1` | **贴进 Excel 复用**：变量留在第 1 行，改数值即重算 |
//! | `"value"` | `=PI()*2^2*3` | **写进计算书**：数值已代入，读者无需回头查参数表 |
//!
//! 两者是**同一套转换**的两个视图 —— `convert` 一次调用同时产出，
//! 命令层只是按 `mode` 挑一套返回。
//!
//! ## ⚠️ 三个诊断字段是补出来的，不是 `convert` 的原始输出
//!
//! `convert` 有两处「静默」行为，都会让用户拿到一份**看不出来有问题**的公式：
//!
//! | 情况 | `convert` 的行为 | 风险 |
//! |---|---|---|
//! | 表达式引用了未声明的符号 | 保留原名（不丢弃、不报错） | Excel 里一个 `#NAME?`，用户不知哪来的 |
//! | **值模式下某变量没给数值** | 代入 `0`（**无告警**） | 少填一个参数 → 结果照算，但**是错的** |
//!
//! 所以这里补三个字段（`ref` 模式只填 `unresolvedNames`）：
//!
//! - `unresolvedNames`：公式里残留的、未在参数表中声明的标识符
//! - `unresolvedRefs`：值模式下仍残留的单元格引用（数值**非法**时回退所致，如 `NaN`）
//! - `missingInputs`：值模式下**没提供数值**的变量名 —— 前端应显著提示
//!   （这是工程计算里最容易出事的一类错误：漏填参数但结果看起来正常）
//!
//! ## 无状态
//!
//! 三个命令都是**纯函数**（不碰数据库、不碰偏好），因此不接收 `State`。
//! `excel_migrate` 只返回迁移后的 schema，**不落库** ——
//! 落库由 `formula_save` 负责，避免一个命令改两处状态。

use crate::error::{CmdResult, CommandError};
use civilcalc_core::excel::{
    migrate_schema_excel_fields, scan_residuals, validate_excel_expression, ExcelFormulaConverter,
    ParamCell, ValidationResult,
};
use civilcalc_core::schema::{FormulaSchema, FunctionDoc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Excel 契约里「变量所在行」——第 1 行横向排列（`A1`/`B1`/…）
const VARIABLE_ROW: u32 = 1;

/// Excel 契约里「步骤结果起始行」——A 列纵向，从第 2 行起（`A2`/`A3`/…）
const STEP_START_ROW: u32 = 2;

/// 转换模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExcelMode {
    /// 单元格引用式（`=PI()*A1^2*B1`）
    Ref,
    /// 带数值式（`=PI()*2^2*3`）
    Value,
}

/// 一个多结果的 Excel 视图。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExcelOutputView {
    /// 结果符号（分号段的赋值目标）
    pub symbol: String,
    /// 结果名称（取自 `resultOutputs`，缺失时回退符号）
    pub name: String,
    /// 按 `mode` 选出的公式
    pub formula: String,
}

/// 一个分支的 Excel 视图。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExcelBranchView {
    /// 分支标签（重名时带序号后缀，与转换器一致）
    pub label: String,
    /// 按 `mode` 选出的公式
    pub formula: String,
    /// 适用条件（原样透传，供前端展示）
    pub condition: Option<String>,
}

/// `excel_convert` 的返回。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExcelResult {
    /// 请求的模式（回显，前端可据此确认结果对应哪一套）
    pub mode: ExcelMode,
    /// 主公式（多结果时 = **最后一段**，与引擎 `primary` 口径一致）
    pub main_formula: String,
    /// 多结果逐条（单结果时为空）
    pub outputs: Vec<ExcelOutputView>,
    /// 分支逐条
    pub branches: Vec<ExcelBranchView>,
    /// 参数对照表：变量 → 单元格 / 单位 / 释义
    pub param_mapping: Vec<ParamCell>,
    /// 用到的 Excel 函数（含中文说明，前端可做"函数清单"）
    pub used_functions: Vec<FunctionDoc>,
    /// 全量告警（编译失败 / 数值非法 / 引用无法映射 …）
    pub warnings: Vec<String>,
    /// 值模式下仍残留的单元格引用（说明该变量的数值**非法**，如 `NaN`）
    pub unresolved_refs: Vec<String>,
    /// 公式里残留的、未在参数表中声明的标识符
    pub unresolved_names: Vec<String>,
    /// 值模式下**没提供数值**的变量名（它们在公式里被当成 `0`）
    ///
    /// ⚠️ `convert` 对缺失输入按 `0` 处理且**不告警** —— 漏填参数会让结果照算但错掉。
    /// 前端必须据此给出显著提示。
    pub missing_inputs: Vec<String>,
    /// 变量所在行（契约：第 1 行横向）
    pub variable_row: u32,
    /// 步骤结果起始行（契约：A 列纵向，从第 2 行起）
    pub step_start_row: u32,
}

/// Excel 公式转换（两模式）。
///
/// `inputs` 在 `ref` 模式下**被忽略**（引用式不含数值）；
/// `value` 模式下缺某个变量的值 → 该变量回退成单元格引用，并进 `unresolvedRefs`。
#[tauri::command]
pub fn excel_convert(
    schema: FormulaSchema,
    mode: ExcelMode,
    inputs: HashMap<String, f64>,
) -> CmdResult<ExcelResult> {
    // 引用式只用到 cell_ref_map；值式才需要参数值。
    // 但 `convert` 一次调用同时产出两套，所以两种模式都传 inputs ——
    // 这样 `warnings` 里「输入不是有效数值」这类信息在 ref 模式下也能看到（用户提前知道）。
    //
    // ⚠️ 转换器收的是**字符串**参数表（它自己走 `number_formatter::parse`，
    // 能处理千分位、全角数字、尾随单位）—— 不要把 f64 直接格式化后绕开那层解析。
    let param_values: HashMap<String, String> = inputs
        .iter()
        .map(|(k, v)| (k.clone(), v.to_string()))
        .collect();
    let converted = ExcelFormulaConverter::new().convert(&schema, &param_values);

    let (main_formula, outputs, branches) = match mode {
        ExcelMode::Ref => (
            converted.cell_formulas.get("主公式").cloned(),
            converted
                .output_formulas
                .iter()
                .map(|o| ExcelOutputView {
                    symbol: o.symbol.clone(),
                    name: o.name.clone(),
                    formula: o.cell_formula.clone(),
                })
                .collect::<Vec<_>>(),
            collect_branches(&schema, &converted.cell_formulas),
        ),
        ExcelMode::Value => (
            converted.value_formulas.get("主公式").cloned(),
            converted
                .output_formulas
                .iter()
                .map(|o| ExcelOutputView {
                    symbol: o.symbol.clone(),
                    name: o.name.clone(),
                    formula: o.value_formula.clone(),
                })
                .collect::<Vec<_>>(),
            collect_branches(&schema, &converted.value_formulas),
        ),
    };

    // 主公式理论上总存在；缺失时给空串而不是报错 ——
    // 前端拿到空公式会显示"暂无"，比抛错更符合「面板打开失败」的场景。
    let main_formula = main_formula.unwrap_or_default();

    // ── 缺输入诊断（仅值模式）──
    // 引用模式下数值不参与公式，缺输入无所谓
    let missing_inputs: Vec<String> = if mode == ExcelMode::Value {
        let mut seen: HashSet<&str> = HashSet::new();
        schema
            .variables
            .iter()
            .map(|v| v.symbol.as_str())
            .filter(|sym| !inputs.contains_key(*sym) && seen.insert(sym))
            .map(str::to_string)
            .collect()
    } else {
        Vec::new()
    };

    // ── 残留符号诊断 ──
    let mut all_formulas: Vec<&str> = vec![main_formula.as_str()];
    all_formulas.extend(outputs.iter().map(|o| o.formula.as_str()));
    all_formulas.extend(branches.iter().map(|b| b.formula.as_str()));

    let mut unresolved_refs: Vec<String> = Vec::new();
    let mut unresolved_names: Vec<String> = Vec::new();
    let mut seen_refs: HashSet<String> = HashSet::new();
    let mut seen_names: HashSet<String> = HashSet::new();

    for formula in all_formulas {
        let scan = scan_residuals(formula);
        // ref 模式下单元格引用是**预期**的，报出来只会是噪音
        if mode == ExcelMode::Value {
            for r in scan.cell_refs {
                if seen_refs.insert(r.clone()) {
                    unresolved_refs.push(r);
                }
            }
        }
        for n in scan.names {
            if seen_names.insert(n.clone()) {
                unresolved_names.push(n);
            }
        }
    }

    Ok(ExcelResult {
        mode,
        main_formula,
        outputs,
        branches,
        param_mapping: converted.param_mapping,
        used_functions: converted.used_functions,
        warnings: converted.warnings,
        unresolved_refs,
        unresolved_names,
        missing_inputs,
        variable_row: VARIABLE_ROW,
        step_start_row: STEP_START_ROW,
    })
}

/// 按 `mode` 取出分支公式。
///
/// ⚠️ **不能用「`cell_formulas` 里除主公式外的所有键」** ——
/// 那是 `BTreeMap`，分支 `分支` / `分支2` / `分支10` 会按**字典序**排成
/// `分支` < `分支10` < `分支2`，与用户看到的顺序不符。
///
/// 这里按 `schema.alt_expressions` 的顺序重建标签，标签生成规则与
/// `ExcelFormulaConverter::convert` 里的 BUG-13 处理**逐行一致**
/// （空标签 → `分支`；重名 → 追加序号）。
fn collect_branches(
    schema: &FormulaSchema,
    formulas: &std::collections::BTreeMap<String, String>,
) -> Vec<ExcelBranchView> {
    let mut label_count: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::new();

    for alt in &schema.alt_expressions {
        // 空表达式被转换器跳过（不进 map）
        if alt.expression.trim().is_empty() {
            continue;
        }

        let base = if alt.label.trim().is_empty() {
            "分支".to_string()
        } else {
            alt.label.clone()
        };
        let count = label_count.entry(base.clone()).or_insert(0);
        *count += 1;
        let label = if *count == 1 {
            base
        } else {
            format!("{base}{count}")
        };

        let formula = formulas.get(&label).cloned().unwrap_or_default();
        out.push(ExcelBranchView {
            label,
            formula,
            condition: alt.condition.clone(),
        });
    }

    out
}

/// 校验 AI 生成的 Excel 公式是否与本地转换结果等价。
///
/// `excelFormula` **覆盖** `schema.excelExpression` ——
/// 这样前端可以「先校验再决定要不要存」，不必先把公式写进 schema。
#[tauri::command]
pub fn excel_validate(
    mut schema: FormulaSchema,
    excel_formula: String,
) -> CmdResult<ValidationResult> {
    if excel_formula.trim().is_empty() {
        return Err(CommandError::InvalidArgument {
            message: "excelFormula 为空".to_string(),
        });
    }

    schema.excel_expression = Some(excel_formula);
    Ok(validate_excel_expression(&schema))
}

/// 把公式的 Excel 字段迁移到当前契约版本（v3）。
///
/// **只返回迁移结果，不落库** —— 落库交给 `formula_save`。
/// 已经是当前版本的公式原样返回（不覆盖用户手工修正过的字段）。
#[tauri::command]
pub fn excel_migrate(schema: FormulaSchema) -> CmdResult<FormulaSchema> {
    Ok(migrate_schema_excel_fields(&schema))
}

#[cfg(test)]
mod tests {
    use super::*;
    use civilcalc_core::schema::{AltExpression, FormulaSource, FormulaVar, ResultOutput, SourceKind};

    // ============================================================ 测试助手

    fn var(symbol: &str) -> FormulaVar {
        FormulaVar {
            symbol: symbol.to_string(),
            desc: format!("{symbol} 的释义"),
            unit: Some("mm".to_string()),
            default: None,
            min: None,
            max: None,
            required: true,
        }
    }

    fn schema(expression: &str, symbols: &[&str]) -> FormulaSchema {
        FormulaSchema {
            id: "usr:1".to_string(),
            result_name: "测试公式".to_string(),
            result_symbol: "X".to_string(),
            result_unit: Some("mm".to_string()),
            result_outputs: Vec::new(),
            expression: expression.to_string(),
            source_equations: Vec::new(),
            alt_expressions: Vec::new(),
            constants: HashMap::new(),
            variables: symbols.iter().map(|s| var(s)).collect(),
            domain: String::new(),
            tags: Vec::new(),
            reference_basis: None,
            design_notes: None,
            explanation: None,
            image_ids: Vec::new(),
            revised_from: None,
            source: FormulaSource {
                kind: SourceKind::Custom,
                ref_: None,
                verified: false,
                model: None,
                created_by: None,
            },
            steps_template: None,
            doc_template_id: None,
            excel_expression: None,
            excel_alt_expressions: None,
            excel_steps_template: None,
            excel_function_docs: None,
            schema_version: 3,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn inputs(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    // ============================================================ 两模式

    #[test]
    fn ref_mode_keeps_cell_refs() {
        let s = schema("pi*r^2*h", &["r", "h"]);
        let r = excel_convert(s, ExcelMode::Ref, inputs(&[("r", 2.0), ("h", 3.0)])).unwrap();
        assert_eq!(r.mode, ExcelMode::Ref);
        assert_eq!(r.main_formula, "=PI()*A1^2*B1");
    }

    #[test]
    fn value_mode_substitutes_numbers() {
        let s = schema("pi*r^2*h", &["r", "h"]);
        let r = excel_convert(s, ExcelMode::Value, inputs(&[("r", 2.0), ("h", 3.0)])).unwrap();
        assert_eq!(r.main_formula, "=PI()*2^2*3");
    }

    /// ref 模式忽略数值：即使给了 inputs，公式里仍是引用
    #[test]
    fn ref_mode_has_no_numbers_even_with_inputs() {
        let s = schema("a+b", &["a", "b"]);
        let r = excel_convert(s, ExcelMode::Ref, inputs(&[("a", 1.0), ("b", 2.0)])).unwrap();
        assert_eq!(r.main_formula, "=A1+B1");
    }

    /// ⚠️ 值模式缺输入 → `convert` 静默代入 **0**，必须由 `missingInputs` 补报
    ///
    /// 这是工程计算里最容易出事的一类错误：漏填参数，结果照算，但**是错的**。
    #[test]
    fn value_mode_reports_missing_inputs() {
        let s = schema("a+b", &["a", "b"]);
        let r = excel_convert(s, ExcelMode::Value, inputs(&[("a", 1.0)])).unwrap();

        assert_eq!(r.main_formula, "=1+0", "缺 b → 按 0 代入（源项目行为）");
        assert_eq!(r.missing_inputs, ["b"], "必须报出缺的变量名");
        assert!(
            r.unresolved_refs.is_empty(),
            "按 0 代入不会残留引用，所以这里应为空: {:?}",
            r.unresolved_refs
        );
    }

    /// 引用模式不报缺输入（数值不参与公式）
    #[test]
    fn ref_mode_does_not_report_missing_inputs() {
        let s = schema("a+b", &["a", "b"]);
        let r = excel_convert(s, ExcelMode::Ref, HashMap::new()).unwrap();
        assert!(r.missing_inputs.is_empty());
    }

    /// 值模式缺多个输入 → 按变量顺序列出，不重复
    #[test]
    fn missing_inputs_follow_variable_order_and_dedupe() {
        let s = schema("a+b+c", &["c", "a", "b", "a"]);
        let r = excel_convert(s, ExcelMode::Value, inputs(&[("b", 2.0)])).unwrap();
        assert_eq!(r.missing_inputs, ["c", "a"], "按变量顺序，且去重");
    }

    /// 数值非法（`NaN`）→ 回退单元格引用 + 告警 + 进 `unresolvedRefs`
    #[test]
    fn invalid_value_falls_back_to_cell_ref() {
        let s = schema("a+b", &["a", "b"]);
        let r = excel_convert(s, ExcelMode::Value, inputs(&[("a", f64::NAN), ("b", 2.0)])).unwrap();

        assert_eq!(r.main_formula, "=A1+2", "非法值 → 保留引用 A1");
        assert_eq!(r.unresolved_refs, ["A1"]);
        assert!(
            r.warnings.iter().any(|w| w.contains('a')),
            "应有告警: {:?}",
            r.warnings
        );
        assert!(
            r.missing_inputs.is_empty(),
            "NaN 也算「提供了值」，只是非法"
        );
    }

    /// **ref 模式不收集 `unresolvedRefs`** —— 引用是预期的，报了全是噪音
    #[test]
    fn ref_mode_does_not_report_cell_refs() {
        let s = schema("a+b", &["a", "b"]);
        let r = excel_convert(s, ExcelMode::Ref, HashMap::new()).unwrap();
        assert!(
            r.unresolved_refs.is_empty(),
            "ref 模式不应报单元格引用: {:?}",
            r.unresolved_refs
        );
    }

    /// 未声明的符号两种模式都要报
    #[test]
    fn undeclared_symbol_reported_in_both_modes() {
        // `z` 不在 variables 里 → 生成的公式里保留原名
        let s = schema("a+z", &["a"]);
        for mode in [ExcelMode::Ref, ExcelMode::Value] {
            let r = excel_convert(s.clone(), mode, inputs(&[("a", 1.0)])).unwrap();
            assert_eq!(
                r.unresolved_names,
                ["z"],
                "{mode:?} 模式应报出未声明符号: {:?}",
                r.unresolved_names
            );
        }
    }

    /// 正常公式不应有残留符号（函数名、单元格引用、数值都不算）
    #[test]
    fn clean_formula_has_no_residuals() {
        let s = schema("sqrt(a)+pi", &["a"]);
        for mode in [ExcelMode::Ref, ExcelMode::Value] {
            let r = excel_convert(s.clone(), mode, inputs(&[("a", 4.0)])).unwrap();
            assert!(
                r.unresolved_names.is_empty(),
                "{mode:?} 不应有残留标识符: {:?}（公式 {}）",
                r.unresolved_names,
                r.main_formula
            );
        }
    }

    // ============================================================ 参数对照表

    #[test]
    fn param_mapping_carries_unit_and_desc() {
        let s = schema("a+b", &["a", "b"]);
        let r = excel_convert(s, ExcelMode::Ref, HashMap::new()).unwrap();
        assert_eq!(r.param_mapping.len(), 2);
        assert_eq!(r.param_mapping[0].variable, "a");
        assert_eq!(r.param_mapping[0].cell_ref, "A1");
        assert_eq!(r.param_mapping[0].unit, "mm");
        assert_eq!(r.param_mapping[1].cell_ref, "B1");
    }

    #[test]
    fn layout_constants_match_contract() {
        let s = schema("a", &["a"]);
        let r = excel_convert(s, ExcelMode::Ref, HashMap::new()).unwrap();
        assert_eq!(r.variable_row, 1, "变量在第 1 行横向");
        assert_eq!(r.step_start_row, 2, "步骤结果从 A2 起");
    }

    // ============================================================ 多结果

    fn multi_schema() -> FormulaSchema {
        let mut s = schema("x = a+b; y = a-b", &["a", "b"]);
        s.result_outputs = vec![
            ResultOutput {
                symbol: "x".to_string(),
                name: "和".to_string(),
                unit: "mm".to_string(),
            },
            ResultOutput {
                symbol: "y".to_string(),
                name: "差".to_string(),
                unit: "mm".to_string(),
            },
        ];
        s
    }

    #[test]
    fn multi_output_ref_mode() {
        let r = excel_convert(multi_schema(), ExcelMode::Ref, HashMap::new()).unwrap();
        assert_eq!(r.outputs.len(), 2);
        assert_eq!(r.outputs[0].symbol, "x");
        assert_eq!(r.outputs[0].name, "和");
        assert_eq!(r.outputs[0].formula, "=A1+B1");
        assert_eq!(r.outputs[1].formula, "=A1-B1");
        // 主公式 = 最后一段
        assert_eq!(r.main_formula, "=A1-B1");
    }

    #[test]
    fn multi_output_value_mode() {
        let r = excel_convert(multi_schema(), ExcelMode::Value, inputs(&[("a", 3.0), ("b", 1.0)])).unwrap();
        assert_eq!(r.outputs[0].formula, "=3+1");
        assert_eq!(r.outputs[1].formula, "=3-1");
        assert_eq!(r.main_formula, "=3-1");
    }

    #[test]
    fn single_output_has_empty_outputs_list() {
        let r = excel_convert(schema("a+b", &["a", "b"]), ExcelMode::Ref, HashMap::new()).unwrap();
        assert!(r.outputs.is_empty());
    }

    // ============================================================ 分支

    #[test]
    fn branches_follow_alt_expression_order() {
        let mut s = schema("a+b", &["a", "b"]);
        s.alt_expressions = vec![
            AltExpression {
                label: "受拉".to_string(),
                expression: "a-b".to_string(),
                condition: Some("a>b".to_string()),
            },
            AltExpression {
                label: "受压".to_string(),
                expression: "a*b".to_string(),
                condition: None,
            },
        ];

        let r = excel_convert(s, ExcelMode::Ref, HashMap::new()).unwrap();
        assert_eq!(r.branches.len(), 2);
        assert_eq!(r.branches[0].label, "受拉");
        assert_eq!(r.branches[0].formula, "=A1-B1");
        assert_eq!(r.branches[0].condition.as_deref(), Some("a>b"));
        assert_eq!(r.branches[1].label, "受压");
        assert_eq!(r.branches[1].formula, "=A1*B1");
    }

    /// 分支重名 → 追加序号（与转换器 BUG-13 处理一致）
    #[test]
    fn duplicate_branch_labels_get_suffix() {
        let mut s = schema("a+b", &["a", "b"]);
        s.alt_expressions = vec![
            AltExpression {
                label: "分支".to_string(),
                expression: "a-b".to_string(),
                condition: None,
            },
            AltExpression {
                label: "分支".to_string(),
                expression: "a*b".to_string(),
                condition: None,
            },
        ];

        let r = excel_convert(s, ExcelMode::Ref, HashMap::new()).unwrap();
        assert_eq!(r.branches.len(), 2);
        assert_eq!(r.branches[0].label, "分支");
        assert_eq!(r.branches[0].formula, "=A1-B1");
        assert_eq!(r.branches[1].label, "分支2");
        assert_eq!(r.branches[1].formula, "=A1*B1");
    }

    /// ⚠️ 回归：`分支10` 不能因为字典序排到 `分支2` 前面
    ///
    /// 若按 `BTreeMap` 键序取分支就会踩到（`"分支10" < "分支2"`）。
    #[test]
    fn branch_order_is_not_lexicographic() {
        let mut s = schema("a", &["a"]);
        s.alt_expressions = (1..=10)
            .map(|i| AltExpression {
                label: "分支".to_string(),
                expression: format!("a+{i}"),
                condition: None,
            })
            .collect();

        let r = excel_convert(s, ExcelMode::Ref, HashMap::new()).unwrap();
        assert_eq!(r.branches.len(), 10);
        let labels: Vec<&str> = r.branches.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(
            labels,
            ["分支", "分支2", "分支3", "分支4", "分支5", "分支6", "分支7", "分支8", "分支9", "分支10"],
            "必须按 altExpressions 顺序，不是字典序"
        );
        assert_eq!(r.branches[1].formula, "=A1+2");
        assert_eq!(r.branches[9].formula, "=A1+10");
    }

    /// 空白分支表达式被转换器跳过 → 这里也不列出（避免给一个空公式）
    #[test]
    fn blank_branch_expression_is_skipped() {
        let mut s = schema("a+b", &["a", "b"]);
        s.alt_expressions = vec![
            AltExpression {
                label: "空的".to_string(),
                expression: "   ".to_string(),
                condition: None,
            },
            AltExpression {
                label: "实的".to_string(),
                expression: "a-b".to_string(),
                condition: None,
            },
        ];

        let r = excel_convert(s, ExcelMode::Ref, HashMap::new()).unwrap();
        assert_eq!(r.branches.len(), 1, "空分支不列出");
        assert_eq!(r.branches[0].label, "实的");
    }

    #[test]
    fn blank_branch_label_falls_back_to_default() {
        let mut s = schema("a+b", &["a", "b"]);
        s.alt_expressions = vec![AltExpression {
            label: "  ".to_string(),
            expression: "a-b".to_string(),
            condition: None,
        }];
        let r = excel_convert(s, ExcelMode::Ref, HashMap::new()).unwrap();
        assert_eq!(r.branches[0].label, "分支");
    }

    // ============================================================ 函数清单

    #[test]
    fn used_functions_are_exposed() {
        let s = schema("sqrt(a)+abs(b)", &["a", "b"]);
        let r = excel_convert(s, ExcelMode::Ref, HashMap::new()).unwrap();
        let names: Vec<&str> = r.used_functions.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["abs", "sqrt"]);
        assert_eq!(r.used_functions[1].excel_name, "SQRT");
    }

    // ============================================================ 编译失败

    #[test]
    fn compile_failure_surfaces_warning_and_original() {
        let s = schema("a + (b", &["a", "b"]);
        let r = excel_convert(s, ExcelMode::Ref, HashMap::new()).unwrap();
        assert_eq!(r.main_formula, "=a + (b", "编译失败回退原式");
        assert!(
            r.warnings.iter().any(|w| w.contains("编译失败")),
            "应告警: {:?}",
            r.warnings
        );
    }

    // ============================================================ excel_validate

    #[test]
    fn validate_accepts_matching_formula() {
        let s = schema("sqrt(a - 1) + b", &["a", "b"]);
        let r = excel_validate(s, "=SQRT(A1-1)+B1".to_string()).unwrap();
        assert!(r.ok, "应通过: {:?}", r.message);
    }

    /// 传入的 `excelFormula` **覆盖** schema 里已有的值
    #[test]
    fn validate_overrides_schema_excel_expression() {
        let mut s = schema("a+b", &["a", "b"]);
        s.excel_expression = Some("=完全错误的公式".to_string());
        let r = excel_validate(s, "=A1+B1".to_string()).unwrap();
        assert!(r.ok, "应使用传入的公式: {:?}", r.message);
        assert_eq!(r.ai_formula.as_deref(), Some("=A1+B1"));
    }

    #[test]
    fn validate_rejects_mismatch() {
        let s = schema("a+b", &["a", "b"]);
        let r = excel_validate(s, "=A1+A1".to_string()).unwrap();
        assert!(!r.ok);
        assert!(
            r.message.as_deref().unwrap_or_default().contains("不一致"),
            "实际: {:?}",
            r.message
        );
    }

    #[test]
    fn validate_rejects_empty_formula() {
        let s = schema("a+b", &["a", "b"]);
        let e = excel_validate(s, "   ".to_string()).unwrap_err();
        assert_eq!(e.kind_str(), "invalidArgument");
    }

    /// 校验结果可直接序列化为 IPC 形状（camelCase）
    #[test]
    fn validation_result_serde_shape() {
        let s = schema("a+b", &["a", "b"]);
        let r = excel_validate(s, "=A1+B1".to_string()).unwrap();
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["ok"], serde_json::json!(true));
        assert!(v.get("localFormula").is_some(), "必须 camelCase");
        assert!(v.get("sampleInputs").is_some());
        assert!(v.get("local_formula").is_none(), "不得泄漏 snake_case");
    }

    // ============================================================ excel_migrate

    #[test]
    fn migrate_fills_excel_fields() {
        let mut s = schema("sqrt(a)+b", &["a", "b"]);
        s.schema_version = 1;

        let out = excel_migrate(s).unwrap();
        assert_eq!(out.schema_version, 3);
        assert_eq!(out.excel_expression.as_deref(), Some("=SQRT(A1)+B1"));
        assert!(out.excel_function_docs.is_some());
    }

    #[test]
    fn migrate_is_idempotent() {
        let mut s = schema("sqrt(a)+b", &["a", "b"]);
        s.schema_version = 1;

        let once = excel_migrate(s).unwrap();
        let twice = excel_migrate(once.clone()).unwrap();
        assert_eq!(once, twice, "已是 v3 的公式再迁移应原样返回");
    }

    #[test]
    fn migrate_does_not_touch_current_version() {
        let mut s = schema("a+b", &["a", "b"]);
        s.excel_expression = Some("=手工写的".to_string());
        let out = excel_migrate(s.clone()).unwrap();
        assert_eq!(out, s, "当前版本公式不得被覆盖");
    }

    // ============================================================ ExcelResult 序列化

    #[test]
    fn excel_result_serde_shape() {
        let s = schema("a+b", &["a", "b"]);
        let r = excel_convert(s, ExcelMode::Ref, HashMap::new()).unwrap();
        let v = serde_json::to_value(&r).unwrap();

        assert_eq!(v["mode"], serde_json::json!("ref"), "模式应序列化为小写");
        assert!(v.get("mainFormula").is_some());
        assert!(v.get("paramMapping").is_some());
        assert!(v.get("usedFunctions").is_some());
        assert!(v.get("unresolvedRefs").is_some());
        assert!(v.get("unresolvedNames").is_some());
        assert!(v.get("missingInputs").is_some());
        assert!(v.get("variableRow").is_some());
        assert!(v.get("main_formula").is_none(), "不得泄漏 snake_case");
    }

    #[test]
    fn excel_mode_deserializes_from_lowercase() {
        let ref_mode: ExcelMode = serde_json::from_str("\"ref\"").unwrap();
        let value_mode: ExcelMode = serde_json::from_str("\"value\"").unwrap();
        assert_eq!(ref_mode, ExcelMode::Ref);
        assert_eq!(value_mode, ExcelMode::Value);
        // 未知值必须报错（不能静默当成某个模式）
        assert!(serde_json::from_str::<ExcelMode>("\"REF\"").is_err());
    }
}
