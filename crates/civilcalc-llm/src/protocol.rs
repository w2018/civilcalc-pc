//! 两套协议的请求 / 响应 DTO，以及 `ChatMessage` 的**多模态序列化**。
//!
//! 源：`civilcalc-android-v2/core/llm/LlmClient.kt`（DTO 部分 + 文件末尾的
//! `buildChatContent` / `toResponseInputItem` / `ChatMessageSerializer`）
//!
//! ## 两套协议
//!
//! | | `/chat/completions` | `/responses` |
//! |---|---|---|
//! | 请求体 | [`OpenAiChatRequest`] | [`DeepseekResponseRequest`] |
//! | 系统提示 | 放在 `messages[0]`（role=system） | 提到顶层 `instructions` |
//! | 输入 | `messages` 原样 | `input[]`，**user/assistant 都要保留** |
//! | JSON 模式 | `response_format:{type}` | `text:{format:{type}}` |
//! | 思考档位 | `thinking.type` + `reasoning_effort` | `reasoning.effort` |
//! | 流式标记 | `stream` + `stream_options.include_usage` | `stream` |
//!
//! ## 🔴 `ChatMessage` 的 content 有两种形状
//!
//! - **无图** → `"content": "纯字符串"`（兼容不支持 content 数组的 provider）
//! - **有图** → `"content": [{"type":"text",...},{"type":"image_url",{"url":...}}]`
//!
//! 所以 `ChatMessage` 不能派生 `Serialize`，必须手写（见 [`ChatMessage`] 的 impl）。
//!
//! ## 🔴 序列化取舍（与源 kotlinx 的两处差异，均为**有意**）
//!
//! 1. **`temperature` / `stream` 总是序列化**。源用 `Json{}` 默认
//!    `encodeDefaults=false`，理论上会把等于默认值的字段省掉 —— 那会让
//!    `temperature:0.0` 根本不发出去，服务端用自己的默认温度（多为 1.0），
//!    公式解析结果变得不确定。`docs/04-数据契约.md` §6.4 把 `temperature`
//!    写成必填的「硬编码 0.0」，故**按契约总是发送**。
//! 2. **`Option` 字段为 `None` 时省略**（源 `explicitNulls=false` 的效果）——
//!    发 `"response_format":null` 会被部分 provider 拒。
//!
//! 另有一处**防御性放宽**：`OpenAiChatResponse.choices` 加了 `default`。
//! 源把 `choices` 设为必填，而 API 报错时响应体往往只有 `{"error":{...}}`
//! （没有 `choices`）→ 反序列化会先抛异常，**永远走不到 `error` 分支**，
//! 真实错误被误报成 `SCHEMA_PARSE_ERROR`。加 `default` 才能正确上报错误。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::usage::Usage;

// =============================================================================
// 消息
// =============================================================================

/// 一条对话消息。
///
/// `images` 只在**用户**消息上有意义；非空时 `content` 序列化为多模态 parts 数组。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// `system` / `user` / `assistant`
    pub role: String,
    pub content: String,
    /// 图片 data URL 列表（仅用户消息携带）
    pub images: Vec<String>,
}

impl Default for ChatMessage {
    /// 反序列化时角色缺失的兜底值（与源 `?: "assistant"` 一致）。
    fn default() -> Self {
        Self {
            role: "assistant".to_string(),
            content: String::new(),
            images: Vec::new(),
        }
    }
}

impl ChatMessage {
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: content.into(),
            images: Vec::new(),
        }
    }

    #[must_use]
    pub fn user(content: impl Into<String>, images: Vec<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
            images,
        }
    }

    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
            images: Vec::new(),
        }
    }

    #[must_use]
    pub fn is_assistant(&self) -> bool {
        self.role == "assistant"
    }
}

/// OpenAI 视觉多模态 content：无图 → 纯字符串；有图 → parts 数组。
#[must_use]
pub fn build_chat_content(text: &str, images: &[String]) -> Value {
    if images.is_empty() {
        return Value::String(text.to_string());
    }
    let mut parts = Vec::with_capacity(images.len() + 1);
    parts.push(json!({ "type": "text", "text": text }));
    for url in images {
        parts.push(json!({ "type": "image_url", "image_url": { "url": url } }));
    }
    Value::Array(parts)
}

