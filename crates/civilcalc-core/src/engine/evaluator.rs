//! 表达式求值（栈式 RPN 求值 + 逐步记录）。
//!
//! 源：`civilcalc-android-v2/core/formula/engine/Evaluator.kt`
//!
//! ## ⚠️ 舍入语义（BUG-04，已实测确认）
//!
//! 源项目 `roundFinal` 用 Kotlin `Double.roundToLong()`，
//! 等价于 Java `Math.round`，即 **ties toward +∞**（`floor(x + 0.5)`），
//! **不是** Rust 默认的 `f64::round()`（away from zero）。
//!
//! 实测差异：`Math.round(-2.5) = -2`，而 `(-2.5_f64).round() = -3`。
//! 因此本模块用 `(x + 0.5).floor()`。
//!
//! ## 其他关键行为
//!
//! - **中间结果不截断**（保留 f64 全精度）：`1/3*3 == 1`（截断会得 0.999999）
//! - 仅**最终结果**做 4 位小数展示舍入
//! - `NaN` / `Inf` **显式报错**，不静默转 0 或 `Long.MAX_VALUE`
//! - 大数（超出 i64 量级）**跳过舍入**，避免 `*1e4` 溢出

use crate::engine::function_table;
use crate::engine::types::{CompiledExpr, FormulaResult, RpnToken, OP_UNARY_MINUS};
use crate::schema::{EvalError, EvalResult, EvalStep};

/// 最终结果的展示舍入因子（4 位小数）
const FINAL_FACTOR: f64 = 10_000.0;

/// 4 位小数展示舍入。
///
/// 对齐 Kotlin：
/// ```kotlin
/// if (abs(value) >= Long.MAX_VALUE / FINAL_FACTOR) return value
/// return (value * FINAL_FACTOR).roundToLong() / FINAL_FACTOR
/// ```
///
/// 第一行是**溢出保护**：超出 i64 可表示量级时跳过舍入，
/// 否则 `*1e4` 会溢出为 `Long.MAX_VALUE`（源项目 BUG-04 的一半修复）。
pub fn round_final(value: f64) -> f64 {
    if value.abs() >= (i64::MAX as f64) / FINAL_FACTOR {
        return value;
    }
    // Kotlin roundToLong == Java Math.round == floor(x + 0.5)
    let scaled = value * FINAL_FACTOR;
    (scaled + 0.5).floor() / FINAL_FACTOR
}

