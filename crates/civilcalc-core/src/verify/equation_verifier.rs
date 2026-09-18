//! 原始方程/条件式的代入验算（纯函数，不依赖 UI 与网络）。
//!
//! 源：`EquationVerifier.kt`（203 行）
//!
//! ## 职责
//!
//! 公式页把「条件方程组 → 解 → 结果 → 代入验算」排成一段解题过程。
//! 这里把需求里给出的**原始方程**逐条代入当前取值（用户填的系数 + 本次算出的结果量），
//! 算出两侧数值并判断是否相等。
//!
//! **验算全部由本地表达式引擎完成、不经模型** —— 因此结论可以直接当依据用。

use crate::engine::expr_utils::{replace_whole_word, AfterRule};
use crate::engine::lexer::TokenType;
use crate::engine::{self, function_table, implicit_mul, lexer, types::CompileResult};
use crate::excel::number_formatter;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// 圈号（Unicode 只到 ⑳，超过退回 `(21)` 这种括号数字）
const CIRCLED: [char; 20] = [
    '①', '②', '③', '④', '⑤', '⑥', '⑦', '⑧', '⑨', '⑩', '⑪', '⑫', '⑬', '⑭', '⑮', '⑯', '⑰', '⑱', '⑲',
    '⑳',
];

/// 绝对容差。
///
/// 求值器把最终结果折算到 **4 位小数**（源项目 `Evaluator.roundFinal`，见 P2-1），
/// 两侧各自最多差 `5e-5`，因此这个量级的差是显示精度造成的、**不能算「不成立」**。
const EPS_ABS: f64 = 1e-4;

/// 大数（`1e6` 以上）再叠一点相对容差，避免量级放大后 `1e-4` 显得过严。
const EPS_REL: f64 = 1e-9;

/// 一条原始方程的代入验算结果。
///
/// `holds` 为空表示「**无法核对**」而不是「不成立」：不是完整等式、缺取值、
/// 两侧无法解析都会落到这里，并把原因写进 `note` ——
/// 与项目其它降级路径一致，**宁可显示「未核对」也不静默丢掉一条方程**。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EquationCheck {
    /// 序号，从 1 起（空白项不占号）
    pub index: usize,
    /// 圈号标记（①②③…），与原始方程的给出顺序对应
    pub mark: String,
    /// 原始方程原文（已 trim）
    pub source: String,
    /// 代入当前取值后的左侧文本（如 `1+2+3`）
    pub left_text: String,
    /// 代入当前取值后的右侧文本
    pub right_text: String,
    /// 两侧数值是否相等；`None` = 无法核对
    pub holds: Option<bool>,
    /// 无法核对的原因（`holds == None` 时有值）
    #[serde(default)]
    pub note: Option<String>,
}

impl EquationCheck {
    /// 是否已得出明确结论（成立或不成立）
    pub fn is_checked(&self) -> bool {
        self.holds.is_some()
    }

    fn unchecked(index: usize, mark: String, source: &str, note: &str) -> Self {
        Self {
            index,
            mark,
            source: source.to_string(),
            left_text: String::new(),
            right_text: String::new(),
            holds: None,
            note: Some(note.to_string()),
        }
    }
}

/// 序号 → 圈号；超过 20 条用括号数字（圈号 Unicode 只到 ⑳）。
pub fn mark_of(index: usize) -> String {
    if (1..=CIRCLED.len()).contains(&index) {
        CIRCLED[index - 1].to_string()
    } else {
        format!("({index})")
    }
}

/// 逐条核对。
///
/// 空白项**直接跳过**（是空行而非内容），其余无论成败都产出对应结果 ——
/// 序号按非空项连续编号。
pub fn verify(equations: &[String], values: &HashMap<String, f64>) -> Vec<EquationCheck> {
    let mut checks = Vec::new();
    for raw in equations {
        let source = raw.trim();
        if source.is_empty() {
            continue;
        }
        checks.push(check_one(checks.len() + 1, source, values));
    }
    checks
}