impl Serialize for ChatMessage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serde_json::Map::new();
        map.insert("role".to_string(), Value::String(self.role.clone()));
        map.insert(
            "content".to_string(),
            build_chat_content(&self.content, &self.images),
        );
        Value::Object(map).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ChatMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = Value::deserialize(deserializer)?;
        let role = v
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("assistant")
            .to_string();
        // 响应的 content 恒为字符串；数组形态（回显请求）时只取 text 项拼接
        let content = match v.get("content") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<String>(),
            _ => String::new(),
        };
        Ok(Self {
            role,
            content,
            images: Vec::new(),
        })
    }
}

/// Responses API 的输入项。
///
/// 🔴 **user 与 assistant 历史都必须保留**，且各自 role 正确
/// （assistant 的文本项类型是 `output_text`，user 是 `input_text`；图片只属于 user 轮）。
/// 只发 user 会让模型收到一串连续提问，从而**逐个作答**并浪费输出 token。
#[must_use]
pub fn to_response_input_item(msg: &ChatMessage) -> ResponseInputItem {
    let is_assistant = msg.is_assistant();
    let mut content = Vec::with_capacity(1 + msg.images.len());
    content.push(ResponseInputContent {
        r#type: if is_assistant { "output_text" } else { "input_text" }.to_string(),
        text: Some(msg.content.clone()),
        image_url: None,
    });
    if !is_assistant {
        for url in &msg.images {
            content.push(ResponseInputContent {
                r#type: "input_image".to_string(),
                text: None,
                image_url: Some(url.clone()),
            });
        }
    }
    ResponseInputItem {
        role: if is_assistant { "assistant" } else { "user" }.to_string(),
        content,
    }
}

// =============================================================================
// 请求 DTO
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResponseFormat {
    #[serde(rename = "type")]
    pub r#type: String,
}

/// `thinking.type`（chat/completions 的开关式控制）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThinkingConfig {
    #[serde(rename = "type")]
    pub r#type: String,
}

/// `/responses` 的思考档位：`none`（关闭）/ `low` / `high` / `max`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReasoningConfig {
    pub effort: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamOptions {
    #[serde(rename = "include_usage")]
    pub include_usage: bool,
}

impl Default for StreamOptions {
    fn default() -> Self {
        Self {
            include_usage: true,
        }
    }
}

/// 联网搜索工具。各家 JSON 形状不同，见 [`web_search_tool`]。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebSearchTool {
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_search: Option<WebSearchConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebSearchConfig {
    #[serde(default = "default_true")]
    pub enable: bool,
    #[serde(default = "default_max_keyword")]
    pub max_keyword: i32,
    #[serde(default = "default_true")]
    pub force_search: bool,
    #[serde(default = "default_limit")]
    pub limit: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_location: Option<UserLocation>,
}

fn default_true() -> bool {
    true
}
fn default_max_keyword() -> i32 {
    3
}
fn default_limit() -> i32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserLocation {
    #[serde(rename = "type", default = "default_approximate")]
    pub r#type: String,
    #[serde(default = "default_china")]
    pub country: String,
    #[serde(default = "default_hubei")]
    pub region: Option<String>,
    #[serde(default = "default_wuhan")]
    pub city: Option<String>,
}

fn default_approximate() -> String {
    "approximate".to_string()
}
fn default_china() -> String {
    "China".to_string()
}
fn default_hubei() -> Option<String> {
    Some("Hubei".to_string())
}
fn default_wuhan() -> Option<String> {
    Some("Wuhan".to_string())
}

impl Default for UserLocation {
    fn default() -> Self {
        Self {
            r#type: default_approximate(),
            country: default_china(),
            region: default_hubei(),
            city: default_wuhan(),
        }
    }
}

/// 各家联网工具的 JSON 形状不同（源实测）：
///
/// | baseUrl 含 | 形状 |
/// |---|---|
/// | `bigmodel`（GLM） | `{type:"web_search", web_search:{enable:true}}` —— 需 **非空对象** |
/// | `xiaomimimo`（小米） | `{type:"web_search", web_search:{enable,max_keyword:3,force_search:true,limit:1,user_location:{...}}}` |
/// | 其余 | `{type:"web_search"}` —— **不带** `web_search` |
#[must_use]
pub fn web_search_tool(base_url: &str) -> WebSearchTool {
    if base_url.contains("bigmodel") {
        WebSearchTool {
            r#type: "web_search".to_string(),
            web_search: Some(WebSearchConfig {
                enable: true,
                max_keyword: 3,
                force_search: true,
                limit: 1,
                user_location: None,
            }),
        }
    } else if base_url.contains("xiaomimimo") {
        WebSearchTool {
            r#type: "web_search".to_string(),
            web_search: Some(WebSearchConfig {
                enable: true,
                max_keyword: 3,
                force_search: true,
                limit: 1,
                user_location: Some(UserLocation::default()),
            }),
        }
    } else {
        WebSearchTool {
            r#type: "web_search".to_string(),
            web_search: None,
        }
    }
}

