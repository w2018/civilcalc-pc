//! 公式的二维排版结构（纯数据，不依赖任何 UI 框架）。
//!
//! 源：`MathLayout.kt`
//!
//! 表达式本身是一维字符串（`x = (b1*a22 - a12*b2)/(a11*a22 - a12*a21)`），
//! 直接看很难认。这里把它按运算树整理成分式/上标/根号等结构，
//! 由前端渲染成示意图，用户读公式与核对参数都更直观。

use crate::engine::types::{RpnToken, OP_UNARY_MINUS};
// 数值写法统一走 crate::number_format（原先本文件有一份私有重复实现，P4-4 去重）
use crate::number_format::format_number;
use crate::engine::{function_table, implicit_mul, lexer, parser, splitter, types::CompileResult};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// 下标数字（`0` → `₀`）
const SUBSCRIPT_DIGITS: [char; 10] = ['₀', '₁', '₂', '₃', '₄', '₅', '₆', '₇', '₈', '₉'];

// =============================================================================
// 布局树
// =============================================================================

/// `Text` 节点的语义分类（前端据此上色：符号 / 函数 / 运算符各一色）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextKind {
    /// 普通文本
    #[default]
    Normal,
    /// 数字字面量
    Number,
    /// 变量或常量符号
    Symbol,
    /// 运算符（`+` / `−` / `×` / ` = ` …）
    Operator,
    /// 函数名
    Function,
}

/// 排版树的节点。
///
/// `Vec<MathNode>` 表示「一串并排的节点」—— 不用单独的 `Row` 节点，
/// 因为序列本身就是行（源项目同样如此）。
///
/// serde 用 `type` 作判别字段（`{"type":"fraction",…}`）；
/// `Text` 的样式字段叫 `kind`（与源项目字段名一致，且与 `type` 不冲突）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MathNode {
    /// 普通文本（数字/符号/运算符/函数名）；`text` 已做显示美化（`pi`→`π`、`a11`→`a₁₁`）
    Text {
        text: String,
        #[serde(default)]
        kind: TextKind,
    },

    /// 分数：分子 / 分母
    Fraction {
        numerator: Vec<MathNode>,
        denominator: Vec<MathNode>,
    },

    /// 上标（幂）
    Superscript {
        base: Vec<MathNode>,
        exponent: Vec<MathNode>,
    },

    /// 根号；`index` 非空表示 n 次根
    Radical {
        radicand: Vec<MathNode>,
        #[serde(default)]
        index: Option<i32>,
    },

    /// 绝对值 `|x|`
    Absolute { inner: Vec<MathNode> },

    /// 函数调用：`name(arg, arg)`
    Function {
        name: String,
        args: Vec<Vec<MathNode>>,
    },

    /// 括号分组（一期未产出，保留位）
    Group { items: Vec<MathNode> },
}

impl MathNode {
    fn text(text: impl Into<String>, kind: TextKind) -> Self {
        Self::Text {
            text: text.into(),
            kind,
        }
    }

    fn operator(text: impl Into<String>) -> Self {
        Self::text(text, TextKind::Operator)
    }
}

/// 排版后的一行：可选「符号 =」前缀 + 右侧结构 + 可选行尾编号
/// （方程组用 ①②③，与原始方程顺序对应）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MathLine {
    /// 赋值目标符号（多结果公式的每段各一行）
    #[serde(default)]
    pub symbol: Option<String>,
    /// 该行的节点串
    pub nodes: Vec<MathNode>,
    /// 行尾编号（如 `①`），方程组用
    #[serde(default)]
    pub mark: Option<String>,
}

/// 整条公式的排版结果。
///
/// `warnings` 非空表示有段落没能排版 —— **前端应回落到原文展示**
/// （与 `ExcelFormulaConverter` 的降级惯例一致，绝不静默丢内容）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MathLayout {
    pub lines: Vec<MathLine>,
    pub warnings: Vec<String>,
}

impl MathLayout {
    /// 是否排出了至少一行（`false` 时前端必须回落原文）
    pub fn is_usable(&self) -> bool {
        !self.lines.is_empty()
    }

    /// 是否多行（多结果公式）
    pub fn is_multi(&self) -> bool {
        self.lines.len() > 1
    }
}

// =============================================================================
// 排版
// =============================================================================

