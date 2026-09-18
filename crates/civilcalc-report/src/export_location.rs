//! 计算书落盘目标与结果。
//!
//! 源：`core/report/ExportLocation.kt`（21 行）
//! 契约：`docs/04-数据契约.md` §5.3
//!
//! ## 与源项目的差异：SAF → 原生对话框
//!
//! 源项目跑在 Android，落盘必须走 SAF（Storage Access Framework）：
//! `SafTree(uri, displayName)` / `SelectedDocument(uri)` 里的 `uri` 是
//! `content://…` 形式的授权句柄。**PC 端没有 SAF** —— 直接用原生
//! 文件/目录对话框拿到**真实路径**（`dialog` 插件），所以字段名从
//! `uri` 改成 `path`，`SafTree` 变 [`DocumentOutputTarget::ChosenDirectory`]。
//!
//! | 源变体 | PC 变体 | 说明 |
//! |---|---|---|
//! | `PrivateCache` | [`DocumentOutputTarget::PrivateCache`] | `app_cache_dir/reports`（预览用，可清理） |
//! | `ScopedDownloads` | [`DocumentOutputTarget::ScopedDownloads`] | 系统「下载」目录 |
//! | `SafTree(uri, name)` | [`DocumentOutputTarget::ChosenDirectory`] | 原生**目录**选择对话框 |
//! | `SelectedDocument(uri)` | [`DocumentOutputTarget::ChosenFile`] | 原生**保存**对话框 |
//!
//! ## 三档落盘与「同名不覆盖」
//!
//! 无论哪个目标，落盘都必须：
//! 1. **先写临时文件 → `rename`（原子）** —— 中途失败不会留下半截文件
//! 2. **失败即删草稿**
//! 3. **同名不覆盖**（自动加序号 `报告(1).docx`）
//!
//! 这三条由 `src-tauri` 的 `export.rs`（路径校验 + 同名加序号）与
//! `commands/report.rs`（原子写）保证，本模块只描述「写到哪」。

use serde::{Deserialize, Serialize};

/// 计算书落盘目标。
///
/// `#[serde(tag = "kind")]` 内部标记 + camelCase 变体名，
/// 与项目其余 IPC 类型一致（前端按 `kind` 穷尽处理）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DocumentOutputTarget {
    /// 应用私有缓存目录（`app_cache_dir/reports`）。
    ///
    /// 用于**导出前的 HTML 预览**与「导出到临时位置再打开」——
    /// 这个目录会被系统清理，不适合放用户要留存的成果。
    PrivateCache,

    /// 系统「下载」目录。
    ScopedDownloads,

    /// 用户在原生**目录**选择对话框里挑的目录（真实路径）
    ChosenDirectory { dir: String },

    /// 用户在原生**保存**对话框里挑的完整文件路径
    ChosenFile { path: String },
}

impl DocumentOutputTarget {
    /// 目标是否由用户显式指定（决定是否需要在落盘前做越界校验）。
    ///
    /// [`DocumentOutputTarget::PrivateCache`] 与
    /// [`DocumentOutputTarget::ScopedDownloads`] 是应用自己算出来的位置，
    /// 不需要校验；另两个来自用户输入，**必须**过
    /// `export::safe_output_path`（防 `..` 越界）。
    pub fn is_user_chosen(&self) -> bool {
        matches!(
            self,
            DocumentOutputTarget::ChosenDirectory { .. } | DocumentOutputTarget::ChosenFile { .. }
        )
    }

    /// 用户指定的路径（应用自有目标返回 `None`）
    pub fn chosen_path(&self) -> Option<&str> {
        match self {
            DocumentOutputTarget::ChosenDirectory { dir } => Some(dir.as_str()),
            DocumentOutputTarget::ChosenFile { path } => Some(path.as_str()),
            _ => None,
        }
    }

    /// 排障用短名
    pub fn kind_name(&self) -> &'static str {
        match self {
            DocumentOutputTarget::PrivateCache => "privateCache",
            DocumentOutputTarget::ScopedDownloads => "scopedDownloads",
            DocumentOutputTarget::ChosenDirectory { .. } => "chosenDirectory",
            DocumentOutputTarget::ChosenFile { .. } => "chosenFile",
        }
    }
}

/// 一次成功导出的结果。
///
/// ⚠️ `path` 是**最终实际写入的路径** —— 若发生同名冲突，它会带序号
/// （`报告(1).docx`）。前端「打开文件」必须用它，**不要**用用户原本选的路径，
/// 否则会打开一个不存在的文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentExportResult {
    /// 最终落盘路径（源项目为 `uri`，PC 端为真实路径）
    pub path: String,
    /// 展示用文件名（含扩展名）
    pub display_name: String,
    /// 文件字节数
    pub size_bytes: i64,
}

impl DocumentExportResult {
    /// 由最终路径与字节数构造（`display_name` 取路径末段）
    pub fn new(path: impl Into<String>, size_bytes: i64) -> Self {
        let path = path.into();
        let display_name = path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(path.as_str())
            .to_string();
        Self {
            path,
            display_name,
            size_bytes,
        }
    }

    /// 人类可读体积（前端「已导出 12.3 KB」用）
    pub fn size_display(&self) -> String {
        human_size(self.size_bytes)
    }
}

