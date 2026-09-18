//! LLM HTTP 客户端（双协议 + 流式 + 多模态 + 联网搜索）。
//!
//! 源：`civilcalc-android-v2/core/llm/LlmClient.kt`（`LlmClient` 类，约 580 行）
//!
//! ## 设计：网络与决策分离
//!
//! 源把「发请求 → 读响应 → 判定成败」揉在一起，只能靠真服务器测。
//! 这里把**决策**抽成纯函数：
//!
//! | 纯函数 | 职责 |
//! |---|---|
//! | [`use_responses_api`] | 协议路由 |
//! | [`thinking_params`] | 各家思考参数拼装（最易错的一处） |
//! | [`build_chat_request`] / [`build_responses_request`] | 请求体构造 |
//! | [`interpret_chat_response`] / [`interpret_responses_response`] | 非流式响应判定 |
//! | [`crate::stream`] 的 `chat_end` / `responses_end` | 流式结束判定 |
//!
//! 于是除「真的联网」外的全部逻辑都能离线测。
//!
//! ## 🔴 三条实测知识（重写最易丢失的资产）
//!
//! 1. **GLM glm-5.3 系列强制思考**：`thinking.type=disabled` 直接 400（错误码 1210）。
//!    「关闭」必须降级为 `reasoning_effort=low`。
//! 2. **小米 MiMo 无档位**：chat/completions 只有 `thinking.type` 开关，
//!    不发 `reasoning_effort`；其 responses 也无 `max` 档（降为 `high`）。
//! 3. **`/responses` 只有 `reasoning.effort`**，没有 `thinking.type`。
//!
//! ## 🔴 回调必须是 `Send`
//!
//! `on_signal` 的约束是 `&mut (dyn FnMut(Signal) + Send)` —— Tauri 的 `async` 命令
//! 要求 future 可跨线程（`Send`），回调若不 `Send` 会在**命令层**编译失败（错误点在 src-tauri，
//! 不在本 crate，容易被误判成 tauri 的锅）。
//!
//! ## 🔴 取消
//!
//! `chat_stream` 接受 `cancel: Option<&AtomicBool>`；置位后**下一个 chunk 到达时立即返回**，
//! 并 drop 响应体（`reqwest` 会随之关闭连接）。错误详情固定为 `已取消`，命令层据此映射。
//!
//! ## 🔴 超时设置
//!
//! 源（OkHttp）：connect 30s / **read 300s** / write 60s。
//! read 放宽到 300s 是因为**非流式**调用要等完整回复（实测 MiMo 178s）；
//! 流式路径下该超时只约束**相邻 chunk 间隔**，放宽不影响流式体验。
//!
//! ⚠️ **`reqwest` 0.12 没有 `write_timeout`**（`ClientBuilder` 只有
//! `connect_timeout` / `read_timeout` / `timeout` / `pool_idle_timeout`）。
//! 源的 60s 写超时**无对应项**，只能省掉 —— 请求体是内存里的 JSON，
//! 写出阻塞的唯一成因是网络中断，此时 `read_timeout` 会兜住。
//! 不要用 `.timeout(60s)` 顶替：那是**整体**超时，会把非流式的 300s 读一并掐掉。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::StreamExt;

use crate::config::{forces_thinking, ApiProtocol, ResolvedLlmProfile, ThinkingLevel};
use crate::error::{map_http_error, map_reqwest_error, LlmError, LlmErrorCode};
use crate::protocol::{
    to_response_input_item, web_search_tool, ChatMessage, ChatResult, DeepseekResponseRequest,
    DeepseekResponseResponse, OpenAiChatRequest, OpenAiChatResponse, ReasoningConfig,
    ResponseFormat, ResponseTextConfig, StreamOptions, ThinkingConfig,
};
use crate::stream::{chat_end, responses_end, Signal, StreamEnd, StreamState};

/// HTTP 客户端。
#[derive(Debug, Clone)]
pub struct LlmClient {
    http: reqwest::Client,
}

impl Default for LlmClient {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmClient {
    /// 按源项目的超时设置构造（connect 30s / read 300s）。
    ///
    /// ⚠️ 源的 write 60s 在 `reqwest` 0.12 无对应方法（见模块文档）。
    #[must_use]
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { http }
    }

    /// 注入自定义 `reqwest::Client`（测试或需要共享连接池时用）。
    #[must_use]
    pub fn with_http(http: reqwest::Client) -> Self {
        Self { http }
    }

    // -------------------------------------------------------------------------
    // 非流式
    // -------------------------------------------------------------------------

    /// 非流式调用。
    ///
    /// ⚠️ `usage` 由各协议内部解析（已归一化两套字段命名）；厂商不返回时
    /// **宁可统计缺一条，也不再补发一次请求冒充真实用量**（既重复计费又写脏统计）。
    pub async fn chat(
        &self,
        profile: &ResolvedLlmProfile,
        messages: &[ChatMessage],
        json_mode: bool,
        web_search: bool,
        max_tokens: Option<i32>,
    ) -> Result<ChatResult, LlmError> {
        if use_responses_api(profile, web_search) {
            self.responses_chat(profile, messages, json_mode, web_search)
                .await
        } else {
            self.chat_completions(profile, messages, json_mode, web_search, max_tokens)
                .await
        }
    }

