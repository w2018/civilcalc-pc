//! 拼音索引（替代源项目 `PinyinIndex.kt`）。
//!
//! 源项目用 `pinyin4j`；PC 端用 **`pinyin` crate**（ADR-008）。
//!
//! ## 🔴 修正了源项目的一个 bug（**行为差异，已确认**）
//!
//! 源项目 `PinyinIndex.kt`：
//!
//! ```kotlin
//! fun getFirstLetters(text: String): String =
//!     pinyinArray(text).joinToString("") { it.firstOrNull()?.uppercase()?.toString() ?: "" }
//!     //                                                    ^^^^^^^^^ 大写
//!
//! fun getAllPinyinVariants(text: String): Set<String> {
//!     variants.addAll(getFullPinyin(text))    // 小写
//!     variants.add(getFirstLetters(text))     // 大写 ←
//!     variants.add(text.lowercase())
//! }
//!
//! fun match(query: String, target: String): Boolean {
//!     val q = query.lowercase().trim()        // 小写
//!     return targetPinyin.any { it.contains(q) }   // 大写的首字母串永远匹配不上小写查询
//! }
//! ```
//!
//! `getFirstLetters` 返回**大写**（`"LZJM"`），而 `match` 用**小写**查询比对，
//! 因此 **`"lzjm"` 永远搜不到「梁正截面」** —— 首字母检索实际是**死代码**。
//! 源项目**没有**对应测试，所以一直没被发现。
//!
//! PC 端 [`first_letters`] 返回**小写**，让首字母检索真正可用。
//! 语义上不违背原意（`getFirstLetters` 的用途就是"用缩写搜"），
//! 因此判定为**修正**而非偏离。全拼检索（`getFullPinyin`）本就工作，行为不变。
//!
//! ## 多音字
//!
//! 源项目 `toHanyuPinyinStringArray(ch)?.firstOrNull()` —— **取第一个读音**。
//! 这里用 `pinyin` crate 的 `plain()`，同样是单读音，行为一致。
//!
//! ## 非汉字
//!
//! 原样保留（`"a1"` → `["a", "1"]`），因此 `all_variants` 里会出现单字符串，
//! 与源项目 `perChar.filter { it.length > 1 }` 的过滤口径一致。

use pinyin::ToPinyin;
use std::collections::HashSet;

/// 逐字符转拼音；非汉字保留原字符。
///
/// 对齐源项目 `PinyinIndex.pinyinArray`。
pub fn pinyin_array(text: &str) -> Vec<String> {
    text.chars()
        .map(|c| {
            c.to_pinyin()
                .map(|p| p.plain().to_string())
                .unwrap_or_else(|| c.to_string())
        })
        .collect()
}

/// 全拼变体。
///
/// 对齐源项目 `getFullPinyin`：
/// 1. 整串拼接（`"梁正截面"` → `"liangzhengjiemian"`）
/// 2. **各字拼音中长度 > 1 的**（过滤掉单字符非汉字）
///
/// 返回**去重**且**保序**的列表（源项目用 `distinct()` 保序）。
pub fn full_pinyin(text: &str) -> Vec<String> {
    let per_char = pinyin_array(text);
    let mut out: Vec<String> = Vec::new();

    // 1. 整串
    push_distinct(&mut out, per_char.concat().to_lowercase());

    // 2. 各字拼音（长度 > 1 的）
    for p in per_char.iter().filter(|p| p.chars().count() > 1) {
        push_distinct(&mut out, p.to_lowercase());
    }

    out
}

/// 非空且未出现过的才追加（源项目 `distinct()` 的保序语义）
fn push_distinct(out: &mut Vec<String>, s: String) {
    if !s.is_empty() && !out.iter().any(|x| x == &s) {
        out.push(s);
    }
}

/// 首字母缩写，**小写**。
///
/// ⚠️ 源项目此处返回**大写**，导致 [`matches`] 永远匹配不上（见模块文档）。
/// 这里改为小写以修正该 bug。
///
/// `"梁正截面"` → `"lzjm"`
pub fn first_letters(text: &str) -> String {
    pinyin_array(text)
        .iter()
        .filter_map(|p| p.chars().next())
        .collect::<String>()
        .to_lowercase()
}

/// 全部可匹配的变体：全拼 ∪ 首字母 ∪ 原文小写。
///
/// 对齐源项目 `getAllPinyinVariants`（首字母部分改为小写）。
pub fn all_variants(text: &str) -> HashSet<String> {
    let mut set: HashSet<String> = full_pinyin(text).into_iter().collect();
    set.insert(first_letters(text));
    set.insert(text.to_lowercase());
    set
}

