//! SSE 流式解析（两套协议）。
//!
//! 源：`civilcalc-android-v2/core/llm/LlmClient.kt`（`responsesChatStream` /
//! `chatCompletionsStream` 的读行循环）
//!
//! ## 为什么抽成纯状态机
//!
//! 源把「读行 → 解析 → 累积 → 回调」揉在网络循环里，只能靠真服务器测。
//! 这里把**解析与累积**抽成 [`StreamState`]（零 IO），网络层只负责喂行 ——
//! 于是全部流式语义都能用字符串直接测，不必起 mock server。
//!
//! ## 两套协议的流事件形状完全不同
//!
//! | | `/chat/completions` | `/responses` |
//! |---|---|---|
//! | 分帧 | `data: {json}`，末帧 `data: [DONE]` | 同左（也以 `[DONE]` 收尾） |
//! | 思考增量 | `choices[0].delta.reasoning_content`（兼容 `reasoning` / `thinking_content`） | `type=response.reasoning_text.delta` 的 `delta` |
//! | 正文增量 | `choices[0].delta.content` | `type=response.output_text.delta` 的 `delta` |
//! | 用量 | 末帧 `usage`（此时 `choices` 为空数组） | `type=response.completed` 里 `response.usage` |
//! | 截断 | `finish_reason=length` | `response.status=incomplete` |
//!
//! ## 🔴 回调拿到的是**累积全文**，不是增量
//!
//! 源每次都 `onThinking(reasoning.toString())`（StringBuilder 的全文）。
//! 前端据此**整体替换**显示，所以这里 [`Signal`] 携带的也是累积全文 ——
//! 别"顺手优化"成只发增量，会与前端约定不符。
//!
//! ## 🔴 截断判定**优先于**空内容判定
//!
//! `finish_reason=length` 时正文可能整段为空（思考 token 把 `max_tokens` 吃满），
//! 必须显式报「输出被截断」而不是笼统的「无有效响应内容」。见 [`chat_end`]。

use crate::protocol::ChatResult;
use crate::usage::Usage;

/// 从一行 SSE 取出 payload。
///
/// 非 `data:` 行返回 `None`（源 `if (!line.startsWith("data:")) continue`）。
/// 返回值已 `trim()`（源 `removePrefix("data:").trim()`）。
#[must_use]
pub fn sse_payload(line: &str) -> Option<&str> {
    line.strip_prefix("data:").map(str::trim)
}

/// `/chat/completions` 的单帧。
///
/// `allow(large_enum_variant)`：`Usage` 有 12 个字段，与其余变体体积差大；
/// 但这些都是**每帧即抛**的短命值（不进集合、不驻留），boxing 只会增加分配与噪音。
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatChunk {
    /// 用量帧（末帧，`choices` 为空数组）
    Usage(Usage),
    /// 增量帧
    Chunk {
        finish_reason: Option<String>,
        /// 思考增量（三种字段名任取其一）
        reasoning: Option<String>,
        /// 正文增量
        content: Option<String>,
    },
    /// 与本流程无关（心跳、其他事件、解析失败）
    Ignore,
}

