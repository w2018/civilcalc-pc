//! Excel 数值字面量规范化。
//!
//! 源：`core/formula/export/ExcelNumberFormatter.kt`
//!
//! ## 为什么要有这一层
//!
//! 所有代入 Excel 公式的数值（用户输入、默认值、常量）都必须先经过这里，
//! 否则产物不是 Excel 能直接解析的字面量。要处理的情况：
//!
//! | 输入 | 问题 | 输出 |
//! |---|---|---|
//! | `1,234.5` | 千分位逗号 | `1234.5` |
//! | `１２３` | 全角数字 | `123` |
//! | `25 mm` | 尾随单位 | `25` |
//! | `-5` | `-5^2` 在 Excel 里是 `(-5)^2` 还是 `-(5^2)`？**是前者** | `(-5)` |
//! | `1e300` | Excel 要显式指数符号 | `1E+300` |
//!
//! ## ⚠️ 负数**必须加括号**
//!
//! Excel 的 `^` 优先级高于一元负号：`-5^2` 会被解析成 `(-5)^2 = 25`，
//! 而数学上应是 `-(5^2) = -25`。加括号后 `(-5)^2` 语义明确。
//!
//! ## ⚠️ 千分位不能用正则 lookbehind
//!
//! 源项目用 `(?<=\d),(?=\d)`，但 **Rust 的 `regex` crate 不支持 lookaround**。
//! 这里改成单趟字符扫描（也更快）—— **不要**退回 `replace` 循环：
//! `1,234,567` 里第一个逗号删掉后，第二个逗号的判定位置就变了，
//! 循环替换容易漏或误删。

use regex::Regex;
use std::sync::LazyLock;

/// 数值头部：可选正负号 + 整数/小数 + 可选科学计数法指数
static NUMERIC_HEAD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[+\-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+\-]?\d+)?").expect("NUMERIC_HEAD 正则合法")
});

/// 单位尾巴：字母（含 `µ`）+ 角度符号
static UNIT_TAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-zµΩ°′″]+$").expect("UNIT_TAIL 正则合法")
});

/// 全角 → 半角（数字、小数点、逗号、正负号）
fn normalize_full_width(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '０'..='９' => out.push((b'0' + (c as u32 - '０' as u32) as u8) as char),
            '．' => out.push('.'),
            '，' => out.push(','),
            '－' => out.push('-'),
            '＋' => out.push('+'),
            other => out.push(other),
        }
    }
    out
}

