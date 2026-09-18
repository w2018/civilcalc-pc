//! LLM 层错误。
//!
//! 源：`civilcalc-android-v2/core/llm/LlmError.kt`（66 行，含 3 个顶层函数）
//!
//! ## 文案分离铁律（源项目 BUG-26）
//!
//! - [`LlmError::user_message`] 是**唯一**可以进 UI 的文案，逐字对齐源项目
//! - `error_detail` / `raw_response` / `http_status` 是调试信息，**只进日志**
//!   （由上层经日志门面输出），绝不拼进用户提示
//!
//! 因此 [`std::fmt::Display`] 实现为 `user_message()` —— 它一旦被 `?` 传播出去，
//! 落到的就是 UI 文案。调试细节请显式调 [`LlmError::detail`]。
//!
//! > 注：本 crate **不依赖 `civilcalc-core`**（避免循环），所以调试细节的落盘
//! > 由上层（`src-tauri`）负责，这里只提供 [`LlmError::detail`] 取字符串。
//!
//! ## 与 `CoreError::AiError` 的对接
//!
//! ```
//! use civilcalc_llm::error::{map_http_error, LlmErrorCode};
//!
//! // HTTP 401 → AUTH_INVALID；detail 与用户文案严格分离
//! let e = map_http_error(401, Some("{\"error\":\"invalid key\"}".into()));
//! assert_eq!(e.code, LlmErrorCode::AuthInvalid);
//! assert_eq!(e.user_message(), "API Key 无效或已过期，请在设置中检查模型配置");
//! assert_eq!(e.ai_code(), "AUTH_INVALID");
//! assert!(e.detail().contains("invalid key"));   // 调试信息只在 detail 里
//! ```

use serde::{Deserialize, Serialize};

/// LLM 错误码（10 个，对齐源项目 `LlmErrorCode`）。
///
/// ⚠️ serde 表示为 `SCREAMING_SNAKE_CASE`，与源项目 Kotlin 枚举名逐字一致 ——
/// 这个字符串会作为 `CoreError::AiError.aiCode` 传到前端，**不能改**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LlmErrorCode {
    /// HTTP 401 / Key 为空
    AuthInvalid,
    /// HTTP 403 / 429
    QuotaExceeded,
    /// HTTP 400
    ModelInvalid,
    /// 超时 / IO 异常 / 其余 HTTP 状态
    Network,
    /// AI 返回内容无法解析为公式
    SchemaParseError,
    /// `finish_reason = length`
    OutputTruncated,
    /// `finish_reason = content_filter`
    ContentFiltered,
    /// 模型未开启视觉能力但请求带了图
    VisionUnsupported,
    /// HTTP 404 / 405（接口不存在）
    ProtocolUnsupported,
    /// 其他
    Unknown,
}

impl LlmErrorCode {
    /// 错误码字符串（写入 `CoreError::AiError.ai_code` / `CommandError::AiError.ai_code`）
    pub fn as_str(self) -> &'static str {
        match self {
            LlmErrorCode::AuthInvalid => "AUTH_INVALID",
            LlmErrorCode::QuotaExceeded => "QUOTA_EXCEEDED",
            LlmErrorCode::ModelInvalid => "MODEL_INVALID",
            LlmErrorCode::Network => "NETWORK",
            LlmErrorCode::SchemaParseError => "SCHEMA_PARSE_ERROR",
            LlmErrorCode::OutputTruncated => "OUTPUT_TRUNCATED",
            LlmErrorCode::ContentFiltered => "CONTENT_FILTERED",
            LlmErrorCode::VisionUnsupported => "VISION_UNSUPPORTED",
            LlmErrorCode::ProtocolUnsupported => "PROTOCOL_UNSUPPORTED",
            LlmErrorCode::Unknown => "UNKNOWN",
        }
    }

    /// 用户可见文案（**逐字对齐源项目 `LlmError.userMessage()`，不得改写**）
    pub fn user_message(self) -> &'static str {
        match self {
            LlmErrorCode::AuthInvalid => "API Key 无效或已过期，请在设置中检查模型配置",
            LlmErrorCode::QuotaExceeded => "调用受限或额度不足，请稍后再试或检查账户余额",
            LlmErrorCode::ModelInvalid => "模型名无效或请求格式错误，请在设置中检查模型名称",
            LlmErrorCode::Network => "网络连接失败，请检查网络后重试",
            LlmErrorCode::SchemaParseError => "AI 返回内容无法解析为公式，请重试或换个说法描述",
            LlmErrorCode::OutputTruncated => "AI 输出被截断，请重试或缩短描述内容",
            LlmErrorCode::ContentFiltered => "内容被安全策略拦截，请调整描述后重试",
            LlmErrorCode::VisionUnsupported => {
                "当前模型未开启视觉能力，请在设置中编辑模型开启「支持视觉」，或移除图片后重试"
            }
            LlmErrorCode::ProtocolUnsupported => {
                "该模型服务不支持当前的「接入协议」，请在设置-模型配置中改回「自动」或换一种协议"
            }
            LlmErrorCode::Unknown => "AI 调用失败，请稍后重试",
        }
    }
}

