//! 备份与 WebDAV 错误。
//!
//! 源：
//! - `core/backup/BackupCrypto.kt`（3 个固定文案的异常）
//! - `core/backup/WebDavException.kt`（`code: Int?` + `userMessage: String`）
//! - `core/backup/WebDavClient.kt` / `WebDavRepository.kt`（各分支中文文案）
//!
//! ## 为什么单独一个类型而不是靠字符串判断
//!
//! 源项目注释原话：
//!
//! > 单独一个异常类型而不是靠字符串判断：UI 层据此原样提示，测试也能断言到状态码。
//!
//! Rust 侧同理 —— [`BackupError::WebDav`] 带 `status`，UI 可原样提示，
//! 测试可断言状态码，而不是去 `contains("404")`。
//!
//! ## 文案分离
//!
//! [`BackupError::user_message`] 是**唯一**可进 UI 的文案（逐字对齐源项目）；
//! 其余字段（`status`、`detail`）只进日志。

use serde::{Deserialize, Serialize};

/// 备份 / WebDAV 错误。
///
/// ⚠️ 用**邻接标记**（`tag = "kind", content = "detail"`）而非内部标记：
/// serde 的内部标记**不支持** newtype 变体装 `String`。邻接标记既保留统一的
/// `kind` 判别字段，又让构造保持 `BackupError::Format("...".into())` 的简洁写法。
///
/// 序列化形状：
/// - 单元变体：`{"kind":"needPassword"}`
/// - newtype 变体：`{"kind":"format","detail":"..."}`
/// - 结构变体：`{"kind":"webDav","detail":{"status":507,"userMessage":"..."}}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "detail",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BackupError {
    /// 这是加密备份，需要密码（源项目 `BackupNeedPasswordException`）
    NeedPassword,

    /// 密码错误或包已损坏（源项目 `BackupWrongPasswordException`）
    WrongPassword,

    /// 备份包不完整（源项目 `BackupTruncatedException`）
    Truncated,

    /// WebDAV 操作失败。`status = None` 表示网络层/本地错误
    /// （源项目 `WebDavException(code: Int?, userMessage: String)`）
    WebDav {
        status: Option<u16>,
        user_message: String,
    },

    /// 备份包结构/清单不合法（缺 manifest、条目异常等）
    Format(String),

    /// 加解密失败（非密码类，如分块校验不通过）
    Crypto(String),

    /// 本地文件读写失败
    Io(String),

    /// 备份格式版本不受支持
    Unsupported(String),

    /// 用户主动取消（上传/下载中途点了「取消」）
    ///
    /// 单独一个变体而不是复用 `WebDav`：UI 要**静默收尾**（不弹错误框）——
    /// 用户自己点的取消，再报一次错是噪音。前端按 `code == "BACKUP_CANCELLED"` 判别。
    Cancelled,
}

impl BackupError {
    /// 错误码字符串（写入日志与 IPC 的错误码字段）
    pub fn code(&self) -> &'static str {
        match self {
            BackupError::NeedPassword => "BACKUP_NEED_PASSWORD",
            BackupError::WrongPassword => "BACKUP_WRONG_PASSWORD",
            BackupError::Truncated => "BACKUP_TRUNCATED",
            BackupError::WebDav { .. } => "WEBDAV_ERROR",
            BackupError::Format(_) => "BACKUP_FORMAT_ERROR",
            BackupError::Crypto(_) => "BACKUP_CRYPTO_ERROR",
            BackupError::Io(_) => "STORAGE_ERROR",
            BackupError::Unsupported(_) => "BACKUP_UNSUPPORTED",
            BackupError::Cancelled => "BACKUP_CANCELLED",
        }
    }

    /// **唯一**可进 UI 的文案。前三个变体逐字对齐源项目异常消息。
    pub fn user_message(&self) -> String {
        match self {
            BackupError::NeedPassword => "这是加密备份，请输入密码".to_string(),
            BackupError::WrongPassword => "密码不对，或备份包已损坏".to_string(),
            BackupError::Truncated => "备份包不完整，可能没有下载完".to_string(),
            BackupError::WebDav { user_message, .. } => user_message.clone(),
            BackupError::Format(m) => format!("备份包格式不正确：{m}"),
            BackupError::Crypto(m) => format!("备份包解密失败：{m}"),
            BackupError::Io(m) => format!("文件读写失败：{m}"),
            BackupError::Unsupported(m) => format!("不支持的备份包：{m}"),
            BackupError::Cancelled => "已取消".to_string(),
        }
    }

    /// HTTP 状态码（仅 `WebDav` 有）
    pub fn http_status(&self) -> Option<u16> {
        match self {
            BackupError::WebDav { status, .. } => *status,
            _ => None,
        }
    }

    /// 调试细节（`user_message` 之外的补充信息），供日志使用
    pub fn detail(&self) -> String {
        match self {
            BackupError::Format(m) | BackupError::Crypto(m) | BackupError::Io(m) => m.clone(),
            BackupError::Unsupported(m) => m.clone(),
            BackupError::WebDav { status, .. } => match status {
                Some(s) => format!("WebDAV HTTP {s}"),
                None => "WebDAV 网络/本地错误".to_string(),
            },
            _ => String::new(),
        }
    }
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.user_message())
    }
}

