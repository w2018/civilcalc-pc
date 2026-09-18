//! 分步求值（`eval_steps`）。
//!
//! 源：`FormulaEngineImpl.kt` 的 `evalSteps`（约 130 行）
//!
//! ## 为什么单独一个文件
//!
//! `engine/mod.rs` 已经承载了 `compile` / `eval` / `eval_multi` 三个入口，
//! 分步求值是第四条独立链路（有自己的变量表、Excel 映射与失败语义），
//! 塞进去会让那个文件难以定位。
//!
//! ## 三条必须保留的行为（都来自源项目的 BUG 修复）
//!
//! | # | 行为 | 不这么做会怎样 |
//! |---|---|---|
//! | BUG-03 | 步骤表达式**可含多段**（`a = …; b = …`），逐段求值并绑定赋值目标，**步骤结果 = 最后一段** | 多段步骤只算第一段 |
//! | BUG-10 | Excel 公式**统一输出「单元格引用式」**（变量 → 第 1 行横向；步骤结果 → A 列纵向） | 同一列表里混用引用式与代值式，用户看不出哪列是哪种 |
//! | — | 每段一个单元格：第 j 个子段 → `A(j+2)` | 多个子段共用单元格，后面的覆盖前面的 |
//!
//! ## 单元格契约（与 `docs/06` 一致）
//!
//! - **变量** → 第 1 行横向：`variable_order[0]` → `A1`、`[1]` → `B1` …
//! - **步骤结果** → A 列纵向：第 0 个子段 → `A2`、第 1 个 → `A3` …
//!
//! 两套编号**不冲突**（行 1 vs 行 ≥2），所以可以合成一张表给 Excel 转换器用。

use std::collections::HashMap;

use crate::engine::types::CompileResult;
use crate::engine::{evaluator, function_table, lexer, parser, splitter};
use crate::excel::converter::{substitute_values, ExcelFormulaConverter};
use crate::schema::{EvalError, StepResult, StepTemplate};

