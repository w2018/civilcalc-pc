//! 命令层错误（对齐源项目错误码）。
//!
//! ## 设计
//!
//! - `#[serde(tag = "kind")]` 产生**可判别联合**，前端 TS 用同名联合类型，
//!   编译期即可穷尽处理（见 `docs/05-项目开发方案.md` §3.2.3）
//! - `From<CoreError>` / `From<LlmError>` / `From<BackupError>` 由**编译器强制穷尽匹配**
//! - **文案分离铁律**：用户可见文案与调试信息严格分开；调试细节只进日志
//!   （源项目 `LlmError.userMessage()` 的做法）

use serde::Serialize;

/// 命令错误（13 个 kind）
///
/// ⚠️ `rename_all_fields` 不可省：`#[serde(rename_all)]` 只重命名**变体名**，
/// 不重命名结构变体的**字段**。少了它 `ai_code` 会序列化成 `"ai_code"`，
/// 而前端契约（`docs/04` §7）要求 `aiCode`。
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum CommandError {
    Validation { message: String },
    NotFound { message: String },
    Unauthorized { message: String },
    Network { message: String },
    Storage { message: String },
    AiError { message: String, ai_code: String },
    Export { message: String },
    Parse { message: String },
    BuiltinSource { message: String },
    PathTraversal { message: String },
    InvalidArgument { message: String },
    /// 用户主动取消（上传/下载中途点了「取消」）
    ///
    /// 🔴 **必须是独立 kind**：前端据此**静默收尾**（不弹错误框）——
    /// 用户自己点的取消，再报一次错是噪音。同时它也避免了「把取消当成成功」：
    /// 若取消时返回 `Ok(())`，只 await 而没监听 `backup://cancelled` 的前端
    /// 会显示「上传成功」——那是**假成功**，比多一个错误更糟。
    Cancelled { message: String },
    Unknown { message: String },
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::Validation { message }
            | CommandError::NotFound { message }
            | CommandError::Unauthorized { message }
            | CommandError::Network { message }
            | CommandError::Storage { message }
            | CommandError::Export { message }
            | CommandError::Parse { message }
            | CommandError::BuiltinSource { message }
            | CommandError::PathTraversal { message }
            | CommandError::InvalidArgument { message }
            | CommandError::Cancelled { message }
            | CommandError::Unknown { message } => write!(f, "{message}"),
            CommandError::AiError { message, .. } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for CommandError {}

/// 领域层错误 → 命令层错误（编译期穷尽匹配）
impl From<civilcalc_core::CoreError> for CommandError {
    fn from(e: civilcalc_core::CoreError) -> Self {
        use civilcalc_core::CoreError as C;
        match e {
            C::Validation { message: m } => CommandError::Validation { message: m },
            C::NotFound { message: m } => CommandError::NotFound { message: m },
            C::Unauthorized { message: m } => CommandError::Unauthorized { message: m },
            C::Network { message: m } => CommandError::Network { message: m },
            C::Storage { message: m } => CommandError::Storage { message: m },
            C::AiError { message, ai_code } => CommandError::AiError { message, ai_code },
            C::Export { message: m } => CommandError::Export { message: m },
            C::Parse { message: m } => CommandError::Parse { message: m },
            C::BuiltinSource { message: m } => CommandError::BuiltinSource { message: m },
            C::PathTraversal { message: m } => CommandError::PathTraversal { message: m },
            C::InvalidArgument { message: m } => CommandError::InvalidArgument { message: m },
            C::Unknown { message: m } => CommandError::Unknown { message: m },
        }
    }
}

/// LLM 层错误 → 命令层错误。
///
/// **文案分离铁律**（源项目 BUG-26）：只把 [`LlmError::user_message`] 送进 UI，
/// 并把错误码写入 `ai_code`；`error_detail` / `http_status` / `raw_response`
/// 只经日志输出。
impl From<civilcalc_llm::LlmError> for CommandError {
    fn from(e: civilcalc_llm::LlmError) -> Self {
        civilcalc_core::log::w(
            "CommandError",
            &format!("LLM 调用失败 [{}] {}", e.ai_code(), e.detail()),
            None,
        );
        CommandError::AiError {
            message: e.user_message().to_string(),
            ai_code: e.ai_code().to_string(),
        }
    }
}

/// 备份层错误 → 命令层错误。
///
/// 映射口径（按"用户下一步该做什么"划分，而非按错误来源）：
///
/// | 备份错误 | 命令错误 | 用户该做什么 |
/// |---|---|---|
/// | `NeedPassword` / `WrongPassword` | `Validation` | 在弹窗里输入/修正密码 |
/// | `Truncated` / `Format` / `Unsupported` | `Validation` | 换一个备份包 |
/// | `Crypto` / `Io` | `Storage` | 检查磁盘/权限 |
/// | `WebDav` | `Network` | 检查网络与云端账号 |
/// | `Cancelled` | `Cancelled` | **什么都不做**（用户自己点的取消） |
impl From<civilcalc_backup::BackupError> for CommandError {
    fn from(e: civilcalc_backup::BackupError) -> Self {
        use civilcalc_backup::BackupError as B;
        let message = e.user_message();

        // 调试细节只进日志
        let detail = e.detail();
        if !detail.is_empty() {
            civilcalc_core::log::w(
                "CommandError",
                &format!("备份操作失败 [{}] {}", e.code(), detail),
                None,
            );
        }

        match e {
            B::NeedPassword | B::WrongPassword | B::Truncated | B::Format(_) | B::Unsupported(_) => {
                CommandError::Validation { message }
            }
            B::Crypto(_) | B::Io(_) => CommandError::Storage { message },
            B::WebDav { .. } => CommandError::Network { message },
            B::Cancelled => CommandError::Cancelled { message },
        }
    }
}

/// 报告层错误 → 命令层错误。
///
/// | 报告错误 | 命令错误 |
/// |---|---|
/// | `Docx` / `Unsupported` | `Export` |
/// | `Io` | `Storage` |
/// | `Template` | `Validation` |
impl From<civilcalc_report::ReportError> for CommandError {
    fn from(e: civilcalc_report::ReportError) -> Self {
        use civilcalc_report::ReportError as R;
        let message = e.user_message();
        civilcalc_core::log::w(
            "CommandError",
            &format!("报告生成失败 [{}] {}", e.code(), e.detail()),
            None,
        );
        match e {
            R::Docx(_) | R::Unsupported(_) => CommandError::Export { message },
            R::Io(_) => CommandError::Storage { message },
            R::Template(_) => CommandError::Validation { message },
        }
    }
}

/// 命令层统一结果别名
pub type CmdResult<T> = std::result::Result<T, CommandError>;

impl CommandError {
    /// `kind` 的字符串形式（与 serde 输出的 `kind` 字段**逐字一致**）。
    ///
    /// 用途：日志与测试断言。前端拿到的仍是 serde 的 `kind` 字段，
    /// 这个方法只是让 Rust 侧也能方便地按 kind 比较。
    pub fn kind_str(&self) -> &'static str {
        match self {
            CommandError::Validation { .. } => "validation",
            CommandError::NotFound { .. } => "notFound",
            CommandError::Unauthorized { .. } => "unauthorized",
            CommandError::Network { .. } => "network",
            CommandError::Storage { .. } => "storage",
            CommandError::AiError { .. } => "aiError",
            CommandError::Export { .. } => "export",
            CommandError::Parse { .. } => "parse",
            CommandError::BuiltinSource { .. } => "builtinSource",
            CommandError::PathTraversal { .. } => "pathTraversal",
            CommandError::InvalidArgument { .. } => "invalidArgument",
            CommandError::Cancelled { .. } => "cancelled",
            CommandError::Unknown { .. } => "unknown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------- CoreError 映射（12 个 kind 全覆盖） ----------------

    #[test]
    fn core_error_maps_all_twelve_kinds() {
        use civilcalc_core::CoreError as C;
        let cases: Vec<(C, &str)> = vec![
            (C::Validation { message: "a".into() }, "validation"),
            (C::NotFound { message: "a".into() }, "notFound"),
            (C::Unauthorized { message: "a".into() }, "unauthorized"),
            (C::Network { message: "a".into() }, "network"),
            (C::Storage { message: "a".into() }, "storage"),
            (
                C::AiError {
                    message: "a".into(),
                    ai_code: "X".into(),
                },
                "aiError",
            ),
            (C::Export { message: "a".into() }, "export"),
            (C::Parse { message: "a".into() }, "parse"),
            (C::BuiltinSource { message: "a".into() }, "builtinSource"),
            (C::PathTraversal { message: "a".into() }, "pathTraversal"),
            (C::InvalidArgument { message: "a".into() }, "invalidArgument"),
            (C::Unknown { message: "a".into() }, "unknown"),
        ];
        for (e, kind) in cases {
            let cmd: CommandError = e.into();
            let v = serde_json::to_value(&cmd).unwrap();
            assert_eq!(v["kind"], serde_json::json!(kind));
        }
    }

    // ---------------- LlmError 映射 ----------------

    #[test]
    fn llm_error_maps_to_ai_error_with_code() {
        use civilcalc_llm::error::map_http_error;
        let e = map_http_error(429, Some("quota body".into()));
        let cmd: CommandError = e.into();
        let v = serde_json::to_value(&cmd).unwrap();

        assert_eq!(v["kind"], serde_json::json!("aiError"));
        assert_eq!(v["aiCode"], serde_json::json!("QUOTA_EXCEEDED"));
        // 用户文案，不含调试细节
        assert_eq!(
            v["message"],
            serde_json::json!("调用受限或额度不足，请稍后再试或检查账户余额")
        );
        assert!(!v["message"].as_str().unwrap().contains("quota body"));
    }

    #[test]
    fn llm_error_keeps_user_message_for_every_code() {
        use civilcalc_llm::error::LlmErrorCode;
        for code in [
            LlmErrorCode::AuthInvalid,
            LlmErrorCode::QuotaExceeded,
            LlmErrorCode::ModelInvalid,
            LlmErrorCode::Network,
            LlmErrorCode::SchemaParseError,
            LlmErrorCode::OutputTruncated,
            LlmErrorCode::ContentFiltered,
            LlmErrorCode::VisionUnsupported,
            LlmErrorCode::ProtocolUnsupported,
            LlmErrorCode::Unknown,
        ] {
            let cmd: CommandError = civilcalc_llm::error::LlmError::of(code).into();
            match cmd {
                CommandError::AiError { message, ai_code } => {
                    assert_eq!(message, code.user_message(), "文案须与源项目一致");
                    assert_eq!(ai_code, code.as_str());
                }
                other => panic!("应映射为 AiError，实际 {other:?}"),
            }
        }
    }

    // ---------------- BackupError 映射 ----------------

    #[test]
    fn backup_error_kind_mapping() {
        use civilcalc_backup::error::{webdav_error, BackupError as B};
        let cases: Vec<(B, &str)> = vec![
            (B::NeedPassword, "validation"),
            (B::WrongPassword, "validation"),
            (B::Truncated, "validation"),
            (B::Format("x".into()), "validation"),
            (B::Unsupported("x".into()), "validation"),
            (B::Crypto("x".into()), "storage"),
            (B::Io("x".into()), "storage"),
            (webdav_error(423), "network"),
        ];
        for (e, kind) in cases {
            let cmd: CommandError = e.into();
            let v = serde_json::to_value(&cmd).unwrap();
            assert_eq!(v["kind"], serde_json::json!(kind));
        }
    }

    #[test]
    fn backup_password_errors_keep_verbatim_message() {
        use civilcalc_backup::BackupError as B;
        for (e, want) in [
            (B::NeedPassword, "这是加密备份，请输入密码"),
            (B::WrongPassword, "密码不对，或备份包已损坏"),
            (B::Truncated, "备份包不完整，可能没有下载完"),
        ] {
            let cmd: CommandError = e.into();
            assert_eq!(cmd.to_string(), want);
        }
    }

    #[test]
    fn webdav_error_keeps_user_message() {
        use civilcalc_backup::error::webdav_error;
        let cmd: CommandError = webdav_error(423).into();
        assert_eq!(cmd.to_string(), "远端文件被占用或锁定");
    }

    // ---------------- ReportError 映射 ----------------

    #[test]
    fn report_error_kind_mapping() {
        use civilcalc_report::ReportError as R;
        let cases: Vec<(R, &str)> = vec![
            (R::Docx("x".into()), "export"),
            (R::Unsupported("x".into()), "export"),
            (R::Io("x".into()), "storage"),
            (R::Template("x".into()), "validation"),
        ];
        for (e, kind) in cases {
            let cmd: CommandError = e.into();
            let v = serde_json::to_value(&cmd).unwrap();
            assert_eq!(v["kind"], serde_json::json!(kind));
        }
    }

    #[test]
    fn report_docx_error_hides_internals() {
        use civilcalc_report::ReportError as R;
        let cmd: CommandError = R::Docx("zip writer offset 42".into()).into();
        assert_eq!(cmd.to_string(), "生成计算书失败，请重试");
        assert!(!cmd.to_string().contains("42"));
    }

    // ---------------- 可判别联合（前端依赖的契约） ----------------

    /// 每个 kind 都带 `message`（`AiError` 额外带 `aiCode`），前端可穷尽处理
    #[test]
    fn serialized_shape_is_discriminated_union() {
        let cmd: CommandError = civilcalc_core::CoreError::Validation { message: "校验失败".into() }.into();
        let v = serde_json::to_value(&cmd).unwrap();
        assert_eq!(v["kind"], serde_json::json!("validation"));
        assert_eq!(v["message"], serde_json::json!("校验失败"));

        let cmd: CommandError = civilcalc_llm::error::LlmError::of(
            civilcalc_llm::error::LlmErrorCode::Unknown,
        )
        .into();
        let v = serde_json::to_value(&cmd).unwrap();
        assert!(v.get("aiCode").is_some(), "AiError 必须带 aiCode");
    }
}

