//! 隐含乘号补全。
//!
//! 源：`civilcalc-android-v2/core/formula/engine/ImplicitMultiplication.kt`
//!
//! ## 为什么需要（源项目注释原文）
//!
//! > 用户与教材常写 `2x`、`3(x+1)`、`)x`，而本地引擎只认显式 `*`。
//! >
//! > 不补会有两种后果：排版与求值直接失败（降级成原文展示），以及更糟的一种 ——
//! > 把 `2x` 里的 x 换成数值后显示成 `21`（看着像模像样、其实错位），
//! > 所以凡是要拿表达式去做文本代入或二维排版的地方，都先过这里补成显式乘号。
//!
//! ## 实现方式（ADR-016）
//!
//! 源项目用带**否定环视**的正则：
//! ```text
//! ((?<![A-Za-z0-9_])[0-9]|\))\s*([0-9A-Za-z(])
//! ```
//! Rust 的 `regex` crate **不支持 lookbehind**。ADR-016 决定**手写单趟扫描**
//! （不引入 `fancy-regex`），理由：
//!
//! 1. 逻辑本身简单，手写可精确控制边界
//! 2. 与 [`crate::excel`] 的 `substitute_values` 单趟扫描风格统一
//! 3. 不引入额外依赖
//!
//! ## 匹配规则（与源正则逐条等价）
//!
//! - **组 1**：`)`，或「**不被 `[A-Za-z0-9_]` 前置**的数字」
//! - **中间**：任意 ASCII 空白（替换时**丢弃**）
//! - **组 2**：`[0-9A-Za-z(]`
//!
//! 替换为 `组1 + "*" + 组2`。
//!
//! ⚠️ 环视与字符类均为 **ASCII-only**（Java `\s` 亦为 ASCII），
//! 因此中文等非 ASCII 字符**不**阻断匹配 —— 与源行为一致。

/// 判断是否为 `[A-Za-z0-9_]`（**ASCII-only**，对齐源正则的字符类）
fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// 判断是否为组 2 的字符 `[0-9A-Za-z(]`（**ASCII-only**）
fn is_group2_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '('
}

/// 判断当前位置是否为组 1 的候选。
///
/// - `)` 恒为候选
/// - 数字：仅当**前驱不是** `[A-Za-z0-9_]`（或位于开头）时才是候选
fn is_group1_at(chars: &[char], i: usize) -> bool {
    let c = chars[i];
    if c == ')' {
        return true;
    }
    if c.is_ascii_digit() {
        return i == 0 || !is_word_char(chars[i - 1]);
    }
    false
}

/// 补全隐含乘号。
pub fn insert(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();

    // 无 ')'' 也无数字时不可能匹配，直接返回（避免无谓分配）
    if !chars.iter().any(|c| *c == ')' || c.is_ascii_digit()) {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len() + 8);
    let mut i = 0usize;

    while i < len {
        if is_group1_at(&chars, i) {
            // 跳过中间的 ASCII 空白
            let mut j = i + 1;
            while j < len && chars[j].is_ascii_whitespace() {
                j += 1;
            }

            if j < len && is_group2_char(chars[j]) {
                // 命中 → 输出 `组1*组2`（**中间空白被丢弃**，与源正则替换一致）
                out.push(chars[i]);
                out.push('*');
                out.push(chars[j]);
                i = j + 1;
                continue;
            }
        }

        out.push(chars[i]);
        i += 1;
    }

    out
}