    async fn chat_completions(
        &self,
        profile: &ResolvedLlmProfile,
        messages: &[ChatMessage],
        json_mode: bool,
        web_search: bool,
        max_tokens: Option<i32>,
    ) -> Result<ChatResult, LlmError> {
        validate_messages(messages)?;
        let req = build_chat_request(profile, messages, json_mode, web_search, max_tokens, false);
        let body = encode_body(&req)?;
        let url = chat_completions_url(&profile.base_url);
        let (status, text) = self.post_and_read(&url, &profile.api_key, body).await?;
        interpret_chat_response(status, &text)
    }

    async fn responses_chat(
        &self,
        profile: &ResolvedLlmProfile,
        messages: &[ChatMessage],
        json_mode: bool,
        web_search: bool,
    ) -> Result<ChatResult, LlmError> {
        validate_messages(messages)?;
        let req = build_responses_request(profile, messages, json_mode, web_search, false);
        let body = encode_body(&req)?;
        let url = responses_url(&profile.base_url);
        let (status, text) = self.post_and_read(&url, &profile.api_key, body).await?;
        interpret_responses_response(status, &text)
    }

    // -------------------------------------------------------------------------
    // 流式
    // -------------------------------------------------------------------------

    /// 流式调用。
    ///
    /// `on_signal` 依次收到 [`Signal::Thinking`] / [`Signal::Content`]（均为**累积全文**）、
    /// [`Signal::Usage`]（若厂商返回）、[`Signal::Done`]。
    ///
    /// ⚠️ **不向 API 发送 `max_tokens`**（源注释）：GLM/MiMo/DeepSeek 的思考 token
    /// 计入该上限，设小值会截断思考导致正文为空（实测复杂需求思考即需 ~10K token），
    /// 故公式链路一律不设上限。
    pub async fn chat_stream(
        &self,
        profile: &ResolvedLlmProfile,
        messages: &[ChatMessage],
        json_mode: bool,
        web_search: bool,
        cancel: Option<&AtomicBool>,
        on_signal: &mut (dyn FnMut(Signal) + Send),
    ) -> Result<ChatResult, LlmError> {
        if use_responses_api(profile, web_search) {
            self.responses_stream(profile, messages, json_mode, web_search, cancel, on_signal)
                .await
        } else {
            self.chat_completions_stream(profile, messages, json_mode, web_search, cancel, on_signal)
                .await
        }
    }

    async fn chat_completions_stream(
        &self,
        profile: &ResolvedLlmProfile,
        messages: &[ChatMessage],
        json_mode: bool,
        web_search: bool,
        cancel: Option<&AtomicBool>,
        on_signal: &mut (dyn FnMut(Signal) + Send),
    ) -> Result<ChatResult, LlmError> {
        validate_messages(messages)?;
        let req = build_chat_request(profile, messages, json_mode, web_search, None, true);
        let body = encode_body(&req)?;
        let url = chat_completions_url(&profile.base_url);
        let mut state = StreamState::default();
        self.consume_stream(&url, &profile.api_key, body, cancel, |line| {
            let signals = state.feed_chat(line);
            emit(on_signal, signals)
        })
        .await?;
        finish_stream(&state, true)
    }

    async fn responses_stream(
        &self,
        profile: &ResolvedLlmProfile,
        messages: &[ChatMessage],
        json_mode: bool,
        web_search: bool,
        cancel: Option<&AtomicBool>,
        on_signal: &mut (dyn FnMut(Signal) + Send),
    ) -> Result<ChatResult, LlmError> {
        validate_messages(messages)?;
        let req = build_responses_request(profile, messages, json_mode, web_search, true);
        let body = encode_body(&req)?;
        let url = responses_url(&profile.base_url);
        let mut state = StreamState::default();
        self.consume_stream(&url, &profile.api_key, body, cancel, |line| {
            let signals = state.feed_responses(line);
            emit(on_signal, signals)
        })
        .await?;
        finish_stream(&state, false)
    }

    // -------------------------------------------------------------------------
    // 网络原语
    // -------------------------------------------------------------------------

