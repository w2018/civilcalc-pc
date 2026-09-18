//! 模型测试：把「对话式测试」做成一组命令。
//!
//! 源：`civilcalc-android-v2` 的 `SettingsViewModel.runModelTest` / `compressConversation` /
//! `persistConversation`（约 200 行），加上 `ModelTestTurn.kt`（数据类）。
//!
//! ## 命令
//!
//! - `model_test_send`：流式发事件（独立通道 `modelTest://*`），成功把本轮问答落盘到
//!   偏好 `model_test_conversation`，并记用量。
//! - `model_test_conversation`：读回持久化的多轮记忆。
//! - `model_test_clear`：清空记忆。
//! - `model_test_set_system_prompt`：独立系统提示词。
//! - `model_test_compress`：手动触发上下文压缩（独立一次 LLM 调用，其用量同样入库）。
//!
//! ## 取消
//!
//! 复用 `NetState` 的 AI 取消标志（`begin_ai` 每次清、中断即 drop 响应体）。
//! 当前没有单独的 `model_test_cancel` 命令 —— 同一时刻只有一次 LLM 长调用，
//! `ai_cancel` 同样会中断模型测试（与「一个活动 AI 交互」的语义一致）。
//!
//! ## 上下文自动压缩
//!
//! 估算 token（字符数 / 2）超过「上下文大小 × 压缩阈值%」时，把历史对话压缩成摘要，
//! 再带上新的一轮发出去。压缩失败不阻断发送（原文照发）。

use crate::commands::ai::usage_to_row;
use crate::error::{CmdResult, CommandError};
use crate::state::AppState;
use civilcalc_core::log;
use civilcalc_llm::{
    ChatMessage, ChatResult, LlmClient, NormalizeUsage, ResolvedLlmProfile, Usage,
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

/// 模型测试对话记忆中的一轮消息。
///
/// 图片 data URL **不入库**（避免撑爆配置）；但**思考过程入库**（需求 3）——
/// 用户回头看记录时要能看到「当时模型是怎么想的」，
/// 而思考只在流式过程中出现过一次，不存就永远找不回来了。
///
/// ## 旧数据兼容
///
/// `thinking` 用 `#[serde(default)]`：这个字段是后加的，
/// 磁盘上已有的对话（没有这个键）必须还能反序列化出来 ——
/// 否则用户升级后会**丢掉全部对话记忆**。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelTestTurn {
    pub role: String,
    pub content: String,
    /// 思考过程全文（仅助手轮有；用户轮为 `None`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
}

/// 模型测试系统提示词默认值。
///
/// ⚠️ **与前端 `src/types/modelTest.ts` 的同名常量逐字一致**，
/// 由契约测试 `default_test_prompt_matches_ts` 钉住（改一边不改另一边会红）。
///
/// 用 `\n` 转义而不是多行字符串字面量：这样两侧都是「一行内的字符串字面量」，
/// 契约测试可以直接逐字比对，不需要处理缩进与换行差异。
const DEFAULT_TEST_SYSTEM_PROMPT: &str = "你是一名专业AI工作助理，具备多领域问题解决能力。\n输出要求：\n1. 回答简洁，逻辑清晰\n2. 优先使用图表呈现关键信息\n3. 回答末尾提供3个相关衍生问题";

/// 对话记忆最多保留的轮数（与源 `MAX_CONV_TURNS` 一致）。
const MAX_CONV_TURNS: usize = 200;

const KEY_CONVERSATION: &str = "model_test_conversation";
const KEY_SYSTEM_PROMPT: &str = "model_test_system_prompt";

/// 🔴 事件名与 `ai://*` **同构**（`modelTest://*`）——
/// 见 `docs/05` §1.3.4 事件表与 `docs/08` §4。
/// 曾误写成 `modelTest:xxx`（单冒号），已改正并有测试钉住。
const EVENT_THINKING: &str = "modelTest://thinking";
const EVENT_CONTENT: &str = "modelTest://content";
const EVENT_USAGE: &str = "modelTest://usage";
const EVENT_DONE: &str = "modelTest://done";
const EVENT_ERROR: &str = "modelTest://error";

/// 流式事件节流窗口（毫秒）。与 `ai.rs` 一致：中间过程限流，结束强制补发尾部。
const THROTTLE_MS: i64 = 50;

