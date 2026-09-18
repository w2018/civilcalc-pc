//! 计算书里的数值格式化。
//!
//! 源：`DocxGenerator.kt` 的私有 `formatNumber`
//!
//! ```kotlin
//! private fun formatNumber(d: Double): String {
//!     return if (d == d.toLong().toDouble()) d.toLong().toString() else {
//!         String.format(Locale.US, "%.6f", d).trimEnd('0').trimEnd('.')
//!     }
//! }
//! ```
//!
//! ## 口径
//!
//! - **整数不带小数点**（`6` 而不是 `6.000000`）
//! - 非整数最多 **6 位小数**，再去尾零（`3.141593`、`1.5`、`0.333333`）
//! - 用 `Locale.US` 风格的小数点（`.`，不是某些地区的 `,`）
//!
//! ## 为什么单独一个模块
//!
//! 计算书正文（`civilcalc-report::docx`）与结果复制文本（[`crate::result_outputs`]）都要用。
//! 若把它放在 `docx.rs` 里，`result_outputs` 就得反向依赖 `docx` ——
//! 两个本应平行的模块被绑在一起。
//!
//! > 本模块原先在 `civilcalc-report`，P4-4 迁到 core：源 `ResultOutputs` 属
//! > `:core/formula/`，且 `civilcalc-llm` 的 normalizer 也要用同一条链路
//! > （`llm → core` 是允许的方向，见 `docs/02` 的依赖图）。
//! > 迁入时顺带**消除了 `display/math_layout.rs` 里的一份私有重复实现**。
//!
//! ## 与其它两个「数值格式化」的区别（别混用）
//!
//! | 函数 | 用途 | 口径 |
//! |---|---|---|
//! | [`format_number`] | **计算书 / 结果文本** | 整数不带点，小数 6 位去尾零 |
//! | `excel::number_formatter::format` | **Excel 公式字面量** | 负数加括号、科学计数法带 `E+` |
//! | 前端 `utils/format.ts` | 界面展示 | 按量级自适应精度 |

/// 计算书口径的数值文本。见模块文档。
///
/// 非有限值输出 `NaN` / `Infinity` / `-Infinity`（与 Java 的 `%.6f` 一致）——
/// **不**输出 Rust 默认的 `inf`，否则同一份数据在两端产物里长得不一样。
pub fn format_number(v: f64) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }

    // 整数值：直接输出，不带小数点
    //
    // ⚠️ `v as i64` 对超范围值会**饱和**到 `i64::MAX`，因此 `1e300` 不会
    //    被误判成整数（`i64::MAX as f64` ≠ `1e300`），会走到下面的小数分支。
    //    与 Kotlin 的 `d.toLong()` 饱和行为一致。
    if v == (v as i64) as f64 {
        return (v as i64).to_string();
    }

    format!("{v:.6}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_have_no_decimal_point() {
        assert_eq!(format_number(0.0), "0");
        assert_eq!(format_number(6.0), "6");
        assert_eq!(format_number(-3.0), "-3");
        assert_eq!(format_number(1_000_000.0), "1000000");
    }

    /// `-0.0` 视作 `0`（`-0.0 == 0.0` 为真）
    #[test]
    fn negative_zero_is_zero() {
        assert_eq!(format_number(-0.0), "0");
    }

    #[test]
    fn decimals_trimmed_to_at_most_six() {
        assert_eq!(format_number(1.5), "1.5");
        // ⚠️ 别用 `3.14` 之类的字面量 —— clippy 的 `approx_constant` 会判它是 PI 近似
        assert_eq!(format_number(3.25), "3.25");
        assert_eq!(format_number(1.0 / 3.0), "0.333333");
        assert_eq!(format_number(2.0 / 3.0), "0.666667", "四舍五入到 6 位");
    }

    /// 6 位之后的小数被**截掉**（不是截断字符串，是格式化时就只留 6 位）
    #[test]
    fn trailing_zeros_removed() {
        assert_eq!(format_number(1.100000), "1.1");
        assert_eq!(format_number(1.2000001), "1.2");
        assert_eq!(format_number(0.5), "0.5");
    }

    #[test]
    fn small_values_keep_precision() {
        assert_eq!(format_number(0.000001), "0.000001");
        // 小于 1e-6 会被 %.6f 打成 0.000000 → 去尾零 → 0
        assert_eq!(format_number(1e-9), "0");
    }

    /// 极大值走小数分支（`as i64` 饱和但判等失败），输出完整十进制
    #[test]
    fn huge_values_do_not_panic() {
        let s = format_number(1e20);
        assert!(!s.is_empty());
        assert!(!s.contains('e'), "不得输出科学计数法: {s}");
        // 1e20 是整数值但超出 i64 → 走到 %.6f 分支 → 全是 0 尾 → 被 trim 成 "100000000000000000000"
        assert_eq!(s, "100000000000000000000");
    }

    #[test]
    fn non_finite_values_match_java_spelling() {
        assert_eq!(format_number(f64::NAN), "NaN");
        assert_eq!(format_number(f64::INFINITY), "Infinity");
        assert_eq!(format_number(f64::NEG_INFINITY), "-Infinity");
    }

    /// 负小数保留符号
    #[test]
    fn negative_decimals() {
        assert_eq!(format_number(-1.5), "-1.5");
        assert_eq!(format_number(-0.25), "-0.25");
    }
}