/// 解析整条公式（含分号分段）为排版结构；解析不了的段落记入 `warnings`。
pub fn build(expression: &str) -> MathLayout {
    build_with_constants(expression, &HashMap::new())
}

/// 同 [`build`]，但带上 schema 自定义常量。
pub fn build_with_constants(expression: &str, constants: &HashMap<String, f64>) -> MathLayout {
    let segments = splitter::split(expression);
    if segments.is_empty() {
        crate::log::w("MathLayout", "表达式为空，无法排版", None);
        return MathLayout {
            lines: Vec::new(),
            warnings: vec!["表达式为空，无法排版".to_string()],
        };
    }

    let env = CompileEnv::new(constants);
    let mut warnings = Vec::new();
    let mut lines = Vec::new();

    for (idx, segment) in segments.iter().enumerate() {
        match build_segment(&segment.expression, &env) {
            Some(nodes) => lines.push(MathLine {
                symbol: segment.symbol.clone(),
                nodes,
                mark: None,
            }),
            None => {
                let label = match &segment.symbol {
                    Some(sym) => format!("第 {} 段「{sym} = …」", idx + 1),
                    None => format!("第 {} 段", idx + 1),
                };
                warnings.push(format!("{label} 无法排版，已按原文展示"));
                crate::log::w("MathLayout", &format!("表达式排版失败：{label}"), None);
            }
        }
    }

    MathLayout { lines, warnings }
}

/// 把一个表达式排成一串节点（**不拆行**）；解析失败返回 `None`，由调用方决定回落方式。
pub fn build_nodes(expression: &str) -> Option<Vec<MathNode>> {
    build_nodes_with_constants(expression, &HashMap::new())
}

/// 同 [`build_nodes`]，但带上 schema 自定义常量。
pub fn build_nodes_with_constants(
    expression: &str,
    constants: &HashMap<String, f64>,
) -> Option<Vec<MathNode>> {
    build_segment(expression, &CompileEnv::new(constants))
}

/// 把一条用户写的等式排成 `左侧 = 右侧` 的节点串（如 `x+y+z=6`）。
///
/// 等式**不是赋值语句**（`=` 不在开头），所以不能直接丢给表达式解析 ——
/// 先按**第一个** `=` 切成两半。
///
/// 无 `=`、`=` 在开头、`=` 在结尾、任一侧排不出来 → `None`
/// （回落方式由调用方决定，这里只保证不硬排出一条错位的式子）。
pub fn build_equation(equation: &str) -> Option<Vec<MathNode>> {
    build_equation_with_constants(equation, &HashMap::new())
}

/// 同 [`build_equation`]，但带上 schema 自定义常量。
pub fn build_equation_with_constants(
    equation: &str,
    constants: &HashMap<String, f64>,
) -> Option<Vec<MathNode>> {
    let eq = equation.find('=')?;
    // `=` 在开头（`=6`）或在结尾（`x+y=`）都不是完整等式
    if eq == 0 || eq == equation.len() - 1 {
        return None;
    }

    let left = build_nodes_with_constants(&equation[..eq], constants)?;
    let right = build_nodes_with_constants(&equation[eq + 1..], constants)?;

    let mut out = left;
    out.push(MathNode::operator(" = "));
    out.extend(right);
    Some(out)
}

/// 「结果」行的节点串：多结果 `(x, y, z) = (1, 2, 3)`；单结果不带括号（`V = 31.4159`）。
///
/// `results` 为按展示顺序排好的「符号 → 数值文本」，**数值文本由调用方按自己的口径格式化**
/// （排版层不管精度）。
pub fn build_result_line(results: &[(String, String)]) -> Vec<MathNode> {
    if results.is_empty() {
        return Vec::new();
    }
    let multi = results.len() > 1;
    let mut out = Vec::new();

    if multi {
        out.push(MathNode::operator("("));
    }
    for (index, (symbol, _)) in results.iter().enumerate() {
        if index > 0 {
            out.push(MathNode::operator(", "));
        }
        out.push(MathNode::text(symbol.clone(), TextKind::Symbol));
    }
    if multi {
        out.push(MathNode::operator(")"));
    }

    out.push(MathNode::operator(" = "));

    if multi {
        out.push(MathNode::operator("("));
    }
    for (index, (_, value)) in results.iter().enumerate() {
        if index > 0 {
            out.push(MathNode::operator(", "));
        }
        out.push(MathNode::text(value.clone(), TextKind::Number));
    }
    if multi {
        out.push(MathNode::operator(")"));
    }

    out
}

