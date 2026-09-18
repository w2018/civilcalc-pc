//! 计算书正文的文本清洗。
//!
//! 源：`core/report/ReportText.kt`（50 行）
//! 契约：`docs/04-数据契约.md` §5.4
//!
//! ## 为什么需要清洗
//!
//! AI 产出的「公式详解」是 Markdown（`**加粗**`、`==高亮==`、`- 列表`、
//! 字面量 `\n`）。直接塞进 docx 会把标记原样打出来（用户看到一堆星号）。
//! 这里统一成「一行一段」的纯文本；**强调标记交给渲染层决定是否加粗**
//! （docx 走 [`bold_segments`] 逐 run 输出，HTML 预览可自行处理）。
//!
//! ## 🔴 铁律：去列表前缀必须在拆行之前
//!
//! `- ① 取长；② 取宽。` 如果先拆行，会得到 `["-", "① 取长；", "② 取宽。"]` ——
//! **多出一个孤立的 `-` 行**。
//!
//! 机制：`-` 后面是**空格**，而空格是 `explanation::normalize` 认定的
//! 分点边界字符，于是它会在 ① 之前也插一个换行，把 `- ` 切成单独一行。
//!
//! 所以顺序是 [`strip_list_prefixes`] → `normalize` → 逐行 trim。
//! 有回归测试钉住（`order_matters_wrong_order_leaves_stray_dash`）。

use civilcalc_core::schema::explanation;

/// 强调标记：Markdown 加粗
const BOLD: &str = "**";

/// 强调标记：本项目自定的高亮
const HIGHLIGHT: &str = "==";

/// 详解文本 → 逐行纯文本。
///
/// 步骤（顺序**不可调换**）：
/// 1. 逐行去列表前缀（`- ` / `* `）
/// 2. `explanation::normalize` —— 还原字面量 `\n`、把挤在一行的 ①②③ 拆开
/// 3. 逐行 trim → 去掉 `**` 与 `==` → 再 trim
/// 4. 过滤空行、孤立的 `-` 与 `*`
pub fn plain_lines(text: &str) -> Vec<String> {
    explanation::normalize(&strip_list_prefixes(text))
        .split('\n')
        .map(|l| l.trim().replace(BOLD, "").replace(HIGHLIGHT, "").trim().to_string())
        .filter(|l| !l.is_empty() && l != "-" && l != "*")
        .collect()
}

/// 一行文本 → 加粗片段序列（`**…**` 之间的片段标记为加粗）。
///
/// 返回 `(文本, 是否加粗)`；空行返回空列表。
///
/// ## ⚠️ 索引奇偶性必须在**过滤空片段之前**判定
///
/// `"**a**"` 按 `**` 切成 `["", "a", ""]` —— 偶数下标是普通文本、
/// 奇数下标是加粗。若先过滤空串再按下标判定，`"a"` 会变成下标 0 而被判成普通文本。
/// 所以实现里先 `enumerate()` 拿原始下标，再 `filter_map`。
///
/// ## ⚠️ 必须传**未剥离 `**`** 的行
///
/// 见 [`plain_lines_keeping_emphasis`] —— 用 [`plain_lines`] 的输出调本函数
/// 永远拿不到加粗（标记已被删）。
pub fn bold_segments(line: &str) -> Vec<(String, bool)> {
    let cleaned = line.replace(HIGHLIGHT, "");
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return Vec::new();
    }

    cleaned
        .split(BOLD)
        .enumerate()
        .filter_map(|(index, part)| {
            if part.is_empty() {
                None
            } else {
                Some((part.to_string(), index % 2 == 1))
            }
        })
        .collect()
}