/// 核对单条方程。
fn check_one(index: usize, source: &str, values: &HashMap<String, f64>) -> EquationCheck {
    let mark = mark_of(index);

    let Some(eq_index) = source.find('=') else {
        return EquationCheck::unchecked(index, mark, source, "不是完整的等式");
    };
    // `=` 在开头（`=6`）或在结尾（`x+y=`）都不是完整等式
    if eq_index == 0 || eq_index == source.len() - 1 {
        return EquationCheck::unchecked(index, mark, source, "不是完整的等式");
    }

    // ⚠️ **隐含乘号必须在代入之前补**（见模块文档）
    let left = implicit_mul::insert(source[..eq_index].trim());
    let right = implicit_mul::insert(source[eq_index + 1..].trim());

    let constants = function_table::constants_owned();
    let functions = function_table::function_name_set();

    let Some(left_symbols) = symbols_of(&left, &constants, &functions) else {
        return EquationCheck::unchecked(index, mark, source, "等式左侧无法解析");
    };
    let Some(right_symbols) = symbols_of(&right, &constants, &functions) else {
        return EquationCheck::unchecked(index, mark, source, "等式右侧无法解析");
    };

    // 代入表 = 内置常量 + 当前取值（常量在前，取值可覆盖同名）
    let mut substitutes = constants.clone();
    substitutes.extend(values.iter().map(|(k, v)| (k.clone(), *v)));

    // 去重**保持首现顺序**（左侧符号在前，与源项目 `(left + right).distinct()` 一致）
    let mut missing: Vec<String> = Vec::new();
    for symbol in left_symbols.iter().chain(right_symbols.iter()) {
        if !substitutes.contains_key(symbol.as_str()) && !missing.contains(symbol) {
            missing.push(symbol.clone());
        }
    }

    if !missing.is_empty() {
        return EquationCheck {
            index,
            mark,
            source: source.to_string(),
            left_text: substitute(&left, &substitutes),
            right_text: substitute(&right, &substitutes),
            holds: None,
            note: Some(missing_note(&missing, values)),
        };
    }

    let left_value = evaluate(&left, values);
    let right_value = evaluate(&right, values);
    let left_text = substitute(&left, &substitutes);
    let right_text = substitute(&right, &substitutes);

    let (Some(lv), Some(rv)) = (left_value, right_value) else {
        return EquationCheck {
            index,
            mark,
            source: source.to_string(),
            left_text,
            right_text,
            holds: None,
            note: Some("两侧无法求值".to_string()),
        };
    };

    let tolerance = EPS_ABS + EPS_REL * lv.abs().max(rv.abs()).max(1.0);
    EquationCheck {
        index,
        mark,
        source: source.to_string(),
        left_text,
        right_text,
        holds: Some((lv - rv).abs() <= tolerance),
        note: None,
    }
}

/// 缺取值时把原因写清楚，并尽量给出下一步怎么核对。
///
/// 多根方程（一元二次等）的结果符号是 `x1`/`x2`，而原方程里的未知量写的是 `x` ——
/// 一个 `x` 代不进两个根，只能提示用户把每个根分别代回。
/// **不替用户猜该代哪个根**。
fn missing_note(missing: &[String], values: &HashMap<String, f64>) -> String {
    let mut parts = vec![format!("缺取值：{}", missing.join("、"))];

    for symbol in missing {
        let mut roots: Vec<&String> = values
            .keys()
            .filter(|k| {
                k.len() > symbol.len()
                    && k.starts_with(symbol.as_str())
                    && k[symbol.len()..].bytes().all(|b| b.is_ascii_digit())
            })
            .collect();
        roots.sort();

        if !roots.is_empty() {
            let list = roots
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("、");
            parts.push(format!("结果是 {list}，请分别代回核对"));
        }
    }

    parts.join("；")
}

