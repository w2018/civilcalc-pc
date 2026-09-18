//! 括号保护与优先级工具（**Excel 转换与引擎共用同一份实现**）。
//!
//! 源：`civilcalc-android-v2/core/formula/engine/ExprUtils.kt`
//!
//! ## 为什么必须共用（源项目 BUG-06 教训）
//!
//! 括号保护逻辑原先在 `ExcelFormulaConverter` 与 `FormulaEngineImpl`
//! **各写了一份**，导致行为漂移。现在两处统一委托本模块。
//!
//! ## ⚠️ 优先级刻度：本模块用 2/3/4，**不是** Parser 的 1/2/3/4
//!
//! 这是源项目的既有设计：`find_top_prec` 的返回值只与
//! **Excel 转换传入的 `parentPrec`** 比较，后者同样使用 2/3/4 刻度：
//!
//! ```text
//! "+" "-"        → 2
//! "*" "/" "%"    → 3
//! "^"            → 4
//! ```
//!
//! 移植时不要"统一"成 Parser 的刻度，否则括号判定会错。

/// 判断 `expr` 是否被**一对完整括号**包裹。
///
/// 修复源项目的一个启发式缺陷：仅看首尾字符会把 `(a+b)*(c+d)` 误判为原子。
/// 正确做法是扫描括号深度，**在深度归零的位置**判断是否已是字符串末尾。
pub fn is_fully_parenthesized(expr: &str) -> bool {
    let chars: Vec<char> = expr.chars().collect();
    if chars.len() < 2 || chars[0] != '(' || chars[chars.len() - 1] != ')' {
        return false;
    }

    let mut depth: i32 = 0;
    for (i, c) in chars.iter().enumerate() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    // 深度在最后一个字符处归零才算完整包裹
                    return i == chars.len() - 1;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

/// 是否为"原子"（无需括号保护）。
///
/// 原子包括：空串、完整括号包裹、函数调用、数字字面量、变量名。
pub fn is_atom(expr: &str) -> bool {
    if expr.is_empty() {
        return true;
    }
    if is_fully_parenthesized(expr) {
        return true;
    }

    let first = expr.chars().next().expect("已排除空串");

    // 函数调用：字母开头且含 '('
    if first.is_alphabetic() && expr.contains('(') {
        return true;
    }
    // 数字字面量：数字开头且不含运算符
    if first.is_ascii_digit() && !contains_any(expr, "+-*/%^") {
        return true;
    }
    // 变量名：字母开头且不含运算符与 '('
    if first.is_alphabetic() && !contains_any(expr, "+-*/%^(") {
        return true;
    }
    false
}

/// 作为**左**操作数时是否需要加括号。
///
/// 规则：子式优先级 **小于** 父级时需要括号（`in 1 until parentPrec`）。
pub fn needs_parens_left(expr: &str, parent_prec: i32) -> bool {
    if is_atom(expr) {
        return false;
    }
    let inner_prec = find_top_prec(expr);
    (1..parent_prec).contains(&inner_prec)
}

/// 作为**右**操作数时是否需要加括号。
///
/// 规则（源项目 BUG-06 扩展）：
/// - 父运算符**右结合**（`^`）：子式优先级 **小于** 父级才加括号
///   （`a^(b^c)` 可省略）
/// - 父运算符**左结合**（`+ - * / %`）：子式优先级 **小于等于** 父级都加括号
///   —— 否则 `x/(a+b)*(c+d)`、`x-(a+b)` 会被 Excel 重解析为错误的结合顺序
///
/// 多余的括号只是冗余，**绝不错误**。
pub fn needs_parens_right(expr: &str, parent_prec: i32, is_right_assoc: bool) -> bool {
    if is_atom(expr) {
        return false;
    }
    let inner_prec = find_top_prec(expr);
    if is_right_assoc {
        (1..parent_prec).contains(&inner_prec)
    } else {
        (1..=parent_prec).contains(&inner_prec)
    }
}

/// 找出表达式在**括号深度为 0** 处的最低优先级（刻度 2/3/4）。
///
/// 无顶层运算符时返回 `0`。跳过一元正负号
/// （出现在开头，或紧跟运算符 / 左括号之后）。
pub fn find_top_prec(expr: &str) -> i32 {
    let chars: Vec<char> = expr.chars().collect();
    let mut depth: i32 = 0;
    let mut min_prec: i32 = 5; // 哨兵：高于最高优先级 4

    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];

        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
        } else if depth == 0 && is_top_operator(c) {
            // 跳过一元正负号：出现在开头，或紧跟运算符 / 左括号
            let is_unary_sign = (c == '-' || c == '+')
                && (i == 0 || matches!(chars[i - 1], '(' | '+' | '-' | '*' | '/' | '%' | '^'));
            if is_unary_sign {
                i += 1;
                continue;
            }

            let p = match c {
                '+' | '-' => 2,
                '*' | '/' | '%' => 3,
                '^' => 4,
                _ => 0,
            };
            if p >= 1 && p <= min_prec {
                min_prec = p;
            }
        }

        i += 1;
    }

    if min_prec <= 4 {
        min_prec
    } else {
        0
    }
}