    /// 发 POST 并读完整响应体（**非 2xx 也读体**，交由 `interpret_*` 判定）。
    async fn post_and_read(
        &self,
        url: &str,
        api_key: &str,
        body: String,
    ) -> Result<(u16, String), LlmError> {
        let resp = self
            .http
            .post(url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json; charset=utf-8")
            .body(body)
            .send()
            .await
            .map_err(|e| map_reqwest_error(&e))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(|e| map_reqwest_error(&e))?;
        Ok((status, text))
    }

    /// 读 SSE 流：按行切分并交给 `on_line`，**自动处理跨 chunk 的半行**。
    ///
    /// `on_line` 返回 `true` 表示已收到 `[DONE]`，可提前结束读取。
    async fn consume_stream<F>(
        &self,
        url: &str,
        api_key: &str,
        body: String,
        cancel: Option<&AtomicBool>,
        mut on_line: F,
    ) -> Result<(), LlmError>
    where
        F: FnMut(&str) -> bool + Send,
    {
        /// 取消时返回的错误详情（命令层据此识别，见 `commands/ai.rs`）。
        const CANCELLED: &str = "已取消";
        let resp = self
            .http
            .post(url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json; charset=utf-8")
            .body(body)
            .send()
            .await
            .map_err(|e| map_reqwest_error(&e))?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            // 出错时响应体是普通 JSON（不是 SSE），直接读出来映射
            let err_body = resp.text().await.unwrap_or_default();
            return Err(map_http_error(status, Some(err_body)));
        }

        let mut buf = String::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            // 🔴 取消要**立即断开**：每读到一个 chunk 先查标志，命中就 drop 掉 stream
            //    （`reqwest` 的 body 被 drop 时会关闭连接），不继续消费剩余 token
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err(LlmError::new(LlmErrorCode::Unknown, CANCELLED, None, None));
            }
            let bytes = chunk.map_err(|e| map_reqwest_error(&e))?;
            // 多字节字符可能被 chunk 边界切开 —— 用 lossy 会有 U+FFFD，
            // 故先按字节累积、只在**完整行**（以 \n 结尾）上解码。
            buf.push_str(&String::from_utf8_lossy(&bytes));
            while let Some(pos) = buf.find('\n') {
                let line: String = buf[..pos].trim_end_matches('\r').to_string();
                buf.drain(..=pos);
                if on_line(&line) {
                    return Ok(());
                }
            }
        }
        // 处理无尾换行的残留
        let tail = buf.trim_end_matches('\r');
        if !tail.is_empty() {
            let _ = on_line(tail);
        }
        Ok(())
    }
}

// =============================================================================
// 纯函数：协议路由与参数拼装
// =============================================================================

/// 协议路由：用户显式选择优先；`Auto` = DeepSeek 联网走 `/responses`，其余走 `/chat/completions`。
#[must_use]
pub fn use_responses_api(profile: &ResolvedLlmProfile, web_search: bool) -> bool {
    match profile.api_protocol {
        ApiProtocol::Responses => true,
        ApiProtocol::ChatCompletions => false,
        ApiProtocol::Auto => profile.base_url.contains("deepseek") && web_search,
    }
}

/// 思考参数（各家字段/语义不同）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThinkingParams {
    pub thinking: Option<ThinkingConfig>,
    pub reasoning_effort: Option<String>,
    pub reasoning: Option<ReasoningConfig>,
}

/// 拼装思考参数。
///
/// 逐条对应源的实测结论：
///
/// - `for_responses`：只有 `reasoning.effort`（`none`/`low`/`high`/`max`）；
///   GLM 强制思考 → `Off` 降级 `low`（`none` 会被拒）；MiMo 无 `max` → 降 `high`
/// - 非 responses 且**强制思考**（GLM）：`thinking.type=enabled` +
///   `reasoning_effort`（`Off`/`Low` → `low`）
/// - 非 responses 且非强制：`thinking.type` 开关式；`reasoning_effort` 只在
///   `Low`/`High`/`Max` 时给，且 MiMo 的 `Max` 不发（无档位）
#[must_use]
pub fn thinking_params(profile: &ResolvedLlmProfile, for_responses: bool) -> ThinkingParams {
    let level = profile.thinking_level;
    let forced = forces_thinking(&profile.base_url);

    if for_responses {
        let effort = match level {
            ThinkingLevel::Auto => None,
            ThinkingLevel::Off => Some(if forced { "low" } else { "none" }),
            ThinkingLevel::Low => Some("low"),
            ThinkingLevel::High => Some("high"),
            ThinkingLevel::Max => Some(if profile.base_url.contains("xiaomimimo") {
                "high"
            } else {
                "max"
            }),
        };
        return ThinkingParams {
            thinking: None,
            reasoning_effort: None,
            reasoning: effort.map(|e| ReasoningConfig {
                effort: e.to_string(),
            }),
        };
    }

    // GLM glm-5.3 系列强制思考：发 disabled 会 400（1210），只能用 reasoning_effort 降本
    if forced {
        let effort = match level {
            ThinkingLevel::Auto => None,
            ThinkingLevel::Off | ThinkingLevel::Low => Some("low"),
            ThinkingLevel::High => Some("high"),
            ThinkingLevel::Max => Some("max"),
        };
        return ThinkingParams {
            thinking: Some(ThinkingConfig {
                r#type: "enabled".to_string(),
            }),
            reasoning_effort: effort.map(str::to_string),
            reasoning: None,
        };
    }

    let thinking = match level {
        ThinkingLevel::Auto => None,
        ThinkingLevel::Off => Some(ThinkingConfig {
            r#type: "disabled".to_string(),
        }),
        _ => Some(ThinkingConfig {
            r#type: "enabled".to_string(),
        }),
    };
    // 小米 MiMo 的 chat/completions 只有开关，没有 reasoning_effort 档位
    let effort = match level {
        ThinkingLevel::Low => Some("low"),
        ThinkingLevel::High => Some("high"),
        ThinkingLevel::Max => {
            if profile.base_url.contains("xiaomimimo") {
                None
            } else {
                Some("max")
            }
        }
        _ => None,
    };
    ThinkingParams {
        thinking,
        reasoning_effort: effort.map(str::to_string),
        reasoning: None,
    }
}