/// 与 [`plain_lines`] 相同的规范化与过滤，但**保留 `**` 强调标记**。
///
/// ## 🔴 为什么需要两个函数（源项目的一处 bug）
///
/// 源项目 `DocxGenerator.writeBodyLines` 的写法是：
///
/// ```kotlin
/// for (line in ReportText.plainLines(body)) {        // ← 这里已经把 ** 删掉了
///     val segments = ReportText.boldSegments(line)   // ← 永远拿不到加粗
/// ```
///
/// `plainLines` 内部会 `replace(BOLD, "")`，所以后面那次 `boldSegments`
/// 是**死代码** —— 源项目导出的 docx 里，详解正文**从来没有加粗过**。
///
/// 契约 `docs/04` §5.4 明确要求「`**加粗**` 转 docx 时**按段加粗**」，
/// 因此 PC 端**修正**为：渲染层用本函数取行（保留标记），再交给
/// [`bold_segments`] 切段。
///
/// ## 用哪个
///
/// | 场景 | 函数 |
/// |---|---|
/// | 需要纯文本（日志、搜索、纯文本导出） | [`plain_lines`] |
/// | 要渲染（docx / HTML，需要加粗） | **本函数** + [`bold_segments`] |
pub fn plain_lines_keeping_emphasis(text: &str) -> Vec<String> {
    explanation::normalize(&strip_list_prefixes(text))
        .split('\n')
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && l != "-" && l != "*")
        .collect()
}