/// 查询是否命中目标（中文 / 全拼 / 首字母）。
///
/// 对齐源项目 `PinyinIndex.match`：
/// 1. 原文小写包含 → 命中
/// 2. 否则任一拼音变体包含 → 命中
///
/// 空查询（或纯空白）→ **恒 `false`**（源项目 `if (q.isBlank()) return false`）。
pub fn matches(query: &str, target: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return false;
    }
    if target.to_lowercase().contains(&q) {
        return true;
    }
    all_variants(target).iter().any(|v| v.contains(&q))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------- pinyin_array ----------------

    #[test]
    fn array_converts_chinese_and_keeps_others() {
        let a = pinyin_array("梁a1");
        assert_eq!(a.len(), 3);
        assert_eq!(a[0], "liang");
        assert_eq!(a[1], "a");
        assert_eq!(a[2], "1");
    }

    #[test]
    fn array_handles_empty() {
        assert!(pinyin_array("").is_empty());
    }

    // ---------------- 全拼 ----------------

    #[test]
    fn full_pinyin_contains_whole_and_per_char() {
        let v = full_pinyin("梁正");
        assert!(v.contains(&"liangzheng".to_string()), "整串应在: {v:?}");
        assert!(v.contains(&"liang".to_string()), "单字应在: {v:?}");
        assert!(v.contains(&"zheng".to_string()), "单字应在: {v:?}");
    }

    #[test]
    fn full_pinyin_is_deduplicated() {
        let v = full_pinyin("人人");
        let n = v.iter().filter(|x| x.as_str() == "ren").count();
        assert_eq!(n, 1, "重复的单字拼音应去重");
    }

    /// 单字符非汉字被过滤（源项目 `length > 1` 口径）
    #[test]
    fn full_pinyin_filters_single_char_non_chinese() {
        let v = full_pinyin("a梁");
        assert!(v.contains(&"aliang".to_string()), "整串仍在");
        assert!(v.contains(&"liang".to_string()), "汉字拼音在");
        assert!(!v.contains(&"a".to_string()), "单字符非汉字应被过滤");
    }

    #[test]
    fn full_pinyin_of_pure_ascii() {
        // 全是单字符 → 只剩整串
        let v = full_pinyin("abc");
        assert_eq!(v, vec!["abc".to_string()]);
    }

    // ---------------- 首字母 ----------------

    #[test]
    fn first_letters_is_lowercase() {
        assert_eq!(first_letters("梁正截面"), "lzjm");
        // ⚠️ 源项目此处返回 "LZJM"（大写），导致匹配失效
        assert_ne!(first_letters("梁正截面"), "LZJM");
    }

    #[test]
    fn first_letters_keeps_non_chinese() {
        assert_eq!(first_letters("梁a正"), "laz");
    }

    #[test]
    fn first_letters_empty() {
        assert_eq!(first_letters(""), "");
    }

    // ---------------- 变体集合 ----------------

    #[test]
    fn all_variants_has_three_kinds() {
        let v = all_variants("梁正");
        assert!(v.contains("liangzheng"), "整串全拼");
        assert!(v.contains("liang"), "单字全拼");
        assert!(v.contains("lz"), "首字母");
        assert!(v.contains("梁正"), "原文小写");
    }

    // ---------------- 匹配 ----------------

    #[test]
    fn matches_chinese_substring() {
        assert!(matches("梁正", "梁正截面受弯"));
        assert!(matches("截面", "梁正截面受弯"));
    }

    #[test]
    fn matches_full_pinyin() {
        assert!(matches("liangzheng", "梁正截面"));
        assert!(matches("liang", "梁正截面"));
        // 前缀也应命中（contains 语义）
        assert!(matches("liangzh", "梁正截面"));
    }

    /// **回归**：首字母缩写必须能搜到（源项目此功能失效）
    #[test]
    fn matches_first_letters_abbreviation() {
        assert!(
            matches("lzjm", "梁正截面"),
            "首字母缩写应命中 —— 这是源项目失效的功能"
        );
        assert!(matches("lzj", "梁正截面"), "前缀缩写也应命中");
    }

    #[test]
    fn matches_is_case_insensitive() {
        assert!(matches("LZJM", "梁正截面"));
        assert!(matches("LiAng", "梁正截面"));
    }

    #[test]
    fn matches_trims_query() {
        assert!(matches("  lzjm  ", "梁正截面"));
    }

    #[test]
    fn blank_query_never_matches() {
        assert!(!matches("", "梁正截面"));
        assert!(!matches("   ", "梁正截面"));
        assert!(!matches("", ""));
    }

    #[test]
    fn non_matching_query_returns_false() {
        assert!(!matches("zzzz", "梁正截面"));
        assert!(!matches("混凝土", "梁正截面"));
    }

    #[test]
    fn matches_ascii_target() {
        assert!(matches("abc", "abcdef"));
        assert!(matches("ABC", "abcdef"));
        assert!(!matches("xyz", "abcdef"));
    }

    #[test]
    fn matches_mixed_target() {
        assert!(matches("hrb400", "钢筋HRB400"));
        assert!(matches("gangjin", "钢筋HRB400"));
        assert!(matches("gj", "钢筋HRB400"), "首字母");
    }

    // ---------------- 真实公式名（内置库里的典型值） ----------------

    /// 用内置库里的真实公式名验证（拼音值取自实际输出，不是猜的）
    #[test]
    fn realistic_formula_names() {
        for (name, full, abbr) in [
            (
                "梁正截面受弯承载力",
                "liangzhengjiemianshouwanchengzaili",
                "lzjmswczl",
            ),
            (
                "柱轴心受压承载力",
                "zhuzhouxinshouyachengzaili",
                "zzxsyczl",
            ),
            ("钢筋混凝土", "gangjinhunningtu", "gjhnt"),
        ] {
            assert!(matches(full, name), "全拼应命中 {name}");
            assert!(matches(abbr, name), "首字母应命中 {name}");
            assert!(matches(name, name), "原文应命中 {name}");
            // 前缀也应命中
            assert!(matches(&full[..6], name), "全拼前缀应命中 {name}");
        }
    }

    /// ⚠️ 注意「混凝土」是 `hunningtu` 不是 `huoningtu` —— 别凭印象写拼音
    #[test]
    fn huningtu_not_huoningtu() {
        assert!(matches("hunningtu", "混凝土"));
        assert!(!matches("huoningtu", "混凝土"));
    }
}

