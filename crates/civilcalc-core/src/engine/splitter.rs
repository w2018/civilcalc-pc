//! 表达式分段工具。
//!
//! 源：`civilcalc-android-v2/core/formula/engine/ExpressionSplitter.kt`
//!
//! 供 BUG-01（多输出）与 BUG-08（等号剥离）共用。
//!
//! ## 语义
//!
//! 把 `"X = expr"` / `"a = e1; b = e2"` 拆为输出段列表，
//! 每段含**赋值目标符号**（可空）与右侧表达式。
//!
//! ## ⚠️ 两条约束（源项目注释原文）
//!
//! 1. **赋值目标仅支持 ASCII 标识符**；中文赋值目标（如 `直接费 = …`）
//!    不支持剥离，数据层须改写为拼音/英文符号
//! 2. 中段出现的 `=` 不是赋值前缀，会**原样保留**（由编译器报错）
//!    —— 这正是 BUG-01「中段等式型表达式编译失败并向上报错」的实现基础

use once_cell::sync::Lazy;
use regex::Regex;

/// 赋值前缀正则：`^([A-Za-z_][A-Za-z0-9_']*)\s*=\s*`
///
/// 仅匹配**行首**（`^` 锚定），且赋值目标只允许 ASCII 标识符。
static ASSIGN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^([A-Za-z_][A-Za-z0-9_']*)\s*=\s*").expect("ASSIGN 正则为常量，不会失败")
});

/// 一个输出段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputSegment {
    /// 赋值目标符号（`"X2 = …"` 中的 `X2`）；无赋值前缀时为 `None`
    pub symbol: Option<String>,
    /// 右侧表达式（已 trim）
    pub expression: String,
}

/// 按分号拆段，并提取每段的赋值目标。
///
/// 空段（连续分号、首尾分号）会被过滤掉。
pub fn split(expr: &str) -> Vec<OutputSegment> {
    expr.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|seg| match ASSIGN.captures(seg) {
            Some(caps) => {
                // 捕获组 1 = 赋值目标（正则已排除周围空白）
                let symbol = caps.get(1).map(|m| m.as_str().to_string());
                // 整段匹配的结束位置之后即右侧表达式
                let whole = caps.get(0).expect("捕获组 0 必然存在");
                OutputSegment {
                    symbol,
                    expression: seg[whole.end()..].trim().to_string(),
                }
            }
            None => OutputSegment {
                symbol: None,
                expression: seg.to_string(),
            },
        })
        .collect()
}