/// 步骤值的展示口径：整数不带小数点，非整数**保留 4 位小数**再去尾零。
///
/// ⚠️ 与 `civilcalc-report` 的 `format_number`（6 位）**不同** ——
/// 源项目这两处本来就是两套口径，不要合并。
fn format_step_value(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if value == (value as i64) as f64 {
        return (value as i64).to_string();
    }
    format!("{value:.4}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

/// 逐步骤求值，产出计算书与分步面板要用的 [`StepResult`]。
///
/// # 参数
///
/// - `steps`：步骤模板（`schema.steps_template`）。空列表直接返回空结果
/// - `inputs`：变量取值
/// - `constants`：`schema.constants`（会与内置常量合并）
/// - `excel_steps`：AI 生成的 Excel 步骤模板（`schema.excel_steps_template`）。
///   给了就用它的表达式当 Excel 公式，否则**本地转换**（BUG-10）
/// - `variable_order`：变量的出现顺序，决定变量 → 单元格的编号
///
/// # 失败语义
///
/// **任一步骤失败即整体失败**（返回 `Err`）—— 与源项目一致。
/// 半截的分步结果比没有更危险：读者会以为后面的步骤不存在。
pub fn eval_steps(
    steps: &[StepTemplate],
    inputs: &HashMap<String, f64>,
    constants: &HashMap<String, f64>,
    excel_steps: Option<&[StepTemplate]>,
    variable_order: &[String],
) -> Result<Vec<StepResult>, EvalError> {
    if steps.is_empty() {
        return Ok(Vec::new());
    }

    let mut merged_constants = function_table::constants_owned();
    merged_constants.extend(constants.iter().map(|(k, v)| (k.clone(), *v)));
    let functions = function_table::function_name_set();

    // 前序步骤结果 + 原始 inputs —— 后续步骤都从这里取符号
    let mut variable_table: HashMap<String, f64> = inputs.clone();
    let mut results: Vec<StepResult> = Vec::with_capacity(steps.len());

    // 变量 → 单元格（第 1 行横向契约）
    let base_cell_refs: HashMap<String, String> = variable_order
        .iter()
        .enumerate()
        .map(|(i, sym)| (sym.clone(), crate::excel::index_to_cell_ref(i)))
        .collect();

    // 步骤结果符号 → 单元格
    let mut step_cells: HashMap<String, String> = HashMap::new();
    // 步骤结果符号 → 数值（供「代入数值版」用）
    let mut step_values: HashMap<String, f64> = HashMap::new();
    // 单元格 → 数值（变量取本组输入，步骤结果取计算值）
    let mut cell_values: HashMap<String, f64> = HashMap::new();
    for (sym, cell) in &base_cell_refs {
        if let Some(v) = inputs.get(sym) {
            cell_values.insert(cell.clone(), *v);
        }
    }

    // 子段计数：跨步骤累加（第 j 个子段 → A(j+2)）
    let mut sub_seg_counter = 0usize;

    for (index, step) in steps.iter().enumerate() {
        let sub_segments = splitter::split(&step.expression);
        if sub_segments.is_empty() {
            return Err(step_error(&step.label, "表达式为空"));
        }

        let mut last_value = 0.0f64;
        let mut last_clean_expr = step.expression.clone();
        let mut last_cell = format!("A{}", sub_seg_counter + 2);

        for seg in &sub_segments {
            let clean_expr = seg.expression.clone();
            last_cell = format!("A{}", sub_seg_counter + 2);

            let compiled = compile_segment(&clean_expr, &merged_constants, &functions)
                .map_err(|reason| step_error(&step.label, &format!("编译失败: {reason}")))?;

            let evaluated = evaluator::eval(&compiled, &variable_table)
                .map_err(|e| step_error(&step.label, &format!("求值失败: {e}")))?;

            last_value = evaluated.primary;
            last_clean_expr = clean_expr;

            // 绑定赋值目标（如 `x = …` 中的 `x`），供后续步骤引用
            if let Some(sym) = seg.symbol.as_ref() {
                variable_table.insert(sym.clone(), last_value);
                step_cells.insert(sym.clone(), last_cell.clone());
                step_values.insert(sym.clone(), last_value);
            }

            sub_seg_counter += 1;
        }

        // 兼容：同时绑定 step.symbol（数据可能用 symbol 引用前序结果）
        variable_table.insert(step.symbol.clone(), last_value);
        step_cells.insert(step.symbol.clone(), last_cell.clone());
        step_values.insert(step.symbol.clone(), last_value);

        let value = last_value;

        // 完整代入公式：`label = expression = value 单位`
        let value_str = format_step_value(value);
        let mut substituted = format!("{} = {} = {}", step.label, last_clean_expr, value_str);
        if !step.unit.is_empty() {
            substituted.push(' ');
            substituted.push_str(&step.unit);
        }

        // Excel 公式：优先 AI 给的，否则本地转换（BUG-10：两条路径都是引用式）
        let excel_expr = match excel_steps {
            Some(es) if index < es.len() => es[index].expression.clone(),
            _ => {
                let ref_map = merged_ref_map(&base_cell_refs, &step_cells);
                let mut warnings = Vec::new();
                let converted = ExcelFormulaConverter::new().convert_expression(
                    &last_clean_expr,
                    &ref_map,
                    &mut warnings,
                    &merged_constants,
                );
                if converted.starts_with('=') {
                    converted
                } else {
                    format!("={converted}")
                }
            }
        };

        // 当前步骤结果也进入代值表，供本步及后续步骤的「代入数值版」使用
        cell_values.insert(last_cell.clone(), value);

        let mut value_map: HashMap<String, String> = HashMap::new();
        for (sym, v) in inputs {
            value_map.insert(sym.clone(), format_step_value(*v));
        }
        for (sym, v) in &step_values {
            value_map.insert(sym.clone(), format_step_value(*v));
        }
        let ref_map = merged_ref_map(&base_cell_refs, &step_cells);
        let excel_value_expr = substitute_values(&excel_expr, &ref_map, &value_map).formula;

        results.push(StepResult {
            symbol: step.symbol.clone(),
            label: step.label.clone(),
            group: step.group.clone(),
            value,
            unit: step.unit.clone(),
            expression: step.expression.clone(),
            substituted_expression: substituted,
            excel_formula: excel_expr,
            excel_value_formula: excel_value_expr,
        });
    }

    Ok(results)
}

/// 变量单元格表 + 步骤单元格表 → 一张合并表（两套编号不冲突，见模块文档）
fn merged_ref_map(
    base: &HashMap<String, String>,
    steps: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut m = base.clone();
    m.extend(steps.iter().map(|(k, v)| (k.clone(), v.clone())));
    m
}

fn step_error(label: &str, detail: &str) -> EvalError {
    EvalError::Compile {
        position: None,
        msg: format!("步骤 '{label}' {detail}"),
    }
}

/// 编译单个表达式段（不经 `engine::compile` 的 `normalize`）。
///
/// ⚠️ 步骤表达式里的 `=` 已在 [`splitter::split`] 阶段被剥成 `symbol`，
/// 所以这里不需要 `normalize`；直接用 lexer 能拿到「`=` 残留就报错」的严格行为。
fn compile_segment(
    expr: &str,
    constants: &HashMap<String, f64>,
    functions: &std::collections::HashSet<String>,
) -> Result<crate::engine::types::CompiledExpr, String> {
    let mut lx = lexer::Lexer::new(expr, constants, functions);
    let tokens = lx.tokenize().map_err(|e| e.to_string())?;
    match parser::parse(&tokens) {
        CompileResult::Ok { expr } => Ok(expr),
        CompileResult::Error { reason, .. } => Err(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::StepTemplate;

    fn step(symbol: &str, label: &str, expression: &str) -> StepTemplate {
        StepTemplate {
            symbol: symbol.to_string(),
            label: label.to_string(),
            group: None,
            expression: expression.to_string(),
            unit: String::new(),
            note: None,
        }
    }

    fn step_u(symbol: &str, label: &str, expression: &str, unit: &str) -> StepTemplate {
        StepTemplate {
            unit: unit.to_string(),
            ..step(symbol, label, expression)
        }
    }

    fn inputs(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    fn order(syms: &[&str]) -> Vec<String> {
        syms.iter().map(|s| (*s).to_string()).collect()
    }

    fn run(steps: &[StepTemplate], ins: &[(&str, f64)], vars: &[&str]) -> Vec<StepResult> {
        eval_steps(steps, &inputs(ins), &HashMap::new(), None, &order(vars)).expect("应成功")
    }

    // ------------------------------------------------------------ 基本

    #[test]
    fn empty_steps_returns_empty() {
        let r = eval_steps(&[], &inputs(&[]), &HashMap::new(), None, &[]).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn single_step_basic() {
        let steps = vec![step("A", "面积", "b*h")];
        let r = run(&steps, &[("b", 2.0), ("h", 3.0)], &["b", "h"]);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].symbol, "A");
        assert_eq!(r[0].label, "面积");
        assert_eq!(r[0].value, 6.0);
        assert_eq!(r[0].expression, "b*h");
    }

    /// 代入公式形如 `label = expr = value`
    #[test]
    fn substituted_expression_format() {
        let steps = vec![step("A", "面积", "b*h")];
        let r = run(&steps, &[("b", 2.0), ("h", 3.0)], &["b", "h"]);
        assert_eq!(r[0].substituted_expression, "面积 = b*h = 6");
    }

    /// 有单位时拼在末尾
    #[test]
    fn substituted_expression_with_unit() {
        let steps = vec![step_u("A", "面积", "b*h", "m2")];
        let r = run(&steps, &[("b", 2.0), ("h", 3.0)], &["b", "h"]);
        assert_eq!(r[0].substituted_expression, "面积 = b*h = 6 m2");
        assert_eq!(r[0].unit, "m2");
    }

    // ------------------------------------------------------------ 步骤间引用

    /// 后续步骤可引用前序步骤的 `symbol`
    #[test]
    fn later_step_references_earlier_symbol() {
        let steps = vec![step("A", "面积", "b*h"), step("V", "体积", "A*d")];
        let r = run(&steps, &[("b", 2.0), ("h", 3.0), ("d", 4.0)], &["b", "h", "d"]);
        assert_eq!(r[0].value, 6.0);
        assert_eq!(r[1].value, 24.0, "体积 = 6*4");
    }

    // ------------------------------------------------------------ BUG-03：多段

    /// **BUG-03**：步骤表达式含多段时，逐段求值，**步骤结果 = 最后一段**
    #[test]
    fn bug03_multi_segment_step_uses_last_segment() {
        let steps = vec![step("Z", "复合", "a = b*2; c = a+1; Z = c*10")];
        let r = run(&steps, &[("b", 3.0)], &["b"]);
        // b=3 → a=6 → c=7 → Z=70
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].value, 70.0, "应取最后一段");
    }

    /// 多段里的中间符号可被**后续步骤**引用
    #[test]
    fn bug03_intermediate_symbol_visible_to_later_steps() {
        let steps = vec![
            step("Z", "复合", "a = b*2; c = a+1; Z = c*10"),
            step("W", "引用中间量", "a + c"),
        ];
        let r = run(&steps, &[("b", 3.0)], &["b"]);
        assert_eq!(r[1].value, 13.0, "a=6, c=7 → 13");
    }

    /// 无赋值前缀的多段：不影响求值，最后一段为准
    #[test]
    fn multi_segment_without_symbols() {
        let steps = vec![step("Z", "两段", "1+1; 2+3")];
        let r = run(&steps, &[], &[]);
        assert_eq!(r[0].value, 5.0);
    }

    // ------------------------------------------------------------ BUG-10：Excel

    /// **BUG-10**：本地转换输出**单元格引用式**（变量第 1 行横向）
    #[test]
    fn bug10_local_excel_uses_cell_refs() {
        let steps = vec![step("A", "面积", "b*h")];
        let r = run(&steps, &[("b", 2.0), ("h", 3.0)], &["b", "h"]);
        assert_eq!(r[0].excel_formula, "=A1*B1", "变量 → 第 1 行横向");
    }

    /// 步骤结果 → A 列纵向（第 0 个子段 → A2）
    #[test]
    fn bug10_step_result_maps_to_column_a() {
        let steps = vec![step("A", "面积", "b*h"), step("V", "体积", "A*d")];
        let r = run(&steps, &[("b", 2.0), ("h", 3.0), ("d", 4.0)], &["b", "h", "d"]);
        assert_eq!(r[0].excel_formula, "=A1*B1");
        // 第二步引用第一步的结果 → A2（不是 A1）
        assert_eq!(r[1].excel_formula, "=A2*C1");
    }

    /// 多段步骤的每个子段各占一个单元格
    #[test]
    fn bug10_each_sub_segment_gets_its_own_cell() {
        let steps = vec![step("Z", "复合", "a = b*2; c = a+1; Z = c*10")];
        let r = run(&steps, &[("b", 3.0)], &["b"]);
        // 三段 → A2 / A3 / A4；表达式引用 a、c 时用它们的单元格
        assert!(r[0].excel_formula.contains("A3"), "应引用 c 的单元格 A3: {}", r[0].excel_formula);
    }

    /// 有 AI 的 Excel 步骤模板时**优先用它**
    #[test]
    fn ai_excel_steps_take_precedence() {
        let steps = vec![step("A", "面积", "b*h")];
        let ai = vec![step("A", "面积", "=B1*A1")];
        let r = eval_steps(
            &steps,
            &inputs(&[("b", 2.0), ("h", 3.0)]),
            &HashMap::new(),
            Some(&ai),
            &order(&["b", "h"]),
        )
        .unwrap();
        assert_eq!(r[0].excel_formula, "=B1*A1", "应用 AI 的表达式");
    }

    /// AI 模板数量不足时，多出来的步骤回落本地转换
    #[test]
    fn ai_excel_steps_shorter_than_steps_falls_back() {
        let steps = vec![step("A", "面积", "b*h"), step("V", "体积", "A*d")];
        let ai = vec![step("A", "面积", "=B1*A1")];
        let r = eval_steps(
            &steps,
            &inputs(&[("b", 2.0), ("h", 3.0), ("d", 4.0)]),
            &HashMap::new(),
            Some(&ai),
            &order(&["b", "h", "d"]),
        )
        .unwrap();
        assert_eq!(r[0].excel_formula, "=B1*A1");
        assert_eq!(r[1].excel_formula, "=A2*C1", "第二条应回落本地转换");
    }

    // ------------------------------------------------------------ 代入数值版

    /// 「代入数值版」把单元格引用换成实际数值
    #[test]
    fn excel_value_formula_substitutes_numbers() {
        let steps = vec![step("A", "面积", "b*h")];
        let r = run(&steps, &[("b", 2.0), ("h", 3.0)], &["b", "h"]);
        assert_eq!(r[0].excel_value_formula, "=2*3");
    }

    /// 第二步的代入数值版包含第一步的计算结果（经 A2 替换）
    #[test]
    fn excel_value_formula_uses_previous_step_value() {
        let steps = vec![step("A", "面积", "b*h"), step("V", "体积", "A*d")];
        let r = run(&steps, &[("b", 2.0), ("h", 3.0), ("d", 4.0)], &["b", "h", "d"]);
        assert_eq!(r[1].excel_value_formula, "=6*4", "A2 应替换成 6");
    }

    // ------------------------------------------------------------ 失败语义

    #[test]
    fn empty_step_expression_fails() {
        let steps = vec![step("A", "空", "")];
        let e = eval_steps(&steps, &inputs(&[]), &HashMap::new(), None, &[]).unwrap_err();
        assert!(format!("{e}").contains("表达式为空"));
    }

    #[test]
    fn compile_error_mentions_step_label() {
        let steps = vec![step("A", "坏步骤", "b + (h")];
        let e = eval_steps(
            &steps,
            &inputs(&[("b", 1.0), ("h", 2.0)]),
            &HashMap::new(),
            None,
            &order(&["b", "h"]),
        )
        .unwrap_err();
        let msg = format!("{e}");
        assert!(msg.contains("坏步骤"), "错误应带步骤名: {msg}");
        assert!(msg.contains("编译失败"));
    }

    /// 任一步骤失败 → **整体失败**（不返回半截结果）
    #[test]
    fn failure_aborts_whole_run() {
        let steps = vec![step("A", "好的", "b*2"), step("B", "坏的", "c + (d")];
        let e = eval_steps(
            &steps,
            &inputs(&[("b", 1.0), ("c", 1.0), ("d", 2.0)]),
            &HashMap::new(),
            None,
            &order(&["b", "c", "d"]),
        )
        .unwrap_err();
        assert!(format!("{e}").contains("坏的"));
    }

    // ------------------------------------------------------------ 常量

    #[test]
    fn builtin_and_schema_constants_available() {
        let steps = vec![step("S", "周长", "2*pi*r"), step("T", "系数", "k*r")];
        let constants = HashMap::from([("k".to_string(), 10.0)]);
        let r = eval_steps(
            &steps,
            &inputs(&[("r", 1.0)]),
            &constants,
            None,
            &order(&["r"]),
        )
        .unwrap();

        // ⚠️ 不要写 `6.2832` 字面量 —— clippy 的 `approx_constant` 会判它是 TAU 近似。
        //     用常量构造期望值，同时也就表达了「2π 保留 4 位小数」这层含义。
        let two_pi_4dp = (std::f64::consts::TAU * 10_000.0).round() / 10_000.0;
        assert_eq!(r[0].value, two_pi_4dp, "2π 保留 4 位小数");
        assert_eq!(r[1].value, 10.0, "schema 常量生效");
    }

    // ------------------------------------------------------------ 值格式化

    #[test]
    fn step_value_formatting() {
        assert_eq!(format_step_value(6.0), "6");
        assert_eq!(format_step_value(-3.0), "-3");
        // 4 位小数（不是 6 位）
        assert_eq!(format_step_value(1.0 / 3.0), "0.3333");
        assert_eq!(format_step_value(2.0 / 3.0), "0.6667");
        assert_eq!(format_step_value(1.5), "1.5");
        assert_eq!(format_step_value(f64::NAN), "NaN");
        assert_eq!(format_step_value(f64::INFINITY), "Infinity");
        assert_eq!(format_step_value(f64::NEG_INFINITY), "-Infinity");
    }

    /// 步骤值格式化与计算书的 6 位口径不同 —— 钉住这个刻意的差异
    #[test]
    fn step_value_uses_four_decimals_not_six() {
        assert_eq!(format_step_value(1.0 / 7.0), "0.1429");
        // 6 位会是 0.142857
    }

    // ------------------------------------------------------------ 分组

    #[test]
    fn group_is_passed_through() {
        let steps = vec![StepTemplate {
            group: Some("第一组".to_string()),
            ..step("A", "面积", "b*h")
        }];
        let r = run(&steps, &[("b", 2.0), ("h", 3.0)], &["b", "h"]);
        assert_eq!(r[0].group.as_deref(), Some("第一组"));
    }

    /// 变量顺序决定单元格编号（换个顺序，编号跟着变）
    #[test]
    fn variable_order_drives_cell_numbering() {
        let steps = vec![step("A", "面积", "b*h")];
        let r1 = run(&steps, &[("b", 2.0), ("h", 3.0)], &["b", "h"]);
        assert_eq!(r1[0].excel_formula, "=A1*B1");

        let r2 = run(&steps, &[("b", 2.0), ("h", 3.0)], &["h", "b"]);
        assert_eq!(r2[0].excel_formula, "=B1*A1", "h 在前 → h 是 A1");
    }

    /// 未在 `variable_order` 里的符号：求值仍可（用 inputs），但 Excel 里保留原名
    #[test]
    fn symbol_outside_variable_order_keeps_name_in_excel() {
        let steps = vec![step("A", "面积", "b*h")];
        let r = run(&steps, &[("b", 2.0), ("h", 3.0)], &["b"]);
        assert_eq!(r[0].value, 6.0, "求值不受影响");
        assert!(
            r[0].excel_formula.contains('h'),
            "未编号的符号保留原名: {}",
            r[0].excel_formula
        );
    }

    #[test]
    fn many_steps_produce_many_results() {
        let steps: Vec<StepTemplate> = (0..5)
            .map(|i| step(&format!("S{i}"), &format!("第{i}步"), "b*2"))
            .collect();
        let r = run(&steps, &[("b", 1.0)], &["b"]);
        assert_eq!(r.len(), 5);
        assert!(r.iter().all(|s| s.value == 2.0));
    }
}