/// 删掉「两侧都是数字」的逗号（千分位分隔符）。
///
/// 单趟扫描：只看原串的相邻字符决定删不删，不受已删除字符影响。
fn strip_thousands(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == ',' {
            let prev_digit = i > 0 && chars[i - 1].is_ascii_digit();
            let next_digit = i + 1 < chars.len() && chars[i + 1].is_ascii_digit();
            if prev_digit && next_digit {
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// 把用户输入解析为 `f64`。
///
/// 接受：千分位、全角、尾随单位（`25 mm`）、前后空格。
/// 拒绝：空串、纯单位、单位在中间（`25mm30`）、非有限值。
///
/// 失败返回 `None` —— 调用方应**保留原单元格引用并告警**，而不是静默当 0。
pub fn parse(raw: &str) -> Option<f64> {
    let normalized = normalize_full_width(raw);
    let trimmed = normalized.trim().replace(' ', "");
    if trimmed.is_empty() {
        return None;
    }

    let s = strip_thousands(&trimmed);
    let m = NUMERIC_HEAD.find(&s)?;

    // 数值头之后只允许是单位（否则像 `1.2.3` 这种要拒绝）
    let tail = &s[m.end()..];
    if !tail.is_empty() && !UNIT_TAIL.is_match(tail) {
        return None;
    }

    let v: f64 = m.as_str().parse().ok()?;
    v.is_finite().then_some(v)
}

/// 把用户输入转成 Excel 数值字面量（负数带括号）。
///
/// 空/空白输入由调用方处理；无法解析返回 `None`。
pub fn to_excel_literal(raw: &str) -> Option<String> {
    format(parse(raw)?)
}

/// `f64` → Excel 数值字面量。
///
/// 非有限值返回 `None`（调用方应报错，而不是静默写出 `NaN`）。
pub fn format(value: f64) -> Option<String> {
    if !value.is_finite() {
        return None;
    }
    if value == 0.0 {
        // `-0.0` 也走这里 —— Excel 里 `-0` 是没意义的噪音
        return Some("0".to_string());
    }

    let abs = value.abs();
    // 超出 `[1e-9, 1e15)` 就用科学计数法（Excel 对这种量级也更认指数写法）
    let body = if (1e-9..1e15).contains(&abs) {
        plain(value)
    } else {
        scientific(value)
    };

    Some(if value < 0.0 {
        format!("({body})")
    } else {
        body
    })
}

/// 科学计数法：`1.5E-10` / `1E+300`（**指数必须带显式符号**）。
///
/// ⚠️ Rust 的 `{:.6E}` 输出 `1.000000E300` —— **不带 `+`**，
/// 而 Excel 要求 `1E+300`。Kotlin 的 `String.format("%.6E")` 自带 `+`，
/// 这是移植时最容易漏的一处差异。
fn scientific(v: f64) -> String {
    let s = format!("{v:.6E}");
    let (mant, exp) = match s.split_once('E') {
        Some((m, e)) => (m, e),
        // 理论上不会发生（`{:.6E}` 必带 E），兜底返回原串
        None => return s,
    };

    let mant = mant.trim_end_matches('0').trim_end_matches('.');
    let exp = if exp.starts_with('-') {
        exp.to_string()
    } else {
        format!("+{exp}")
    };
    format!("{mant}E{exp}")
}

/// 普通十进制（不用科学计数法，去掉尾随零）。
///
/// Rust 的 `{}` 对 `f64` 输出**最短往返表示**且不用科学计数法，
/// 与 Kotlin 的 `BigDecimal(toString()).toPlainString()` 行为一致。
fn plain(v: f64) -> String {
    format!("{v}")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------- parse ----------------

    #[test]
    fn parses_plain_numbers() {
        assert_eq!(parse("0"), Some(0.0));
        assert_eq!(parse("14.3"), Some(14.3));
        assert_eq!(parse("-5"), Some(-5.0));
        assert_eq!(parse("+5"), Some(5.0));
        assert_eq!(parse(".5"), Some(0.5));
        assert_eq!(parse("5."), Some(5.0));
    }

    #[test]
    fn parses_with_thousands_separator() {
        assert_eq!(parse("1,234.5"), Some(1234.5));
        // 连续千分位：单趟扫描要能全部删掉
        assert_eq!(parse("1,234,567"), Some(1_234_567.0));
        assert_eq!(parse("12,345,678.9"), Some(12_345_678.9));
    }

    /// 逗号两侧不都是数字时**不能删**（那可能是别的东西）
    #[test]
    fn keeps_comma_when_not_thousands_separator() {
        // 前导逗号：`NUMERIC_HEAD` 要求以数字或正负号开头 → 整体拒绝
        // （不是"跳过逗号取后面" —— 那会把 `,abc` 也当成合法输入）
        assert_eq!(parse(",123"), None);
        // `123,` 尾部是逗号 → 不是合法单位 → 拒绝
        assert_eq!(parse("123,"), None);
    }

    #[test]
    fn parses_full_width() {
        assert_eq!(parse("１２３"), Some(123.0));
        assert_eq!(parse("１２．５"), Some(12.5));
        assert_eq!(parse("－５"), Some(-5.0));
        assert_eq!(parse("＋５"), Some(5.0));
    }

    #[test]
    fn parses_with_trailing_unit() {
        assert_eq!(parse("25 mm"), Some(25.0));
        assert_eq!(parse("25mm"), Some(25.0));
        assert_eq!(parse("3.5 kN·m"), None, "含 · 的复合单位不在白名单 → 拒绝");
        assert_eq!(parse("90°"), Some(90.0));
        assert_eq!(parse("30′"), Some(30.0));
    }

    #[test]
    fn rejects_invalid() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("   "), None);
        assert_eq!(parse("abc"), None);
        assert_eq!(parse("mm"), None);
        // 单位在中间 → 拒绝（不是"取前面数字"）
        assert_eq!(parse("25mm30"), None);
        assert_eq!(parse("1.2.3"), None);
    }

    #[test]
    fn parses_scientific_notation() {
        assert_eq!(parse("1e3"), Some(1000.0));
        assert_eq!(parse("1.5E-3"), Some(0.0015));
        assert_eq!(parse("1E+3"), Some(1000.0));
    }

    // ---------------- format ----------------

    #[test]
    fn formats_zero_as_bare_zero() {
        assert_eq!(format(0.0), Some("0".to_string()));
        assert_eq!(format(-0.0), Some("0".to_string()), "负零也归一为 0");
    }

    #[test]
    fn formats_plain_numbers() {
        assert_eq!(format(14.3), Some("14.3".to_string()));
        assert_eq!(format(1234.5), Some("1234.5".to_string()));
        assert_eq!(format(100.0), Some("100".to_string()), "去掉尾随零");
        assert_eq!(format(1e14), Some("100000000000000".to_string()));
    }

    /// ⚠️ 负数**必须加括号**：Excel 里 `-5^2` = `(-5)^2` = 25，不是 -25
    #[test]
    fn negative_is_parenthesized() {
        assert_eq!(format(-5.0), Some("(-5)".to_string()));
        assert_eq!(format(-0.5), Some("(-0.5)".to_string()));
    }

    /// 极大/极小走科学计数法，且**指数带显式符号**（Excel 要求）
    #[test]
    fn large_and_small_use_scientific_with_sign() {
        assert_eq!(format(1e300), Some("1E+300".to_string()));
        assert_eq!(format(1.5e-10), Some("1.5E-10".to_string()));
        assert_eq!(format(1e15), Some("1E+15".to_string()));
        // 负数：科学计数法的负号在**括号内**（`(-1E+300)` 而不是 `-(1E+300)`）
        assert_eq!(format(-1e300), Some("(-1E+300)".to_string()));
        assert_eq!(format(-1.5e-10), Some("(-1.5E-10)".to_string()));
    }

    /// 边界：正好 1e-9 走普通分支（`< 1e-9` 是严格小于）
    #[test]
    fn boundary_at_one_e_minus_nine() {
        assert_eq!(format(1e-9), Some("0.000000001".to_string()));
        // 略小于 → 科学计数法
        assert_eq!(format(9.9e-10), Some("9.9E-10".to_string()));
    }

    #[test]
    fn non_finite_returns_none() {
        assert_eq!(format(f64::NAN), None);
        assert_eq!(format(f64::INFINITY), None);
        assert_eq!(format(f64::NEG_INFINITY), None);
    }

    // ---------------- to_excel_literal ----------------

    #[test]
    fn literal_combines_parse_and_format() {
        assert_eq!(to_excel_literal("1,234.5"), Some("1234.5".to_string()));
        assert_eq!(to_excel_literal("25 mm"), Some("25".to_string()));
        assert_eq!(to_excel_literal("-5"), Some("(-5)".to_string()));
        assert_eq!(to_excel_literal("１２３"), Some("123".to_string()));
        assert_eq!(to_excel_literal("abc"), None);
    }
}