/// 逐行去列表前缀。
fn strip_list_prefixes(text: &str) -> String {    text.split('\n')
        .map(|l| strip_list_prefix(l.trim()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 去单行的列表前缀。
///
/// ⚠️ `* ` 的判定要排除 `**` —— 否则 `**加粗**` 开头的行会被削掉一个 `*`，
/// 后面 [`bold_segments`] 的奇偶性就全乱了。
fn strip_list_prefix(line: &str) -> String {
    if let Some(rest) = line.strip_prefix("- ") {
        return rest.to_string();
    }
    if line.starts_with("* ") && !line.starts_with("**") {
        return line[2..].to_string();
    }
    line.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------ 源项目用例

    /// 源 `plainLinesSplitsCircledPointsAndStripsMarkers`
    #[test]
    fn plain_lines_splits_circled_points_and_strips_markers() {
        let lines = plain_lines("- ① 取长；② 取宽；③ 相乘。");
        assert_eq!(lines, ["① 取长；", "② 取宽；", "③ 相乘。"]);
    }

    /// 源 `boldSegmentsAlternateUnderEmphasis`
    #[test]
    fn bold_segments_alternate_under_emphasis() {
        assert_eq!(
            bold_segments("求矩形面积，**长**与宽相乘。"),
            vec![
                ("求矩形面积，".to_string(), false),
                ("长".to_string(), true),
                ("与宽相乘。".to_string(), false),
            ]
        );
    }

    // ------------------------------------------------------------ 顺序铁律

    /// **顺序铁律**：先拆行再去前缀会多出一个孤立的 `-` 行
    ///
    /// 这条测试用「错误顺序」跑一遍，证明它确实会产生脏数据 ——
    /// 免得后人觉得「先拆后去」也一样。
    ///
    /// 机制：`- ① 取长；② 取宽。` 里的 `-` 后面是**空格**，
    /// 空格属于分点边界字符，于是 `normalize` 会在 ① 之前也插一个换行，
    /// 把 `- ` 切成单独一行 `-`。最终得到 `["-", "① 取长；", "② 取宽。"]`。
    #[test]
    fn order_matters_wrong_order_leaves_stray_dash() {
        // 错误顺序：先 normalize（会在 ① 之前也插换行）
        let wrong = explanation::normalize("- ① 取长；② 取宽。");
        let wrong_lines: Vec<String> = wrong
            .split('\n')
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(
            wrong_lines,
            ["-", "① 取长；", "② 取宽。"],
            "错误顺序下 `- ` 被切成孤立的一行"
        );

        // 正确顺序：先去前缀，再 normalize → 没有孤立 `-`
        let right = plain_lines("- ① 取长；② 取宽。");
        assert_eq!(right, ["① 取长；", "② 取宽。"]);
        assert!(!right.iter().any(|l| l == "-"), "不得留下孤立的 -");
    }

    /// 若先拆行再逐行去前缀，`- ① …` 的第一行会变成 `① …`，
    /// 但**空行与孤立 `-` 的过滤规则**就管不到中间产生的垃圾 ——
    /// 这里直接验证正确顺序下无残留标记。
    #[test]
    fn no_marker_residue() {
        let lines = plain_lines("- ① **取长**；② ==取宽==。");
        assert_eq!(lines, ["① 取长；", "② 取宽。"]);
        for l in &lines {
            assert!(!l.contains("**") && !l.contains("=="), "残留标记: {l}");
        }
    }

    // ------------------------------------------------------------ 列表前缀

    #[test]
    fn dash_and_star_prefixes_are_stripped() {
        assert_eq!(strip_list_prefix("- 取长"), "取长");
        assert_eq!(strip_list_prefix("* 取长"), "取长");
        assert_eq!(strip_list_prefix("取长"), "取长");
    }

    /// `**加粗**` 开头的行**不能**被当成 `* ` 列表项
    #[test]
    fn bold_line_is_not_treated_as_star_list() {
        assert_eq!(strip_list_prefix("**重点**"), "**重点**");
        // 加了前缀的才算列表
        assert_eq!(strip_list_prefix("* **重点**"), "**重点**");
    }

    /// 只有 `- ` / `* `（带空格）才算列表前缀；`-abc` 是普通文本
    #[test]
    fn prefix_requires_trailing_space() {
        assert_eq!(strip_list_prefix("-abc"), "-abc");
        assert_eq!(strip_list_prefix("*abc"), "*abc");
        assert_eq!(strip_list_prefix("-"), "-");
    }

    #[test]
    fn multiline_prefixes_stripped_per_line() {
        let out = strip_list_prefixes("- 甲\n* 乙\n丙");
        assert_eq!(out, "甲\n乙\n丙");
    }

    /// 行首空白先被 trim 掉，因此缩进的列表项也能识别
    #[test]
    fn indented_list_items_are_stripped() {
        assert_eq!(strip_list_prefixes("  - 甲"), "甲");
    }

    // ------------------------------------------------------------ 过滤规则

    #[test]
    fn blank_lines_and_stray_markers_filtered() {
        let lines = plain_lines("甲\n\n乙\n-\n*\n   \n丙");
        assert_eq!(lines, ["甲", "乙", "丙"]);
    }

    /// 空输入与纯空白
    #[test]
    fn empty_input_yields_no_lines() {
        assert!(plain_lines("").is_empty());
        assert!(plain_lines("   \n  \n").is_empty());
        assert!(plain_lines("- \n* \n**\n==\n").is_empty());
    }

    // ------------------------------------------------------------ 加粗片段

    /// 索引奇偶性在过滤空片段**之前**判定 —— 否则 `**a**` 会判成非加粗
    #[test]
    fn bold_parity_survives_empty_part_filtering() {
        assert_eq!(bold_segments("**a**"), vec![("a".to_string(), true)]);
        assert_eq!(
            bold_segments("**a**b"),
            vec![("a".to_string(), true), ("b".to_string(), false)]
        );
        // 未闭合的 `**a`：切成 ["", "a"] → a 在下标 1 → 加粗
        assert_eq!(bold_segments("**a"), vec![("a".to_string(), true)]);
    }

    /// `==高亮==` 在加粗判定前就被抹掉（本项目高亮不用加粗表达）
    #[test]
    fn highlight_markers_removed_before_bold_split() {
        assert_eq!(
            bold_segments("==高亮==文本"),
            vec![("高亮文本".to_string(), false)]
        );
        assert_eq!(
            bold_segments("==高==**粗**"),
            vec![("高".to_string(), false), ("粗".to_string(), true)]
        );
    }

    #[test]
    fn bold_segments_on_blank_returns_empty() {
        assert!(bold_segments("").is_empty());
        assert!(bold_segments("   ").is_empty());
        assert!(bold_segments("==").is_empty());
        assert!(bold_segments("****").is_empty());
    }

    /// 多段加粗：0/2/4 普通，1/3 加粗
    #[test]
    fn multiple_bold_spans() {
        assert_eq!(
            bold_segments("a**b**c**d**e"),
            vec![
                ("a".to_string(), false),
                ("b".to_string(), true),
                ("c".to_string(), false),
                ("d".to_string(), true),
                ("e".to_string(), false),
            ]
        );
    }

    /// 🔴 **陷阱**：用 `plain_lines` 的输出调 `bold_segments` 拿不到加粗
    ///
    /// 这正是源项目 `DocxGenerator.writeBodyLines` 的死代码 ——
    /// 它先 `plainLines`（已删 `**`）再 `boldSegments`，于是详解正文
    /// **从来没有加粗过**。契约 §5.4 要求「按段加粗」，故 PC 端用
    /// [`plain_lines_keeping_emphasis`] 修正。
    #[test]
    fn plain_lines_output_has_no_emphasis_left() {
        let lines = plain_lines("- ① 求**矩形**面积。");
        assert_eq!(lines, ["① 求矩形面积。"]);
        assert_eq!(
            bold_segments(&lines[0]),
            vec![("① 求矩形面积。".to_string(), false)],
            "标记已被删，无法还原加粗"
        );
    }

    /// 渲染层应走这条路：保留标记 → `bold_segments` 切段
    #[test]
    fn keeping_emphasis_then_bold_segments_works() {
        let lines = plain_lines_keeping_emphasis("- ① 求**矩形**面积。");
        assert_eq!(lines, ["① 求**矩形**面积。"]);
        assert_eq!(
            bold_segments(&lines[0]),
            vec![
                ("① 求".to_string(), false),
                ("矩形".to_string(), true),
                ("面积。".to_string(), false),
            ]
        );
    }

    /// `plain_lines_keeping_emphasis` 的其余行为与 `plain_lines` 一致
    #[test]
    fn keeping_emphasis_matches_plain_lines_otherwise() {
        let raw = "- 甲\n\n* 乙\n-\n*\n   \n③ 丙";
        let plain = plain_lines(raw);
        let kept: Vec<String> = plain_lines_keeping_emphasis(raw)
            .iter()
            .map(|l| l.replace("**", "").replace("==", ""))
            .collect();
        assert_eq!(kept, plain, "除标记外，两者结果应一致");
    }

    /// 分点拆分、字面量换行、去前缀在保留版本里同样生效
    #[test]
    fn keeping_emphasis_still_normalizes() {
        let lines = plain_lines_keeping_emphasis("- ① **取长**；② 取宽。");
        assert_eq!(lines, ["① **取长**；", "② 取宽。"]);
        assert_eq!(bold_segments(&lines[0]), vec![
            ("① ".to_string(), false),
            ("取长".to_string(), true),
            ("；".to_string(), false),
        ]);
    }

    /// `==高亮==` 由 `bold_segments` 自己抹掉（不在保留版本里删）
    #[test]
    fn highlight_removed_by_bold_segments() {
        let lines = plain_lines_keeping_emphasis("==高亮==文本");
        assert_eq!(lines, ["==高亮==文本"]);
        assert_eq!(
            bold_segments(&lines[0]),
            vec![("高亮文本".to_string(), false)]
        );
    }

    /// 直接对原始行用 `bold_segments` 才能保住加粗信息
    #[test]
    fn bold_segments_keeps_emphasis_on_raw_line() {
        let raw = "① 求**矩形**面积。";
        assert_eq!(
            bold_segments(raw),
            vec![
                ("① 求".to_string(), false),
                ("矩形".to_string(), true),
                ("面积。".to_string(), false),
            ]
        );
    }

    // ------------------------------------------------------------ 多字节安全

    #[test]
    fn multibyte_text_is_not_corrupted() {
        let lines = plain_lines("- 钢筋混凝土**梁**正截面");
        assert_eq!(lines, ["钢筋混凝土梁正截面"]);
        assert_eq!(
            bold_segments("混凝土**梁**"),
            vec![("混凝土".to_string(), false), ("梁".to_string(), true)]
        );
    }
}