/// 求值编译后的表达式。
pub fn eval(compiled: &CompiledExpr, inputs: &std::collections::HashMap<String, f64>) -> FormulaResult<EvalResult, EvalError> {
    let mut stack: Vec<f64> = Vec::new();
    let mut steps: Vec<EvalStep> = Vec::new();
    let mut step_counter: i32 = 0;

    for token in &compiled.rpn {
        match token {
            RpnToken::Number { value } => stack.push(*value),

            RpnToken::Constant { value, .. } => stack.push(*value),

            RpnToken::Variable { name } => match inputs.get(name) {
                Some(v) => stack.push(*v),
                None => {
                    return Err(EvalError::compile(crate::schema::err_variable_missing(name)));
                }
            },

            RpnToken::Operator { op, .. } => match op.as_str() {
                OP_UNARY_MINUS => {
                    let Some(a) = stack.pop() else {
                        return Err(stack_underflow());
                    };
                    let result = -a;
                    stack.push(result);
                    step_counter += 1;
                    steps.push(EvalStep {
                        step: step_counter,
                        description: "一元负号".to_string(),
                        formula: format!("-{}", kt_double(a)),
                        substituted_formula: format!("-{} = {}", kt_double(a), kt_double(result)),
                        result: Some(result),
                        error: None,
                    });
                }

                "+" | "-" | "*" | "/" | "^" | "%" => {
                    let Some(b) = stack.pop() else {
                        return Err(stack_underflow());
                    };
                    let Some(a) = stack.pop() else {
                        return Err(stack_underflow());
                    };

                    let raw_result = match op.as_str() {
                        "+" => a + b,
                        "-" => a - b,
                        "*" => a * b,
                        "/" => {
                            // 除零显式报错（源项目文案：除数不能为零）
                            if b == 0.0 {
                                return Err(EvalError::compile(crate::schema::ERR_DIVIDE_BY_ZERO));
                            }
                            a / b
                        }
                        "^" => a.powf(b),
                        "%" => a % b,
                        _ => unreachable!("已由 match 分支限定"),
                    };

                    stack.push(raw_result);
                    step_counter += 1;
                    let a_s = kt_double(a);
                    let b_s = kt_double(b);
                    steps.push(EvalStep {
                        step: step_counter,
                        description: op_name(op).to_string(),
                        formula: format!("{a_s} {op} {b_s}"),
                        substituted_formula: format!("{a_s} {op} {b_s} = {}", kt_double(raw_result)),
                        result: Some(raw_result),
                        error: None,
                    });
                }

                other => {
                    return Err(EvalError::compile(format!("未知运算符: {other}")));
                }
            },

            RpnToken::Function { name, arg_count } => {
                let Some(def) = function_table::get_function(name) else {
                    return Err(EvalError::compile(crate::schema::err_unknown_function(name)));
                };
                if stack.len() < *arg_count {
                    return Err(EvalError::compile(crate::schema::err_function_arg_count(name)));
                }

                // 从栈中取出 arg_count 个参数（保持原顺序）
                let mut args = vec![0.0f64; *arg_count];
                for i in (0..*arg_count).rev() {
                    args[i] = stack.pop().expect("已确认栈足够深");
                }

                // 定义域错误 → EvalError::Domain（携带符号名，供 UI 聚焦）
                let result = match (def.eval)(&args) {
                    Ok(v) => v,
                    Err(msg) => return Err(EvalError::Domain {
                        symbol: name.clone(),
                        detail: msg.to_string(),
                    }),
                };

                // 函数结果溢出/无效（如 pow(-1, 0.5) → NaN）显式报错
                if !result.is_finite() {
                    return Err(EvalError::Domain {
                        symbol: name.clone(),
                        detail: crate::schema::err_function_overflow(name),
                    });
                }

                stack.push(result);
                step_counter += 1;
                let args_str = args
                    .iter()
                    .map(|v| kt_double(*v))
                    .collect::<Vec<_>>()
                    .join(", ");
                steps.push(EvalStep {
                    step: step_counter,
                    description: format!("函数 {name}"),
                    formula: format!("{name}({args_str})"),
                    substituted_formula: format!("{name}({args_str}) = {}", kt_double(result)),
                    result: Some(result),
                    error: None,
                });
            }
        }
    }

    if stack.len() != 1 {
        return Err(EvalError::compile(crate::schema::err_stack_size(stack.len())));
    }

    let raw = stack[0];
    // 最终结果非有限 → 显式报错（覆盖 `*` 等运算符产生的 Inf）
    if !raw.is_finite() {
        return Err(EvalError::compile(crate::schema::ERR_NOT_FINITE));
    }

    let mut result = EvalResult::primary_only(round_final(raw));
    result.steps = steps;
    Ok(result)
}

/// 栈下溢（源项目会抛 `IndexOutOfBoundsException` 被外层捕获）。
///
/// 此处给出更明确的文案（源项目的 JVM 内部异常字符串对用户无意义）。
fn stack_underflow() -> EvalError {
    EvalError::compile("表达式不完整（运算符缺少操作数）")
}

/// 运算符中文名（用于步骤描述）
fn op_name(op: &str) -> &str {
    match op {
        "+" => "加法",
        "-" => "减法",
        "*" => "乘法",
        "/" => "除法",
        "^" => "乘方",
        "%" => "取模",
        other => other,
    }
}

