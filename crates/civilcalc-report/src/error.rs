//! 报告生成（Word 计算书）错误。
//!
//! 源：`core/report/`（源项目**没有**独立的报告异常类型，
//! `DocxGenerator` 直接抛 `IOException` / `IllegalStateException`）。
//!
//! ## 为什么 PC 端要单独建类型
//!
//! 源项目用 POI（Apache POI 5.3.0），异常类型来自第三方库，业务层只能靠
//! 消息字符串区分。PC 端改用 `docx-rs`，且 `CommandError` 需要**编译期穷尽匹配**
//! （`docs/05` §3.2.3），因此把失败面显式建模：
//!
//! | 变体 | 场景 |
//! |---|---|
//! | [`ReportError::Docx`] | `docx-rs` 打包/写出失败（内部结构问题） |
//! | [`ReportError::Io`] | 落盘失败（磁盘满、权限、路径被占用） |
//! | [`ReportError::Template`] | 模板/导出选项不合法（如章节为空、封面字段缺失） |
//! | [`ReportError::Unsupported`] | 请求了当前不支持的形态（如图片格式、单位换算） |
//!
//! ## 文案分离
//!
//! [`ReportError::user_message`] 是**唯一**可进 UI 的文案；`detail` 只进日志。

use serde::{Deserialize, Serialize};

/// 报告生成错误。
///
/// ⚠️ 用**邻接标记**（`tag = "kind", content = "detail"`）而非内部标记：
/// serde 的内部标记**不支持** newtype 变体装 `String`
/// （会报 `cannot serialize tagged newtype variant ... containing a string`）。
/// 邻接标记既保留统一的 `kind` 判别字段，又让构造保持 `ReportError::Docx("...".into())` 的简洁写法。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "detail",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ReportError {
    /// `docx-rs` 生成/写出失败
    Docx(String),

    /// 落盘失败
    Io(String),

    /// 模板或导出选项不合法
    Template(String),

    /// 当前不支持的形态
    Unsupported(String),
}

impl ReportError {
    /// 错误码字符串
    pub fn code(&self) -> &'static str {
        match self {
            ReportError::Docx(_) => "EXPORT_ERROR",
            ReportError::Io(_) => "STORAGE_ERROR",
            ReportError::Template(_) => "VALIDATION_ERROR",
            ReportError::Unsupported(_) => "EXPORT_ERROR",
        }
    }

    /// **唯一**可进 UI 的文案。
    ///
    /// 注意：源项目报告失败的提示是笼统的（如"导出失败，请重试"），
    /// 这里保持同样克制的粒度，不把内部细节透给用户。
    pub fn user_message(&self) -> String {
        match self {
            ReportError::Docx(_) => "生成计算书失败，请重试".to_string(),
            ReportError::Io(_) => "保存文件失败，请检查目标位置是否可写".to_string(),
            ReportError::Template(m) => format!("导出选项不合法：{m}"),
            ReportError::Unsupported(m) => format!("当前不支持：{m}"),
        }
    }

    /// 调试细节，供日志使用
    pub fn detail(&self) -> &str {
        match self {
            ReportError::Docx(m)
            | ReportError::Io(m)
            | ReportError::Template(m)
            | ReportError::Unsupported(m) => m,
        }
    }
}

impl std::fmt::Display for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.user_message())
    }
}

impl std::error::Error for ReportError {}

impl From<std::io::Error> for ReportError {
    fn from(e: std::io::Error) -> Self {
        ReportError::Io(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable() {
        assert_eq!(ReportError::Docx("x".into()).code(), "EXPORT_ERROR");
        assert_eq!(ReportError::Io("x".into()).code(), "STORAGE_ERROR");
        assert_eq!(ReportError::Template("x".into()).code(), "VALIDATION_ERROR");
        assert_eq!(ReportError::Unsupported("x".into()).code(), "EXPORT_ERROR");
    }

    /// 用户文案不含内部细节
    #[test]
    fn user_message_hides_internals() {
        let e = ReportError::Docx("zip writer panicked at offset 42".into());
        assert_eq!(e.user_message(), "生成计算书失败，请重试");
        assert!(!e.user_message().contains("42"));
        assert!(e.detail().contains("42"));
    }

    #[test]
    fn template_error_includes_reason() {
        let e = ReportError::Template("章节列表为空".into());
        assert_eq!(e.user_message(), "导出选项不合法：章节列表为空");
    }

    #[test]
    fn io_error_converts() {
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let e: ReportError = io.into();
        assert_eq!(e.code(), "STORAGE_ERROR");
        assert_eq!(e.detail(), "denied");
    }

    #[test]
    fn display_shows_user_message() {
        assert_eq!(
            format!("{}", ReportError::Io("boom".into())),
            "保存文件失败，请检查目标位置是否可写"
        );
    }

    #[test]
    fn serde_shape_is_adjacently_tagged() {
        let v = serde_json::to_value(ReportError::Unsupported("EMF 图片".into())).unwrap();
        assert_eq!(v["kind"], serde_json::json!("unsupported"));
        assert_eq!(v["detail"], serde_json::json!("EMF 图片"));
    }

    #[test]
    fn serde_roundtrip_all_variants() {
        for e in [
            ReportError::Docx("a".into()),
            ReportError::Io("b".into()),
            ReportError::Template("c".into()),
            ReportError::Unsupported("d".into()),
        ] {
            let s = serde_json::to_string(&e).unwrap();
            let back: ReportError = serde_json::from_str(&s).unwrap();
            assert_eq!(back, e, "往返失败: {s}");
        }
    }
}