/// 构造 `/chat/completions` 请求体。
#[must_use]
pub fn build_chat_request(
    profile: &ResolvedLlmProfile,
    messages: &[ChatMessage],
    json_mode: bool,
    web_search: bool,
    max_tokens: Option<i32>,
    stream: bool,
) -> OpenAiChatRequest {
    let tp = thinking_params(profile, false);
    let mut req = OpenAiChatRequest::new(profile.model.clone(), messages.to_vec());
    req.response_format = json_mode.then(|| ResponseFormat {
        r#type: "json_object".to_string(),
    });
    req.max_tokens = max_tokens;
    if web_search {
        req.tools = Some(vec![web_search_tool(&profile.base_url)]);
        req.tool_choice = Some("auto".to_string());
    }
    if stream {
        req.stream = true;
        req.stream_options = Some(StreamOptions::default());
    }
    req.thinking = tp.thinking;
    req.reasoning_effort = tp.reasoning_effort;
    req
}

/// 构造 `/responses` 请求体（系统提示提到顶层 `instructions`）。
#[must_use]
pub fn build_responses_request(
    profile: &ResolvedLlmProfile,
    messages: &[ChatMessage],
    json_mode: bool,
    web_search: bool,
    stream: bool,
) -> DeepseekResponseRequest {
    let tp = thinking_params(profile, true);
    DeepseekResponseRequest {
        model: profile.model.clone(),
        instructions: messages
            .iter()
            .find(|m| m.role == "system")
            .map(|m| m.content.clone()),
        input: messages
            .iter()
            .filter(|m| m.role != "system")
            .map(to_response_input_item)
            .collect(),
        tools: web_search.then(|| vec![web_search_tool(&profile.base_url)]),
        text: json_mode.then(|| ResponseTextConfig {
            format: Some(ResponseFormat {
                r#type: "json_object".to_string(),
            }),
        }),
        stream,
        reasoning: tp.reasoning,
    }
}

/// `/chat/completions` 端点。源直接字符串拼接（`${baseUrl}/chat/completions`）。
#[must_use]
pub fn chat_completions_url(base_url: &str) -> String {
    format!("{base_url}/chat/completions")
}

/// `/responses` 端点。
#[must_use]
pub fn responses_url(base_url: &str) -> String {
    format!("{base_url}/responses")
}

// =============================================================================
// 纯函数：响应解读
// =============================================================================

/// 解读 `/chat/completions` 的非流式响应。
///
/// ⚠️ **判定顺序与源一致**：先「有无 choices」→ 再 `finish_reason` 截断/拦截。
/// 注意 `content` 为空串**不算**「无有效内容」（源只在 `choices` 缺失时报错）。
pub fn interpret_chat_response(status: u16, body: &str) -> Result<ChatResult, LlmError> {
    if body.is_empty() {
        return Err(LlmError::new(LlmErrorCode::Network, "空响应", None, None));
    }
    if !(200..300).contains(&status) {
        return Err(map_http_error(status, Some(body.to_string())));
    }
    let parsed: OpenAiChatResponse = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return Err(LlmError::new(
                LlmErrorCode::SchemaParseError,
                format!("响应解析失败: {e}"),
                None,
                None,
            ))
        }
    };
    if let Some(err) = &parsed.error {
        return Err(LlmError::new(
            LlmErrorCode::Unknown,
            err.message.clone().unwrap_or_else(|| "API 错误".to_string()),
            Some(status),
            Some(body.to_string()),
        ));
    }
    let Some(content) = parsed.content() else {
        return Err(LlmError::new(
            LlmErrorCode::SchemaParseError,
            "无有效响应内容",
            None,
            None,
        ));
    };
    if parsed.finish_reason() == Some("length") {
        return Err(LlmError::new(
            LlmErrorCode::OutputTruncated,
            "输出被截断",
            None,
            None,
        ));
    }
    if parsed.finish_reason() == Some("content_filter") {
        return Err(LlmError::new(
            LlmErrorCode::ContentFiltered,
            "内容被安全策略拦截",
            None,
            None,
        ));
    }
    Ok(ChatResult {
        content: content.trim().to_string(),
        usage: parsed.usage.map(|u| u.normalized()),
    })
}

/// 解读 `/responses` 的非流式响应。
///
/// ⚠️ 非流式路径**不检查 `status`**（源只在流式的 `response.completed` 里查 `incomplete`）。
pub fn interpret_responses_response(status: u16, body: &str) -> Result<ChatResult, LlmError> {
    if body.is_empty() {
        return Err(LlmError::new(LlmErrorCode::Network, "空响应", None, None));
    }
    if !(200..300).contains(&status) {
        return Err(map_http_error(status, Some(body.to_string())));
    }
    let parsed: DeepseekResponseResponse = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return Err(LlmError::new(
                LlmErrorCode::SchemaParseError,
                format!("响应解析失败: {e}"),
                None,
                None,
            ))
        }
    };
    if let Some(err) = &parsed.error {
        return Err(LlmError::new(
            LlmErrorCode::Unknown,
            err.message.clone().unwrap_or_else(|| "API 错误".to_string()),
            Some(status),
            Some(body.to_string()),
        ));
    }
    let text = parsed.output_text();
    if text.is_empty() {
        return Err(LlmError::new(
            LlmErrorCode::SchemaParseError,
            "无有效响应内容",
            None,
            None,
        ));
    }
    Ok(ChatResult {
        content: text,
        usage: parsed.usage.map(|u| u.normalized()),
    })
}

