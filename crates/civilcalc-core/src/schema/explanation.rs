//! 详解等 AI 长文本的显示规范化。
//!
//! 源：`civilcalc-android-v2/core/schema/ExplanationText.kt`
//!
//! ## 为什么需要它（源项目注释原文）
//!
//! > 模型对「分点换行」的执行很不稳定：同一个提示词下，有时 ①②③ 各占一行、
//! > 有时全挤在同一段，偶尔还会把换行写成字面量 `\n`（两个字符）。
//! > 这里做确定性处理，保证一个分点一行，不依赖模型发挥。
//!
//! 这是**纯函数**，可直接单测。

/// 圆圈数字分点（①…⑳，Unicode 连续区间 U+2460..=U+2473）
fn is_circled(c: char) -> bool {
    ('\u{2460}'..='\u{2473}').contains(&c)
}

/// 允许出现在分点前的边界字符。
///
/// 只有紧跟在标点或空白之后才认定是新分点的开始 —— 这样「第①条」「图①」
/// 这类行内引用不会被拆断，而「②③另计」这种连写的复合指代也保持在一起。
const BOUNDARY: &[char] = &[
    '；', ';', '。', '，', ',', '：', ':', '、', '）', ')', '】', ']', '」', '”', '"', ' ', '\t',
    '·',
];

/// 强调开标记：换行要插在它之前，否则会把 `**` 或 `==` 标记拆断
const EMPHASIS: &[&str] = &["**", "=="];

/// 渲染前的统一规范化：先还原字面量换行，再按分点边界拆行。
pub fn normalize(text: &str) -> String {
    if text.is_empty() {
        return text.to_string();
    }
    split_circled_points(&unescape_literal_newlines(text))
}

/// 把模型写出的字面量 `\n` / `\r\n`（双转义残留）还原为真实换行。
///
/// **代码块内不动**（可能是代码或正则示例的一部分）：以 ``` 分段，偶数段在代码块外。
pub fn unescape_literal_newlines(text: &str) -> String {
    if !text.contains("\\n") {
        return text.to_string();
    }
    text.split("```")
        .enumerate()
        .map(|(index, segment)| {
            if index % 2 == 0 {
                segment.replace("\\r\\n", "\n").replace("\\n", "\n")
            } else {
                segment.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("```")
}

/// 圆圈数字分点规范化：①②③ 挤在同一行时按分点边界拆行，保证一个分点一行。
pub fn split_circled_points(text: &str) -> String {
    if !text.chars().any(is_circled) {
        return text.to_string();
    }
    text.split('\n')
        .map(split_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// 单行内的分点拆分。
fn split_line(line: &str) -> String {
    let mut sb: Vec<char> = Vec::with_capacity(line.len() + 8);

    for c in line.chars() {
        if is_circled(c) && !sb.is_empty() {
            // 找出紧贴行尾的强调标记（** 优先于 ==，与源项目一致）
            let marker = EMPHASIS.iter().find(|m| ends_with_chars(&sb, m));
            let marker_len = marker.map(|m| m.chars().count()).unwrap_or(0);

            // 标记之前的那一个字符
            let before_idx = sb.len() as isize - marker_len as isize - 1;
            let before = if before_idx >= 0 {
                sb.get(before_idx as usize).copied()
            } else {
                None
            };

            if let Some(b) = before {
                if BOUNDARY.contains(&b) {
                    if marker.is_some() {
                        sb.truncate(sb.len() - marker_len);
                    }
                    sb.push('\n');
                    if let Some(m) = marker {
                        sb.extend(m.chars());
                    }
                }
            }
        }
        sb.push(c);
    }

    sb.into_iter().collect()
}

/// `sb` 是否以 `s` 结尾（按字符比较）
fn ends_with_chars(sb: &[char], s: &str) -> bool {
    let sc: Vec<char> = s.chars().collect();
    sb.len() >= sc.len() && sb[sb.len() - sc.len()..] == sc[..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_stays_empty() {
        assert_eq!(normalize(""), "");
    }

    #[test]
    fn literal_newlines_are_unescaped() {
        assert_eq!(normalize(r"a\nb"), "a\nb");
        assert_eq!(normalize(r"a\r\nb"), "a\nb");
    }

    #[test]
    fn code_block_literal_newlines_are_kept() {
        // 奇数段（代码块内）不动
        let input = r"before\n```code\nblock```after\nend";
        let out = normalize(input);
        assert!(out.contains(r"code\nblock"), "代码块内的字面量 \\n 不应被替换: {out}");
        assert!(out.contains("before\n"), "代码块外的字面量 \\n 应被替换: {out}");
    }

    #[test]
    fn circled_points_on_one_line_get_split() {
        // 前一个分点号后跟标点 → 认定为边界 → 拆行
        let out = split_circled_points("① 第一点；② 第二点");
        assert_eq!(out, "① 第一点；\n② 第二点");
    }

    #[test]
    fn inline_circled_reference_is_not_split() {
        // 「第①条」是行内引用，前一个字符不是边界 → 不拆
        let out = split_circled_points("见第①条");
        assert_eq!(out, "见第①条");
    }

    #[test]
    fn consecutive_circled_marks_stay_together() {
        // 「②③另计」连写 → ② 不拆（其前是 ① 不是边界），③ 也不拆（其前是 ② 不是边界）
        let out = split_circled_points("① 甲；②③另计");
        assert_eq!(out, "① 甲；\n②③另计");
    }

    #[test]
    fn emphasis_marker_is_not_broken() {
        // ** 紧贴在分点号之前时，换行要插在 ** 之前
        let out = split_circled_points("前一点；**① 加粗分点");
        assert_eq!(out, "前一点；\n**① 加粗分点");
    }

    #[test]
    fn already_multiline_is_preserved() {
        let out = split_circled_points("① 甲\n② 乙");
        assert_eq!(out, "① 甲\n② 乙");
    }
}