/// 编译环境（常量表 + 函数名集合）。
///
/// 预构造一次、多段复用 —— `engine::compile` 每次都重建这两张表。
struct CompileEnv {
    constants: HashMap<String, f64>,
    functions: HashSet<String>,
}

impl CompileEnv {
    fn new(constants: &HashMap<String, f64>) -> Self {
        let mut merged = function_table::constants_owned();
        merged.extend(constants.iter().map(|(k, v)| (k.clone(), *v)));
        Self {
            constants: merged,
            functions: function_table::function_name_set(),
        }
    }
}

/// 排版单个表达式段。空表达式或解析失败返回 `None`。
fn build_segment(expression: &str, env: &CompileEnv) -> Option<Vec<MathNode>> {
    if expression.trim().is_empty() {
        return None;
    }

    // ⚠️ 用户/教材写法里的隐含乘号先补成显式 `*`：
    //    不补则 `2x` 解析失败，只能降级成原文
    let normalized = implicit_mul::insert(expression);

    let rpn = compile_segment(&normalized, env)?;
    build_from_rpn(&rpn)
}

/// 编译单个表达式段，返回 RPN。
///
/// ⚠️ **刻意不用 [`crate::engine::compile`]** —— 它内部的 `normalize` 会走
/// `splitter` 把 `=` 当赋值前缀剥掉。那对求值是对的，但**对排版是错的**：
/// [`build_equation`] 的左右两侧可能残留 `=`（如 `a=b=c` 的右半边 `b=c`），
/// 走 `compile` 会把它静默排成 `c`，产出一条**错位的式子**。
///
/// 源项目直接把整串丢给 lexer —— `=` 会让词法分析报错 → 降级。这里保持同样行为。
fn compile_segment(expr: &str, env: &CompileEnv) -> Option<Vec<RpnToken>> {
    let mut lx = lexer::Lexer::new(expr, &env.constants, &env.functions);
    let tokens = lx.tokenize().ok()?;
    match parser::parse(&tokens) {
        CompileResult::Ok { expr } => Some(expr.rpn),
        CompileResult::Error { .. } => None,
    }
}

/// 中缀已由 `Parser` 转成 RPN，这里用栈还原成运算树。
fn build_from_rpn(rpn: &[RpnToken]) -> Option<Vec<MathNode>> {
    let mut stack: Vec<Vec<MathNode>> = Vec::new();

    for token in rpn {
        match token {
            RpnToken::Number { value } => stack.push(vec![MathNode::text(
                format_number(*value),
                TextKind::Number,
            )]),

            RpnToken::Variable { name } => {
                stack.push(vec![MathNode::text(pretty_symbol(name), TextKind::Symbol)]);
            }

            RpnToken::Constant { name, .. } => {
                stack.push(vec![MathNode::text(pretty_symbol(name), TextKind::Symbol)]);
            }

            RpnToken::Operator { op, .. } => {
                if op == OP_UNARY_MINUS {
                    let operand = stack.pop()?;
                    let mut nodes = vec![MathNode::operator("−")];
                    nodes.extend(operand);
                    stack.push(nodes);
                } else {
                    let right = stack.pop()?;
                    let left = stack.pop()?;
                    stack.push(binary(op, left, right));
                }
            }

            RpnToken::Function { name, arg_count } => {
                // 参数在栈上是「从左到右」压入的，出栈要倒序取
                let mut args: Vec<Vec<MathNode>> = vec![Vec::new(); *arg_count];
                for i in (0..*arg_count).rev() {
                    args[i] = stack.pop()?;
                }
                stack.push(vec![function_node(name, args)]);
            }
        }
    }

    // 恰好剩一个元素才算成功（多了说明表达式结构异常）
    if stack.len() == 1 {
        stack.pop()
    } else {
        None
    }
}