/// `/chat/completions` 请求体。
///
/// 字段名与 API 一致（snake_case），故无需 `rename_all`；
/// `response_format` / `max_tokens` / `tool_choice` / `stream_options` /
/// `reasoning_effort` 本就是 snake_case，与 Rust 字段名相同。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenAiChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    /// **硬编码 0.0**，且总是发送（见模块文档的取舍说明）
    pub temperature: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<WebSearchTool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<String>,
    pub stream: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingConfig>,
    /// DeepSeek/GLM 的思考档位（low/high/max）；与 `thinking.type=disabled` 互斥
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

impl OpenAiChatRequest {
    /// 非流式请求体（`temperature=0.0`，`stream=false`）。
    #[must_use]
    pub fn new(model: impl Into<String>, messages: Vec<ChatMessage>) -> Self {
        Self {
            model: model.into(),
            messages,
            response_format: None,
            temperature: 0.0,
            max_tokens: None,
            tools: None,
            tool_choice: None,
            stream: false,
            stream_options: None,
            thinking: None,
            reasoning_effort: None,
        }
    }

    /// 开流式（并带上 `stream_options.include_usage`，否则拿不到用量）。
    #[must_use]
    pub fn streaming(mut self) -> Self {
        self.stream = true;
        self.stream_options = Some(StreamOptions::default());
        self
    }

    #[must_use]
    pub fn json_mode(mut self, on: bool) -> Self {
        self.response_format = if on {
            Some(ResponseFormat {
                r#type: "json_object".to_string(),
            })
        } else {
            None
        };
        self
    }