impl std::error::Error for BackupError {}

impl From<std::io::Error> for BackupError {
    fn from(e: std::io::Error) -> Self {
        BackupError::Io(e.to_string())
    }
}

/// HTTP 状态码 → 中文文案（对齐源 `WebDavClient.statusError` 的全部分支）。
///
/// `action` 是动作词（`"上传"` / `"下载"` / `"删除"` / `"创建目录"` / `"读取目录"` …），
/// 为空串时自动省略带动作词的括号 —— 这样 [`webdav_status_message`] 可以直接复用它，
/// **文案只有一份**，不会漂移。
///
/// `body` 是响应体（仅 `else` 分支用）：取前 120 个**字符**（不是字节）作为细节，
/// 帮用户看到服务端到底说了什么。
pub fn status_error(status: u16, body: Option<&str>, action: &str) -> BackupError {
    let msg = match status {
        401 => "账号或应用密码不正确（坚果云要填「应用密码」，不是登录密码）".to_string(),
        403 => "没有权限访问该目录（请检查账号权限或目录路径）".to_string(),
        404 => with_action("远端路径不存在", action),
        405 => with_action("服务器不允许该操作", action),
        409 => {
            if action.is_empty() {
                "远端目录不存在".to_string()
            } else {
                format!("远端目录不存在，无法完成{action}")
            }
        }
        423 => "远端文件被占用或锁定".to_string(),
        507 => {
            if action.is_empty() {
                "云端空间不足".to_string()
            } else {
                format!("云端空间不足，无法{action}")
            }
        }
        s if (500..600).contains(&s) => format!("远端服务器错误（HTTP {s}）"),
        s => match body_summary(body) {
            Some(detail) => format!("远端返回 HTTP {s}：{detail}"),
            None => format!("远端返回 HTTP {s}"),
        },
    };
    BackupError::WebDav {
        status: Some(status),
        user_message: msg,
    }
}

/// `base（动作）`；动作为空时只留 `base`
fn with_action(base: &str, action: &str) -> String {
    if action.is_empty() {
        base.to_string()
    } else {
        format!("{base}（{action}）")
    }
}

/// 响应体摘要：trim → 取前 120 **字符** → 空则 `None`
///
/// ⚠️ 按**字符**而非字节截断（源 `take(120)` 也是按字符）——
/// 按字节切会把多字节汉字劈开，产生乱码。
fn body_summary(body: Option<&str>) -> Option<String> {
    let trimmed = body?.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(120).collect())
}

/// 常见 WebDAV 状态码 → 中文文案（**无动作词**的通用版本）。
///
/// 内部复用 [`status_error`]，所以两者文案永远一致。
pub fn webdav_status_message(status: u16) -> String {
    status_error(status, None, "").user_message()
}