/// 二元运算 → 节点。
///
/// `/` 与 `^` 变成**二维结构**（分式 / 上标），其余拍平成行内文本。
/// 乘号显示成 `×` 而不是 `*`（`*` 在数学排版里不读作乘号）。
fn binary(op: &str, left: Vec<MathNode>, right: Vec<MathNode>) -> Vec<MathNode> {
    match op {
        "/" => vec![MathNode::Fraction {
            numerator: left,
            denominator: right,
        }],
        "^" => vec![MathNode::Superscript {
            base: left,
            exponent: right,
        }],
        "*" => join_with(left, "×", right),
        "+" => join_with(left, " + ", right),
        "-" => join_with(left, " − ", right),
        // `%` 是取模（不是百分号），显示成 `mod` 更准确
        "%" => join_with(left, " mod ", right),
        other => join_with(left, format!(" {other} "), right),
    }
}

fn join_with(left: Vec<MathNode>, op: impl Into<String>, right: Vec<MathNode>) -> Vec<MathNode> {
    let mut out = left;
    out.push(MathNode::operator(op));
    out.extend(right);
    out
}

/// 函数调用 → 节点。四个特例走二维形态，其余保持 `name(args)`。
fn function_node(name: &str, mut args: Vec<Vec<MathNode>>) -> MathNode {
    let arg_count = args.len();
    match (name, arg_count) {
        ("sqrt", 1) => MathNode::Radical {
            radicand: args.remove(0),
            index: None,
        },
        ("cbrt", 1) => MathNode::Radical {
            radicand: args.remove(0),
            index: Some(3),
        },
        ("abs", 1) => MathNode::Absolute {
            inner: args.remove(0),
        },
        // `pow(a, b)` 与 `a^b` 同义，按幂排版更好读
        ("pow", 2) => MathNode::Superscript {
            base: args.remove(0),
            exponent: args.remove(0),
        },
        _ => MathNode::Function {
            name: pretty_function_name(name),
            args,
        },
    }
}

/// 函数名的显示美化（只影响显示，不改表达式）。
fn pretty_function_name(name: &str) -> String {
    match name {
        "log10" => "log₁₀".to_string(),
        "asin" => "arcsin".to_string(),
        "acos" => "arccos".to_string(),
        "atan" => "arctan".to_string(),
        other => other.to_string(),
    }
}

/// 符号显示美化：`pi`→`π`、`a11`→`a₁₁`（**只影响显示**，不改表达式）。
///
/// 两种下标写法都支持：
/// - 单字母 + 1~2 位数字：`a11` → `a₁₁`、`x2` → `x₂`
/// - 字母串 + `_` + 1~2 位数字：`b_1` → `b₁`
///
/// ⚠️ **三位以上数字不做下标** —— 否则 `a123` 会变成 `a₁₂₃`，
/// 与「`a` 的第 123 号」混淆（源项目的刻意限制）。
pub fn pretty_symbol(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "pi" => return "π".to_string(),
        "e" => return "e".to_string(),
        _ => {}
    }

    if let Some(s) = match_index_form(name) {
        return s;
    }
    if let Some(s) = match_underscore_form(name) {
        return s;
    }
    name.to_string()
}

/// `^([A-Za-z])(\d{1,2})$` —— 单字母 + 1~2 位数字
fn match_index_form(name: &str) -> Option<String> {
    let bytes = name.as_bytes();
    // 长度 2~3 字节即「1 字母 + 1~2 数字」
    if bytes.len() < 2 || bytes.len() > 3 {
        return None;
    }
    if !bytes[0].is_ascii_alphabetic() {
        return None;
    }
    let digits = &name[1..];
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!("{}{}", &name[..1], to_subscript(digits)))
}

/// `^([A-Za-z]+)_(\d{1,2})$` —— 字母串 + `_` + 1~2 位数字
fn match_underscore_form(name: &str) -> Option<String> {
    let (letters, digits) = name.split_once('_')?;
    if letters.is_empty() || !letters.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    if digits.is_empty() || digits.len() > 2 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!("{letters}{}", to_subscript(digits)))
}

