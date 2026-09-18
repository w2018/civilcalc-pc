//! 详解正文里的附图标记处理。
//!
//! 源：`civilcalc-android-v2/core/schema/ImageMarker.kt`
//!
//! ## 语义（源项目注释原文）
//!
//! > AI 在确实需要图示辅助的位置插入 `{{img:N}}`，N 为附图序号（从 1 起，按附图给出顺序）。
//! >
//! > 标记只是渲染提示，**不参与 Schema 校验**：越界或格式不符的标记按普通文本处理，
//! > 绝不会因为标记问题导致解析失败。
//!
//! 这是**刻意的容错设计** —— 附图标记是"装饰"，不能让它影响公式解析的成功率。

use once_cell::sync::Lazy;
use regex::Regex;

/// 附图标记正则，兼容模型偶尔写出的空格变体（`{{ img : 1 }}`），捕获组 1 为序号。
///
/// 渲染端与提示词契约**共用同一份正则**，避免两处写法漂移。
pub static IMG_MARKER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\{\{\s*img\s*:\s*(\d+)\s*\}\}").expect("IMG_MARKER 正则为常量，不会失败"));

/// 剔除全部附图标记：**纯文本出口**用（续写上下文、导出等不该出现渲染指令）。
pub fn strip_all(text: &str) -> String {
    if text.is_empty() || !text.contains("img") {
        return text.to_string();
    }
    IMG_MARKER.replace_all(text, "").into_owned()
}

/// 文本实际引用的附图序号（去重升序）。
///
/// 越界序号与无附图时一律返回空 —— 保证渲染端不会去取不存在的图。
pub fn referenced_indexes(text: &str, image_count: usize) -> Vec<usize> {
    if image_count == 0 || !text.contains("img") {
        return Vec::new();
    }
    let mut idx: Vec<usize> = IMG_MARKER
        .captures_iter(text)
        .filter_map(|c| c.get(1))
        .filter_map(|m| m.as_str().parse::<usize>().ok())
        .filter(|n| (1..=image_count).contains(n))
        .collect();
    idx.sort_unstable();
    idx.dedup();
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_all_removes_markers() {
        assert_eq!(strip_all("前 {{img:1}} 后"), "前  后");
        assert_eq!(strip_all("无标记"), "无标记");
        assert_eq!(strip_all(""), "");
    }

    #[test]
    fn strip_all_handles_space_variant() {
        assert_eq!(strip_all("a{{ img : 2 }}b"), "ab");
    }

    #[test]
    fn referenced_indexes_are_sorted_and_deduped() {
        let text = "见 {{img:3}} 与 {{img:1}} 再 {{img:3}}";
        assert_eq!(referenced_indexes(text, 5), vec![1, 3]);
    }

    #[test]
    fn out_of_range_indexes_are_ignored() {
        let text = "{{img:1}} {{img:9}}";
        assert_eq!(referenced_indexes(text, 2), vec![1]);
    }

    #[test]
    fn no_images_returns_empty() {
        assert_eq!(referenced_indexes("{{img:1}}", 0), Vec::<usize>::new());
    }

    #[test]
    fn malformed_marker_is_treated_as_plain_text() {
        // 不参与校验：格式不符就当普通文本，不报错
        assert_eq!(referenced_indexes("{{img:x}}", 3), Vec::<usize>::new());
        assert_eq!(strip_all("{{img:x}}"), "{{img:x}}");
    }
}