/// 取等式一侧出现的变量符号；解析失败返回 `None`（**不猜**）。
fn symbols_of(
    text: &str,
    constants: &HashMap<String, f64>,
    functions: &HashSet<String>,
) -> Option<Vec<String>> {
    let mut lx = lexer::Lexer::new(text, constants, functions);
    let tokens = lx.tokenize().ok()?;
    Some(
        tokens
            .iter()
            .filter(|t| t.kind == TokenType::Variable)
            .map(|t| t.value.clone())
            .collect(),
    )
}

/// 把变量与命名常数换成数值字面量（如 `pi*r^2` + `r=1` → `3.141593*1^2`），
/// 让用户能直接照着算式手算。
///
/// **整词替换** —— 避免 `r` 命中 `rho`、`e` 命中 `exp`。
/// 长符号优先（`a11` 先于 `a1`），同长度按符号名排序保证**结果确定可复现**
/// （`HashMap` 迭代顺序是随机的，不排序会让展示文本在多次运行间跳变）。
fn substitute(text: &str, values: &HashMap<String, f64>) -> String {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '\'';

    let mut entries: Vec<(&String, &f64)> = values.iter().collect();
    entries.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(b.0)));

    let mut out = text.to_string();
    for (symbol, value) in entries {
        let Some(lit) = literal(*value) else {
            continue;
        };
        out = replace_whole_word(&out, symbol, &lit, &is_word, &is_word, AfterRule::NotWordChar);
    }
    out
}