fn is_top_operator(c: char) -> bool {
    matches!(c, '+' | '-' | '*' | '/' | '%' | '^')
}

fn contains_any(s: &str, chars: &str) -> bool {
    chars.chars().any(|c| s.contains(c))
}


// =============================================================================
// 整词替换（手写 lookaround）
// =============================================================================

/// 后边界规则。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterRule {
    /// 后边界必须**不是**词字符（由调用方传入的 `next_is_word` 判定）
    NotWordChar,
    /// 跳过空白后必须紧跟 `(`。**空白与 `(` 都不消费** —— 替换串里不要补 `(`
    OpenParen,
}

/// 整词替换（手写 lookaround —— Rust 的 `regex` **不支持** `(?<!…)` / `(?=…)`）。
///
/// 单趟构建输出，**不边扫边改** —— 否则 needle 与替换串长度不同时索引会漂移
/// （这正是必须一次性替换、而不能 `find` + `replace_range` 循环的原因）。
///
/// ## 为什么前后边界要分开传
///
/// 已知三个调用点的边界集**并不相同**（源项目正则如此）：
///
/// | 用途 | 前边界排除 | 后边界 |
/// |---|---|---|
/// | Excel 函数名回译（`SQRT`→`sqrt`） | `[A-Za-z0-9_.]` | 跳过空白后须跟 `(` |
/// | 单元格引用替换（`A1`→`a`） | `[A-Za-z0-9_$.]` | `[A-Za-z0-9_]` |
/// | 验算代入（`pi`→`3.141593`） | `[A-Za-z0-9_']` | `[A-Za-z0-9_']` |
///
/// `$` 与 `.` 只出现在某些前边界，`'` 只出现在验算那组 ——
/// 合成一个集会出错（例如把 `r` 误替换掉 `rho` 里的 `r`）。
///
/// 未匹配的字符逐个原样拷出，因此对多字节字符（中文）安全。
pub fn replace_whole_word(
    src: &str,
    needle: &str,
    replacement: &str,
    prev_is_word: &dyn Fn(char) -> bool,
    next_is_word: &dyn Fn(char) -> bool,
    after: AfterRule,
) -> String {
    if needle.is_empty() {
        return src.to_string();
    }

    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;

    while i < src.len() {
        if src[i..].starts_with(needle) {
            let before_ok = match src[..i].chars().next_back() {
                None => true,
                Some(c) => !prev_is_word(c),
            };

            let rest = &src[i + needle.len()..];
            let after_ok = match after {
                AfterRule::NotWordChar => match rest.chars().next() {
                    None => true,
                    Some(c) => !next_is_word(c),
                },
                AfterRule::OpenParen => rest.trim_start().starts_with('('),
            };

            if before_ok && after_ok {
                out.push_str(replacement);
                i += needle.len();
                continue;
            }
        }

        // 未匹配：拷一个字符（`i` 始终落在字符边界上）
        let c = src[i..].chars().next().expect("i 落在字符边界上");
        out.push(c);
        i += c.len_utf8();
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------- 源项目 ExprUtilsParensTest 的用例（逐条移植） ----------------

    #[test]
    fn bug06_fully_parenthesized_detection() {
        assert!(is_fully_parenthesized("(a+b)"));
        assert!(is_fully_parenthesized("((a+b)*(c+d))"));
        // 首尾括号不构成完整包裹
        assert!(!is_fully_parenthesized("(a+b)*(c+d)"));
        assert!(!is_fully_parenthesized("(a)+(b)"));
    }

    #[test]
    fn bug06_fully_parenthesized_edge_cases() {
        assert!(!is_fully_parenthesized(""), "空串");
        assert!(!is_fully_parenthesized("("), "长度不足");
        assert!(!is_fully_parenthesized(")("), "首字符不是左括号");
        assert!(!is_fully_parenthesized("(a)(b)"), "深度在第 3 位归零，不是末尾");
        // "()" 深度在末位归零 → 完整包裹
        assert!(is_fully_parenthesized("()"));
    }

    // ---------------- is_atom ----------------

    #[test]
    fn atoms() {
        assert!(is_atom(""), "空串视为原子");
        assert!(is_atom("(a+b)"), "完整括号包裹");
        assert!(is_atom("sqrt(x)"), "函数调用");
        assert!(is_atom("123"), "数字字面量");
        assert!(is_atom("x"), "变量");
        assert!(is_atom("a1"), "含数字的变量");
    }

    #[test]
    fn non_atoms() {
        assert!(!is_atom("a+b"), "含运算符");
        assert!(!is_atom("a*b"), "含运算符");
        assert!(!is_atom("(a+b)*(c+d)"), "首尾括号非完整包裹且有运算符");
        assert!(!is_atom("2*3"), "数字开头但含运算符");
    }

    // ---------------- find_top_prec（刻度 2/3/4） ----------------

    #[test]
    fn top_prec_uses_234_scale() {
        assert_eq!(find_top_prec("a+b"), 2, "+ 为 2");
        assert_eq!(find_top_prec("a-b"), 2);
        assert_eq!(find_top_prec("a*b"), 3, "* 为 3");
        assert_eq!(find_top_prec("a/b"), 3);
        assert_eq!(find_top_prec("a%b"), 3);
        assert_eq!(find_top_prec("a^b"), 4, "^ 为 4");
    }

    #[test]
    fn top_prec_takes_minimum() {
        // a+b*c → 顶层最低是 +（2）
        assert_eq!(find_top_prec("a+b*c"), 2);
        // a*b+c → 顶层最低是 +（2）
        assert_eq!(find_top_prec("a*b+c"), 2);
    }

    #[test]
    fn top_prec_ignores_parenthesized() {
        // 括号内的运算符不参与
        assert_eq!(find_top_prec("(a+b)"), 0);
        assert_eq!(find_top_prec("(a+b)*(c+d)"), 3, "顶层是 *");
    }

    #[test]
    fn top_prec_returns_zero_when_no_operator() {
        assert_eq!(find_top_prec("a"), 0);
        assert_eq!(find_top_prec("123"), 0);
        assert_eq!(find_top_prec("sqrt(x)"), 0);
    }

    #[test]
    fn top_prec_skips_unary_sign() {
        // -a*b → 开头的 - 是一元，不参与；顶层是 *（3）
        assert_eq!(find_top_prec("-a*b"), 3);
        // a*-b → * 后的一元负号跳过；顶层是 *（3）
        assert_eq!(find_top_prec("a*-b"), 3);
        // a-(b) → 二元减号参与（2）
        assert_eq!(find_top_prec("a-(b)"), 2);
    }

    // ---------------- needs_parens_* ----------------

    #[test]
    fn left_operand_rules() {
        // 父级 *（3），子式 +（2）→ 需要括号
        assert!(needs_parens_left("a+b", 3));
        // 父级 +（2），子式 *（3）→ 不需要
        assert!(!needs_parens_left("a*b", 2));
        // 原子 → 不需要
        assert!(!needs_parens_left("a", 3));
        assert!(!needs_parens_left("(a+b)", 3));
    }

    #[test]
    fn right_operand_rules_left_assoc_parent() {
        // 父级 *（3）左结合，子式 *（3）→ 需要括号（否则结合顺序错）
        assert!(needs_parens_right("a*b", 3, false));
        // 父级 +（2）左结合，子式 +（2）→ 需要括号
        assert!(needs_parens_right("a+b", 2, false));
        // 父级 +（2），子式 *（3）→ 不需要
        assert!(!needs_parens_right("a*b", 2, false));
    }

    #[test]
    fn right_operand_rules_right_assoc_parent() {
        // 父级 ^（4）右结合，子式 ^（4）→ 不需要括号（a^(b^c) 可省略）
        assert!(!needs_parens_right("b^c", 4, true));
        // 父级 ^（4）右结合，子式 +（2）→ 需要括号
        assert!(needs_parens_right("b+c", 4, true));
    }

    /// BUG-06 的两个真实场景（源项目测试的表达式形态）
    #[test]
    fn bug06_scenario_k_div_product_of_sums() {
        // k/((a+b)*(c+d))：分母是 *(3)，其右操作数 (c+d) 是原子 → 无需额外括号
        // 这里验证分母整体作为 / 的右操作数时需要括号
        assert!(needs_parens_right("(a+b)*(c+d)", 3, false));
    }

    #[test]
    fn bug06_scenario_subtraction_right_operand() {
        // x-(y+z)：右操作数 (y+z) 是完整括号包裹 → 原子 → 无需额外括号
        // （Excel 输出为 =A1-(B1+C1)，而不是 =A1-B1+C1）
        assert!(!needs_parens_right("(y+z)", 2, false));
    }

    // ---------------- 整词替换 ----------------

    /// 引用替换的边界集：前边界含 `.` 与 `$`，后边界只到 `_`
    fn ref_prev(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '$'
    }
    fn ref_next(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_'
    }
    /// 函数名回译的边界集：前边界含 `.`（`FLOOR.MATH`），后边界走括号规则
    fn func_word(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_' || c == '.'
    }
    /// 验算代入的边界集：含 `'`（Excel 名称里的合法字符）
    fn verify_word(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_' || c == '\''
    }

    #[test]
    fn replace_whole_word_basic() {
        assert_eq!(
            replace_whole_word("A1+A1", "A1", "x", &ref_prev, &ref_next, AfterRule::NotWordChar),
            "x+x"
        );
    }

    /// ⚠️ 后边界已能防住「短引用吃掉长引用前缀」—— 不需要靠替换顺序
    #[test]
    fn replace_whole_word_protects_longer_refs_via_boundary() {
        // `A1` 后面跟 `0`（数字）→ 不是独立引用
        assert_eq!(
            replace_whole_word("A10", "A1", "x", &ref_prev, &ref_next, AfterRule::NotWordChar),
            "A10"
        );
        assert_eq!(
            replace_whole_word("A10", "A10", "x", &ref_prev, &ref_next, AfterRule::NotWordChar),
            "x"
        );
        // 前缀是字母的引用同理
        assert_eq!(
            replace_whole_word("AA1", "A1", "x", &ref_prev, &ref_next, AfterRule::NotWordChar),
            "AA1"
        );
    }

    /// 前边界含 `$`：绝对引用里的 `A$1` 不算独立引用
    #[test]
    fn replace_whole_word_dollar_boundary() {
        assert_eq!(
            replace_whole_word("$A$1", "A$1", "x", &ref_prev, &ref_next, AfterRule::NotWordChar),
            "$A$1"
        );
        assert_eq!(
            replace_whole_word("+A$1", "A$1", "x", &ref_prev, &ref_next, AfterRule::NotWordChar),
            "+x"
        );
    }

    #[test]
    fn replace_whole_word_after_rules() {
        // OpenParen：后跟空白+括号也算；括号**不消费**
        assert_eq!(
            replace_whole_word("SQRT (A1)", "SQRT", "sqrt", &func_word, &func_word, AfterRule::OpenParen),
            "sqrt (A1)"
        );
        // OpenParen：后面不是括号 → 不换
        assert_eq!(
            replace_whole_word("SQRT+A1", "SQRT", "sqrt", &func_word, &func_word, AfterRule::OpenParen),
            "SQRT+A1"
        );
        // NotWordChar：后跟字母 → 不换
        assert_eq!(
            replace_whole_word("A1B", "A1", "x", &ref_prev, &ref_next, AfterRule::NotWordChar),
            "A1B"
        );
        // NotWordChar：后跟 `(` → 换（`(` 不是词字符）
        assert_eq!(
            replace_whole_word("A1(", "A1", "x", &ref_prev, &ref_next, AfterRule::NotWordChar),
            "x("
        );
    }

    /// 验算代入用 `'` 作词字符（`r` 不能命中 `r'`）
    #[test]
    fn replace_whole_word_apostrophe_boundary() {
        assert_eq!(
            replace_whole_word("r'", "r", "1", &verify_word, &verify_word, AfterRule::NotWordChar),
            "r'",
            "`r'` 是一个整体标识符，不该被拆"
        );
        assert_eq!(
            replace_whole_word("r+1", "r", "1", &verify_word, &verify_word, AfterRule::NotWordChar),
            "1+1"
        );
    }

    /// 整词替换不能误伤包含相同子串的长标识符
    #[test]
    fn replace_whole_word_does_not_damage_substrings() {
        // `r` 不能命中 `rho`
        assert_eq!(
            replace_whole_word("rho", "r", "1", &verify_word, &verify_word, AfterRule::NotWordChar),
            "rho"
        );
        // `e` 不能命中 `exp`
        assert_eq!(
            replace_whole_word("exp(x)", "e", "2", &verify_word, &verify_word, AfterRule::NotWordChar),
            "exp(x)"
        );
    }

    /// 多字节字符不能被逐字节拆坏
    #[test]
    fn replace_whole_word_is_utf8_safe() {
        assert_eq!(
            replace_whole_word("梁SQRT(A1)柱", "SQRT", "sqrt", &func_word, &func_word, AfterRule::OpenParen),
            "梁sqrt(A1)柱"
        );
    }

    #[test]
    fn replace_whole_word_empty_needle_is_noop() {
        assert_eq!(
            replace_whole_word("abc", "", "x", &ref_prev, &ref_next, AfterRule::NotWordChar),
            "abc"
        );
    }
}