/// 近似 Kotlin `Double.toString`（Java 语义）。
///
/// 差异点：Rust 的 `{}` 对整数值输出 `1`，而 Java/Kotlin 输出 `1.0`。
/// 步骤文案是**用户可见的**，因此需要对齐。
///
/// 说明：极端量级（`|v| < 1e-3` 或 `>= 1e7`）的 Java 科学计数法格式
/// 此处为近似实现，未经逐值比对 —— 常见工程数值（`1e-3`~`1e7`）已精确对齐。
pub fn kt_double(v: f64) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }

    let abs = v.abs();
    if (1e-3..1e7).contains(&abs) {
        let s = format!("{v}");
        if s.contains('.') {
            s
        } else {
            format!("{s}.0")
        }
    } else {
        // 科学计数法：Rust `{:e}` 形如 `1e10` / `1.5e-7`，补足 Java 的 `.0` 与 `E+`
        let s = format!("{v:e}");
        let (mantissa, exp) = s.split_once('e').unwrap_or((s.as_str(), "0"));
        let mantissa = if mantissa.contains('.') {
            mantissa.to_string()
        } else {
            format!("{mantissa}.0")
        };
        let (sign, digits) = if let Some(rest) = exp.strip_prefix('-') {
            ("-", rest)
        } else {
            ("+", exp.strip_prefix('+').unwrap_or(exp))
        };
        format!("{mantissa}E{sign}{digits}")
    }
}