/// 解析 `/chat/completions` 的一帧 payload。
#[must_use]
pub fn parse_chat_chunk(payload: &str) -> ChatChunk {
    let Ok(ev) = serde_json::from_str::<serde_json::Value>(payload) else {
        return ChatChunk::Ignore;
    };

    // usage 在最后一个 chunk（choices 为空数组时）
    if let Some(u) = ev.get("usage").filter(|v| !v.is_null()) {
        return match serde_json::from_value::<Usage>(u.clone()) {
            Ok(usage) => ChatChunk::Usage(usage),
            Err(_) => ChatChunk::Ignore,
        };
    }

    let Some(choice) = ev
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
    else {
        return ChatChunk::Ignore;
    };

    let finish_reason = choice
        .get("finish_reason")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let delta = choice.get("delta");
    // 兼容多种思考字段：reasoning_content (GLM/DeepSeek/OpenAI)、reasoning (MiMo 可能)、thinking_content
    let reasoning = delta
        .and_then(|d| d.get("reasoning_content"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            delta
                .and_then(|d| d.get("reasoning"))
                .and_then(|v| v.as_str())
        })
        .or_else(|| {
            delta
                .and_then(|d| d.get("thinking_content"))
                .and_then(|v| v.as_str())
        })
        .map(str::to_string);

    let content = delta
        .and_then(|d| d.get("content"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    if finish_reason.is_none() && reasoning.is_none() && content.is_none() {
        return ChatChunk::Ignore;
    }
    ChatChunk::Chunk {
        finish_reason,
        reasoning,
        content,
    }
}

/// `/responses` 的单帧。
///
/// 同 [`ChatChunk`]，`Completed` 带 `Usage` 而体积偏大 —— 短命值，不 boxing。
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponsesChunk {
    ThinkingDelta(String),
    ContentDelta(String),
    Completed {
        incomplete: bool,
        usage: Option<Usage>,
    },
    Ignore,
}

/// 解析 `/responses` 的一帧 payload。
#[must_use]
pub fn parse_responses_event(payload: &str) -> ResponsesChunk {
    let Ok(ev) = serde_json::from_str::<serde_json::Value>(payload) else {
        return ResponsesChunk::Ignore;
    };
    let kind = ev.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match kind {
        "response.reasoning_text.delta" => ev
            .get("delta")
            .and_then(|v| v.as_str())
            .map_or(ResponsesChunk::Ignore, |d| {
                ResponsesChunk::ThinkingDelta(d.to_string())
            }),
        "response.output_text.delta" => ev
            .get("delta")
            .and_then(|v| v.as_str())
            .map_or(ResponsesChunk::Ignore, |d| {
                ResponsesChunk::ContentDelta(d.to_string())
            }),
        "response.completed" => {
            let item = ev.get("response");
            let incomplete = item
                .and_then(|i| i.get("status"))
                .and_then(|v| v.as_str())
                == Some("incomplete");
            let usage = item
                .and_then(|i| i.get("usage"))
                .filter(|v| !v.is_null())
                .and_then(|u| serde_json::from_value::<Usage>(u.clone()).ok());
            ResponsesChunk::Completed { incomplete, usage }
        }
        _ => ResponsesChunk::Ignore,
    }
}

/// 流式过程中向前端发出的信号。
///
/// ⚠️ `Thinking` / `Content` 携带的是**累积全文**（与源 `onThinking(reasoning.toString())` 一致）。
///
/// 同 [`ChatChunk`]，`Usage` 变体偏大 —— 短命值，不 boxing。
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    Thinking(String),
    Content(String),
    Usage(Usage),
    /// 收到 `data: [DONE]`
    Done,
}

/// 流式累积状态（零 IO，可单测）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamState {
    /// 思考过程累积
    pub reasoning: String,
    /// 正文累积
    pub content: String,
    pub usage: Option<Usage>,
    /// `/chat/completions` 的 `finish_reason`
    pub finish_reason: Option<String>,
    /// `/responses` 的 `status=incomplete`
    pub incomplete: bool,
    /// 是否已收到 `[DONE]`
    pub done: bool,
}

impl StreamState {
    /// 喂一行 `/chat/completions` 的 SSE，返回本行产生的信号。
    #[must_use]
    pub fn feed_chat(&mut self, line: &str) -> Vec<Signal> {
        let Some(payload) = sse_payload(line) else {
            return Vec::new();
        };
        if payload == "[DONE]" {
            self.done = true;
            return vec![Signal::Done];
        }
        match parse_chat_chunk(payload) {
            ChatChunk::Usage(u) => {
                // 源在流式路径也调 `.normalized()`（跨协议字段归一化），保持一致
                let u = u.normalized();
                self.usage = Some(u.clone());
                vec![Signal::Usage(u)]
            }
            ChatChunk::Chunk {
                finish_reason,
                reasoning,
                content,
            } => {
                let mut out = Vec::new();
                if let Some(f) = finish_reason {
                    self.finish_reason = Some(f);
                }
                if let Some(r) = reasoning {
                    self.reasoning.push_str(&r);
                    out.push(Signal::Thinking(self.reasoning.clone()));
                }
                if let Some(c) = content {
                    self.content.push_str(&c);
                    out.push(Signal::Content(self.content.clone()));
                }
                out
            }
            ChatChunk::Ignore => Vec::new(),
        }
    }

