//! 组 4：求值与校验（4 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `eval_schema` | `schema`, `inputs` | `EvalResult` |
//! | `eval_formula` | `formulaId`, `inputs`, `thinkingContent?` | `EvalResult` |
//! | `validate_schema_cmd` | `schema` | `void` |
//! | `eval_steps` | `formulaId`, `inputs` | `StepResult[]` |
//!
//! ## `eval_schema` vs `eval_formula` —— 关键区别在**写不写历史**
//!
//! | 命令 | 取 Schema 从 | 写历史 | 用途 |
//! |---|---|---|---|
//! | `eval_schema` | 参数传入 | ❌ | **编辑中/未保存**的实时预览 |
//! | `eval_formula` | 数据库 | ✅ | 用户点「计算」 |
//!
//! 源项目行为：不是每次预览求值都写历史 —— 否则用户拖滑块试参数
//! 会瞬间灌满历史。
//!
//! ## `eval_steps` 与 `eval_formula` 的关系
//!
//! 两者是**并列**的：源项目在「点计算」时同时调
//! `formulaRepository.eval(...)` 与 `evalSteps(...)`
//! （`FormulaViewModel.kt:375-377`），前端拿到两份结果。
//!
//! 因此 PC 端也拆成两个命令 —— 硬合成一个会让「只要结果不要分步」
//! 的场景白算一遍分步（分步含 Excel 转换，比求值贵得多）。

use crate::error::{CmdResult, CommandError};
use crate::state::AppState;
use civilcalc_core::engine::{build_eval_context, check_domain, eval_multi};
use civilcalc_core::schema::{
    validate_schema, EvalResult, FormulaSchema, HistoryEntry, StepResult,
};
use std::collections::HashMap;
use tauri::State;

/// **按 Schema 直接求值**（编辑中/未保存的实时预览）。
///
/// ## 不写历史
///
/// 见模块文档。需要落历史请用 [`eval_formula`]。
///
/// ## 越界只告警不阻断
///
/// 变量的 `min` / `max` 越界会追加到 `result.warnings`，
/// **求值照常进行**（对齐源项目）。真正的定义域错误
/// （如 `sqrt` 负数）由引擎以 `EvalError::Domain` 返回。
#[tauri::command]
pub fn eval_schema(
    schema: FormulaSchema,
    inputs: HashMap<String, f64>,
) -> CmdResult<EvalResult> {
    evaluate(&schema, &inputs)
}

/// **按 ID 求值**（查库取 Schema），并**写一条计算历史**。
///
/// `thinking_content` 为 `None` / 空白时，历史里的 `thinkingContent` 存 `None`。
#[tauri::command]
pub fn eval_formula(
    state: State<'_, AppState>,
    formula_id: String,
    inputs: HashMap<String, f64>,
    thinking_content: Option<String>,
) -> CmdResult<EvalResult> {
    let schema = state
        .db
        .get_formula(&formula_id)?
        .ok_or_else(|| CommandError::NotFound {
            message: format!("公式不存在: {formula_id}"),
        })?;

    let result = evaluate(&schema, &inputs)?;

    // 写历史：失败**不阻断**求值结果的返回（用户已经拿到结果了），
    // 但要记日志 —— 历史丢了属于数据完整性问题，必须留痕。
    let entry = HistoryEntry::from_eval(&schema, &inputs, &result, thinking_content);
    if let Err(e) = state.db.insert_history(&entry) {
        civilcalc_core::log::w(
            "Commands",
            &format!("写入计算历史失败（公式 {formula_id}）: {e}"),
            None,
        );
    }

    Ok(result)
}

/// 校验 Schema（12 类规则）。通过返回 `void`，失败抛 `validation`。
#[tauri::command]
pub fn validate_schema_cmd(schema: FormulaSchema) -> CmdResult<()> {
    validate_schema(&schema).map_err(|e| CommandError::Validation {
        message: e.error_detail,
    })
}