#[cfg(test)]
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;

    // =========================================================================
    // 源项目 MathLayoutBuilderTest.kt:141-156 的 8 个验收用例（逐条移植）
    // =========================================================================

    #[test]
    fn source_case_2x() {
        assert_eq!(insert("2x"), "2*x");
    }

    #[test]
    fn source_case_3_lparen() {
        assert_eq!(insert("3(x+1)"), "3*(x+1)");
    }

    #[test]
    fn source_case_two_paren_groups() {
        assert_eq!(insert("(a+b)(c+d)"), "(a+b)*(c+d)");
    }

    #[test]
    fn source_case_decimal_then_variable() {
        // `2.5x`：小数点不阻断，`5` 前驱是 `.`（非 word char）→ 候选
        assert_eq!(insert("2.5x"), "2.5*x");
    }

    #[test]
    fn source_case_symbols_with_digits_unchanged() {
        assert_eq!(insert("a11*x+b_1"), "a11*x+b_1");
    }

    #[test]
    fn source_case_letter_digit_symbols_unchanged() {
        assert_eq!(insert("x2+y1"), "x2+y1");
    }

    #[test]
    fn source_case_function_name_with_digit_unchanged() {
        // ⚠️ 关键反例：`log10(v)` 里的 `0(` 不是乘号，补了会拆坏函数名
        assert_eq!(insert("log10(v)"), "log10(v)");
    }

    #[test]
    fn source_case_rparen_after_function_then_number() {
        // `V = log10(v)2` → `V = log10(v)*2`
        assert_eq!(insert("V = log10(v)2"), "V = log10(v)*2");
    }

    // =========================================================================
    // 其他等价性验证（对齐源正则的行为）
    // =========================================================================

    #[test]
    fn whitespace_between_operands_is_dropped() {
        // 源正则替换会丢弃 `\s*` 匹配到的空白
        assert_eq!(insert("2 x"), "2*x");
        assert_eq!(insert("(a+b) (c+d)"), "(a+b)*(c+d)");
        assert_eq!(insert("2\tx"), "2*x");
    }

    #[test]
    fn already_explicit_multiplication_untouched() {
        assert_eq!(insert("2*x"), "2*x");
        assert_eq!(insert("(a+b)*(c+d)"), "(a+b)*(c+d)");
    }

    #[test]
    fn plain_expression_untouched() {
        assert_eq!(insert("a+b"), "a+b");
        assert_eq!(insert("a*b+c"), "a*b+c");
        assert_eq!(insert("sqrt(x)"), "sqrt(x)");
        assert_eq!(insert(""), "");
    }

    #[test]
    fn multiple_matches_in_one_pass() {
        // i=0 '2' 开头 → 命中 → "2*x"；随后 `x3y` 中 3 的前驱是 'x'（word char）→ 不补
        assert_eq!(insert("2x3y"), "2*x3y");
    }

    #[test]
    fn chained_parentheses() {
        assert_eq!(insert("(a)(b)(c)"), "(a)*(b)*(c)");
    }

    #[test]
    fn number_after_operator_is_still_a_candidate() {
        // `+2x`：`2` 前驱是 `+`（非 word char）→ 候选
        assert_eq!(insert("+2x"), "+2*x");
        assert_eq!(insert("(2)x"), "(2)*x");
        assert_eq!(insert(",2x"), ",2*x");
    }

    #[test]
    fn rparen_followed_by_operator_untouched() {
        // `)` 后跟运算符 → 组 2 不匹配
        assert_eq!(insert("(a+b)+c"), "(a+b)+c");
        assert_eq!(insert("(a+b)*c"), "(a+b)*c");
        assert_eq!(insert("(a+b)"), "(a+b)");
    }

    // =========================================================================
    // ⚠️ 源实现的既有怪癖（本移植**原样保留**，不做"改进"）
    // =========================================================================
    //
    // 以下行为由源正则 `((?<![A-Za-z0-9_])[0-9]|\))\s*([0-9A-Za-z(])` 决定，
    // Kotlin 版与本 Rust 版**完全一致**。记录在此以免后人误判为移植缺陷。
    //
    // 影响面有限：`insert` 只被 **MathLayout（二维排版）** 与
    // **EquationVerifier（代入验算）** 调用，**不参与 FormulaEngineImpl.compile**，
    // 因此不会影响公式求值结果。

    /// 怪癖 1：连续数字会被从第二个数字处切开 —— `123` → `1*23`
    ///
    /// 原因：位置 0 的 `1` 满足"无前置字符"，组 2 匹配到 `2`；
    /// 之后位置 2 的 `3` 前驱是 `2`（word char），环视失败。
    #[test]
    fn quirk_consecutive_digits_are_split() {
        assert_eq!(insert("123"), "1*23");
    }

    /// 怪癖 2：科学计数法会被打断 —— `1e3` → `1*e3`
    ///
    /// 原因：`1` 无前置字符 → 组 1；`e` 属于组 2 字符集。
    /// 之后 `3` 的前驱是 `e`（word char），环视失败。
    #[test]
    fn quirk_scientific_notation_is_broken() {
        assert_eq!(insert("1e3"), "1*e3");
        assert_eq!(insert("1e3*2"), "1*e3*2");
    }

    /// 源项目注释提到的真实危害：`2x` 代入 x=1 后不能变成 `21`
    #[test]
    fn substitution_safety() {
        // 补全后代入才安全：`2*x` 代入 x=1 → `2*1`
        let fixed = insert("2x");
        assert_eq!(fixed, "2*x");
        // 若不补，朴素替换会得到 `21`（错位但看似合理）
        let naive = "2x".replace('x', "1");
        assert_eq!(naive, "21", "演示朴素替换的错误结果");
    }
}