    #[must_use]
    pub fn with_web_search(mut self, base_url: &str, on: bool) -> Self {
        if on {
            self.tools = Some(vec![web_search_tool(base_url)]);
            self.tool_choice = Some("auto".to_string());
        }
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResponseTextConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<ResponseFormat>,
}

/// `/responses` 请求体。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeepseekResponseRequest {
    pub model: String,
    /// 系统提示提到顶层（不是 `messages[0]`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    pub input: Vec<ResponseInputItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<WebSearchTool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<ResponseTextConfig>,
    pub stream: bool,
    /// 该协议**无** `thinking.type`，只有 `reasoning.effort`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResponseInputItem {
    pub role: String,
    #[serde(default)]
    pub content: Vec<ResponseInputContent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResponseInputContent {
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// `type=input_image` 时的图片 URL（data URL）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

// =============================================================================
// 响应 DTO
// =============================================================================

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ResponseError {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ResponseOutputContent {
    #[serde(rename = "type", default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ResponseOutputItem {
    #[serde(rename = "type", default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub content: Option<Vec<ResponseOutputContent>>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DeepseekResponseResponse {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub error: Option<ResponseError>,
    #[serde(default)]
    pub output: Option<Vec<ResponseOutputItem>>,
    #[serde(default)]
    pub usage: Option<Usage>,
}

impl DeepseekResponseResponse {
    /// 从 `output[]` 提取正文：只取 `type=="message"` 项下 `type=="output_text"` 的文本。
    #[must_use]
    pub fn output_text(&self) -> String {
        self.output
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|it| it.r#type.as_deref() == Some("message"))
            .flat_map(|it| it.content.as_deref().unwrap_or_default())
            .filter(|c| c.r#type.as_deref() == Some("output_text"))
            .filter_map(|c| c.text.as_deref())
            .collect::<String>()
            .trim()
            .to_string()
    }

    /// 流式 `response.completed` 事件里 `status == "incomplete"` 即被截断。
    #[must_use]
    pub fn is_incomplete(&self) -> bool {
        self.status.as_deref() == Some("incomplete")
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct LlmErrorBody {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(rename = "type", default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Choice {
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub message: ChatMessage,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OpenAiChatResponse {
    /// ⚠️ `default`：API 报错时响应体常无 `choices`，加 default 才能走到 `error` 分支
    #[serde(default)]
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub error: Option<LlmErrorBody>,
    #[serde(default)]
    pub usage: Option<Usage>,
}

impl OpenAiChatResponse {
    #[must_use]
    pub fn first_choice(&self) -> Option<&Choice> {
        self.choices.first()
    }

    #[must_use]
    pub fn finish_reason(&self) -> Option<&str> {
        self.first_choice().and_then(|c| c.finish_reason.as_deref())
    }

    #[must_use]
    pub fn content(&self) -> Option<&str> {
        self.first_choice().map(|c| c.message.content.as_str())
    }
}

/// 一次调用的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatResult {
    pub content: String,
    /// 厂商未返回时为 `None`（**不臆造**）
    pub usage: Option<Usage>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------
    // ChatMessage 序列化：两种 content 形状
    // ---------------------------------------------------------------------

    #[test]
    fn content_is_plain_string_without_images() {
        assert_eq!(build_chat_content("你好", &[]), json!("你好"));
    }

    #[test]
    fn content_is_parts_array_with_images() {
        let imgs = vec!["data:image/jpeg;base64,AAA".to_string()];
        let v = build_chat_content("看图", &imgs);
        assert_eq!(
            v,
            json!([
                {"type":"text","text":"看图"},
                {"type":"image_url","image_url":{"url":"data:image/jpeg;base64,AAA"}}
            ])
        );
    }

    #[test]
    fn message_serializes_role_and_content() {
        let m = ChatMessage::user("hi", vec![]);
        assert_eq!(serde_json::to_value(&m).unwrap(), json!({"role":"user","content":"hi"}));
    }

    #[test]
    fn message_serializes_multimodal() {
        let m = ChatMessage::user("hi", vec!["data:image/png;base64,BB".to_string()]);
        let v = serde_json::to_value(&m).unwrap();
        assert!(v["content"].is_array());
        assert_eq!(v["content"][0]["type"], "text");
        assert_eq!(v["content"][1]["image_url"]["url"], "data:image/png;base64,BB");
    }

    /// `images` **不进 JSON**（已折进 content）
    #[test]
    fn message_does_not_leak_images_field() {
        let m = ChatMessage::user("hi", vec!["x".to_string()]);
        let v = serde_json::to_value(&m).unwrap();
        assert!(v.get("images").is_none());
    }

    // ---------------------------------------------------------------------
    // ChatMessage 反序列化
    // ---------------------------------------------------------------------

    #[test]
    fn message_deserializes_string_content() {
        let m: ChatMessage = serde_json::from_value(json!({"role":"assistant","content":"ok"})).unwrap();
        assert_eq!(m.role, "assistant");
        assert_eq!(m.content, "ok");
        assert!(m.images.is_empty(), "响应不带图");
    }

    #[test]
    fn message_deserializes_array_content() {
        let m: ChatMessage = serde_json::from_value(json!({
            "role":"assistant",
            "content":[{"type":"text","text":"AB"},{"type":"text","text":"CD"}]
        }))
        .unwrap();
        assert_eq!(m.content, "ABCD", "数组形态取 text 拼接");
    }

    #[test]
    fn message_missing_role_defaults_to_assistant() {
        let m: ChatMessage = serde_json::from_value(json!({"content":"x"})).unwrap();
        assert_eq!(m.role, "assistant");
    }

    #[test]
    fn message_missing_content_is_empty() {
        let m: ChatMessage = serde_json::from_value(json!({"role":"user"})).unwrap();
        assert_eq!(m.content, "");
    }

    // ---------------------------------------------------------------------
    // to_response_input_item
    // ---------------------------------------------------------------------

    #[test]
    fn response_item_user_keeps_images() {
        let m = ChatMessage::user("看", vec!["data:image/png;base64,X".to_string()]);
        let it = to_response_input_item(&m);
        assert_eq!(it.role, "user");
        assert_eq!(it.content.len(), 2);
        assert_eq!(it.content[0].r#type, "input_text");
        assert_eq!(it.content[0].text.as_deref(), Some("看"));
        assert_eq!(it.content[1].r#type, "input_image");
        assert_eq!(it.content[1].image_url.as_deref(), Some("data:image/png;base64,X"));
        assert!(it.content[1].text.is_none());
    }

    /// 🔴 assistant 轮用 `output_text` 且**不带图**
    #[test]
    fn response_item_assistant_uses_output_text() {
        let m = ChatMessage::assistant("答");
        let it = to_response_input_item(&m);
        assert_eq!(it.role, "assistant");
        assert_eq!(it.content.len(), 1);
        assert_eq!(it.content[0].r#type, "output_text");
        assert_eq!(it.content[0].text.as_deref(), Some("答"));
    }

    /// 🔴 历史必须保留 assistant 轮（只发 user 会让模型逐个作答）
    #[test]
    fn response_input_keeps_conversation_shape() {
        let msgs = [
            ChatMessage::system("sys"),
            ChatMessage::user("q1", vec![]),
            ChatMessage::assistant("a1"),
            ChatMessage::user("q2", vec![]),
        ];
        let input: Vec<_> = msgs
            .iter()
            .filter(|m| m.role != "system")
            .map(to_response_input_item)
            .collect();
        assert_eq!(input.len(), 3);
        assert_eq!(input[0].role, "user");
        assert_eq!(input[1].role, "assistant");
        assert_eq!(input[2].role, "user");
    }

    // ---------------------------------------------------------------------
    // 联网工具形状
    // ---------------------------------------------------------------------

    #[test]
    fn web_search_tool_glm_needs_nonempty_object() {
        let t = web_search_tool("https://open.bigmodel.cn/api/paas/v4");
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["type"], "web_search");
        assert!(v["web_search"].is_object(), "GLM 需非空对象");
        assert_eq!(v["web_search"]["enable"], true);
    }

    #[test]
    fn web_search_tool_mimo_has_full_params() {
        let t = web_search_tool("https://api.xiaomimimo.com/v1");
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["web_search"]["max_keyword"], 3);
        assert_eq!(v["web_search"]["force_search"], true);
        assert_eq!(v["web_search"]["limit"], 1);
        assert_eq!(v["web_search"]["user_location"]["country"], "China");
        assert_eq!(v["web_search"]["user_location"]["city"], "Wuhan");
    }

    #[test]
    fn web_search_tool_others_omit_config() {
        let t = web_search_tool("https://api.deepseek.com/v1");
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["type"], "web_search");
        assert!(v.get("web_search").is_none(), "其余厂商不带 web_search 字段");
    }

    // ---------------------------------------------------------------------
    // 请求体序列化
    // ---------------------------------------------------------------------

    /// 🔴 `temperature` 必须**总是**出现（docs/04 §6.4 硬编码 0.0）
    #[test]
    fn chat_request_always_sends_temperature() {
        let req = OpenAiChatRequest::new("m", vec![ChatMessage::user("x", vec![])]);
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["temperature"], 0.0, "不能因等于默认值而被省略");
        assert_eq!(v["stream"], false, "stream 也总是出现");
    }

    #[test]
    fn chat_request_omits_none_fields() {
        let req = OpenAiChatRequest::new("m", vec![ChatMessage::user("x", vec![])]);
        let v = serde_json::to_value(&req).unwrap();
        for k in [
            "response_format",
            "max_tokens",
            "tools",
            "tool_choice",
            "stream_options",
            "thinking",
            "reasoning_effort",
        ] {
            assert!(v.get(k).is_none(), "None 字段 `{k}` 不应出现（不发 null）");
        }
    }

    #[test]
    fn chat_request_field_names_match_api() {
        let req = OpenAiChatRequest::new("m", vec![])
            .json_mode(true)
            .streaming()
            .with_web_search("https://api.deepseek.com/v1", true);
        let mut req = req;
        req.max_tokens = Some(2048);
        req.thinking = Some(ThinkingConfig {
            r#type: "disabled".to_string(),
        });
        req.reasoning_effort = Some("low".to_string());
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["response_format"]["type"], "json_object");
        assert_eq!(v["max_tokens"], 2048);
        assert_eq!(v["tool_choice"], "auto");
        assert_eq!(v["stream_options"]["include_usage"], true);
        assert_eq!(v["thinking"]["type"], "disabled");
        assert_eq!(v["reasoning_effort"], "low");
        assert!(v.get("reasoningEffort").is_none(), "不得漏成 camelCase");
    }

    #[test]
    fn chat_request_json_mode_off_clears_format() {
        let req = OpenAiChatRequest::new("m", vec![]).json_mode(true).json_mode(false);
        assert!(req.response_format.is_none());
    }

    // ---------------------------------------------------------------------
    // responses 请求体
    // ---------------------------------------------------------------------

    #[test]
    fn responses_request_shapes() {
        let msgs = [
            ChatMessage::system("S"),
            ChatMessage::user("U", vec![]),
            ChatMessage::assistant("A"),
        ];
        let req = DeepseekResponseRequest {
            model: "deepseek-chat".to_string(),
            instructions: Some("S".to_string()),
            input: msgs
                .iter()
                .filter(|m| m.role != "system")
                .map(to_response_input_item)
                .collect(),
            tools: None,
            text: Some(ResponseTextConfig {
                format: Some(ResponseFormat {
                    r#type: "json_object".to_string(),
                }),
            }),
            stream: true,
            reasoning: Some(ReasoningConfig {
                effort: "none".to_string(),
            }),
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["instructions"], "S", "系统提示在顶层");
        assert_eq!(v["input"].as_array().unwrap().len(), 2, "system 不进 input");
        assert_eq!(v["input"][0]["role"], "user");
        assert_eq!(v["input"][1]["role"], "assistant");
        assert_eq!(v["text"]["format"]["type"], "json_object");
        assert_eq!(v["reasoning"]["effort"], "none");
        assert_eq!(v["stream"], true);
        assert!(v.get("tools").is_none());
    }

    // ---------------------------------------------------------------------
    // 响应解析
    // ---------------------------------------------------------------------

    #[test]
    fn chat_response_parses_choices_and_usage() {
        let r: OpenAiChatResponse = serde_json::from_str(
            r#"{"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":" hi "}}],
                "usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}}"#,
        )
        .unwrap();
        assert_eq!(r.content(), Some(" hi "));
        assert_eq!(r.finish_reason(), Some("stop"));
        assert_eq!(r.usage.unwrap().total_tokens, 12);
    }

    /// 🔴 报错响应体没有 `choices` —— 必须能解析出来才能上报 `error`
    #[test]
    fn chat_response_error_without_choices_parses() {
        let r: OpenAiChatResponse =
            serde_json::from_str(r#"{"error":{"message":"bad key","type":"auth","code":"401"}}"#).unwrap();
        assert!(r.choices.is_empty());
        let e = r.error.expect("错误体应被解析");
        assert_eq!(e.message.as_deref(), Some("bad key"));
    }

    #[test]
    fn chat_response_finish_reason_length() {
        let r: OpenAiChatResponse = serde_json::from_str(
            r#"{"choices":[{"finish_reason":"length","message":{"role":"assistant","content":"x"}}]}"#,
        )
        .unwrap();
        assert_eq!(r.finish_reason(), Some("length"));
    }

    #[test]
    fn responses_response_extracts_output_text() {
        let r: DeepseekResponseResponse = serde_json::from_str(
            r#"{"status":"completed","output":[
                {"type":"reasoning","content":[{"type":"reasoning_text","text":"ignore"}]},
                {"type":"message","content":[{"type":"output_text","text":"AB"},{"type":"output_text","text":"CD"}]}
            ],"usage":{"input_tokens":5,"output_tokens":2,"total_tokens":7}}"#,
        )
        .unwrap();
        assert_eq!(r.output_text(), "ABCD", "只取 message 下的 output_text");
        assert!(!r.is_incomplete());
        assert_eq!(r.usage.unwrap().input_tokens, Some(5));
    }

    #[test]
    fn responses_response_incomplete_flag() {
        let r: DeepseekResponseResponse = serde_json::from_str(r#"{"status":"incomplete"}"#).unwrap();
        assert!(r.is_incomplete());
    }

    #[test]
    fn responses_response_empty_output_gives_empty_text() {
        let r: DeepseekResponseResponse = serde_json::from_str("{}").unwrap();
        assert_eq!(r.output_text(), "");
    }

    #[test]
    fn responses_response_trims_output_text() {
        let r: DeepseekResponseResponse = serde_json::from_str(
            r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"  x  "}]}]}"#,
        )
        .unwrap();
        assert_eq!(r.output_text(), "x");
    }

    // ---------------------------------------------------------------------
    // 往返
    // ---------------------------------------------------------------------

    #[test]
    fn chat_message_roundtrip_via_request() {
        let req = OpenAiChatRequest::new(
            "m",
            vec![
                ChatMessage::system("S"),
                ChatMessage::user("U", vec!["data:image/png;base64,Q".to_string()]),
            ],
        );
        let s = serde_json::to_string(&req).unwrap();
        let back: OpenAiChatRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(back.messages[0].content, "S");
        // 反序列化只取文本，图片不回填（与源一致）
        assert_eq!(back.messages[1].content, "U");
        assert!(back.messages[1].images.is_empty());
    }
}