/// **分步求值**（查库取 Schema）。
///
/// 返回逐步骤的 [`StepResult`]（含代入公式与 Excel 公式），
/// 供结果区的分步面板与计算书的「分步计算」章节使用。
///
/// ## 没有分步模板时返回空数组（**不是错误**）
///
/// 大多数公式没有 `stepsTemplate` —— 那是「可导出的分步」，
/// 由 AI 生成时按需产出。空数组让前端直接隐藏分步区，无需先判断错误。
///
/// ## 任一步骤失败 → 整体失败
///
/// 不返回半截结果：读者看到前两步而第三步缺失，会以为公式本来就只有两步。
///
/// ## 为什么不是 `eval_formula` 的一部分
///
/// 见模块文档 —— 源项目也是两个并列调用。
#[tauri::command]
pub fn eval_steps(
    state: State<'_, AppState>,
    formula_id: String,
    inputs: HashMap<String, f64>,
) -> CmdResult<Vec<StepResult>> {
    let schema = state
        .db
        .get_formula(&formula_id)?
        .ok_or_else(|| CommandError::NotFound {
            message: format!("公式不存在: {formula_id}"),
        })?;

    eval_steps_of(&schema, &inputs)
}

/// 分步求值核心（**不依赖 Tauri**，便于单测）。
///
/// 变量顺序取自 `schema.variables` 的**声明顺序** —— 它决定
/// 「变量 → 单元格」的编号（第 1 行横向），对齐源项目
/// `FormulaRepositoryImpl.evalSteps` 的 `schema.variables.map { it.symbol }`。
fn eval_steps_of(
    schema: &FormulaSchema,
    inputs: &HashMap<String, f64>,
) -> CmdResult<Vec<StepResult>> {
    let Some(steps) = schema
        .steps_template
        .as_ref()
        .filter(|s| !s.is_empty())
    else {
        return Ok(Vec::new());
    };

    let variable_order: Vec<String> =
        schema.variables.iter().map(|v| v.symbol.clone()).collect();

    // ⚠️ 这里用**全路径**调用引擎，避免与同名命令函数冲突
    civilcalc_core::engine::eval_steps(
        steps,
        inputs,
        &schema.constants,
        schema.excel_steps_template.as_deref(),
        &variable_order,
    )
    .map_err(|e| CommandError::Parse {
        message: e.to_string(),
    })
}