impl std::fmt::Display for LlmErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// LLM 调用错误。
///
/// 对应源项目 `data class LlmError(code, errorDetail, httpStatus, rawResponse) : Exception(errorDetail)`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmError {
    pub code: LlmErrorCode,

    /// 调试细节。**不进 UI**（源项目 `errorDetail`）
    pub error_detail: String,

    /// HTTP 状态码；`None` = 网络层/本地错误
    pub http_status: Option<u16>,

    /// 原始响应体，排障用。**不进 UI**（源项目 `rawResponse`）
    pub raw_response: Option<String>,
}

impl LlmError {
    pub fn new(
        code: LlmErrorCode,
        error_detail: impl Into<String>,
        http_status: Option<u16>,
        raw_response: Option<String>,
    ) -> Self {
        Self {
            code,
            error_detail: error_detail.into(),
            http_status,
            raw_response,
        }
    }

    /// 仅错误码的便捷构造（无 HTTP 上下文）
    pub fn of(code: LlmErrorCode) -> Self {
        Self::new(code, code.user_message(), None, None)
    }

    /// **唯一**可进 UI 的文案
    pub fn user_message(&self) -> &'static str {
        self.code.user_message()
    }

    /// 错误码字符串（供 `ai_code` 字段）
    pub fn ai_code(&self) -> &'static str {
        self.code.as_str()
    }

    /// 调试细节（`errorDetail` + 可选的 HTTP 状态与响应体），供日志使用
    pub fn detail(&self) -> String {
        let mut s = self.error_detail.clone();
        if let Some(st) = self.http_status {
            s.push_str(&format!(" (HTTP {st})"));
        }
        if let Some(raw) = &self.raw_response {
            s.push_str(&format!(" | raw={raw}"));
        }
        s
    }
}

/// `Display` = **用户文案**（见模块文档的「文案分离铁律」）。
impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.user_message())
    }
}

impl std::error::Error for LlmError {}

// =============================================================================
// 映射函数（对齐源项目 mapHttpError / mapException）
// =============================================================================

/// HTTP 状态码 → 错误。1:1 对齐源项目 `mapHttpError`。
///
/// | 状态 | 错误码 | `errorDetail` |
/// |---|---|---|
/// | 401 | `AUTH_INVALID` | `API Key 无效或缺失` |
/// | 403 / 429 | `QUOTA_EXCEEDED` | `调用受限或额度不足` |
/// | 400 | `MODEL_INVALID` | `模型名无效或请求格式错误` |
/// | 404 / 405 | `PROTOCOL_UNSUPPORTED` | `接口不存在（HTTP {status}）` |
/// | 其余 | `NETWORK` | `HTTP {status}` |
pub fn map_http_error(status: u16, body: Option<String>) -> LlmError {
    let (code, detail) = match status {
        401 => (LlmErrorCode::AuthInvalid, "API Key 无效或缺失".to_string()),
        403 | 429 => (LlmErrorCode::QuotaExceeded, "调用受限或额度不足".to_string()),
        400 => (LlmErrorCode::ModelInvalid, "模型名无效或请求格式错误".to_string()),
        404 | 405 => (
            LlmErrorCode::ProtocolUnsupported,
            format!("接口不存在（HTTP {status}）"),
        ),
        _ => (LlmErrorCode::Network, format!("HTTP {status}")),
    };
    LlmError::new(code, detail, Some(status), body)
}