    /// 喂一行 `/responses` 的 SSE，返回本行产生的信号。
    #[must_use]
    pub fn feed_responses(&mut self, line: &str) -> Vec<Signal> {
        let Some(payload) = sse_payload(line) else {
            return Vec::new();
        };
        if payload == "[DONE]" {
            self.done = true;
            return vec![Signal::Done];
        }
        match parse_responses_event(payload) {
            ResponsesChunk::ThinkingDelta(d) => {
                self.reasoning.push_str(&d);
                vec![Signal::Thinking(self.reasoning.clone())]
            }
            ResponsesChunk::ContentDelta(d) => {
                self.content.push_str(&d);
                vec![Signal::Content(self.content.clone())]
            }
            ResponsesChunk::Completed { incomplete, usage } => {
                if incomplete {
                    self.incomplete = true;
                }
                let mut out = Vec::new();
                if let Some(u) = usage {
                    let u = u.normalized();
                    self.usage = Some(u.clone());
                    out.push(Signal::Usage(u));
                }
                out
            }
            ResponsesChunk::Ignore => Vec::new(),
        }
    }

    /// 正文（已 trim）。
    #[must_use]
    pub fn trimmed_content(&self) -> String {
        self.content.trim().to_string()
    }
}

/// 流式结束后的判定结果（纯函数产物，网络层据此映射 [`crate::error::LlmError`]）。
///
/// 同 [`ChatChunk`]：`Ok` 携带 `ChatResult`（含 `Usage`）而体积偏大 —— 一次性返回值，不 boxing。
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEnd {
    Ok(ChatResult),
    /// `finish_reason=length` 或 `status=incomplete`
    Truncated,
    /// 无有效内容
    Empty,
}

/// `/chat/completions` 的结束判定。
///
/// 🔴 **顺序铁律**：先判截断，再判空 —— `finish_reason=length` 时正文可能整段为空，
/// 报「被截断」比报「无法解析」对用户有用得多。
#[must_use]
pub fn chat_end(state: &StreamState) -> StreamEnd {
    if state.finish_reason.as_deref() == Some("length") {
        return StreamEnd::Truncated;
    }
    let text = state.trimmed_content();
    if text.is_empty() {
        return StreamEnd::Empty;
    }
    StreamEnd::Ok(ChatResult {
        content: text,
        usage: state.usage.clone(),
    })
}