/// 展示用数值写法。
///
/// - 整数不带小数点
/// - 小数最多 6 位再去尾零（与界面其它数值口径一致）
/// - 量级极端（`>= 1e6` 或 `< 1e-4`）走 Excel 数值格式（避免 `0.0000123456789`）
/// - **负数加括号**（`+(-3)` 比 `+-3` 好认，也与「代入数值」的写法一致）
///
/// 命名常数（`pi = 3.141592653589793`）直写全精度没法看，这里收成 `3.141593`；
/// 6 位小数带来的偏差远小于 [`EPS_ABS`]，**不会让展示与判定结论打架**。
fn literal(value: f64) -> Option<String> {
    if !value.is_finite() {
        return None;
    }

    let text = if value == (value as i64) as f64 {
        (value as i64).to_string()
    } else {
        let magnitude = value.abs();
        // 量级在「正常范围」之外才走 Excel 数值格式（避免 `0.0000123456789`）
        if !(1e-4..1e6).contains(&magnitude) {
            number_formatter::format(value)?
        } else {
            let s = format!("{value:.6}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        }
    };

    Some(if value < 0.0 {
        format!("({text})")
    } else {
        text
    })
}

/// 求一侧的值；缺取值/解析失败/结果非有限值都返回 `None`。
fn evaluate(expr: &str, values: &HashMap<String, f64>) -> Option<f64> {
    // 内置常量由 `compile` 内部合并；这里没有 schema 自定义常量（`verify` 的入参只有取值）
    let compiled = match engine::compile(expr, &HashMap::new()) {
        CompileResult::Ok { expr } => expr,
        CompileResult::Error { .. } => return None,
    };

    match engine::eval(&compiled, values) {
        Ok(r) => Some(r.primary).filter(|v| v.is_finite()),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{FormulaSchema, FormulaSource, SourceKind};

    fn vals(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    fn eqs(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    // ============================================================ 成立/不成立

    #[test]
    fn three_unknown_system_all_hold() {
        let checks = verify(
            &eqs(&["x+y+z=6", "2x-y+z=3", "x+2y-z=2"]),
            &vals(&[("x", 1.0), ("y", 2.0), ("z", 3.0)]),
        );
        assert_eq!(checks.len(), 3);
        assert!(checks.iter().all(|c| c.holds == Some(true)), "三条都应成立");
        assert_eq!(checks[0].mark, "①");
        assert_eq!(checks[0].left_text, "1+2+3");
        assert_eq!(checks[0].right_text, "6");

        // 原文保留用户写法，代入行补出显式乘号（否则 `2x` 代入会变成 `21`）
        assert_eq!(checks[1].source.split('=').next().unwrap(), "2x-y+z");
        assert_eq!(checks[1].left_text, "2*1-2+3");
        assert_eq!(checks[2].left_text, "1+2*2-3");
    }

    #[test]
    fn wrong_value_marks_not_holding() {
        let checks = verify(
            &eqs(&["x+y+z=6", "2x-y+z=3", "x+2y-z=2"]),
            &vals(&[("x", 1.0), ("y", 2.0), ("z", 4.0)]),
        );
        assert!(
            checks.iter().all(|c| c.holds == Some(false)),
            "z 取 4 时三条都不成立"
        );
        assert_eq!(checks[0].note, None, "成立/不成立都算已核对");
    }

    // ============================================================ 无法核对

    #[test]
    fn missing_value_is_unchecked_with_reason() {
        let checks = verify(&eqs(&["x+y+z=6"]), &vals(&[("x", 1.0), ("y", 2.0)]));
        let check = &checks[0];
        assert_eq!(check.holds, None);
        assert!(!check.is_checked());
        assert!(
            check.note.as_deref().unwrap_or("").contains('z'),
            "原因要写清缺的符号，实际：{:?}",
            check.note
        );
        assert_eq!(check.left_text, "1+2+z");
    }

    #[test]
    fn coefficients_and_unknowns_substituted_together() {
        let checks = verify(
            &eqs(&["a11*x+a12*y=b1"]),
            &vals(&[
                ("a11", 2.0),
                ("a12", 3.0),
                ("x", 1.0),
                ("y", 2.0),
                ("b1", 8.0),
            ]),
        );
        let check = &checks[0];
        assert_eq!(check.holds, Some(true));
        assert_eq!(check.left_text, "2*1+3*2");
        assert_eq!(check.right_text, "8");
    }

    /// 命名常数代入数值便于手算（`pi` 收成 6 位小数）
    #[test]
    fn named_constants_substituted_for_manual_check() {
        // 用常量构造而不是写 `3.141593` 字面量 —— 后者会被 clippy 判为 PI 近似（`approx_constant`）
        let pi_6dp = (std::f64::consts::PI * 1_000_000.0).round() / 1_000_000.0;
        let checks = verify(&eqs(&["V=pi*r^2"]), &vals(&[("V", pi_6dp), ("r", 1.0)]));
        let check = &checks[0];
        assert_eq!(check.holds, Some(true));
        assert_eq!(check.left_text, "3.141593");
        assert_eq!(check.right_text, "3.141593*1^2");
    }

    /// 多根方程缺未知量时提示把各个根分别代回
    #[test]
    fn multi_root_equation_hints_each_root() {
        // 一元二次：结果是 `x1`/`x2`，原方程里的未知量写的是 `x`
        let checks = verify(
            &eqs(&["a*x^2+b*x+c=0"]),
            &vals(&[
                ("a", 1.0),
                ("b", -3.0),
                ("c", 2.0),
                ("x1", 1.0),
                ("x2", 2.0),
            ]),
        );
        let note = checks[0].note.as_deref().unwrap_or("");
        assert_eq!(checks[0].holds, None);
        assert!(note.contains("缺取值：x"), "要报出缺的符号，实际：{note}");
        assert!(note.contains("x1、x2"), "要提示可用的是哪些根，实际：{note}");
        assert_eq!(checks[0].left_text, "1*x^2+(-3)*x+2");
    }

    #[test]
    fn non_equation_is_unchecked() {
        let checks = verify(&eqs(&["x>1", "x+y"]), &vals(&[("x", 1.0), ("y", 2.0)]));
        assert_eq!(checks.len(), 2);
        assert!(checks.iter().all(|c| c.holds.is_none()));
        assert!(checks[0].note.as_deref().unwrap_or("").contains("等式"));
    }

    /// `=` 在开头/结尾也不算完整等式
    #[test]
    fn equals_at_edges_is_unchecked() {
        for bad in ["=6", "x+y="] {
            let checks = verify(&eqs(&[bad]), &vals(&[("x", 1.0)]));
            assert_eq!(checks[0].holds, None, "`{bad}` 应判未核对");
        }
    }

    // ============================================================ 序号与空白

    #[test]
    fn blank_items_skipped_and_index_continuous() {
        let checks = verify(&eqs(&["", "  x = 1  ", "   "]), &vals(&[("x", 1.0)]));
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].index, 1);
        assert_eq!(checks[0].mark, "①");
        assert_eq!(checks[0].source, "x = 1");
        assert_eq!(checks[0].holds, Some(true));
    }

    #[test]
    fn no_equations_yields_nothing() {
        assert!(verify(&[], &vals(&[("x", 1.0)])).is_empty());
    }

    #[test]
    fn marks_use_circled_numbers_then_parens() {
        assert_eq!(mark_of(1), "①");
        assert_eq!(mark_of(20), "⑳");
        assert_eq!(mark_of(21), "(21)");
        assert_eq!(mark_of(0), "(0)");
    }

    // ============================================================ 容差

    /// 判定按求值器的四位小数精度给容差
    #[test]
    fn tolerance_matches_evaluator_precision() {
        let tolerant = verify(
            &eqs(&["a=b"]),
            &vals(&[("a", 1.0 / 3.0), ("b", 0.3333333333333)]),
        );
        assert_eq!(
            tolerant[0].holds,
            Some(true),
            "1/3 与 0.3333333333 折算到 4 位小数后应算成立"
        );

        let strict = verify(&eqs(&["a=b"]), &vals(&[("a", 1.0), ("b", 1.001)]));
        assert_eq!(strict[0].holds, Some(false));
    }

    /// 大数按相对容差放宽
    ///
    /// 容差 = `EPS_ABS + EPS_REL * max(1, |左|, |右|)`。
    /// `1e9` 量级时相对项贡献约 `1.0` —— 差 `0.5` 远超纯绝对容差（`1e-4`）却能判成立，
    /// 这正是相对项存在的意义（量级放大后 `1e-4` 会显得过严）。
    #[test]
    fn large_values_get_relative_tolerance() {
        let checks = verify(&eqs(&["a=b"]), &vals(&[("a", 1e9), ("b", 1e9 + 0.5)]));
        assert_eq!(
            checks[0].holds,
            Some(true),
            "1e9 量级的 0.5 差应由相对容差吸收"
        );

        // 但差到 50 就超出容差了
        let too_far = verify(&eqs(&["a=b"]), &vals(&[("a", 1e9), ("b", 1e9 + 50.0)]));
        assert_eq!(too_far[0].holds, Some(false));
    }

    // ============================================================ 代入文本

    #[test]
    fn substitute_does_not_damage_longer_symbols() {
        let values = vals(&[("r", 1.0), ("rho", 2.0)]);
        // `rho` 不该被 `r` 拆成 `1ho`
        assert_eq!(substitute("rho", &values), "2");
        assert_eq!(substitute("r+rho", &values), "1+2");
    }

    #[test]
    fn substitute_does_not_touch_function_names() {
        let values = vals(&[("e", 2.0)]);
        // `exp` 里的 `e` 不该被替换
        assert_eq!(substitute("exp(x)", &values), "exp(x)");
    }

    /// 长符号优先：`a11` 不能被 `a1` 抢先替换
    #[test]
    fn substitute_prefers_longer_symbols() {
        let values = vals(&[("a1", 1.0), ("a11", 2.0)]);
        assert_eq!(substitute("a11", &values), "2");
    }

    #[test]
    fn literal_formatting() {
        assert_eq!(literal(2.0).as_deref(), Some("2"));
        assert_eq!(literal(1.5).as_deref(), Some("1.5"));
        assert_eq!(literal(-3.0).as_deref(), Some("(-3)"));
        assert_eq!(literal(-2.5).as_deref(), Some("(-2.5)"));
        // 6 位小数去尾零
        assert_eq!(literal(1.0 / 3.0).as_deref(), Some("0.333333"));
        // 非有限值 → None（调用方跳过替换）
        assert_eq!(literal(f64::NAN), None);
        assert_eq!(literal(f64::INFINITY), None);
    }

    /// 极大/极小量级走 Excel 数值格式
    #[test]
    fn literal_uses_excel_format_at_extremes() {
        let small = literal(1e-5).unwrap();
        assert!(!small.contains('E') || small.contains("E-"), "实际 {small}");
        let big = literal(2e6).unwrap();
        assert!(big.contains('E') || big.contains("2000000"), "实际 {big}");
    }

    // ============================================================ 求值

    #[test]
    fn evaluate_returns_none_on_unparsable() {
        assert_eq!(evaluate("a@b", &vals(&[("a", 1.0)])), None);
    }

    #[test]
    fn evaluate_returns_none_for_missing_variable() {
        // 缺 `b` → 引擎报错 → None
        assert_eq!(evaluate("a+b", &vals(&[("a", 1.0)])), None);
    }

    /// 内置常量可用；⚠️ 结果会被**折算到 4 位小数**（源项目 `roundFinal`）
    #[test]
    fn evaluate_uses_builtin_constants() {
        let v = evaluate("pi", &HashMap::new()).expect("pi 应可求值");
        let pi_4dp = (std::f64::consts::PI * 10_000.0).round() / 10_000.0;
        assert_eq!(v, pi_4dp, "求值结果统一保留 4 位小数");
        assert!((v - std::f64::consts::PI).abs() < 1e-4, "与真值差在 4 位精度内");
    }

    // ============================================================ serde

    #[test]
    fn equation_check_serde_shape() {
        let checks = verify(&eqs(&["x+y=3"]), &vals(&[("x", 1.0), ("y", 2.0)]));
        let v = serde_json::to_value(&checks[0]).unwrap();
        assert_eq!(v["index"], serde_json::json!(1));
        assert_eq!(v["mark"], serde_json::json!("①"));
        assert_eq!(v["holds"], serde_json::json!(true));
        assert!(v.get("leftText").is_some());
        assert!(v.get("rightText").is_some());
        assert!(v.get("left_text").is_none(), "不得泄漏 snake_case");
    }

    /// `sourceEquations` 缺省为空且旧 JSON 可解（存量公式无该字段）
    #[test]
    fn source_equations_defaults_to_empty_and_legacy_json_parses() {
        let mut schema = FormulaSchema {
            id: "usr:eq".to_string(),
            result_name: "三元一次方程组".to_string(),
            result_symbol: "x".to_string(),
            result_unit: None,
            result_outputs: Vec::new(),
            expression: "x = 1".to_string(),
            source_equations: vec!["x+y+z=6".to_string(), "2x-y+z=3".to_string()],
            alt_expressions: Vec::new(),
            constants: HashMap::new(),
            variables: Vec::new(),
            domain: String::new(),
            tags: Vec::new(),
            reference_basis: None,
            design_notes: None,
            explanation: None,
            image_ids: Vec::new(),
            revised_from: None,
            source: FormulaSource {
                kind: SourceKind::Ai,
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
        };

        let encoded = serde_json::to_string(&schema).unwrap();
        assert!(encoded.contains("sourceEquations"));
        let decoded: FormulaSchema = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.source_equations, ["x+y+z=6", "2x-y+z=3"]);

        // 存量公式（旧 JSON 无该字段）→ 默认空列表
        schema.source_equations.clear();
        let legacy = serde_json::to_string(&schema).unwrap();
        let decoded: FormulaSchema = serde_json::from_str(&legacy).unwrap();
        assert!(decoded.source_equations.is_empty());
    }
}