/// 网络/IO 异常 → 错误。对齐源项目 `mapException`。
///
/// 源项目判 `SocketTimeoutException` → `连接超时`，其余 `IOException` → `网络异常: {msg}`。
/// Rust 侧对应：`reqwest::Error::is_timeout()` → 超时；其余 reqwest/IO 错误 → 网络异常。
pub fn map_reqwest_error(e: &reqwest::Error) -> LlmError {
    if e.is_timeout() {
        LlmError::new(LlmErrorCode::Network, "连接超时", None, None)
    } else {
        LlmError::new(
            LlmErrorCode::Network,
            format!("网络异常: {e}"),
            None,
            None,
        )
    }
}

/// `std::io::Error` → 错误（对齐源项目 `IOException` 分支）
pub fn map_io_error(e: &std::io::Error) -> LlmError {
    LlmError::new(
        LlmErrorCode::Network,
        format!("网络异常: {e}"),
        None,
        None,
    )
}

impl From<reqwest::Error> for LlmError {
    fn from(e: reqwest::Error) -> Self {
        map_reqwest_error(&e)
    }
}

impl From<std::io::Error> for LlmError {
    fn from(e: std::io::Error) -> Self {
        map_io_error(&e)
    }
}

/// `serde_json` 解析失败 → `SCHEMA_PARSE_ERROR`（对齐源项目 JSON 解析分支）
impl From<serde_json::Error> for LlmError {
    fn from(e: serde_json::Error) -> Self {
        LlmError::new(
            LlmErrorCode::SchemaParseError,
            format!("JSON 解析失败: {e}"),
            None,
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_strings_match_source_enum_names() {
        // 这些字符串会作为 aiCode 传到前端，必须与源项目 Kotlin 枚举名一致
        assert_eq!(LlmErrorCode::AuthInvalid.as_str(), "AUTH_INVALID");
        assert_eq!(LlmErrorCode::QuotaExceeded.as_str(), "QUOTA_EXCEEDED");
        assert_eq!(LlmErrorCode::ModelInvalid.as_str(), "MODEL_INVALID");
        assert_eq!(LlmErrorCode::Network.as_str(), "NETWORK");
        assert_eq!(LlmErrorCode::SchemaParseError.as_str(), "SCHEMA_PARSE_ERROR");
        assert_eq!(LlmErrorCode::OutputTruncated.as_str(), "OUTPUT_TRUNCATED");
        assert_eq!(LlmErrorCode::ContentFiltered.as_str(), "CONTENT_FILTERED");
        assert_eq!(LlmErrorCode::VisionUnsupported.as_str(), "VISION_UNSUPPORTED");
        assert_eq!(
            LlmErrorCode::ProtocolUnsupported.as_str(),
            "PROTOCOL_UNSUPPORTED"
        );
        assert_eq!(LlmErrorCode::Unknown.as_str(), "UNKNOWN");
    }

    #[test]
    fn serde_uses_screaming_snake() {
        let j = serde_json::to_string(&LlmErrorCode::SchemaParseError).unwrap();
        assert_eq!(j, "\"SCHEMA_PARSE_ERROR\"");
        let back: LlmErrorCode = serde_json::from_str("\"VISION_UNSUPPORTED\"").unwrap();
        assert_eq!(back, LlmErrorCode::VisionUnsupported);
    }

    /// 文案逐字对齐源项目 `LlmError.userMessage()`
    #[test]
    fn user_messages_are_verbatim() {
        assert_eq!(
            LlmErrorCode::AuthInvalid.user_message(),
            "API Key 无效或已过期，请在设置中检查模型配置"
        );
        assert_eq!(
            LlmErrorCode::QuotaExceeded.user_message(),
            "调用受限或额度不足，请稍后再试或检查账户余额"
        );
        assert_eq!(
            LlmErrorCode::ModelInvalid.user_message(),
            "模型名无效或请求格式错误，请在设置中检查模型名称"
        );
        assert_eq!(
            LlmErrorCode::Network.user_message(),
            "网络连接失败，请检查网络后重试"
        );
        assert_eq!(
            LlmErrorCode::SchemaParseError.user_message(),
            "AI 返回内容无法解析为公式，请重试或换个说法描述"
        );
        assert_eq!(
            LlmErrorCode::OutputTruncated.user_message(),
            "AI 输出被截断，请重试或缩短描述内容"
        );
        assert_eq!(
            LlmErrorCode::ContentFiltered.user_message(),
            "内容被安全策略拦截，请调整描述后重试"
        );
        assert_eq!(
            LlmErrorCode::VisionUnsupported.user_message(),
            "当前模型未开启视觉能力，请在设置中编辑模型开启「支持视觉」，或移除图片后重试"
        );
        assert_eq!(
            LlmErrorCode::ProtocolUnsupported.user_message(),
            "该模型服务不支持当前的「接入协议」，请在设置-模型配置中改回「自动」或换一种协议"
        );
        assert_eq!(
            LlmErrorCode::Unknown.user_message(),
            "AI 调用失败，请稍后重试"
        );
    }

    /// 1:1 对齐源项目 `mapHttpError` 的分支
    #[test]
    fn http_status_mapping_matches_source() {
        assert_eq!(map_http_error(401, None).code, LlmErrorCode::AuthInvalid);
        assert_eq!(map_http_error(403, None).code, LlmErrorCode::QuotaExceeded);
        assert_eq!(map_http_error(429, None).code, LlmErrorCode::QuotaExceeded);
        assert_eq!(map_http_error(400, None).code, LlmErrorCode::ModelInvalid);
        assert_eq!(
            map_http_error(404, None).code,
            LlmErrorCode::ProtocolUnsupported
        );
        assert_eq!(
            map_http_error(405, None).code,
            LlmErrorCode::ProtocolUnsupported
        );
        assert_eq!(map_http_error(500, None).code, LlmErrorCode::Network);
        assert_eq!(map_http_error(418, None).code, LlmErrorCode::Network);
    }

    #[test]
    fn http_mapping_keeps_status_and_body() {
        let e = map_http_error(429, Some("{\"error\":\"quota\"}".to_string()));
        assert_eq!(e.http_status, Some(429));
        assert_eq!(e.raw_response.as_deref(), Some("{\"error\":\"quota\"}"));
        assert_eq!(e.error_detail, "调用受限或额度不足");
        // 用户文案与调试细节分离
        assert_eq!(e.user_message(), "调用受限或额度不足，请稍后再试或检查账户余额");
        assert_ne!(e.user_message(), e.error_detail);
    }

    #[test]
    fn protocol_unsupported_detail_includes_status() {
        assert_eq!(
            map_http_error(404, None).error_detail,
            "接口不存在（HTTP 404）"
        );
        assert_eq!(
            map_http_error(405, None).error_detail,
            "接口不存在（HTTP 405）"
        );
        assert_eq!(map_http_error(503, None).error_detail, "HTTP 503");
    }

    #[test]
    fn display_shows_user_message_not_detail() {
        let e = LlmError::new(LlmErrorCode::Unknown, "内部细节 stacktrace…", None, None);
        assert_eq!(format!("{e}"), "AI 调用失败，请稍后重试");
        assert!(e.detail().contains("内部细节"));
    }

    #[test]
    fn detail_includes_status_and_raw() {
        let e = LlmError::new(
            LlmErrorCode::ModelInvalid,
            "模型名无效",
            Some(400),
            Some("bad model".into()),
        );
        let d = e.detail();
        assert!(d.contains("模型名无效"));
        assert!(d.contains("HTTP 400"));
        assert!(d.contains("bad model"));
    }

    #[test]
    fn serde_roundtrip() {
        let e = map_http_error(401, Some("unauthorized".into()));
        let s = serde_json::to_string(&e).unwrap();
        let back: LlmError = serde_json::from_str(&s).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn io_error_maps_to_network() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        let e: LlmError = io.into();
        assert_eq!(e.code, LlmErrorCode::Network);
        assert!(e.error_detail.starts_with("网络异常: "));
    }

    #[test]
    fn json_error_maps_to_schema_parse_error() {
        let je = serde_json::from_str::<serde_json::Value>("{ bad").unwrap_err();
        let e: LlmError = je.into();
        assert_eq!(e.code, LlmErrorCode::SchemaParseError);
    }

    #[test]
    fn of_helper_uses_user_message_as_detail() {
        let e = LlmError::of(LlmErrorCode::ContentFiltered);
        assert_eq!(e.error_detail, "内容被安全策略拦截，请调整描述后重试");
        assert_eq!(e.http_status, None);
    }
}