fn to_subscript(digits: &str) -> String {
    digits
        .chars()
        .map(|c| {
            if c.is_ascii_digit() {
                SUBSCRIPT_DIGITS[(c as u8 - b'0') as usize]
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::implicit_mul;

    fn build(expr: &str) -> MathLayout {
        super::build(expr)
    }

    fn texts(nodes: &[MathNode]) -> Vec<String> {
        nodes
            .iter()
            .filter_map(|n| match n {
                MathNode::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    // ============================================================ 基础形态

    #[test]
    fn division_becomes_fraction() {
        let layout = build("A = a/b");
        assert_eq!(layout.lines.len(), 1);
        assert_eq!(layout.lines[0].symbol.as_deref(), Some("A"));

        let node = layout.lines[0].nodes.first().unwrap();
        let MathNode::Fraction {
            numerator,
            denominator,
        } = node
        else {
            panic!("应为分数，实际 {node:?}");
        };
        assert_eq!(numerator.len(), 1);
        assert_eq!(denominator.len(), 1);
        assert_eq!(texts(numerator), ["a"]);
        assert_eq!(texts(denominator), ["b"]);
    }

    /// `(a-b)/c`：分子是「a − b」，整个分子在分数线之上，不能把减号漏到分数外
    #[test]
    fn negated_numerator_keeps_minus_inside_fraction() {
        let layout = build("A = (a-b)/c");
        let MathNode::Fraction {
            numerator,
            denominator,
        } = &layout.lines[0].nodes[0]
        else {
            panic!("应为分数");
        };
        assert_eq!(numerator.len(), 3);
        assert_eq!(texts(numerator), ["a", " − ", "b"]);
        assert_eq!(texts(denominator), ["c"]);
    }

    #[test]
    fn power_becomes_superscript() {
        let layout = build("A = b*h^2");
        let sup = layout.lines[0]
            .nodes
            .iter()
            .find_map(|n| match n {
                MathNode::Superscript { base, exponent } => Some((base, exponent)),
                _ => None,
            })
            .expect("应有上标");
        assert_eq!(texts(sup.0), ["h"]);
        assert_eq!(texts(sup.1), ["2"]);
    }

    #[test]
    fn sqrt_becomes_radical_and_cbrt_keeps_index() {
        let layout = build("x = sqrt(b^2-4*a*c)");
        let MathNode::Radical { radicand, index } = &layout.lines[0].nodes[0] else {
            panic!("应为根号");
        };
        assert_eq!(*index, None);
        // b² − 4×a×c：上标 / 减号 / 数字 / 乘号 / a / 乘号 / c
        assert_eq!(radicand.len(), 7);
        assert!(matches!(radicand[0], MathNode::Superscript { .. }));

        let layout = build("x = cbrt(v)");
        let MathNode::Radical { index, .. } = &layout.lines[0].nodes[0] else {
            panic!("应为根号");
        };
        assert_eq!(*index, Some(3));
    }

    #[test]
    fn pow_function_is_rendered_as_power() {
        let layout = build("x = pow(b,2)-4*a*c");
        assert!(
            layout.lines[0]
                .nodes
                .iter()
                .any(|n| matches!(n, MathNode::Superscript { .. })),
            "pow(b,2) 应排成上标"
        );
    }

    #[test]
    fn absolute_and_other_functions() {
        let layout = build("x = abs(d)");
        assert!(matches!(layout.lines[0].nodes[0], MathNode::Absolute { .. }));

        let layout = build("x = sin(t)");
        let MathNode::Function { name, args } = &layout.lines[0].nodes[0] else {
            panic!("应为函数");
        };
        assert_eq!(name, "sin");
        assert_eq!(args.len(), 1);

        let layout = build("x = log10(v)");
        let MathNode::Function { name, .. } = &layout.lines[0].nodes[0] else {
            panic!("应为函数");
        };
        assert_eq!(name, "log₁₀");
    }

    #[test]
    fn multi_output_splits_into_separate_lines() {
        let layout = build(
            "x = (b1*a22-a12*b2)/(a11*a22-a12*a21); y = (a11*b2-b1*a21)/(a11*a22-a12*a21)",
        );
        assert_eq!(layout.lines.len(), 2);
        assert!(layout.is_multi());
        assert_eq!(
            layout
                .lines
                .iter()
                .map(|l| l.symbol.as_deref().unwrap_or(""))
                .collect::<Vec<_>>(),
            ["x", "y"]
        );
        for line in &layout.lines {
            assert!(
                matches!(line.nodes.first(), Some(MathNode::Fraction { .. })),
                "每段都应是分数"
            );
        }
    }

    // ============================================================ 符号美化

    #[test]
    fn symbols_are_beautified_for_display_only() {
        assert_eq!(pretty_symbol("a11"), "a₁₁");
        assert_eq!(pretty_symbol("x2"), "x₂");
        assert_eq!(pretty_symbol("b_1"), "b₁");
        assert_eq!(pretty_symbol("pi"), "π");
        // 三位以上数字不做下标（避免 a123 → a₁₂₃ 这种误读）
        assert_eq!(pretty_symbol("a123"), "a123");
        assert_eq!(pretty_symbol("sigma"), "sigma");
    }

    #[test]
    fn numbered_variables_render_with_subscripts() {
        let layout = build("x = a11*b2");
        assert_eq!(texts(&layout.lines[0].nodes), ["a₁₁", "×", "b₂"]);
    }

    /// 大小写不敏感的美化（`PI` 也是 π）
    #[test]
    fn pretty_symbol_is_case_insensitive() {
        assert_eq!(pretty_symbol("PI"), "π");
        assert_eq!(pretty_symbol("Pi"), "π");
        // `E` → `e`（源项目行为；注意表达式里 `E` 会被引擎当欧拉数）
        assert_eq!(pretty_symbol("E"), "e");
    }

    /// 下划线形式只吃 1~2 位数字
    #[test]
    fn underscore_form_rejects_long_digits() {
        assert_eq!(pretty_symbol("x_12"), "x₁₂");
        assert_eq!(pretty_symbol("x_123"), "x_123", "3 位不做下标");
        assert_eq!(pretty_symbol("x_"), "x_", "缺数字");
        assert_eq!(pretty_symbol("_1"), "_1", "缺字母");
        assert_eq!(pretty_symbol("x_1_2"), "x_1_2", "多段下划线不匹配");
    }

    // ============================================================ 降级

    #[test]
    fn unparsable_segment_is_reported_not_silently_dropped() {
        // 列一个 lexer 认不出的字符，确认走降级并留下警告（前端据此回落原文）
        let layout = build("x = a@b");
        assert!(layout.lines.is_empty());
        assert!(
            !layout.warnings.is_empty(),
            "应留下降级警告，实际 {:?}",
            layout.warnings
        );
        assert!(!layout.is_usable());
    }

    #[test]
    fn empty_expression_yields_empty_layout() {
        let layout = build("");
        assert!(layout.lines.is_empty());
        assert!(!layout.warnings.is_empty());
    }

    /// 多段里只有一段坏 → 好的一段照排，坏的记警告
    #[test]
    fn partial_failure_keeps_good_segments() {
        let layout = build("x = a+b; y = a@b");
        assert_eq!(layout.lines.len(), 1, "好的一段应保留");
        assert_eq!(layout.lines[0].symbol.as_deref(), Some("x"));
        assert_eq!(layout.warnings.len(), 1);
        assert!(
            layout.warnings[0].contains("第 2 段"),
            "警告应指出是哪一段: {:?}",
            layout.warnings
        );
    }

    /// 有符号的段，警告里要带上符号便于定位
    #[test]
    fn warning_mentions_segment_symbol() {
        let layout = build("V = a@b");
        assert!(
            layout.warnings[0].contains("V"),
            "警告应带符号: {:?}",
            layout.warnings
        );
    }

    // ============================================================ 等式

    #[test]
    fn equation_keeps_both_sides_around_equals_sign() {
        // 用户写的条件方程（`x+y+z=6`）不是赋值语句，按第一个 `=` 切成两半后再排版
        let nodes = super::build_equation("x+y+z=6").expect("应能排版");
        assert_eq!(texts(&nodes), ["x", " + ", "y", " + ", "z", " = ", "6"]);
    }

    #[test]
    fn equation_without_equals_sign_is_rejected() {
        // 回落方式由调用方决定（原文展示），这里只保证不硬排出一条错位的式子
        assert_eq!(super::build_equation("x+y+z"), None);
        assert_eq!(super::build_equation("=6"), None);
        assert_eq!(super::build_equation("x+y+z="), None);
        assert_eq!(super::build_equation("x+y+z=??"), None);
    }

    /// 按**第一个** `=` 切分（右侧若还有 `=` 会解析失败 → 整体 None）
    #[test]
    fn equation_splits_at_first_equals_sign() {
        // `a=b=c`：右侧 `b=c` 含 `=` → 解析失败
        assert_eq!(super::build_equation("a=b=c"), None);
        // `a=b` 正常
        assert_eq!(texts(&super::build_equation("a=b").unwrap()), ["a", " = ", "b"]);
    }

    // ============================================================ 隐式乘法

    #[test]
    fn implicit_multiplication_is_inserted_before_typesetting() {
        // 教材写法 `2x` 要先补成 `2*x`，否则解析失败只能降级成原文
        let nodes = super::build_equation("2x-y+z=3").expect("应能排版");
        assert_eq!(
            texts(&nodes),
            ["2", "×", "x", " − ", "y", " + ", "z", " = ", "3"]
        );
    }

    /// 隐式乘法的边界（源项目同名测试的断言，逐条搬过来守住行为）
    #[test]
    fn implicit_multiplication_boundaries() {
        assert_eq!(implicit_mul::insert("2x"), "2*x");
        assert_eq!(implicit_mul::insert("3(x+1)"), "3*(x+1)");
        assert_eq!(implicit_mul::insert("(a+b)(c+d)"), "(a+b)*(c+d)");
        assert_eq!(implicit_mul::insert("2.5x"), "2.5*x");
        // 含数字的符号名与显式写法不能被改
        assert_eq!(implicit_mul::insert("a11*x+b_1"), "a11*x+b_1");
        assert_eq!(implicit_mul::insert("x2+y1"), "x2+y1");
        // 函数名里的数字后跟括号不是乘号（`log10(v)` 补成 `log10*(v)` 会把函数名拆坏）
        assert_eq!(implicit_mul::insert("log10(v)"), "log10(v)");
        assert_eq!(implicit_mul::insert("V = log10(v)*2"), "V = log10(v)*2");
    }

    // ============================================================ 结果行

    #[test]
    fn result_line_pairs_symbols_with_values() {
        // 多结果：`(x, y, z) = (1, 2, 3)`，符号一排、数值一排，顺序必须一一对应
        let nodes = super::build_result_line(&[
            ("x".to_string(), "1".to_string()),
            ("y".to_string(), "2".to_string()),
            ("z".to_string(), "3".to_string()),
        ]);
        assert_eq!(
            texts(&nodes),
            ["(", "x", ", ", "y", ", ", "z", ")", " = ", "(", "1", ", ", "2", ", ", "3", ")"]
        );

        // 单结果不带括号
        let single = super::build_result_line(&[("V".to_string(), "31.4159".to_string())]);
        assert_eq!(texts(&single), ["V", " = ", "31.4159"]);

        assert!(super::build_result_line(&[]).is_empty());
    }

    // ============================================================ 数值格式

    #[test]
    fn number_formatting_matches_calc_doc() {
        // 整数不带小数点
        assert_eq!(format_number(2.0), "2");
        assert_eq!(format_number(-5.0), "-5");
        assert_eq!(format_number(0.0), "0");
        // 小数最多 6 位再去尾零
        assert_eq!(format_number(2.75), "2.75");
        assert_eq!(format_number(0.125), "0.125");
        assert_eq!(format_number(1.5), "1.5");
        assert_eq!(format_number(1.0 / 3.0), "0.333333");
        assert_eq!(format_number(0.1 + 0.2), "0.3", "浮点误差应被 6 位截断");
    }

    /// 非有限值按 Kotlin 写法（`Infinity` 而不是 Rust 的 `inf`）
    #[test]
    fn non_finite_numbers_use_kotlin_spelling() {
        assert_eq!(format_number(f64::NAN), "NaN");
        assert_eq!(format_number(f64::INFINITY), "Infinity");
        assert_eq!(format_number(f64::NEG_INFINITY), "-Infinity");
    }

    #[test]
    fn numeric_literals_keep_six_decimals_at_most() {
        let layout = build("x = 1/3");
        // 排版层不代入，`1/3` 是分数；数字 `1` 与 `3` 都按整数写法
        let MathNode::Fraction {
            numerator,
            denominator,
        } = &layout.lines[0].nodes[0]
        else {
            panic!("应为分数");
        };
        assert_eq!(texts(numerator), ["1"]);
        assert_eq!(texts(denominator), ["3"]);
    }

    // ============================================================ 常量

    #[test]
    fn custom_constants_are_accepted() {
        let mut constants = HashMap::new();
        constants.insert("alpha".to_string(), 1.5);
        let layout = super::build_with_constants("x = alpha*b", &constants);
        assert!(layout.is_usable(), "自定义常量应能排版: {:?}", layout.warnings);
    }

    /// 内置常量 `pi` 显示为 π
    ///
    /// `r^2` 是 `Superscript` 节点（不是 Text），所以不出现在 `texts` 里 ——
    /// 这里单独把它取出来验一下基数与指数。
    #[test]
    fn builtin_pi_renders_as_symbol() {
        let layout = build("x = pi*r^2");
        assert_eq!(texts(&layout.lines[0].nodes), ["π", "×"]);

        let MathNode::Superscript { base, exponent } = layout.lines[0].nodes.last().unwrap()
        else {
            panic!("最后应是上标: {:?}", layout.lines[0].nodes);
        };
        assert_eq!(texts(base), ["r"]);
        assert_eq!(texts(exponent), ["2"]);
    }

    // ============================================================ 一元负号

    #[test]
    fn unary_minus_is_kept_inline() {
        let layout = build("x = -a");
        assert_eq!(texts(&layout.lines[0].nodes), ["−", "a"]);
    }

    // ============================================================ serde

    #[test]
    fn math_node_serde_shape() {
        let node = MathNode::Text {
            text: "a₁₁".to_string(),
            kind: TextKind::Symbol,
        };
        let v = serde_json::to_value(&node).unwrap();
        assert_eq!(v["type"], serde_json::json!("text"));
        assert_eq!(v["text"], serde_json::json!("a₁₁"));
        assert_eq!(v["kind"], serde_json::json!("symbol"));

        let frac = MathNode::Fraction {
            numerator: vec![MathNode::text("a", TextKind::Symbol)],
            denominator: vec![MathNode::text("b", TextKind::Symbol)],
        };
        let v = serde_json::to_value(&frac).unwrap();
        assert_eq!(v["type"], serde_json::json!("fraction"));
        assert!(v.get("numerator").is_some());

        let radical = MathNode::Radical {
            radicand: vec![],
            index: Some(3),
        };
        let v = serde_json::to_value(&radical).unwrap();
        assert_eq!(v["type"], serde_json::json!("radical"));
        assert_eq!(v["index"], serde_json::json!(3));
    }

    #[test]
    fn math_layout_serde_shape() {
        let layout = build("x = a/b");
        let v = serde_json::to_value(&layout).unwrap();
        assert!(v.get("lines").is_some());
        assert!(v.get("warnings").is_some());
        // `isUsable` / `isMulti` 是方法，不进序列化（前端自己按 lines 判）
        assert!(v.get("isUsable").is_none());
        assert!(v.get("is_usable").is_none());
    }

    /// 空 `warnings` 与 `symbol` 也要能被反序列化（前端回传场景）
    #[test]
    fn math_line_deserializes_without_optional_fields() {
        let line: MathLine = serde_json::from_str(r#"{"nodes":[]}"#).unwrap();
        assert_eq!(line.symbol, None);
        assert_eq!(line.mark, None);
        assert!(line.nodes.is_empty());
    }

    // ============================================================ 内置库覆盖面

    /// 55 条内置公式都必须能排版。
    ///
    /// 示意图是「打开公式就有图」—— 任一条排不出来就会退回等宽原文（功能看着像没上），
    /// 这条守住覆盖面。
    #[test]
    fn every_builtin_formula_can_be_typeset() {
        let loaded = crate::source::builtin_loader::load_embedded().expect("内置库应能加载");
        assert!(!loaded.valid.is_empty(), "内置公式列表不能为空");

        let mut failures = Vec::new();
        for schema in &loaded.valid {
            let layout = super::build_with_constants(&schema.expression, &schema.constants);
            if !layout.is_usable() {
                failures.push(format!("{}：{}", schema.id, layout.warnings.join("；")));
                continue;
            }
            // 多输出公式应排成多行，且每行的符号要与声明一致
            if !schema.result_outputs.is_empty() {
                let rendered = layout
                    .lines
                    .iter()
                    .filter_map(|l| l.symbol.clone())
                    .count();
                let declared = schema.result_outputs.len();
                if rendered != declared {
                    failures.push(format!(
                        "{}：声明 {declared} 个输出，排出行数 {rendered}",
                        schema.id
                    ));
                }
            }
        }

        for f in &failures {
            println!("  ❌ {f}");
        }
        println!(
            "✅ 内置公式可排版 {}/{} 条",
            loaded.valid.len() - failures.len(),
            loaded.valid.len()
        );
        assert!(failures.is_empty(), "有内置公式无法排版：{failures:?}");
    }
}