/// 剥离单个表达式的赋值前缀与前导 `=`，返回右侧表达式（不含赋值目标）。
///
/// 流程：`trim` → 去掉一个前导 `=` → `trim` → 匹配并剥离 ASCII 赋值前缀。
///
/// ⚠️ **不得**用 `replace("=", "")` 替代 —— 那会破坏 `>=` / `<=` / `==`（BUG-08）。
pub fn strip_assignment(expr: &str) -> String {
    let s = expr.trim();
    let s = s.strip_prefix('=').unwrap_or(s).trim();
    match ASSIGN.find(s) {
        Some(m) => s[m.end()..].trim().to_string(),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------- 源项目 FormulaEngineMultiOutputTest.BUG_01_splitterBasics ----------------

    #[test]
    fn bug01_splitter_basics() {
        let segs = split("X2 = X1 + 1; Y2 = Y1 + 2");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].symbol.as_deref(), Some("X2"));
        assert_eq!(segs[0].expression, "X1 + 1");
        assert_eq!(segs[1].symbol.as_deref(), Some("Y2"));
        assert_eq!(segs[1].expression, "Y1 + 2");

        assert_eq!(split("a+b")[0].symbol, None);
        assert_eq!(strip_assignment("x = 2*3"), "2*3");
    }

    // ---------------- 分段 ----------------

    #[test]
    fn single_expression_without_assignment() {
        let segs = split("a+b");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].symbol, None);
        assert_eq!(segs[0].expression, "a+b");
    }

    #[test]
    fn empty_segments_are_filtered() {
        assert!(split("").is_empty());
        assert!(split("  ").is_empty());
        assert!(split(";").is_empty());
        assert!(split(";;").is_empty());
        assert_eq!(split("a;b").len(), 2);
        // 首尾分号产生空段 → 被过滤
        assert_eq!(split(";a;b;").len(), 2);
    }

    #[test]
    fn segments_are_trimmed() {
        let segs = split("  X = 1 + 2  ;  Y = 3  ");
        assert_eq!(segs[0].symbol.as_deref(), Some("X"));
        assert_eq!(segs[0].expression, "1 + 2");
        assert_eq!(segs[1].symbol.as_deref(), Some("Y"));
        assert_eq!(segs[1].expression, "3");
    }

    #[test]
    fn assignment_target_allows_underscore_and_quote() {
        assert_eq!(split("x_1 = 5")[0].symbol.as_deref(), Some("x_1"));
        assert_eq!(split("a' = 5")[0].symbol.as_deref(), Some("a'"));
    }

    #[test]
    fn assignment_target_must_start_with_letter_or_underscore() {
        // 数字开头不是合法赋值目标 → 整段无符号（由编译器后续报错）
        let segs = split("1x = 5");
        assert_eq!(segs[0].symbol, None);
        assert_eq!(segs[0].expression, "1x = 5");
    }

    /// BUG-01：中段 `=`（等式型）不是赋值前缀，原样保留 → 编译器报错
    #[test]
    fn bug01_mid_equality_is_preserved() {
        let segs = split("sin(a) = 1");
        assert_eq!(segs[0].symbol, None, "函数调用不是赋值目标");
        assert_eq!(segs[0].expression, "sin(a) = 1", "中段 = 原样保留，交给编译器报错");
    }

    #[test]
    fn whitespace_around_equals_is_tolerated() {
        assert_eq!(split("X=1")[0].symbol.as_deref(), Some("X"));
        assert_eq!(split("X   =   1")[0].symbol.as_deref(), Some("X"));
        assert_eq!(split("X=1")[0].expression, "1");
        assert_eq!(split("X   =   1")[0].expression, "1");
    }

    // ---------------- strip_assignment ----------------

    #[test]
    fn strip_removes_leading_equals() {
        assert_eq!(strip_assignment("=a+b"), "a+b");
        assert_eq!(strip_assignment("  =  a+b  "), "a+b");
    }

    #[test]
    fn strip_removes_assignment_prefix() {
        assert_eq!(strip_assignment("x = 2*3"), "2*3");
        assert_eq!(strip_assignment("X2 = X1 + 1"), "X1 + 1");
    }

    #[test]
    fn strip_is_noop_without_assignment() {
        assert_eq!(strip_assignment("a+b"), "a+b");
        assert_eq!(strip_assignment("  a+b  "), "a+b");
    }

    /// ⚠️ BUG-08 核心：不得用 `replace("=", "")` —— 会破坏比较运算符
    #[test]
    fn bug08_comparison_operators_are_preserved() {
        // 这些表达式**不**匹配赋值前缀（`=` 前面是 `>` / `<` / `!`）→ 原样保留
        assert_eq!(strip_assignment("a>=b"), "a>=b");
        assert_eq!(strip_assignment("a<=b"), "a<=b");
        assert_eq!(strip_assignment("a!=b"), "a!=b");
    }

    /// ⚠️ 源实现的既有怪癖：`a==b` 会被当成「赋值给 a，表达式为 `=b`」
    ///
    /// 正则 `^([A-Za-z_][A-Za-z0-9_']*)\s*=\s*` 在 `a==b` 上匹配到 `a=`，
    /// 剩余 `=b`。这不是本移植引入的问题 —— Kotlin 版行为完全相同。
    ///
    /// 后果可接受：`=b` 无法编译 → 报错（**失败是显式的**，符合"禁止静默降级"）。
    /// 若将来要支持 `==`，必须同时修改源项目与提示词契约，不能只改这里。
    #[test]
    fn bug08_double_equals_is_treated_as_assignment_prefix() {
        assert_eq!(strip_assignment("a==b"), "=b");
    }

    #[test]
    fn bug08_only_one_leading_equals_is_stripped() {
        // `==a` 只剥一个前导 `=`，剩下 `=a`（保留，交由编译器处理）
        assert_eq!(strip_assignment("==a"), "=a");
    }
}