/// `/responses` 的结束判定（同样先判截断）。
#[must_use]
pub fn responses_end(state: &StreamState) -> StreamEnd {
    if state.incomplete {
        return StreamEnd::Truncated;
    }
    let text = state.trimmed_content();
    if text.is_empty() {
        return StreamEnd::Empty;
    }
    StreamEnd::Ok(ChatResult {
        content: text,
        usage: state.usage.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn feed_all(state: &mut StreamState, protocol: &str, lines: &[&str]) -> Vec<Signal> {
        let mut all = Vec::new();
        for l in lines {
            let mut sig = if protocol == "chat" {
                state.feed_chat(l)
            } else {
                state.feed_responses(l)
            };
            all.append(&mut sig);
        }
        all
    }

    // ---------------------------------------------------------------------
    // sse_payload
    // ---------------------------------------------------------------------

    #[test]
    fn payload_strips_data_prefix_and_trims() {
        assert_eq!(sse_payload("data: {\"a\":1}"), Some("{\"a\":1}"));
        assert_eq!(sse_payload("data:   {\"a\":1}  "), Some("{\"a\":1}"));
        assert_eq!(sse_payload("data:[DONE]"), Some("[DONE]"));
    }

    #[test]
    fn payload_none_for_non_data_lines() {
        assert_eq!(sse_payload(""), None);
        assert_eq!(sse_payload(": keep-alive"), None);
        assert_eq!(sse_payload("event: message"), None);
    }

    // ---------------------------------------------------------------------
    // chat/completions 帧解析
    // ---------------------------------------------------------------------

    #[test]
    fn chat_chunk_content_delta() {
        let c = parse_chat_chunk(r#"{"choices":[{"delta":{"content":"你好"}}]}"#);
        assert_eq!(
            c,
            ChatChunk::Chunk {
                finish_reason: None,
                reasoning: None,
                content: Some("你好".into())
            }
        );
    }

    #[test]
    fn chat_chunk_reasoning_three_field_names() {
        for field in ["reasoning_content", "reasoning", "thinking_content"] {
            let p = format!(r#"{{"choices":[{{"delta":{{"{field}":"想"}}}}]}}"#);
            match parse_chat_chunk(&p) {
                ChatChunk::Chunk { reasoning, .. } => {
                    assert_eq!(reasoning.as_deref(), Some("想"), "字段 {field} 未识别")
                }
                other => panic!("{field}: 期望 Chunk，得到 {other:?}"),
            }
        }
    }

    #[test]
    fn chat_chunk_reasoning_content_wins_over_reasoning() {
        let c = parse_chat_chunk(
            r#"{"choices":[{"delta":{"reasoning_content":"A","reasoning":"B"}}]}"#,
        );
        match c {
            ChatChunk::Chunk { reasoning, .. } => assert_eq!(reasoning.as_deref(), Some("A")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn chat_chunk_usage_frame() {
        let c = parse_chat_chunk(r#"{"choices":[],"usage":{"prompt_tokens":9,"completion_tokens":1,"total_tokens":10}}"#);
        match c {
            ChatChunk::Usage(u) => assert_eq!(u.total_tokens, 10),
            other => panic!("{other:?}"),
        }
    }

    /// usage 帧优先于 choices（源里 usage 分支 `continue`）
    #[test]
    fn chat_chunk_usage_takes_priority_over_choices() {
        let c = parse_chat_chunk(
            r#"{"choices":[{"delta":{"content":"x"}}],"usage":{"prompt_tokens":1}}"#,
        );
        assert!(matches!(c, ChatChunk::Usage(_)), "带 usage 的帧按用量帧处理");
    }

    #[test]
    fn chat_chunk_finish_reason_only() {
        let c = parse_chat_chunk(r#"{"choices":[{"finish_reason":"length"}]}"#);
        match c {
            ChatChunk::Chunk {
                finish_reason,
                content,
                reasoning,
            } => {
                assert_eq!(finish_reason.as_deref(), Some("length"));
                assert!(content.is_none() && reasoning.is_none());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn chat_chunk_ignores_garbage_and_empty() {
        assert_eq!(parse_chat_chunk("not json"), ChatChunk::Ignore);
        assert_eq!(parse_chat_chunk("{}"), ChatChunk::Ignore);
        assert_eq!(parse_chat_chunk(r#"{"choices":[]}"#), ChatChunk::Ignore);
        // delta 里没有任何有效字段
        assert_eq!(
            parse_chat_chunk(r#"{"choices":[{"delta":{"role":"assistant"}}]}"#),
            ChatChunk::Ignore
        );
    }

    // ---------------------------------------------------------------------
    // responses 帧解析
    // ---------------------------------------------------------------------

    #[test]
    fn responses_thinking_delta() {
        let c = parse_responses_event(r#"{"type":"response.reasoning_text.delta","delta":"想"}"#);
        assert_eq!(c, ResponsesChunk::ThinkingDelta("想".into()));
    }

    #[test]
    fn responses_content_delta() {
        let c = parse_responses_event(r#"{"type":"response.output_text.delta","delta":"答"}"#);
        assert_eq!(c, ResponsesChunk::ContentDelta("答".into()));
    }

    #[test]
    fn responses_completed_with_usage() {
        let c = parse_responses_event(
            r#"{"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":5,"output_tokens":2,"total_tokens":7}}}"#,
        );
        match c {
            ResponsesChunk::Completed { incomplete, usage } => {
                assert!(!incomplete);
                assert_eq!(usage.unwrap().total_tokens, 7);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn responses_completed_incomplete() {
        let c = parse_responses_event(
            r#"{"type":"response.completed","response":{"status":"incomplete"}}"#,
        );
        match c {
            ResponsesChunk::Completed { incomplete, usage } => {
                assert!(incomplete);
                assert!(usage.is_none());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn responses_ignores_other_events() {
        assert_eq!(
            parse_responses_event(r#"{"type":"response.created"}"#),
            ResponsesChunk::Ignore
        );
        assert_eq!(parse_responses_event("garbage"), ResponsesChunk::Ignore);
        assert_eq!(parse_responses_event("{}"), ResponsesChunk::Ignore);
    }

    // ---------------------------------------------------------------------
    // 累积：回调收到的是**全文**
    // ---------------------------------------------------------------------

    #[test]
    fn chat_signals_carry_cumulative_text() {
        let mut st = StreamState::default();
        let sig = feed_all(
            &mut st,
            "chat",
            &[
                r#"data: {"choices":[{"delta":{"content":"A"}}]}"#,
                r#"data: {"choices":[{"delta":{"content":"B"}}]}"#,
                r#"data: {"choices":[{"delta":{"content":"C"}}]}"#,
            ],
        );
        let contents: Vec<_> = sig
            .iter()
            .filter_map(|s| match s {
                Signal::Content(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(contents, ["A", "AB", "ABC"], "每次都是累积全文，不是增量");
        assert_eq!(st.content, "ABC");
    }

    #[test]
    fn responses_signals_carry_cumulative_text() {
        let mut st = StreamState::default();
        feed_all(
            &mut st,
            "responses",
            &[
                r#"data: {"type":"response.reasoning_text.delta","delta":"思"}"#,
                r#"data: {"type":"response.reasoning_text.delta","delta":"考"}"#,
                r#"data: {"type":"response.output_text.delta","delta":"正"}"#,
                r#"data: {"type":"response.output_text.delta","delta":"文"}"#,
            ],
        );
        assert_eq!(st.reasoning, "思考");
        assert_eq!(st.content, "正文");
    }

    #[test]
    fn thinking_and_content_are_separate_streams() {
        let mut st = StreamState::default();
        let sig = feed_all(
            &mut st,
            "chat",
            &[
                r#"data: {"choices":[{"delta":{"reasoning_content":"想"}}]}"#,
                r#"data: {"choices":[{"delta":{"content":"答"}}]}"#,
            ],
        );
        assert_eq!(
            sig,
            vec![
                Signal::Thinking("想".into()),
                Signal::Content("答".into())
            ]
        );
    }

    #[test]
    fn done_signal_and_flag() {
        let mut st = StreamState::default();
        let sig = st.feed_chat("data: [DONE]");
        assert_eq!(sig, vec![Signal::Done]);
        assert!(st.done);
    }

    #[test]
    fn non_data_lines_produce_nothing() {
        let mut st = StreamState::default();
        assert!(st.feed_chat("").is_empty());
        assert!(st.feed_chat(": ping").is_empty());
        assert!(st.feed_responses("event: x").is_empty());
        assert_eq!(st, StreamState::default(), "状态不应被无关行改动");
    }

    #[test]
    fn usage_signal_emitted_and_stored() {
        let mut st = StreamState::default();
        let sig = st.feed_chat(
            r#"data: {"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}"#,
        );
        assert_eq!(sig.len(), 1);
        assert!(matches!(sig[0], Signal::Usage(_)));
        assert_eq!(st.usage.as_ref().unwrap().total_tokens, 4);
    }

    /// 真实一条流：思考 → 正文 → 用量 → DONE
    #[test]
    fn full_chat_stream_sequence() {
        let mut st = StreamState::default();
        let raw = [
            r#"data: {"choices":[{"delta":{"reasoning_content":"先想"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"{"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"}"}}]}"#,
            r#"data: {"choices":[{"finish_reason":"stop","delta":{}}]}"#,
            r#"data: {"choices":[],"usage":{"prompt_tokens":100,"completion_tokens":20,"total_tokens":120}}"#,
            "data: [DONE]",
        ];
        feed_all(&mut st, "chat", &raw);
        assert_eq!(st.reasoning, "先想");
        assert_eq!(st.content, "{}");
        assert_eq!(st.finish_reason.as_deref(), Some("stop"));
        assert_eq!(st.usage.as_ref().unwrap().total_tokens, 120);
        assert!(st.done);
        assert_eq!(
            chat_end(&st),
            StreamEnd::Ok(ChatResult {
                content: "{}".into(),
                usage: st.usage.clone()
            })
        );
    }

    // ---------------------------------------------------------------------
    // 结束判定：截断优先于空内容
    // ---------------------------------------------------------------------

    /// 🔴 `finish_reason=length` 且正文为空 → 必须报截断，不是"无内容"
    #[test]
    fn chat_end_truncated_wins_over_empty() {
        let st = StreamState {
            finish_reason: Some("length".into()),
            ..Default::default()
        };
        assert_eq!(chat_end(&st), StreamEnd::Truncated);
    }

    #[test]
    fn chat_end_empty_when_blank() {
        let st = StreamState::default();
        assert_eq!(chat_end(&st), StreamEnd::Empty);
        let blank = StreamState {
            content: "   \n ".into(),
            ..Default::default()
        };
        assert_eq!(chat_end(&blank), StreamEnd::Empty, "纯空白算空");
    }

    #[test]
    fn chat_end_trims_content() {
        let st = StreamState {
            content: "  hi  ".into(),
            ..Default::default()
        };
        match chat_end(&st) {
            StreamEnd::Ok(r) => assert_eq!(r.content, "hi"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn responses_end_incomplete_wins_over_empty() {
        let st = StreamState {
            incomplete: true,
            ..Default::default()
        };
        assert_eq!(responses_end(&st), StreamEnd::Truncated);
    }

    #[test]
    fn responses_end_empty_when_blank() {
        assert_eq!(responses_end(&StreamState::default()), StreamEnd::Empty);
    }

    /// `finish_reason=stop` 不算截断
    #[test]
    fn chat_end_stop_is_ok() {
        let st = StreamState {
            content: "x".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        };
        assert!(matches!(chat_end(&st), StreamEnd::Ok(_)));
    }

    /// 流式**不**判 `content_filter`（源只在非流式判）
    #[test]
    fn chat_end_ignores_content_filter() {
        let st = StreamState {
            content: "x".into(),
            finish_reason: Some("content_filter".into()),
            ..Default::default()
        };
        assert!(matches!(chat_end(&st), StreamEnd::Ok(_)), "流式路径不处理 content_filter");
    }

    #[test]
    fn end_result_carries_usage() {
        let st = StreamState {
            content: "x".into(),
            usage: Some(Usage {
                total_tokens: 9,
                ..Default::default()
            }),
            ..Default::default()
        };
        match chat_end(&st) {
            StreamEnd::Ok(r) => assert_eq!(r.usage.unwrap().total_tokens, 9),
            other => panic!("{other:?}"),
        }
    }

    // ---------------------------------------------------------------------
    // 解析健壮性
    // ---------------------------------------------------------------------

    /// 非法 JSON 的帧被跳过，不影响后续帧
    #[test]
    fn bad_frame_does_not_break_stream() {
        let mut st = StreamState::default();
        feed_all(
            &mut st,
            "chat",
            &[
                "data: {oops",
                r#"data: {"choices":[{"delta":{"content":"ok"}}]}"#,
            ],
        );
        assert_eq!(st.content, "ok");
    }

    /// 只有 `choices` 没有 `delta` 的帧不 panic
    #[test]
    fn choice_without_delta_is_safe() {
        let mut st = StreamState::default();
        assert!(st.feed_chat(r#"data: {"choices":[{}]}"#).is_empty());
    }

    /// 多模态 parts 形态的 delta.content（数组）被安全忽略（不 panic）
    #[test]
    fn array_delta_content_is_ignored() {
        let mut st = StreamState::default();
        let sig = st.feed_chat(r#"data: {"choices":[{"delta":{"content":[{"type":"text","text":"x"}]}}]}"#);
        assert!(sig.is_empty(), "非字符串 content 不处理");
        assert_eq!(st.content, "");
    }

    #[test]
    fn json_helper_shape_matches() {
        // 与 serde_json::json! 的等价性自检（防止手写串写错）
        let expected = json!({"choices":[{"delta":{"content":"A"}}]});
        let parsed: serde_json::Value =
            serde_json::from_str(r#"{"choices":[{"delta":{"content":"A"}}]}"#).unwrap();
        assert_eq!(expected, parsed);
    }
}