/// 节流窗口必须 ≥50ms —— **编译期**钉住，改小会直接编译失败
/// （与 `ai.rs` 同口径；写成运行期 `assert!` 会被 clippy 的
/// `assertions_on_constants` 拒掉）。
const _: () = assert!(THROTTLE_MS >= 50);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TextPayload {
    text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UsagePayload {
    prompt: i64,
    completion: i64,
    total: i64,
    cached: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DonePayload {
    ok: bool,
}

/// 发送一轮模型测试。
///
/// `messages` 是**完整对话**（含本轮新用户消息，排在最后）；`images` 是贴在本轮用户消息上的
/// data URL（最多几个）。成功把「历史 + 本轮用户 + 助手回复」落盘。
#[tauri::command]
pub async fn model_test_send(
    app: AppHandle,
    state: State<'_, AppState>,
    messages: Vec<ModelTestTurn>,
    images: Vec<String>,
) -> CmdResult<()> {
    state.net.begin_ai();
    let (profile, label) = resolve_profile(&state)?;

    let system_prompt = {
        let cfg = state.config.lock().unwrap();
        cfg.get_str_non_empty(KEY_SYSTEM_PROMPT)
            .unwrap_or(DEFAULT_TEST_SYSTEM_PROMPT)
            .to_string()
    };

    let n = messages.len();
    if n == 0 {
        return Err(CommandError::InvalidArgument {
            message: "对话为空，无法发送".to_string(),
        });
    }
    let (history, last) = messages.split_at(n - 1);
    let user_content = last[0].content.clone();
    // 图片只贴在本轮用户消息上
    let user_images = if last[0].role == "user" {
        // 🔴 必须解析成 data URL 再发出去。
        //
        // `images` 收的是**图片引用**（前端附图时图片已入库、拿到 id）。
        // 这里曾直接把它拼进请求，于是 body 里出现
        // `{"type":"image_url","image_url":{"url":"<uuid>"}}` ——
        // 不是合法 URL，API 直接返回 400，**带图的请求必然失败**。
        super::image::resolve_image_refs(&state, &images)
    } else {
        Vec::new()
    };
    if user_content.trim().is_empty() && user_images.is_empty() {
        emit_error(&app, "请输入测试内容或添加图片");
        return Err(CommandError::InvalidArgument {
            message: "请输入测试内容或添加图片".to_string(),
        });
    }

    // 组装 API 消息：系统提示词 + 历史（可能压缩过）+ 本轮用户
    let mut history_for_api: Vec<ChatMessage> =
        history.iter().map(to_chat_message).collect();

    // 上下文自动压缩
    let context_size = state.config.lock().unwrap().test_context_size();
    let threshold = state.config.lock().unwrap().test_compress_threshold();
    let limit = context_size * threshold / 100;
    // 是否真的压缩过 —— 决定落盘时用哪一份历史（见下方注释）
    let mut compressed = false;
    if !history.is_empty()
        && estimate_tokens(&system_prompt, &history_for_api, &user_content) > limit
    {
        let client = LlmClient::new();
        if let Some(summary) =
            compress_conversation(&state, &client, &profile, &label, &history_for_api).await
        {
            history_for_api =
                vec![ChatMessage::user(
                    format!("（此前对话已压缩为以下摘要）\n{summary}"),
                    Vec::new(),
                )];
            compressed = true;
        }
    }

    let mut api_messages: Vec<ChatMessage> = Vec::with_capacity(history_for_api.len() + 2);
    api_messages.push(ChatMessage::system(system_prompt.clone()));
    api_messages.extend(history_for_api.iter().cloned());
    api_messages.push(ChatMessage::user(user_content.clone(), user_images.clone()));

    let client = LlmClient::new();
    let mut em = ModelTestEmitter::new(&app);
    let mut usage: Option<Usage> = None;

    let outcome = {
        let mut on_signal = |sig: civilcalc_llm::Signal| match sig {
            civilcalc_llm::Signal::Thinking(full) => em.push_thinking(full),
            civilcalc_llm::Signal::Content(full) => em.push_content(full),
            civilcalc_llm::Signal::Usage(u) => {
                usage = Some(u.clone());
                em.emit_usage(&u);
            }
            civilcalc_llm::Signal::Done => {}
        };
        client
            .chat_stream(
                &profile,
                &api_messages,
                false,
                profile.web_search,
                Some(state.net.ai_flag()),
                &mut on_signal,
            )
            .await
    };

    em.finish();
    // 本轮的思考全文（`last_thinking` 是**累加后**的完整文本，
    // 事件里发出去的才是增量）—— 要随助手轮一起落盘，见下方说明
    let round_thinking = em.last_thinking.clone();

    match outcome {
        Ok(ChatResult { content, usage: result_usage }) => {
            let final_usage = result_usage.or(usage);
            record_usage(&state, &label, final_usage.as_ref());

            // 落盘对话记忆（图片不入库，思考入库）。
            //
            // 🔴 历史轮次必须**沿用前端传来的原对象**，不能从
            // `history_for_api` 反推 —— 后者是给 API 用的 `ChatMessage`，
            // 只有 role + content，**思考会在这里被丢掉**：
            // 每发一轮就重建一次记忆，上一轮的思考就会一轮一轮消失。
            //
            // 唯一例外是**压缩过**的情况：那时 `history_for_api` 已经
            // 变成一条摘要，原来的轮次不该再留着（否则压缩等于没做）。
            let mut conv: Vec<ModelTestTurn> = if compressed {
                history_for_api.iter().map(to_turn).collect()
            } else {
                history.to_vec()
            };
            conv.push(ModelTestTurn {
                role: "user".to_string(),
                content: user_content.clone(),
                thinking: None,
            });
            conv.push(ModelTestTurn {
                role: "assistant".to_string(),
                content: content.clone(),
                // 空思考归一为 `None`（与历史表的 `thinking_content` 同口径）
                thinking: Some(round_thinking).filter(|t| !t.trim().is_empty()),
            });
            if conv.len() > MAX_CONV_TURNS {
                conv = conv.split_off(conv.len() - MAX_CONV_TURNS);
            }
            persist_conversation(&state, &conv);
            Ok(())
        }
        Err(e) => {
            let err: CommandError = e.into();
            emit_error(&app, &err.to_string());
            record_usage(&state, &label, usage.as_ref());
            Err(err)
        }
    }
}

/// 读回持久化的多轮记忆（跨进程保留）。
#[tauri::command]
pub fn model_test_conversation(state: State<'_, AppState>) -> CmdResult<Vec<ModelTestTurn>> {
    Ok(read_conversation(&state))
}

/// 清空模型测试记忆。
#[tauri::command]
pub fn model_test_clear(state: State<'_, AppState>) -> CmdResult<()> {
    let mut cfg = state.config.lock().unwrap();
    cfg.set_str(KEY_CONVERSATION, "[]");
    Ok(())
}

/// 设置模型测试专属系统提示词（空串 = 回落默认）。
#[tauri::command]
pub fn model_test_set_system_prompt(
    state: State<'_, AppState>,
    prompt: String,
) -> CmdResult<()> {
    let mut cfg = state.config.lock().unwrap();
    cfg.set_str(KEY_SYSTEM_PROMPT, prompt);
    Ok(())
}

/// 手动触发上下文压缩：把整段历史压成一条摘要，替代原有记忆。
#[tauri::command]
pub async fn model_test_compress(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    state.net.begin_ai();
    let (profile, label) = resolve_profile(&state)?;
    let conv = read_conversation(&state);
    if conv.is_empty() {
        emit_done(&app);
        return Ok(());
    }
    let history: Vec<ChatMessage> = conv.iter().map(to_chat_message).collect();
    let client = LlmClient::new();
    if let Some(summary) =
        compress_conversation(&state, &client, &profile, &label, &history).await
    {
        let new_conv = vec![ModelTestTurn {
            role: "user".to_string(),
            content: format!("（此前对话已压缩为以下摘要）\n{summary}"),
            thinking: None,
        }];
        persist_conversation(&state, &new_conv);
    }
    emit_done(&app);
    Ok(())
}

// =============================================================================
// 内部
// =============================================================================

/// 取活跃档位 + Key，组出可直接发请求的 profile。
fn resolve_profile(state: &AppState) -> CmdResult<(ResolvedLlmProfile, String)> {
    let cfg = state.secrets.get_llm_config()?;
    let p = cfg.active_profile().ok_or_else(|| CommandError::NotFound {
        message: "没有可用的模型档位，请先在设置里添加".to_string(),
    })?;
    let key = state
        .secrets
        .get(&crate::secrets::profile_api_key(&p.id))?
        .unwrap_or_default();
    if key.trim().is_empty() {
        return Err(CommandError::Unauthorized {
            message: format!("模型「{}」未设置 API Key", p.label),
        });
    }
    Ok((
        ResolvedLlmProfile::from_profile(p, key),
        p.label.clone(),
    ))
}

/// 把一轮记忆转成 API 消息（图片只在用户消息上、且此处来自记忆 → 一律不带图片）。
fn to_chat_message(t: &ModelTestTurn) -> ChatMessage {
    match t.role.as_str() {
        "user" => ChatMessage::user(t.content.clone(), Vec::new()),
        "assistant" => ChatMessage::assistant(t.content.clone()),
        "system" => ChatMessage::system(t.content.clone()),
        other => ChatMessage {
            role: other.to_string(),
            content: t.content.clone(),
            images: Vec::new(),
        },
    }
}

fn to_turn(m: &ChatMessage) -> ModelTestTurn {
    ModelTestTurn {
        role: m.role.clone(),
        content: m.content.clone(),
        // 从历史里读回来的轮次不再保留思考（压缩后的摘要也没有思考）
        thinking: None,
    }
}

/// 估算对话 token（上下文容量提示用；只影响压缩时机，不写入用量统计）。
fn estimate_tokens(system: &str, history: &[ChatMessage], user: &str) -> i64 {
    let chars = system.chars().count()
        + history.iter().map(|m| m.content.chars().count()).sum::<usize>()
        + user.chars().count();
    ((chars as i64) / 2).max(1)
}

/// 把历史对话压缩成摘要（独立一次 LLM 调用）；失败返回 None 不压缩。
async fn compress_conversation(
    state: &AppState,
    client: &LlmClient,
    profile: &ResolvedLlmProfile,
    label: &str,
    history: &[ChatMessage],
) -> Option<String> {
    let convo = history
        .iter()
        .map(|m| {
            if m.role == "user" {
                format!("用户：{}", m.content)
            } else {
                format!("助手：{}", m.content)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let messages = vec![
        ChatMessage::system(
            "你是对话压缩助手。将给定的对话历史压缩成一份简明摘要：保留关键事实、数据、结论与未决问题，直接输出摘要正文，不要任何解释。",
        ),
        ChatMessage::user(convo, Vec::new()),
    ];
    match client
        .chat(profile, &messages, false, false, Some(1024))
        .await
    {
        Ok(ChatResult { content, usage }) => {
            record_usage(state, label, usage.as_ref());
            let s = content.trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
        Err(_) => None,
    }
}

/// 用量入库（厂商未返回时**不写**）。
fn record_usage(state: &AppState, model_label: &str, usage: Option<&Usage>) {
    let Some(u) = usage else {
        return;
    };
    let row = usage_to_row(&NormalizeUsage::from(u), model_label);
    if let Err(e) = state.db.insert_usage(&row) {
        log::w("Commands", "模型测试用量入库失败", Some(&e.to_string()));
    }
}

/// 读回对话记忆（解析失败兜底空）。
fn read_conversation(state: &AppState) -> Vec<ModelTestTurn> {
    let cfg = state.config.lock().unwrap();
    match cfg.get_str(KEY_CONVERSATION) {
        Some(s) if !s.trim().is_empty() => serde_json::from_str(s).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// 落盘对话记忆（JSON 数组）。
fn persist_conversation(state: &AppState, turns: &[ModelTestTurn]) {
    let json = serde_json::to_string(turns).unwrap_or_else(|_| "[]".to_string());
    let mut cfg = state.config.lock().unwrap();
    cfg.set_str(KEY_CONVERSATION, json);
}

fn emit_done(app: &AppHandle) {
    let _ = app.emit(EVENT_DONE, DonePayload { ok: true });
}

fn emit_error(app: &AppHandle, message: &str) {
    let _ = app.emit(
        EVENT_ERROR,
        &CommandError::Unknown {
            message: message.to_string(),
        },
    );
}

/// 模型测试流式事件发射器（与 `ai.rs` 的 `AiEmitter` 同构，但走 `modelTest://` 通道）。
struct ModelTestEmitter<'a> {
    app: &'a AppHandle,
    last_emit: Instant,
    sent_thinking: usize,
    sent_content: usize,
    last_thinking: String,
    last_content: String,
}

impl<'a> ModelTestEmitter<'a> {
    fn new(app: &'a AppHandle) -> Self {
        Self {
            app,
            last_emit: Instant::now() - Duration::from_millis(THROTTLE_MS as u64),
            sent_thinking: 0,
            sent_content: 0,
            last_thinking: String::new(),
            last_content: String::new(),
        }
    }

    fn push_thinking(&mut self, full: String) {
        self.last_thinking = full;
        if self.throttle_ready() {
            self.flush_thinking();
        }
    }

    fn push_content(&mut self, full: String) {
        self.last_content = full;
        if self.throttle_ready() {
            self.flush_content();
        }
    }

    fn emit_usage(&self, u: &Usage) {
        let n = u.normalized();
        let _ = self.app.emit(
            EVENT_USAGE,
            UsagePayload {
                prompt: n.prompt_tokens,
                completion: n.completion_tokens,
                total: n.total_tokens,
                cached: n.cached_tokens(),
            },
        );
    }

    fn finish(&mut self) {
        self.flush_thinking();
        self.flush_content();
        let _ = self.app.emit(EVENT_DONE, DonePayload { ok: true });
    }

    fn throttle_ready(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.last_emit).as_millis() >= THROTTLE_MS as u128 {
            self.last_emit = now;
            true
        } else {
            false
        }
    }

    fn flush_thinking(&mut self) {
        let delta = text_delta(&self.last_thinking, &mut self.sent_thinking);
        if !delta.is_empty() {
            let _ = self.app.emit(EVENT_THINKING, TextPayload { text: delta });
        }
    }

    fn flush_content(&mut self) {
        let delta = text_delta(&self.last_content, &mut self.sent_content);
        if !delta.is_empty() {
            let _ = self.app.emit(EVENT_CONTENT, TextPayload { text: delta });
        }
    }
}

/// 取 `full` 中尚未发出的后缀，并把 `sent` 推进到 `full.len()`（字符边界安全）。
fn text_delta(full: &str, sent: &mut usize) -> String {
    if *sent >= full.len() {
        return String::new();
    }
    let s = full; // 重命名以便阅读
    let out = s[*sent..].to_string();
    *sent = s.len();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 事件名是**跨端契约**（前端按字面量订阅）——
    /// 钉住它们，防止再出现「文档写 `modelTest://`、代码写 `modelTest:`」的漂移。
    #[test]
    fn event_names_match_contract() {
        assert_eq!(EVENT_THINKING, "modelTest://thinking");
        assert_eq!(EVENT_CONTENT, "modelTest://content");
        assert_eq!(EVENT_USAGE, "modelTest://usage");
        assert_eq!(EVENT_DONE, "modelTest://done");
        assert_eq!(EVENT_ERROR, "modelTest://error");
    }

    // 节流窗口 ≥50ms 由模块顶部的**编译期断言**保证
    // （写成运行期 `assert!` 会触发 clippy `assertions_on_constants`）。

    /// `text_delta` 只给后缀；同一全文重复调用第二次必须为空。
    #[test]
    fn text_delta_only_suffix() {
        let mut sent = 0usize;
        assert_eq!(text_delta("abc", &mut sent), "abc");
        assert_eq!(text_delta("abc", &mut sent), "");
        assert_eq!(text_delta("abcdef", &mut sent), "def");
    }

    /// 中文（多字节）也要在字符边界上切，不能切出半个字符。
    #[test]
    fn text_delta_is_char_boundary_safe() {
        let mut sent = 0usize;
        assert_eq!(text_delta("混凝土", &mut sent), "混凝土");
        assert_eq!(text_delta("混凝土方量", &mut sent), "方量");
    }

    /// 🔴 需求 3 加了 `thinking` 字段 —— 磁盘上已有的旧对话**必须还能读**。
    ///
    /// 没有 `#[serde(default)]` 时这里会反序列化失败，
    /// 而失败路径是「读不出记忆」→ 用户升级后**对话全没了**。
    #[test]
    fn old_conversation_without_thinking_still_parses() {
        let raw = r#"[{"role":"user","content":"你好"},{"role":"assistant","content":"在的"}]"#;
        let turns: Vec<ModelTestTurn> = serde_json::from_str(raw).expect("旧数据必须能读");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[1].thinking, None);
    }

    /// 用户轮不该写出 `thinking` 键（省空间；也让旧版本读新数据时不会误解）
    #[test]
    fn user_turn_omits_thinking_key() {
        let t = ModelTestTurn {
            role: "user".into(),
            content: "hi".into(),
            thinking: None,
        };
        let json = serde_json::to_string(&t).unwrap();
        assert!(!json.contains("thinking"), "实际：{json}");
    }

    /// 助手轮的思考要能往返（这是「输出记录保留思考过程」的存储基础）
    #[test]
    fn assistant_thinking_roundtrips() {
        let t = ModelTestTurn {
            role: "assistant".into(),
            content: "答".into(),
            thinking: Some("想".into()),
        };
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains("\"thinking\":\"想\""), "实际：{json}");
        let back: ModelTestTurn = serde_json::from_str(&json).unwrap();
        assert_eq!(back.thinking.as_deref(), Some("想"));
    }
}
