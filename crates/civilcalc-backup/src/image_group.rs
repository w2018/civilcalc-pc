//! 图片引用分组 + MIME 推断（纯函数）。
//!
//! 源：`data/backup/ImageRefGrouping.kt`（21 行）+ `BackupPackBuilder.mimeOf`
//!
//! 分组决定上传弹窗里这张图**属于哪个默认勾选档**。
//!
//! ## 引用键格式（来自源项目）
//!
//! | 来源 | 键 |
//! |---|---|
//! | 公式内联图 | `formula:<formulaId>` |
//! | 输入历史 | `hist:search\|refine\|modeltest:<entryId>` |
//!
//! ⚠️ 这是**跨端约定**：Android 写进去的 `refs` 进备份包，PC 端读出来要能分组。
//! 改前缀会让「从 Android 恢复的备份」图片分错档。

/// 图片的引用范围
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ImageRefGroup {
    /// 被公式引用（默认勾选）—— 不勾则公式里的内联图恢复后是空占位
    Formula,
    /// 被输入历史 / 对话引用
    History,
    /// 没有任何引用
    Unreferenced,
}

/// 公式引用键前缀
pub const REF_FORMULA: &str = "formula:";

/// 按引用键分组。
///
/// **一张图同时被公式与对话引用时按「公式」算** —— 它更需要被备份
/// （公式里的图丢了，公式就残了；对话里的图丢了只是少一张附图）。
#[must_use]
pub fn group_of(refs: &[String]) -> ImageRefGroup {
    if refs.iter().any(|r| r.starts_with(REF_FORMULA)) {
        return ImageRefGroup::Formula;
    }
    if refs.iter().any(|r| !r.trim().is_empty()) {
        return ImageRefGroup::History;
    }
    ImageRefGroup::Unreferenced
}

/// 按文件名后缀推断 MIME。
///
/// ⚠️ 默认 `image/jpeg`（源同）：未知后缀按 JPEG 处理。
/// 图片存储只接受 jpeg/png/webp/gif 四种（保存时统一转码），
/// 所以这里的兜底不会真的遇到「其实是 png 却标成 jpeg」的情况。
#[must_use]
pub fn mime_of(file_name: &str) -> &'static str {
    let ext = file_name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/jpeg",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn formula_ref_wins() {
        assert_eq!(group_of(&refs(&["formula:usr:1"])), ImageRefGroup::Formula);
        // 同时被公式与历史引用 → 仍算公式
        assert_eq!(
            group_of(&refs(&["hist:search:12", "formula:usr:1"])),
            ImageRefGroup::Formula
        );
    }

    #[test]
    fn history_ref() {
        assert_eq!(group_of(&refs(&["hist:search:12"])), ImageRefGroup::History);
        assert_eq!(group_of(&refs(&["hist:refine:7"])), ImageRefGroup::History);
        assert_eq!(
            group_of(&refs(&["hist:modeltest:3"])),
            ImageRefGroup::History
        );
    }

    #[test]
    fn unreferenced() {
        assert_eq!(group_of(&[]), ImageRefGroup::Unreferenced);
        // 空白引用键不算引用（源用 `isNotBlank`）
        assert_eq!(group_of(&refs(&["", "   "])), ImageRefGroup::Unreferenced);
    }

    /// 🔴 前缀是跨端约定：`formula:` 才认，别的写法不算公式引用
    #[test]
    fn only_exact_prefix_counts_as_formula() {
        for key in ["Formula:usr:1", " formula:usr:1", "formula", "formulas:1"] {
            assert_ne!(
                group_of(&refs(&[key])),
                ImageRefGroup::Formula,
                "{key} 不该算公式引用"
            );
        }
    }

    #[test]
    fn serde_is_screaming_snake() {
        assert_eq!(
            serde_json::to_string(&ImageRefGroup::Formula).unwrap(),
            "\"FORMULA\""
        );
        assert_eq!(
            serde_json::to_string(&ImageRefGroup::Unreferenced).unwrap(),
            "\"UNREFERENCED\""
        );
    }

    #[test]
    fn mime_of_known_extensions() {
        assert_eq!(mime_of("a.png"), "image/png");
        assert_eq!(mime_of("a.webp"), "image/webp");
        assert_eq!(mime_of("a.gif"), "image/gif");
        assert_eq!(mime_of("a.jpg"), "image/jpeg");
        assert_eq!(mime_of("a.jpeg"), "image/jpeg");
    }

    #[test]
    fn mime_of_is_case_insensitive() {
        assert_eq!(mime_of("A.PNG"), "image/png");
        assert_eq!(mime_of("A.WebP"), "image/webp");
    }

    /// 未知后缀 / 无后缀 / 多点文件名
    #[test]
    fn mime_of_fallbacks() {
        assert_eq!(mime_of("a.bmp"), "image/jpeg", "未知后缀兜底 JPEG");
        assert_eq!(mime_of("noext"), "image/jpeg");
        assert_eq!(mime_of(""), "image/jpeg");
        // 取**最后**一个点之后（`a.b.png` → png）
        assert_eq!(mime_of("a.b.png"), "image/png");
        // 哈希文件名（图片存储的实际形态）
        assert_eq!(mime_of("3f2a1b.jpg"), "image/jpeg");
    }
}