/// 字节数 → 人类可读（`B` / `KB` / `MB`）。
///
/// 用 1024 进制（与资源管理器一致）；`B` 不带小数。
pub fn human_size(bytes: i64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;

    let b = bytes.max(0) as f64;
    if b < KB {
        format!("{} B", bytes.max(0))
    } else if b < MB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{:.1} MB", b / MB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_owned_targets_are_not_user_chosen() {
        assert!(!DocumentOutputTarget::PrivateCache.is_user_chosen());
        assert!(!DocumentOutputTarget::ScopedDownloads.is_user_chosen());
        assert!(DocumentOutputTarget::ChosenDirectory {
            dir: "D:/out".into()
        }
        .is_user_chosen());
        assert!(DocumentOutputTarget::ChosenFile {
            path: "D:/out/a.docx".into()
        }
        .is_user_chosen());
    }

    /// 只有用户指定的目标才有路径 —— 决定是否需要越界校验
    #[test]
    fn chosen_path_only_for_user_targets() {
        assert_eq!(DocumentOutputTarget::PrivateCache.chosen_path(), None);
        assert_eq!(DocumentOutputTarget::ScopedDownloads.chosen_path(), None);
        assert_eq!(
            DocumentOutputTarget::ChosenDirectory {
                dir: "D:/out".into()
            }
            .chosen_path(),
            Some("D:/out")
        );
        assert_eq!(
            DocumentOutputTarget::ChosenFile {
                path: "D:/out/a.docx".into()
            }
            .chosen_path(),
            Some("D:/out/a.docx")
        );
    }

    #[test]
    fn kind_names_are_camel_case() {
        assert_eq!(DocumentOutputTarget::PrivateCache.kind_name(), "privateCache");
        assert_eq!(
            DocumentOutputTarget::ChosenFile {
                path: "x".into()
            }
            .kind_name(),
            "chosenFile"
        );
    }

    #[test]
    fn target_serde_shape() {
        let v = serde_json::to_value(DocumentOutputTarget::PrivateCache).unwrap();
        assert_eq!(v["kind"], serde_json::json!("privateCache"));

        let v = serde_json::to_value(DocumentOutputTarget::ChosenDirectory {
            dir: "D:/out".into(),
        })
        .unwrap();
        assert_eq!(v["kind"], serde_json::json!("chosenDirectory"));
        assert_eq!(v["dir"], serde_json::json!("D:/out"));

        let v = serde_json::to_value(DocumentOutputTarget::ChosenFile {
            path: "D:/a.docx".into(),
        })
        .unwrap();
        assert_eq!(v["kind"], serde_json::json!("chosenFile"));
        assert_eq!(v["path"], serde_json::json!("D:/a.docx"));
    }

    #[test]
    fn target_roundtrip_all_variants() {
        for t in [
            DocumentOutputTarget::PrivateCache,
            DocumentOutputTarget::ScopedDownloads,
            DocumentOutputTarget::ChosenDirectory { dir: "D:/o".into() },
            DocumentOutputTarget::ChosenFile { path: "D:/o/a.docx".into() },
        ] {
            let s = serde_json::to_string(&t).unwrap();
            let back: DocumentOutputTarget = serde_json::from_str(&s).unwrap();
            assert_eq!(back, t, "往返失败: {s}");
        }
    }

    // ------------------------------------------------------------ 导出结果

    #[test]
    fn display_name_taken_from_path_tail() {
        // Windows 反斜杠
        let r = DocumentExportResult::new(r"D:\out\报告.docx", 1024);
        assert_eq!(r.display_name, "报告.docx");
        // POSIX 正斜杠
        let r = DocumentExportResult::new("/home/u/报告.docx", 1024);
        assert_eq!(r.display_name, "报告.docx");
        // 无分隔符（只有文件名）
        let r = DocumentExportResult::new("报告.docx", 1024);
        assert_eq!(r.display_name, "报告.docx");
    }

    /// 同名冲突后的带序号路径要原样成为 `path`（前端据此打开文件）
    #[test]
    fn numbered_path_is_kept_as_is() {
        let r = DocumentExportResult::new(r"D:\out\报告(1).docx", 2048);
        assert_eq!(r.path, r"D:\out\报告(1).docx");
        assert_eq!(r.display_name, "报告(1).docx");
    }

    #[test]
    fn result_serde_is_camel_case() {
        let r = DocumentExportResult::new("a.docx", 10);
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("displayName").is_some());
        assert!(v.get("sizeBytes").is_some());
        assert!(v.get("display_name").is_none(), "不得泄漏 snake_case");
    }

    // ------------------------------------------------------------ 体积展示

    #[test]
    fn human_size_boundaries() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        // 1023 B 仍是 B（1024 进制）
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(1536), "1.5 KB");
        // 1024*1024 - 1 仍是 KB
        assert_eq!(human_size(1024 * 1024 - 1), "1024.0 KB");
        assert_eq!(human_size(1024 * 1024), "1.0 MB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
    }

    /// 负数不该 panic（防御性；正常情况下不会出现）
    #[test]
    fn human_size_clamps_negative() {
        assert_eq!(human_size(-5), "0 B");
    }

    #[test]
    fn size_display_uses_human_size() {
        assert_eq!(DocumentExportResult::new("a.docx", 2048).size_display(), "2.0 KB");
    }
}