/// 由状态码构造 WebDAV 错误（无动作词、无响应体）
pub fn webdav_error(status: u16) -> BackupError {
    status_error(status, None, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 前三个文案逐字对齐源项目异常消息
    #[test]
    fn fixed_messages_are_verbatim() {
        assert_eq!(
            BackupError::NeedPassword.user_message(),
            "这是加密备份，请输入密码"
        );
        assert_eq!(
            BackupError::WrongPassword.user_message(),
            "密码不对，或备份包已损坏"
        );
        assert_eq!(
            BackupError::Truncated.user_message(),
            "备份包不完整，可能没有下载完"
        );
    }

    #[test]
    fn codes_are_stable() {
        assert_eq!(BackupError::NeedPassword.code(), "BACKUP_NEED_PASSWORD");
        assert_eq!(BackupError::WrongPassword.code(), "BACKUP_WRONG_PASSWORD");
        assert_eq!(BackupError::Truncated.code(), "BACKUP_TRUNCATED");
        assert_eq!(webdav_error(404).code(), "WEBDAV_ERROR");
        assert_eq!(BackupError::Format("x".into()).code(), "BACKUP_FORMAT_ERROR");
        assert_eq!(BackupError::Crypto("x".into()).code(), "BACKUP_CRYPTO_ERROR");
        assert_eq!(BackupError::Io("x".into()).code(), "STORAGE_ERROR");
        assert_eq!(
            BackupError::Unsupported("x".into()).code(),
            "BACKUP_UNSUPPORTED"
        );
    }

    #[test]
    fn webdav_carries_status_for_ui_and_tests() {
        let e = webdav_error(423);
        assert_eq!(e.http_status(), Some(423));
        assert_eq!(e.user_message(), "远端文件被占用或锁定");
        // 非 WebDAV 变体没有状态码
        assert_eq!(BackupError::Truncated.http_status(), None);
    }

    #[test]
    fn webdav_status_messages_match_source_branches() {
        assert_eq!(
            webdav_status_message(401),
            "账号或应用密码不正确（坚果云要填「应用密码」，不是登录密码）"
        );
        // ⚠️ 403 与 401 **不是**同一句 —— 旧版把两者合并成「应用密码不正确」是错的，
        //    源项目 403 讲的是目录权限。这里逐字钉住。
        assert_eq!(
            webdav_status_message(403),
            "没有权限访问该目录（请检查账号权限或目录路径）"
        );
        // ⚠️ 404 是「远端**路径**不存在」，409 才是「远端**目录**不存在」——
        //    旧版把 404 写成「远端目录不存在」，与源不符。
        assert_eq!(webdav_status_message(404), "远端路径不存在");
        assert_eq!(webdav_status_message(405), "服务器不允许该操作");
        assert_eq!(webdav_status_message(423), "远端文件被占用或锁定");
        assert_eq!(webdav_status_message(507), "云端空间不足");
        assert_eq!(webdav_status_message(500), "远端服务器错误（HTTP 500）");
        assert_eq!(webdav_status_message(599), "远端服务器错误（HTTP 599）");
        assert_eq!(webdav_status_message(418), "远端返回 HTTP 418");
    }

    #[test]
    fn io_error_converts() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");
        let e: BackupError = io.into();
        assert_eq!(e.code(), "STORAGE_ERROR");
        assert!(e.user_message().contains("文件读写失败"));
        assert_eq!(e.detail(), "no such file");
    }

    #[test]
    fn display_shows_user_message() {
        assert_eq!(
            format!("{}", BackupError::WrongPassword),
            "密码不对，或备份包已损坏"
        );
    }

    #[test]
    fn serde_shape_is_adjacently_tagged() {
        // 单元变体：只有 kind
        let v = serde_json::to_value(BackupError::NeedPassword).unwrap();
        assert_eq!(v["kind"], serde_json::json!("needPassword"));
        assert!(v.get("detail").is_none());

        // newtype 变体：kind + detail
        let v = serde_json::to_value(BackupError::Format("缺 manifest".into())).unwrap();
        assert_eq!(v["kind"], serde_json::json!("format"));
        assert_eq!(v["detail"], serde_json::json!("缺 manifest"));

        // 结构变体：kind + detail（内层是结构）
        let v = serde_json::to_value(webdav_error(507)).unwrap();
        assert_eq!(v["kind"], serde_json::json!("webDav"));
        assert_eq!(v["detail"]["status"], serde_json::json!(507));
        assert_eq!(v["detail"]["userMessage"], serde_json::json!("云端空间不足"));
    }

    #[test]
    fn serde_roundtrip_all_variants() {
        for e in [
            BackupError::NeedPassword,
            BackupError::WrongPassword,
            BackupError::Truncated,
            webdav_error(423),
            BackupError::WebDav {
                status: None,
                user_message: "网络不通".into(),
            },
            BackupError::Format("x".into()),
            BackupError::Crypto("y".into()),
            BackupError::Io("z".into()),
            BackupError::Unsupported("v9".into()),
            BackupError::Cancelled,
        ] {
            let s = serde_json::to_string(&e).unwrap();
            let back: BackupError = serde_json::from_str(&s).unwrap();
            assert_eq!(back, e, "往返失败: {s}");
        }
    }

    /// 🔴 动作词插值（源 `statusError(code, body, action)` 的完整形态）
    #[test]
    fn status_error_interpolates_action() {
        assert_eq!(
            status_error(404, None, "上传").user_message(),
            "远端路径不存在（上传）"
        );
        assert_eq!(
            status_error(405, None, "创建目录").user_message(),
            "服务器不允许该操作（创建目录）"
        );
        assert_eq!(
            status_error(409, None, "上传").user_message(),
            "远端目录不存在，无法完成上传"
        );
        assert_eq!(
            status_error(507, None, "上传").user_message(),
            "云端空间不足，无法上传"
        );
        // 无动作词时括号/尾句自动省略
        assert_eq!(status_error(404, None, "").user_message(), "远端路径不存在");
        assert_eq!(status_error(409, None, "").user_message(), "远端目录不存在");
        assert_eq!(status_error(507, None, "").user_message(), "云端空间不足");
    }

    /// 文案与动作词无关的分支（401/403/423/5xx）
    #[test]
    fn status_error_action_independent_branches() {
        for action in ["", "上传", "下载"] {
            assert_eq!(
                status_error(401, None, action).user_message(),
                "账号或应用密码不正确（坚果云要填「应用密码」，不是登录密码）"
            );
            assert_eq!(
                status_error(403, None, action).user_message(),
                "没有权限访问该目录（请检查账号权限或目录路径）"
            );
            assert_eq!(
                status_error(423, None, action).user_message(),
                "远端文件被占用或锁定"
            );
            assert_eq!(
                status_error(500, None, action).user_message(),
                "远端服务器错误（HTTP 500）"
            );
        }
    }

    /// 未知状态码：响应体摘要进文案；空/空白响应体则不带摘要
    #[test]
    fn status_error_uses_body_summary() {
        assert_eq!(
            status_error(418, None, "上传").user_message(),
            "远端返回 HTTP 418"
        );
        assert_eq!(
            status_error(418, Some("   "), "上传").user_message(),
            "远端返回 HTTP 418",
            "全空白等于没有"
        );
        assert_eq!(
            status_error(418, Some("  teapot  "), "上传").user_message(),
            "远端返回 HTTP 418：teapot",
            "摘要要 trim"
        );
    }

    /// 🔴 摘要按**字符**截断到 120，不按字节（否则汉字会被劈成乱码）
    #[test]
    fn status_error_body_summary_is_char_based() {
        let long = "汉".repeat(200);
        let msg = status_error(418, Some(&long), "").user_message();
        let detail = msg.strip_prefix("远端返回 HTTP 418：").unwrap();
        assert_eq!(detail.chars().count(), 120);
        assert_eq!(detail, "汉".repeat(120), "不能出现半个字符的乱码");
    }

    /// 摘要恰好 120 字符时不截断
    #[test]
    fn status_error_body_summary_boundary() {
        let exact = "x".repeat(120);
        let msg = status_error(418, Some(&exact), "").user_message();
        assert!(msg.ends_with(&exact));
        let one_more = "x".repeat(121);
        let msg = status_error(418, Some(&one_more), "").user_message();
        assert_eq!(msg, format!("远端返回 HTTP 418：{}", "x".repeat(120)));
    }

    /// `webdav_status_message` 与 `status_error(.., "")` 必须逐字一致（防两套文案漂移）
    #[test]
    fn status_message_delegates_to_status_error() {
        for code in [400u16, 401, 403, 404, 405, 409, 418, 423, 500, 507, 599] {
            assert_eq!(
                webdav_status_message(code),
                status_error(code, None, "").user_message(),
                "HTTP {code} 文案不一致"
            );
        }
    }

    /// 取消是独立变体，文案与错误码固定（前端据此静默收尾）
    #[test]
    fn cancelled_is_its_own_variant() {
        assert_eq!(BackupError::Cancelled.code(), "BACKUP_CANCELLED");
        assert_eq!(BackupError::Cancelled.user_message(), "已取消");
        assert_eq!(BackupError::Cancelled.detail(), "");
        assert_eq!(BackupError::Cancelled.http_status(), None);
    }

    #[test]
    fn detail_is_empty_for_password_errors() {
        // 密码类错误的 user_message 已足够，detail 不额外暴露信息
        assert_eq!(BackupError::NeedPassword.detail(), "");
        assert_eq!(BackupError::WrongPassword.detail(), "");
    }
}