// =============================================================================
// 内部辅助
// =============================================================================

/// 源的 `require(messages.isNotEmpty() && messages.first().role == "system")`。
fn validate_messages(messages: &[ChatMessage]) -> Result<(), LlmError> {
    if messages.is_empty() || messages.first().map(|m| m.role.as_str()) != Some("system") {
        return Err(LlmError::new(
            LlmErrorCode::Unknown,
            "messages[0] 必须是 system 消息",
            None,
            None,
        ));
    }
    Ok(())
}

fn encode_body<T: serde::Serialize>(req: &T) -> Result<String, LlmError> {
    serde_json::to_string(req).map_err(|e| {
        LlmError::new(
            LlmErrorCode::SchemaParseError,
            format!("请求体序列化失败: {e}"),
            None,
            None,
        )
    })
}

/// 把信号转给回调；返回是否应停止读取（收到 `Done`）。
fn emit(on_signal: &mut (dyn FnMut(Signal) + Send), signals: Vec<Signal>) -> bool {
    for sig in signals {
        let done = matches!(sig, Signal::Done);
        on_signal(sig);
        if done {
            return true;
        }
    }
    false
}

/// 流式结束判定 → `Result`。
fn finish_stream(state: &StreamState, chat: bool) -> Result<ChatResult, LlmError> {
    let end = if chat {
        chat_end(state)
    } else {
        responses_end(state)
    };
    match end {
        StreamEnd::Ok(r) => Ok(r),
        StreamEnd::Truncated => Err(LlmError::new(
            LlmErrorCode::OutputTruncated,
            if chat {
                "输出被截断（finish_reason=length）"
            } else {
                "输出不完整（response.status=incomplete）"
            },
            None,
            None,
        )),
        StreamEnd::Empty => Err(LlmError::new(
            LlmErrorCode::SchemaParseError,
            "无有效响应内容",
            None,
            None,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ApiProtocol;

    fn profile(base_url: &str, protocol: ApiProtocol, level: ThinkingLevel) -> ResolvedLlmProfile {
        ResolvedLlmProfile {
            base_url: base_url.to_string(),
            model: "test-model".to_string(),
            api_key: "sk-test".to_string(),
            web_search: false,
            thinking_level: level,
            vision: false,
            api_protocol: protocol,
        }
    }

    const DEEPSEEK: &str = "https://api.deepseek.com/v1";
    const GLM: &str = "https://open.bigmodel.cn/api/paas/v4";
    const MIMO: &str = "https://api.xiaomimimo.com/v1";

    fn msgs() -> Vec<ChatMessage> {
        vec![
            ChatMessage::system("S"),
            ChatMessage::user("U", vec![]),
        ]
    }

    // ---------------------------------------------------------------------
    // 协议路由
    // ---------------------------------------------------------------------

    #[test]
    fn route_explicit_protocol_wins() {
        assert!(use_responses_api(
            &profile(GLM, ApiProtocol::Responses, ThinkingLevel::Auto),
            false
        ));
        assert!(!use_responses_api(
            &profile(DEEPSEEK, ApiProtocol::ChatCompletions, ThinkingLevel::Auto),
            true
        ));
    }

    /// Auto：只有 **DeepSeek + 联网** 才走 /responses
    #[test]
    fn route_auto_deepseek_websearch_only() {
        assert!(use_responses_api(
            &profile(DEEPSEEK, ApiProtocol::Auto, ThinkingLevel::Auto),
            true
        ));
        assert!(!use_responses_api(
            &profile(DEEPSEEK, ApiProtocol::Auto, ThinkingLevel::Auto),
            false
        ));
        assert!(!use_responses_api(
            &profile(GLM, ApiProtocol::Auto, ThinkingLevel::Auto),
            true
        ));
    }

    // ---------------------------------------------------------------------
    // 思考参数
    // ---------------------------------------------------------------------

    /// 🔴 GLM 强制思考：`Off` 必须降级为 `low`（发 disabled 会 400/1210）
    #[test]
    fn glm_forced_thinking_off_degrades_to_low() {
        let tp = thinking_params(&profile(GLM, ApiProtocol::ChatCompletions, ThinkingLevel::Off), false);
        assert_eq!(tp.thinking, Some(ThinkingConfig { r#type: "enabled".into() }));
        assert_eq!(tp.reasoning_effort.as_deref(), Some("low"));
    }

    #[test]
    fn glm_forced_thinking_levels() {
        let p = |l| profile(GLM, ApiProtocol::ChatCompletions, l);
        assert_eq!(thinking_params(&p(ThinkingLevel::Low), false).reasoning_effort.as_deref(), Some("low"));
        assert_eq!(thinking_params(&p(ThinkingLevel::High), false).reasoning_effort.as_deref(), Some("high"));
        assert_eq!(thinking_params(&p(ThinkingLevel::Max), false).reasoning_effort.as_deref(), Some("max"));
        let auto = thinking_params(&p(ThinkingLevel::Auto), false);
        assert_eq!(auto.thinking, Some(ThinkingConfig { r#type: "enabled".into() }));
        assert!(auto.reasoning_effort.is_none(), "Auto 不下发档位");
    }

    /// DeepSeek（非强制）：`Off` → `thinking.type=disabled`，且**不发** effort
    #[test]
    fn deepseek_off_disables_thinking() {
        let tp = thinking_params(&profile(DEEPSEEK, ApiProtocol::ChatCompletions, ThinkingLevel::Off), false);
        assert_eq!(tp.thinking, Some(ThinkingConfig { r#type: "disabled".into() }));
        assert!(tp.reasoning_effort.is_none());
    }

    #[test]
    fn deepseek_auto_sends_nothing() {
        let tp = thinking_params(&profile(DEEPSEEK, ApiProtocol::ChatCompletions, ThinkingLevel::Auto), false);
        assert!(tp.thinking.is_none());
        assert!(tp.reasoning_effort.is_none());
    }

    #[test]
    fn deepseek_low_enables_with_effort() {
        let tp = thinking_params(&profile(DEEPSEEK, ApiProtocol::ChatCompletions, ThinkingLevel::Low), false);
        assert_eq!(tp.thinking, Some(ThinkingConfig { r#type: "enabled".into() }));
        assert_eq!(tp.reasoning_effort.as_deref(), Some("low"));
    }

    /// 🔴 小米 MiMo 的 chat/completions **无档位**：Max 不发 effort
    #[test]
    fn mimo_has_no_reasoning_effort() {
        let max = thinking_params(&profile(MIMO, ApiProtocol::ChatCompletions, ThinkingLevel::Max), false);
        assert_eq!(max.thinking, Some(ThinkingConfig { r#type: "enabled".into() }));
        assert!(max.reasoning_effort.is_none(), "MiMo 无档位控制");

        let low = thinking_params(&profile(MIMO, ApiProtocol::ChatCompletions, ThinkingLevel::Low), false);
        assert_eq!(low.reasoning_effort.as_deref(), Some("low"), "MiMo 仍接受 low/high");

        let off = thinking_params(&profile(MIMO, ApiProtocol::ChatCompletions, ThinkingLevel::Off), false);
        assert_eq!(off.thinking, Some(ThinkingConfig { r#type: "disabled".into() }));
    }

    /// `/responses`：DeepSeek 支持 none/low/high/max
    #[test]
    fn responses_effort_levels() {
        let p = |l| profile(DEEPSEEK, ApiProtocol::Responses, l);
        assert_eq!(thinking_params(&p(ThinkingLevel::Off), true).reasoning.unwrap().effort, "none");
        assert_eq!(thinking_params(&p(ThinkingLevel::Low), true).reasoning.unwrap().effort, "low");
        assert_eq!(thinking_params(&p(ThinkingLevel::High), true).reasoning.unwrap().effort, "high");
        assert_eq!(thinking_params(&p(ThinkingLevel::Max), true).reasoning.unwrap().effort, "max");
        assert!(thinking_params(&p(ThinkingLevel::Auto), true).reasoning.is_none());
    }

    /// `/responses` + GLM：`Off` 降级 `low`（`none` 会被拒）
    #[test]
    fn responses_glm_off_degrades_to_low() {
        let tp = thinking_params(&profile(GLM, ApiProtocol::Responses, ThinkingLevel::Off), true);
        assert_eq!(tp.reasoning.unwrap().effort, "low");
    }

    /// `/responses` + MiMo：无 `max` 档 → 降 `high`
    #[test]
    fn responses_mimo_max_degrades_to_high() {
        let tp = thinking_params(&profile(MIMO, ApiProtocol::Responses, ThinkingLevel::Max), true);
        assert_eq!(tp.reasoning.unwrap().effort, "high");
    }

    /// responses 路径**不发** `thinking.type`（该协议没有此字段）
    #[test]
    fn responses_never_sends_thinking_type() {
        for l in [
            ThinkingLevel::Auto,
            ThinkingLevel::Off,
            ThinkingLevel::Low,
            ThinkingLevel::High,
            ThinkingLevel::Max,
        ] {
            let tp = thinking_params(&profile(DEEPSEEK, ApiProtocol::Responses, l), true);
            assert!(tp.thinking.is_none(), "{l:?} 不应有 thinking.type");
            assert!(tp.reasoning_effort.is_none());
        }
    }

    // ---------------------------------------------------------------------
    // 请求体构造
    // ---------------------------------------------------------------------

    #[test]
    fn chat_request_json_mode_and_websearch() {
        let req = build_chat_request(
            &profile(DEEPSEEK, ApiProtocol::ChatCompletions, ThinkingLevel::Auto),
            &msgs(),
            true,
            true,
            None,
            false,
        );
        assert_eq!(req.response_format.as_ref().unwrap().r#type, "json_object");
        assert_eq!(req.tool_choice.as_deref(), Some("auto"));
        assert_eq!(req.tools.as_ref().unwrap().len(), 1);
        assert!(!req.stream);
        assert!(req.stream_options.is_none());
        assert_eq!(req.temperature, 0.0);
    }

    #[test]
    fn chat_request_streaming_adds_options() {
        let req = build_chat_request(
            &profile(DEEPSEEK, ApiProtocol::ChatCompletions, ThinkingLevel::Auto),
            &msgs(),
            false,
            false,
            None,
            true,
        );
        assert!(req.stream);
        assert!(req.stream_options.unwrap().include_usage);
        assert!(req.response_format.is_none());
        assert!(req.tools.is_none());
    }

    #[test]
    fn chat_request_carries_thinking_params() {
        let req = build_chat_request(
            &profile(GLM, ApiProtocol::ChatCompletions, ThinkingLevel::Off),
            &msgs(),
            false,
            false,
            None,
            false,
        );
        assert_eq!(req.thinking.unwrap().r#type, "enabled");
        assert_eq!(req.reasoning_effort.as_deref(), Some("low"));
    }

    #[test]
    fn chat_request_passes_max_tokens() {
        let req = build_chat_request(
            &profile(DEEPSEEK, ApiProtocol::ChatCompletions, ThinkingLevel::Auto),
            &msgs(),
            false,
            false,
            Some(2048),
            false,
        );
        assert_eq!(req.max_tokens, Some(2048));
    }

    #[test]
    fn responses_request_moves_system_to_instructions() {
        let req = build_responses_request(
            &profile(DEEPSEEK, ApiProtocol::Responses, ThinkingLevel::Auto),
            &msgs(),
            true,
            true,
            true,
        );
        assert_eq!(req.instructions.as_deref(), Some("S"));
        assert_eq!(req.input.len(), 1, "system 不进 input");
        assert_eq!(req.input[0].role, "user");
        assert_eq!(req.text.as_ref().unwrap().format.as_ref().unwrap().r#type, "json_object");
        assert!(req.tools.is_some());
        assert!(req.stream);
    }

    #[test]
    fn responses_request_carries_reasoning() {
        let req = build_responses_request(
            &profile(DEEPSEEK, ApiProtocol::Responses, ThinkingLevel::High),
            &msgs(),
            false,
            false,
            false,
        );
        assert_eq!(req.reasoning.unwrap().effort, "high");
        assert!(req.text.is_none());
        assert!(!req.stream);
    }

    #[test]
    fn endpoints_are_plain_concatenation() {
        assert_eq!(chat_completions_url(DEEPSEEK), "https://api.deepseek.com/v1/chat/completions");
        assert_eq!(responses_url(DEEPSEEK), "https://api.deepseek.com/v1/responses");
    }

    // ---------------------------------------------------------------------
    // 非流式响应解读
    // ---------------------------------------------------------------------

    #[test]
    fn interpret_chat_ok_trims_and_normalizes() {
        let body = r#"{"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"  hi  "}}],
                       "usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}}"#;
        let r = interpret_chat_response(200, body).unwrap();
        assert_eq!(r.content, "hi");
        let u = r.usage.unwrap();
        assert_eq!(u.prompt_tokens, 7, "input_tokens 归一化到 prompt_tokens");
        assert_eq!(u.completion_tokens, 3);
    }

    #[test]
    fn interpret_chat_length_is_truncated() {
        let body = r#"{"choices":[{"finish_reason":"length","message":{"role":"assistant","content":"x"}}]}"#;
        assert_eq!(
            interpret_chat_response(200, body).unwrap_err().code,
            LlmErrorCode::OutputTruncated
        );
    }

    #[test]
    fn interpret_chat_content_filter() {
        let body = r#"{"choices":[{"finish_reason":"content_filter","message":{"role":"assistant","content":"x"}}]}"#;
        assert_eq!(
            interpret_chat_response(200, body).unwrap_err().code,
            LlmErrorCode::ContentFiltered
        );
    }

    #[test]
    fn interpret_chat_api_error_body() {
        let body = r#"{"error":{"message":"bad key","type":"auth","code":"401"}}"#;
        let e = interpret_chat_response(200, body).unwrap_err();
        assert_eq!(e.code, LlmErrorCode::Unknown);
        assert!(e.error_detail.contains("bad key"));
        assert_eq!(e.http_status, Some(200));
    }

    #[test]
    fn interpret_chat_no_choices_is_parse_error() {
        let body = r#"{"choices":[]}"#;
        assert_eq!(
            interpret_chat_response(200, body).unwrap_err().code,
            LlmErrorCode::SchemaParseError
        );
    }

    /// 空串 content 但 choices 存在 → 不算「无有效内容」（与源一致）
    #[test]
    fn interpret_chat_empty_content_still_ok() {
        let body = r#"{"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":""}}]}"#;
        assert_eq!(interpret_chat_response(200, body).unwrap().content, "");
    }

    #[test]
    fn interpret_chat_empty_body_is_network_error() {
        assert_eq!(
            interpret_chat_response(200, "").unwrap_err().code,
            LlmErrorCode::Network
        );
    }

    #[test]
    fn interpret_chat_http_status_mapping() {
        assert_eq!(interpret_chat_response(401, "{}").unwrap_err().code, LlmErrorCode::AuthInvalid);
        assert_eq!(interpret_chat_response(429, "{}").unwrap_err().code, LlmErrorCode::QuotaExceeded);
        assert_eq!(interpret_chat_response(400, "{}").unwrap_err().code, LlmErrorCode::ModelInvalid);
        assert_eq!(interpret_chat_response(404, "{}").unwrap_err().code, LlmErrorCode::ProtocolUnsupported);
        assert_eq!(interpret_chat_response(500, "{}").unwrap_err().code, LlmErrorCode::Network);
    }

    #[test]
    fn interpret_chat_bad_json_is_parse_error() {
        assert_eq!(
            interpret_chat_response(200, "{not json").unwrap_err().code,
            LlmErrorCode::SchemaParseError
        );
    }

    #[test]
    fn interpret_responses_ok() {
        let body = r#"{"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":" 结果 "}]}],
                       "usage":{"input_tokens":5,"output_tokens":2,"total_tokens":7}}"#;
        let r = interpret_responses_response(200, body).unwrap();
        assert_eq!(r.content, "结果");
        assert_eq!(r.usage.unwrap().prompt_tokens, 5);
    }

    #[test]
    fn interpret_responses_blank_output_is_parse_error() {
        let body = r#"{"status":"completed","output":[]}"#;
        assert_eq!(
            interpret_responses_response(200, body).unwrap_err().code,
            LlmErrorCode::SchemaParseError
        );
    }

    #[test]
    fn interpret_responses_error_body() {
        let body = r#"{"error":{"message":"quota","code":"429"}}"#;
        let e = interpret_responses_response(200, body).unwrap_err();
        assert_eq!(e.code, LlmErrorCode::Unknown);
        assert!(e.error_detail.contains("quota"));
    }

    #[test]
    fn interpret_responses_http_errors() {
        assert_eq!(interpret_responses_response(401, "{}").unwrap_err().code, LlmErrorCode::AuthInvalid);
        assert_eq!(interpret_responses_response(500, "{}").unwrap_err().code, LlmErrorCode::Network);
        assert_eq!(interpret_responses_response(200, "").unwrap_err().code, LlmErrorCode::Network);
    }

    /// 非流式 responses **不检查 status**（incomplete 不报截断）
    #[test]
    fn interpret_responses_ignores_status_field() {
        let body = r#"{"status":"incomplete","output":[{"type":"message","content":[{"type":"output_text","text":"部分"}]}]}"#;
        assert_eq!(interpret_responses_response(200, body).unwrap().content, "部分");
    }

    // ---------------------------------------------------------------------
    // 消息校验
    // ---------------------------------------------------------------------

    #[test]
    fn validate_requires_system_first() {
        assert!(validate_messages(&msgs()).is_ok());
        assert!(validate_messages(&[]).is_err());
        assert!(validate_messages(&[ChatMessage::user("x", vec![])]).is_err());
        assert!(validate_messages(&[ChatMessage::assistant("a")]).is_err());
    }

    // ---------------------------------------------------------------------
    // 流式结束映射
    // ---------------------------------------------------------------------

    #[test]
    fn finish_stream_maps_truncated_and_empty() {
        let truncated = StreamState {
            finish_reason: Some("length".into()),
            ..Default::default()
        };
        assert_eq!(finish_stream(&truncated, true).unwrap_err().code, LlmErrorCode::OutputTruncated);

        let empty = StreamState::default();
        assert_eq!(finish_stream(&empty, true).unwrap_err().code, LlmErrorCode::SchemaParseError);

        let incomplete = StreamState {
            incomplete: true,
            ..Default::default()
        };
        assert_eq!(finish_stream(&incomplete, false).unwrap_err().code, LlmErrorCode::OutputTruncated);
    }

    #[test]
    fn finish_stream_ok_carries_content_and_usage() {
        let st = StreamState {
            content: " 正文 ".into(),
            usage: Some(crate::usage::Usage {
                total_tokens: 3,
                ..Default::default()
            }),
            ..Default::default()
        };
        let r = finish_stream(&st, true).unwrap();
        assert_eq!(r.content, "正文");
        assert_eq!(r.usage.unwrap().total_tokens, 3);
    }

    // ---------------------------------------------------------------------
    // emit
    // ---------------------------------------------------------------------

    #[test]
    fn emit_stops_on_done() {
        let mut seen: Vec<Signal> = Vec::new();
        let stop = emit(
            &mut |s| seen.push(s),
            vec![Signal::Content("a".into()), Signal::Done, Signal::Content("b".into())],
        );
        assert!(stop, "收到 Done 应停止读取");
        assert_eq!(seen.len(), 2, "Done 之后的信号不再转发");
    }

    #[test]
    fn emit_continues_without_done() {
        let mut seen: Vec<Signal> = Vec::new();
        let stop = emit(&mut |s| seen.push(s), vec![Signal::Content("a".into())]);
        assert!(!stop);
        assert_eq!(seen.len(), 1);
    }

    // ---------------------------------------------------------------------
    // 客户端构造
    // ---------------------------------------------------------------------

    #[test]
    fn client_constructs_with_timeouts() {
        // 只验证能构造（超时配置在 builder 内部，无需断言）
        let _ = LlmClient::new();
        let _ = LlmClient::default();
    }

    /// 请求体可序列化（端到端串联：build → encode → 再解析回来）
    #[test]
    fn request_body_serializes_end_to_end() {
        let req = build_chat_request(
            &profile(GLM, ApiProtocol::ChatCompletions, ThinkingLevel::Off),
            &msgs(),
            true,
            false,
            None,
            true,
        );
        let s = encode_body(&req).unwrap();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["model"], "test-model");
        assert_eq!(v["messages"][0]["role"], "system");
        assert_eq!(v["thinking"]["type"], "enabled");
        assert_eq!(v["reasoning_effort"], "low");
        assert_eq!(v["stream"], true);
        assert_eq!(v["temperature"], 0.0);
    }
}