/// 求值核心（两个命令共用）。
///
/// 顺序：
/// 1. `build_eval_context` —— 常量 ∪ 输入 ∪ 变量 `default` 兜底
/// 2. 定义域检查 —— 越界追加 `warnings`（**不阻断**）
/// 3. `eval_multi` —— 按分号拆段逐段求值，`primary` = 最后一段
fn evaluate(schema: &FormulaSchema, inputs: &HashMap<String, f64>) -> CmdResult<EvalResult> {
    let ctx = build_eval_context(schema, inputs);

    let mut result = eval_multi(&schema.expression, &schema.constants, &ctx).map_err(|e| {
        CommandError::Parse {
            message: e.to_string(),
        }
    })?;

    // 定义域告警（源项目：只告警，不阻断）
    for var in &schema.variables {
        if let Some(v) = ctx.get(&var.symbol) {
            if let Some(w) = check_domain(var, *v) {
                result.warnings.push(w);
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use civilcalc_core::schema::{FormulaSource, FormulaVar, CURRENT_SCHEMA_VERSION};

    fn var(symbol: &str, default: Option<f64>, min: Option<f64>, max: Option<f64>) -> FormulaVar {
        FormulaVar {
            symbol: symbol.to_string(),
            desc: format!("参数{symbol}"),
            unit: Some("m".to_string()),
            default,
            min,
            max,
            required: true,
        }
    }

    fn schema(expr: &str, vars: Vec<FormulaVar>, constants: &[(&str, f64)]) -> FormulaSchema {
        FormulaSchema {
            id: "usr:1".into(),
            result_name: "结果".into(),
            result_symbol: "y".into(),
            result_unit: None,
            result_outputs: vec![],
            expression: expr.into(),
            source_equations: vec![],
            alt_expressions: vec![],
            constants: constants.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            variables: vars,
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

    fn inputs(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    // ---------------- 基本求值 ----------------

    #[test]
    fn simple_expression_evaluates() {
        let s = schema("a+b", vec![], &[]);
        let r = evaluate(&s, &inputs(&[("a", 1.0), ("b", 2.0)])).unwrap();
        assert_eq!(r.primary, 3.0);
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn constants_are_available() {
        // ⚠️ 不要用 `E` 做测试常量：词法分析对**常量名不区分大小写**，
        //    `E` 会命中内置常量 `e`（欧拉数 2.718…），得到 2*2.718 而不是 2*206000。
        let s = schema("a*fy", vec![], &[("fy", 360.0)]);
        let r = evaluate(&s, &inputs(&[("a", 2.0)])).unwrap();
        assert_eq!(r.primary, 720.0);
    }

    /// **陷阱回归 1**：常量名不区分大小写 —— `E` 会被当成欧拉数 `e`
    ///
    /// 这是源项目行为（词法分析先 `lower()` 再查常量表），
    /// 用户若把常量命名为 `E`（弹性模量的常见写法）会踩到。
    /// 前端应在参数表里提示，或让用户改用 `Es`。
    ///
    /// ⚠️ 断言要按 [`round_final`] 的 **4 位小数**口径算 ——
    /// 求值结果统一保留 4 位（源项目 `roundFinal`），不是全精度。
    #[test]
    fn constant_lookup_is_case_insensitive_trap() {
        let round4 = |x: f64| (x * 10_000.0).round() / 10_000.0;

        let s = schema("a*E", vec![], &[("E", 206000.0)]);
        let r = evaluate(&s, &inputs(&[("a", 2.0)])).unwrap();
        assert_eq!(
            r.primary,
            round4(2.0 * std::f64::consts::E),
            "E 应命中内置欧拉数 e（约 5.4366），而不是自定义的 206000"
        );

        // 换成不冲突的名字才拿到自定义常量值
        let s = schema("a*Es", vec![], &[("Es", 206000.0)]);
        let r = evaluate(&s, &inputs(&[("a", 2.0)])).unwrap();
        assert_eq!(r.primary, 412000.0);
    }

    /// **陷阱回归 2**：求值结果**统一保留 4 位小数**（源项目 `roundFinal`）
    #[test]
    fn results_are_rounded_to_four_decimals() {
        let s = schema("1/3", vec![], &[]);
        let r = evaluate(&s, &HashMap::new()).unwrap();
        assert_eq!(r.primary, 0.3333, "1/3 应保留 4 位");

        let s = schema("2/3", vec![], &[]);
        let r = evaluate(&s, &HashMap::new()).unwrap();
        assert_eq!(r.primary, 0.6667, "四舍五入到 4 位");

        let s = schema("1/1", vec![], &[]);
        let r = evaluate(&s, &HashMap::new()).unwrap();
        assert_eq!(r.primary, 1.0);
    }

    /// 变量 `default` 兜底：没传值时用默认值
    #[test]
    fn variable_default_fills_missing_input() {
        let s = schema("a+b", vec![var("a", Some(10.0), None, None)], &[]);
        let r = evaluate(&s, &inputs(&[("b", 5.0)])).unwrap();
        assert_eq!(r.primary, 15.0, "a 应取默认值 10");
    }

    /// 显式传值优先于 `default`
    #[test]
    fn explicit_input_beats_default() {
        let s = schema("a", vec![var("a", Some(10.0), None, None)], &[]);
        let r = evaluate(&s, &inputs(&[("a", 3.0)])).unwrap();
        assert_eq!(r.primary, 3.0);
    }

    // ---------------- 多输出（BUG-01） ----------------

    #[test]
    fn multi_segment_binds_symbols_and_primary_is_last() {
        let s = schema("X1 = a+b; X2 = X1*c", vec![], &[]);
        let r = evaluate(&s, &inputs(&[("a", 1.0), ("b", 2.0), ("c", 4.0)])).unwrap();

        assert_eq!(r.primary, 12.0, "primary 应为最后一段");
        assert_eq!(r.outputs.len(), 2);
        assert_eq!(r.outputs[0].symbol.as_deref(), Some("X1"));
        assert_eq!(r.outputs[0].value, 3.0);
        assert_eq!(r.outputs[1].symbol.as_deref(), Some("X2"));
        assert_eq!(r.outputs[1].value, 12.0);
    }

    /// 单段公式的 `outputs` 有**一个**元素，其 `symbol` 为 `None`
    ///
    /// （`eval_multi` 对每个分号段都推一个 output，与段有没有赋值符号无关。）
    #[test]
    fn single_segment_has_one_output_without_symbol() {
        let s = schema("a+b", vec![], &[]);
        let r = evaluate(&s, &inputs(&[("a", 1.0), ("b", 1.0)])).unwrap();
        assert_eq!(r.outputs.len(), 1);
        assert_eq!(r.outputs[0].symbol, None, "单段没有赋值符号");
        assert_eq!(r.outputs[0].value, 2.0);
        assert_eq!(r.primary, 2.0);
    }

    // ---------------- 定义域告警 ----------------

    /// 越界只告警，**求值照常**
    #[test]
    fn domain_violation_warns_but_does_not_block() {
        let s = schema("a", vec![var("a", None, Some(0.0), Some(10.0))], &[]);
        let r = evaluate(&s, &inputs(&[("a", 99.0)])).unwrap();

        assert_eq!(r.primary, 99.0, "越界不应阻断求值");
        assert_eq!(r.warnings.len(), 1);
        assert!(r.warnings[0].contains("不能大于"));
    }

    #[test]
    fn domain_warning_both_bounds() {
        let s = schema("a", vec![var("a", None, Some(5.0), None)], &[]);
        let r = evaluate(&s, &inputs(&[("a", 1.0)])).unwrap();
        assert!(r.warnings[0].contains("不能小于"));
    }

    #[test]
    fn in_range_has_no_warning() {
        let s = schema("a", vec![var("a", None, Some(0.0), Some(10.0))], &[]);
        let r = evaluate(&s, &inputs(&[("a", 5.0)])).unwrap();
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn multiple_violations_accumulate() {
        let s = schema(
            "a+b",
            vec![
                var("a", None, Some(0.0), Some(1.0)),
                var("b", None, Some(0.0), Some(1.0)),
            ],
            &[],
        );
        let r = evaluate(&s, &inputs(&[("a", 5.0), ("b", 5.0)])).unwrap();
        assert_eq!(r.warnings.len(), 2);
    }

    /// 未提供值的变量不参与告警（避免"没填就报越界"）
    #[test]
    fn missing_input_does_not_warn() {
        let s = schema("a", vec![var("a", None, Some(0.0), Some(10.0))], &[]);
        // a 没传也没默认值 → 编译/求值会失败（未定义变量），但**不该有告警**
        let _ = evaluate(&s, &inputs(&[]));
        // 这里只断言不 panic
    }

    // ---------------- 编译错误 → parse ----------------

    #[test]
    fn compile_error_maps_to_parse_error() {
        // 括号未闭合 → 语法分析必失败
        let s = schema("a + (b", vec![], &[]);
        let e = evaluate(&s, &inputs(&[("a", 1.0), ("b", 2.0)])).unwrap_err();
        assert_eq!(e.kind_str(), "parse");
    }

    #[test]
    fn empty_expression_is_parse_error() {
        let s = schema("", vec![], &[]);
        assert!(evaluate(&s, &HashMap::new()).is_err());
    }

    // ---------------- 校验 ----------------

    #[test]
    fn valid_schema_passes_validation() {
        let s = schema("a+b", vec![var("a", None, None, None), var("b", None, None, None)], &[]);
        assert!(validate_schema(&s).is_ok());
    }

    #[test]
    fn empty_variable_desc_fails_validation() {
        let mut s = schema("a", vec![var("a", None, None, None)], &[]);
        s.variables[0].desc = String::new();
        let e = validate_schema(&s).unwrap_err();
        assert!(!e.error_detail.is_empty());
    }

    #[test]
    fn empty_expression_fails_validation() {
        let mut s = schema("a", vec![var("a", None, None, None)], &[]);
        s.expression = "  ".into();
        assert!(validate_schema(&s).is_err());
    }

    // ---------------- 分步求值 ----------------

    /// 造一个带分步模板的 schema
    fn with_steps(mut s: FormulaSchema, steps: &[(&str, &str, &str)]) -> FormulaSchema {
        s.steps_template = Some(
            steps
                .iter()
                .map(|(sym, label, expr)| civilcalc_core::schema::StepTemplate {
                    symbol: (*sym).to_string(),
                    label: (*label).to_string(),
                    group: None,
                    expression: (*expr).to_string(),
                    unit: String::new(),
                    note: None,
                })
                .collect(),
        );
        s
    }

    /// **没有分步模板 → 空数组，不是错误**
    #[test]
    fn no_steps_template_yields_empty_ok() {
        let s = schema("a+b", vec![var("a", None, None, None), var("b", None, None, None)], &[]);
        let r = eval_steps_of(&s, &inputs(&[("a", 1.0), ("b", 2.0)])).unwrap();
        assert!(r.is_empty());
    }

    /// **空的分步模板也走空数组**（`filter(|s| !s.is_empty())`）
    #[test]
    fn empty_steps_list_yields_empty_ok() {
        let s = with_steps(schema("a+b", vec![var("a", None, None, None)], &[]), &[]);
        let r = eval_steps_of(&s, &inputs(&[("a", 1.0)])).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn steps_are_evaluated_in_order() {
        let s = with_steps(
            schema(
                "b*h",
                vec![var("b", None, None, None), var("h", None, None, None)],
                &[],
            ),
            &[("A", "面积", "b*h"), ("V", "体积", "A*d")],
        );
        let mut s = s;
        s.variables.push(var("d", None, None, None));

        let r = eval_steps_of(&s, &inputs(&[("b", 2.0), ("h", 3.0), ("d", 4.0)])).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].value, 6.0);
        assert_eq!(r[1].value, 24.0);
    }

    /// Excel 公式是**单元格引用式**（变量 → 第 1 行横向）
    #[test]
    fn steps_excel_formula_uses_cell_refs() {
        let s = with_steps(
            schema(
                "b*h",
                vec![var("b", None, None, None), var("h", None, None, None)],
                &[],
            ),
            &[("A", "面积", "b*h")],
        );
        let r = eval_steps_of(&s, &inputs(&[("b", 2.0), ("h", 3.0)])).unwrap();
        assert_eq!(r[0].excel_formula, "=A1*B1");
        assert_eq!(r[0].excel_value_formula, "=2*3");
    }

    /// 步骤失败 → 整体失败（错误信息带步骤名）
    #[test]
    fn failing_step_fails_whole_command() {
        let s = with_steps(
            schema("b*h", vec![var("b", None, None, None)], &[]),
            &[("A", "好的", "b*2"), ("B", "坏的", "b + (h")],
        );
        let e = eval_steps_of(&s, &inputs(&[("b", 1.0)])).unwrap_err();
        assert_eq!(e.kind_str(), "parse");
        assert!(e.to_string().contains("坏的"), "错误应带步骤名: {e}");
    }

    /// schema 常量对分步可见
    #[test]
    fn schema_constants_visible_to_steps() {
        let s = with_steps(
            schema("k*b", vec![var("b", None, None, None)], &[("k", 10.0)]),
            &[("A", "结果", "k*b")],
        );
        let r = eval_steps_of(&s, &inputs(&[("b", 3.0)])).unwrap();
        assert_eq!(r[0].value, 30.0);
    }

    /// 分步结果**可序列化**（要经 IPC 返回给前端）
    #[test]
    fn step_results_serialize() {
        let s = with_steps(
            schema(
                "b*h",
                vec![var("b", None, None, None), var("h", None, None, None)],
                &[],
            ),
            &[("A", "面积", "b*h")],
        );
        let r = eval_steps_of(&s, &inputs(&[("b", 2.0), ("h", 3.0)])).unwrap();
        let v = serde_json::to_value(&r).unwrap();
        assert!(v[0].get("substitutedExpression").is_some());
        assert!(v[0].get("excelFormula").is_some());
        assert!(v[0].get("excelValueFormula").is_some());
        assert!(v[0].get("substituted_expression").is_none(), "不得泄漏 snake_case");
    }
}