#[cfg(test)]
// 测试中刻意使用 3.1416 / 12.5664 等"被 4 位小数舍入后的 π 近似值"，
// 这是**验收断言**（对齐源项目期望值），不是误用常量。
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;
    use crate::engine::lexer::Lexer;
    use crate::engine::parser;
    use std::collections::HashMap;

    fn compile(input: &str) -> CompiledExpr {
        let constants = function_table::constants_owned();
        let functions = function_table::function_name_set();
        let mut lx = Lexer::new(input, &constants, &functions);
        let tokens = lx.tokenize().expect("词法分析应成功");
        parser::parse(&tokens).expr().expect("解析应成功").clone()
    }

    fn eval_ok(input: &str, inputs: HashMap<String, f64>) -> f64 {
        let c = compile(input);
        let r = eval(&c, &inputs);
        assert!(r.is_ok(), "求值应成功: {r:?}");
        r.unwrap().primary
    }

    fn eval_err(input: &str, inputs: HashMap<String, f64>) -> EvalError {
        let c = compile(input);
        let r = eval(&c, &inputs);
        assert!(r.is_err(), "求值应失败");
        r.unwrap_err()
    }

    fn no_inputs() -> HashMap<String, f64> {
        HashMap::new()
    }

    fn inputs(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    // ================= 源项目 EvaluatorPrecisionTest 的用例（逐条移植） =================

    /// BUG-04：1/3*3 == 1（原中间 6 位截断会得到 0.999999）
    #[test]
    fn bug04_no_intermediate_truncation() {
        assert!((eval_ok("1/3*3", no_inputs()) - 1.0).abs() < 1e-12);
        assert!((eval_ok("(1/3)*3", no_inputs()) - 1.0).abs() < 1e-12);
    }

    /// BUG-04：1e308*10 上溢为 Inf → 报错，而非静默转 Long.MAX_VALUE 量级
    #[test]
    fn bug04_overflow_reports() {
        let err = eval_err("x*10", inputs(&[("x", 1e308)]));
        assert!(matches!(err, EvalError::Compile { .. }), "应为编译类错误: {err:?}");
    }

    /// BUG-04：NaN（pow 负底数开方）报错而非静默转 0
    #[test]
    fn bug04_nan_reports() {
        let err = eval_err("pow(-1, 0.5)", no_inputs());
        match err {
            EvalError::Domain { symbol, .. } => assert_eq!(symbol, "pow"),
            other => panic!("应为 Domain 错误，实际 {other:?}"),
        }
    }

    /// BUG-04：常规结果仍保留 4 位小数展示舍入
    #[test]
    fn bug04_final_rounding_kept() {
        assert!((eval_ok("pi", no_inputs()) - 3.1416).abs() < 1e-12);
    }

    /// BUG-04：大数结果不被 roundFinal 破坏（超出 i64 量级跳过舍入）
    #[test]
    fn bug04_huge_result_not_corrupted() {
        let v = eval_ok("x*10", inputs(&[("x", 1e300)]));
        assert!((v - 1e301).abs() < 1e288, "应为 1e301，实际 {v}");
    }

    // ================= 源项目 LexerScientificNotationTest 的求值侧 =================

    #[test]
    fn bug07_scientific_notation_evaluates() {
        assert!((eval_ok("1e3*2", no_inputs()) - 2000.0).abs() < 1e-9);
        assert!((eval_ok("2e-3", no_inputs()) - 0.002).abs() < 1e-12);
        assert!((eval_ok("1.5e+10*2", no_inputs()) - 1.5e10 * 2.0).abs() < 1e3);
    }

    #[test]
    fn bug07_euler_constant_evaluates() {
        assert!((eval_ok("e", no_inputs()) - 2.7183).abs() < 1e-9);
        assert!((eval_ok("e*2", no_inputs()) - 5.4366).abs() < 1e-9);
    }

    #[test]
    fn bug07_variable_after_number() {
        assert!((eval_ok("2*e1", inputs(&[("e1", 3.0)])) - 6.0).abs() < 1e-12);
    }

    // ================= 舍入语义 =================

    #[test]
    fn round_final_matches_java_math_round() {
        // Java: Math.round(3.141592653589793 * 10000) = 31416 → 3.1416
        assert_eq!(round_final(3.141592653589793), 3.1416);
        // Java: Math.round(12.566370614359172 * 10000) = 125664 → 12.5664
        assert_eq!(round_final(12.566370614359172), 12.5664);
        assert_eq!(round_final(1.0), 1.0);
        assert_eq!(round_final(2.0), 2.0);
    }

    #[test]
    fn round_final_uses_floor_x_plus_half_not_away_from_zero() {
        // -0.00005 * 10000 = -0.5 → Java Math.round(-0.5) = 0 → 0.0
        // 若误用 Rust round()（away from zero）会得 -1 → -0.0001
        assert_eq!(round_final(-0.00005), 0.0);
    }

    #[test]
    fn round_final_skips_huge_values() {
        let big = 1e301;
        assert_eq!(round_final(big), big, "超出 i64 量级应跳过舍入");
    }

    #[test]
    fn kt_double_formats_like_java() {
        assert_eq!(kt_double(1.0), "1.0");
        assert_eq!(kt_double(-2.0), "-2.0");
        assert_eq!(kt_double(12.5664), "12.5664");
        assert_eq!(kt_double(0.0), "0.0");
        assert_eq!(kt_double(-0.0), "-0.0");
        assert_eq!(kt_double(f64::NAN), "NaN");
        assert_eq!(kt_double(f64::INFINITY), "Infinity");
        assert_eq!(kt_double(f64::NEG_INFINITY), "-Infinity");
    }

    // ================= 运算符与错误 =================

    #[test]
    fn arithmetic_basics() {
        assert!((eval_ok("1+2", no_inputs()) - 3.0).abs() < f64::EPSILON);
        assert!((eval_ok("5-2", no_inputs()) - 3.0).abs() < f64::EPSILON);
        assert!((eval_ok("3*4", no_inputs()) - 12.0).abs() < f64::EPSILON);
        assert!((eval_ok("8/2", no_inputs()) - 4.0).abs() < f64::EPSILON);
        assert!((eval_ok("2^10", no_inputs()) - 1024.0).abs() < 1e-9);
        assert!((eval_ok("7%3", no_inputs()) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn unary_minus_negates() {
        assert!((eval_ok("-3", no_inputs()) + 3.0).abs() < f64::EPSILON);
        assert!((eval_ok("2*-3", no_inputs()) + 6.0).abs() < f64::EPSILON);
        assert!((eval_ok("2--3", no_inputs()) - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn divide_by_zero_reports_exact_message() {
        let err = eval_err("1/0", no_inputs());
        match err {
            EvalError::Compile { msg, .. } => assert_eq!(msg, "除数不能为零"),
            other => panic!("应为编译类错误，实际 {other:?}"),
        }
    }

    #[test]
    fn missing_variable_reports_name() {
        let err = eval_err("a+b", inputs(&[("a", 1.0)]));
        match err {
            EvalError::Compile { msg, .. } => assert_eq!(msg, "变量 'b' 未提供值"),
            other => panic!("应为编译类错误，实际 {other:?}"),
        }
    }

    #[test]
    fn domain_error_carries_symbol_for_ui_focus() {
        let err = eval_err("sqrt(x)", inputs(&[("x", -1.0)]));
        match err {
            EvalError::Domain { symbol, detail } => {
                assert_eq!(symbol, "sqrt", "UI 需要符号名来聚焦参数框");
                assert_eq!(detail, "sqrt 参数必须 >= 0");
            }
            other => panic!("应为 Domain 错误，实际 {other:?}"),
        }
    }

    // ================= 步骤记录 =================

    #[test]
    fn steps_are_recorded_in_order() {
        let c = compile("1+2*3");
        let r = eval(&c, &no_inputs()).unwrap();
        assert_eq!(r.steps.len(), 2, "两个运算符 → 两步");
        assert_eq!(r.steps[0].description, "乘法");
        assert_eq!(r.steps[0].result, Some(6.0));
        assert_eq!(r.steps[1].description, "加法");
        assert_eq!(r.steps[1].result, Some(7.0));
        // 步骤序号从 1 递增
        assert_eq!(r.steps[0].step, 1);
        assert_eq!(r.steps[1].step, 2);
    }

    #[test]
    fn step_text_uses_java_style_double_formatting() {
        let c = compile("1+2");
        let r = eval(&c, &no_inputs()).unwrap();
        assert_eq!(r.steps[0].formula, "1.0 + 2.0");
        assert_eq!(r.steps[0].substituted_formula, "1.0 + 2.0 = 3.0");
    }

    #[test]
    fn function_step_is_recorded() {
        let c = compile("sqrt(4)");
        let r = eval(&c, &no_inputs()).unwrap();
        assert_eq!(r.steps.len(), 1);
        assert_eq!(r.steps[0].description, "函数 sqrt");
        assert_eq!(r.steps[0].formula, "sqrt(4.0)");
    }

    #[test]
    fn unary_minus_step_is_recorded() {
        let c = compile("-3");
        let r = eval(&c, &no_inputs()).unwrap();
        assert_eq!(r.steps[0].description, "一元负号");
    }

    // ================= 常量与函数 =================

    #[test]
    fn constants_are_inlined_at_compile_time() {
        // 常量在编译期已替换为字面量，因此求值步骤为空
        let c = compile("pi");
        let r = eval(&c, &no_inputs()).unwrap();
        assert!(r.steps.is_empty(), "常量不产生步骤");
        assert!((r.primary - 3.1416).abs() < 1e-12);
    }

    #[test]
    fn multi_arg_function_evaluates() {
        assert!((eval_ok("pow(2,10)", no_inputs()) - 1024.0).abs() < 1e-9);
        assert!((eval_ok("hypot(3,4)", no_inputs()) - 5.0).abs() < 1e-12);
        assert!((eval_ok("max(3,7)", no_inputs()) - 7.0).abs() < f64::EPSILON);
    }
}
