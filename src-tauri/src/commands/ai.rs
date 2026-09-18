//! 组 9：AI 生成（6 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `normalize_from_paste` | `raw`, `images`, `parentImageIds`, `renameTo?`, `revisedFromId?` | [`NormalizeResult`] |
//! | `normalize_from_query` | `desc`, `images` | [`NormalizeResult`] |
//! | `explain_formula` | `schema` | `FormulaSchema` |
//! | `refine_formula` | `schema`, `requirement` | [`NormalizeResult`] |
//! | `resolve_images` | `imageIds` | `Record<number, string>` |
//! | `ai_cancel` | — | `void` |
//!
//! ## 🔴 事件载荷是**增量**，不是累积全文
//!
//! `docs/08` §4 写的是 `{ text: string }`，前端示例是
//! `thinkingText.value += e.payload.text` —— 即**累加**。
//!
//! 而 `civilcalc-llm` 的回调给的是**累积全文**（对齐源 Kotlin 的
//! `onThinking(reasoning.toString())`）。两边语义不同，所以**命令层必须做差分**：
//! 记住上次已发的字节数，只发新增的后缀。见 [`AiEmitter`]。
//!
//! ## 🔴 事件节流 ≥50ms
//!
//! 模型可能每几十毫秒吐一个 token，逐条 `emit` 会把 IPC 打满。
//! 节流只影响**中间过程**；结束时 [`AiEmitter::finish`] **强制补发**剩余后缀，
//! 保证前端拿到完整文本。
//!
//! ## 🔴 取消要立即断流
//!
//! `ai_cancel` 置 [`crate::state::NetState`] 的标志；该标志一路传进
//! `LlmClient::chat_stream`，读循环每个 chunk 检查一次，命中即 drop 响应体
//! （连接随之关闭）。**不是**等请求跑完再丢弃结果。
//!
//! ## 🔴 用量必须入库（P4-6 验收项）
//!
//! 无论成功/失败，只要厂商返回了 usage 就写一条 `llm_usage_stats`。
//! 模型未返回时不写 —— **不臆造估算值**（源注释：宁可统计缺一条）。

use crate::error::{CmdResult, CommandError};
use crate::secrets::profile_api_key;
use crate::state::AppState;
use civilcalc_core::schema::FormulaSchema;
use civilcalc_llm::{
    build_revise_request, build_user_text, LlmClient, LlmExplainer, LlmNormalizer, NormalizeRequest,
    NormalizeUsage, ResolvedLlmProfile, Signal, Usage,
};
use civilcalc_store::UsageRow;
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, State};

// =============================================================================
// 事件
// =============================================================================

/// 思考过程增量
const EVENT_THINKING: &str = "ai://thinking";
/// 正文增量
const EVENT_CHUNK: &str = "ai://chunk";
/// 生成结束
const EVENT_DONE: &str = "ai://done";
/// 生成失败（载荷是 `CommandError`）
const EVENT_ERROR: &str = "ai://error";

/// 事件节流间隔（毫秒）。`docs/07` 要求 ≥50ms。
const THROTTLE_MS: u128 = 50;

/// 节流窗口必须 ≥50ms —— **编译期**钉住，改小会直接编译失败。
const _: () = assert!(THROTTLE_MS >= 50);

#[derive(Debug, Clone, Serialize)]
struct TextPayload {
    text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DonePayload {
    ok: bool,
}

// =============================================================================
// DTO
// =============================================================================

/// 一次 AI 生成的结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizeResult {
    pub schema: FormulaSchema,
    /// 厂商未返回用量时为 `None`（**不臆造**）
    pub usage: Option<NormalizeUsage>,
}

/// 把归一化用量转成入库行（store 层的 `UsageRow`）。
///
/// 作为自由函数而非 `NormalizeUsage` 的固有方法：后者现在定义在 `civilcalc_llm`，
/// 命令层不能给外部类型加固有方法。
#[must_use]
pub(crate) fn usage_to_row(u: &NormalizeUsage, model_label: &str) -> UsageRow {
    UsageRow {
        model_label: model_label.to_string(),
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
        cached_tokens: u.cached_tokens,
        reasoning_tokens: u.reasoning_tokens,
    }
}

// =============================================================================
// 事件发射器（增量 + 节流）
// =============================================================================

/// 把「累积全文」的信号转成「增量」事件，并做 ≥50ms 节流。
struct AiEmitter<'a> {
    app: &'a AppHandle,
    last_emit: Instant,
    /// 已发出的思考文本**字节数**
    sent_thinking: usize,
    /// 已发出的正文**字节数**
    sent_content: usize,
    /// 上一次拿到的累积全文（`finish` 时补发用）
    last_thinking: String,
    last_content: String,
    /// 捕获的用量
    usage: Option<Usage>,
}

impl<'a> AiEmitter<'a> {
    fn new(app: &'a AppHandle) -> Self {
        Self {
            app,
            // 减去节流窗口，保证**第一个**信号立刻发出（否则用户看不到开头）
            last_emit: Instant::now() - std::time::Duration::from_millis(THROTTLE_MS as u64),
            sent_thinking: 0,
            sent_content: 0,
            last_thinking: String::new(),
            last_content: String::new(),
            usage: None,
        }
    }

    fn push(&mut self, sig: Signal) {
        match sig {
            Signal::Thinking(full) => {
                self.last_thinking = full;
                if self.throttle_ready() {
                    self.flush_thinking();
                }
            }
            Signal::Content(full) => {
                self.last_content = full;
                if self.throttle_ready() {
                    self.flush_content();
                }
            }
            Signal::Usage(u) => self.usage = Some(u),
            Signal::Done => {}
        }
    }

    /// 结束时**强制**补发剩余后缀（节流不能吃掉尾部）。
    fn finish(&mut self) {
        self.flush_thinking();
        self.flush_content();
        let _ = self.app.emit(EVENT_DONE, DonePayload { ok: true });
    }

    fn throttle_ready(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.last_emit).as_millis() >= THROTTLE_MS {
            self.last_emit = now;
            true
        } else {
            false
        }
    }

    /// 发思考增量。
    ///
    /// 只在**真正发出**时推进 `sent_thinking` —— 被节流掉的增量会累积到下一次一起发。
    fn flush_thinking(&mut self) {
        let delta = text_delta(&self.last_thinking, &mut self.sent_thinking);
        if !delta.is_empty() {
            let _ = self.app.emit(EVENT_THINKING, TextPayload { text: delta });
        }
    }

    fn flush_content(&mut self) {
        let delta = text_delta(&self.last_content, &mut self.sent_content);
        if !delta.is_empty() {
            let _ = self.app.emit(EVENT_CHUNK, TextPayload { text: delta });
        }
    }
}

/// 取 `full` 中尚未发出的后缀，并把 `sent` 推进到 `full.len()`。
///
/// ⚠️ 只在 `sent` 是**字符边界**时才安全 —— 它总是上一次 `full` 的长度，
/// 而 `full` 只会在尾部追加，所以永远落在边界上。
fn text_delta(full: &str, sent: &mut usize) -> String {
    if *sent >= full.len() {
        return String::new();
    }
    let delta = full[*sent..].to_string();
    *sent = full.len();
    delta
}

// =============================================================================
// 命令
// =============================================================================

/// 解析「公式文本 / 自然语言」为公式。
///
/// `parent_image_ids` 是**父公式**的附图（续写场景）：它们必须排在
/// `images` **之前**，否则模型按新顺序写的 `{{img:N}}` 与既有详解里的序号错位。
#[tauri::command]
pub async fn normalize_from_paste(
    app: AppHandle,
    state: State<'_, AppState>,
    raw: String,
    images: Vec<String>,
    parent_image_ids: Vec<String>,
    rename_to: Option<String>,
    revised_from_id: Option<String>,
) -> CmdResult<NormalizeResult> {
    state.net.begin_ai();
    let (profile, label) = resolve_profile(&state)?;
    let (merged, image_ids) = merge_images(&state, &parent_image_ids, &images)?;

    let req = NormalizeRequest {
        raw: &raw,
        prompt_a: None,
        images: &merged,
        rename_to: rename_to.as_deref(),
        revised_from_id: revised_from_id.as_deref(),
    };

    let mut out = run_normalize(&app, &state, &profile, &label, &req).await?;
    // 把「这次带了哪些图」记到公式上（详解的 {{img:N}} 靠它渲染）
    out.schema.image_ids = image_ids;
    Ok(out)
}

/// 从自然语言描述生成公式（与 `normalize_from_paste` 同链路，只是语义入口不同）。
#[tauri::command]
pub async fn normalize_from_query(
    app: AppHandle,
    state: State<'_, AppState>,
    desc: String,
    images: Vec<String>,
) -> CmdResult<NormalizeResult> {
    state.net.begin_ai();
    let (profile, label) = resolve_profile(&state)?;
    let (merged, image_ids) = merge_images(&state, &[], &images)?;

    let req = NormalizeRequest {
        raw: &desc,
        prompt_a: None,
        images: &merged,
        rename_to: None,
        revised_from_id: None,
    };

    let mut out = run_normalize(&app, &state, &profile, &label, &req).await?;
    // 同 `normalize_from_paste`：公式要记住自己的附图
    out.schema.image_ids = image_ids;
    Ok(out)
}

/// 为既有公式按需生成「计算公式详解」，返回**填好 explanation 的** schema。
#[tauri::command]
pub async fn explain_formula(
    app: AppHandle,
    state: State<'_, AppState>,
    schema: FormulaSchema,
) -> CmdResult<FormulaSchema> {
    state.net.begin_ai();
    let (profile, label) = resolve_profile(&state)?;
    let user_text = build_user_text(&schema);
    let images = super::image::resolve_image_refs(&state, &schema.image_ids);

    let client = LlmClient::new();
    let explainer = LlmExplainer::new(client);
    let mut em = AiEmitter::new(&app);
    let mut usage: Option<Usage> = None;

    // ⚠️ explainer 只有 `on_usage`（源亦如此）—— 详解链路**不流式**，
    //    前端等 `explain_formula` 的返回值即可；这里只发 `ai://done` 收尾。
    let outcome = {
        let mut on_usage = |u: Usage| usage = Some(u);
        explainer
            .explain(
                &profile,
                &user_text,
                &images,
                Some(state.net.ai_flag()),
                &mut on_usage,
            )
            .await
    };
    em.finish();

    record_usage(&state, &label, usage.as_ref());

    match outcome {
        Ok(explanation) => {
            let mut out = schema;
            out.explanation = Some(explanation);
            Ok(out)
        }
        Err(e) => Err(fail(&app, e)),
    }
}

/// 续写微调：在既有公式基础上按需求改写。
///
/// ## 附图（需求 2）
///
/// `images` 是本次微调**新增**的图片引用（前端附图时图片已入库，给的是 id）。
/// 顺序铁律：**父公式的附图必须排在前**，新图在后 —— 模型按「第 N 张」
/// 写 `{{img:N}}`，父图前置才能与既有详解里的序号对齐。
#[tauri::command]
pub async fn refine_formula(
    app: AppHandle,
    state: State<'_, AppState>,
    schema: FormulaSchema,
    requirement: String,
    images: Vec<String>,
) -> CmdResult<NormalizeResult> {
    state.net.begin_ai();
    let (profile, label) = resolve_profile(&state)?;

    // 名称版本化：`圆柱体积` → `圆柱体积-v1.0`（连续微调递增）
    let rename_to = civilcalc_llm::next_version_name(&schema.result_name);
    // 续写请求文本 = 既有公式结构摘要 + 微调需求
    let raw = build_revise_request(&schema, &requirement);
    // 🔴 父公式附图必须**排在前**，保证既有详解里的 {{img:N}} 序号不错位
    let (merged, image_ids) = merge_images(&state, &schema.image_ids, &images)?;

    let req = NormalizeRequest {
        raw: &raw,
        prompt_a: None,
        images: &merged,
        rename_to: Some(&rename_to),
        revised_from_id: Some(&schema.id),
    };

    let mut out = run_normalize(&app, &state, &profile, &label, &req).await?;
    // 微调后的新公式沿用「父图在前 + 本次新增在后」的完整附图列表
    out.schema.image_ids = image_ids;
    Ok(out)
}

/// 把图片 ID 列表解析成 `序号 → data URL`（**序号从 1 起**）。
///
/// 找不到的 ID **跳过但不占位** —— 序号必须与调用方传入的顺序一一对应，
/// 否则详解里的 `{{img:N}}` 会指错图。所以这里保持「按入参顺序、连续编号」。
#[tauri::command]
pub fn resolve_images(
    state: State<'_, AppState>,
    image_ids: Vec<String>,
) -> CmdResult<BTreeMap<usize, String>> {
    let store = open_image_store(&state)?;
    let mut out = BTreeMap::new();
    for (i, id) in image_ids.iter().enumerate() {
        if let Some(url) = store.load_data_url(id) {
            out.insert(i + 1, url);
        }
    }
    Ok(out)
}

/// 取消当前 AI 任务（立即断流，见模块文档）。
#[tauri::command]
pub fn ai_cancel(state: State<'_, AppState>) {
    state.net.cancel_ai();
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
        .get(&profile_api_key(&p.id))?
        .unwrap_or_default();
    if key.trim().is_empty() {
        return Err(CommandError::Unauthorized {
            message: format!("模型「{}」未设置 API Key", p.label),
        });
    }
    Ok((ResolvedLlmProfile::from_profile(p, key), p.label.clone()))
}

/// 打开图片存储（目录来自 `DesktopPaths::images_dir`）。
fn open_image_store(
    state: &AppState,
) -> CmdResult<civilcalc_store::LlmImageStore> {
    civilcalc_store::LlmImageStore::new(state.paths.images_dir()).map_err(|e| {
        CommandError::Storage {
            message: format!("图片目录不可用: {e}"),
        }
    })
}

/// 合并附图：**父公式附图在前**，本次新增在后。
///
/// 返回 `(data URL 列表, 图片 id 列表)`，两者**等长同序**。
///
/// ## 顺序铁律
///
/// 模型按「第 N 张」写 `{{img:N}}`，父图必须前置才能与既有详解里的序号对齐
/// （见模块文档）。
///
/// ## 🔴 为什么还要把 id 带出来
///
/// `data URL` 是发给模型的，`id` 是**回填给 `schema.imageIds`** 的。
/// 少了 id，公式就「不记得自己带了哪几张图」——
/// 详解里的 `{{img:N}}` 渲染不出来、再次微调时父图前置不了。
/// 见 [`super::image::resolve_image_refs_with_ids`]。
///
/// ## 🔴 两个参数收的都是「图片引用」，由
/// [`super::image::resolve_image_refs_with_ids`] 统一解析
///
/// 它同时认**图片 id**（前端附图时图片就已入库）与**data URL**（兼容形态）。
/// 这里曾把 `new_data_urls` 直接当 data URL 丢给 `store.save`，
/// 而前端传的是 id —— 于是**附图被静默丢掉**（请求成功但没有图）。
fn merge_images(
    state: &AppState,
    parent_refs: &[String],
    new_refs: &[String],
) -> CmdResult<(Vec<String>, Vec<String>)> {
    let (mut urls, mut ids) = super::image::resolve_image_refs_with_ids(state, parent_refs);
    let (u2, i2) = super::image::resolve_image_refs_with_ids(state, new_refs);
    urls.extend(u2);
    ids.extend(i2);
    Ok((urls, ids))
}
/// 归一化的公共执行体（两个入口共用）。
async fn run_normalize(
    app: &AppHandle,
    state: &AppState,
    profile: &ResolvedLlmProfile,
    label: &str,
    req: &NormalizeRequest<'_>,
) -> CmdResult<NormalizeResult> {
    let normalizer = LlmNormalizer::new(LlmClient::new());
    let mut em = AiEmitter::new(app);
    let mut usage: Option<Usage> = None;

    // ⚠️ normalizer 只暴露 `on_thinking`（**没有**正文回调）—— 源亦如此：
    //    归一化期间 UI 只能看到思考过程，正文要等返回的 schema。
    let outcome = {
        let mut on_thinking = |text: String| em.push(Signal::Thinking(text));
        let mut on_usage = |u: Usage| usage = Some(u);
        normalizer
            .normalize_from_paste(
                profile,
                req,
                Some(state.net.ai_flag()),
                &mut on_thinking,
                &mut on_usage,
            )
            .await
    };
    em.finish();

    // 🔴 无论成败，有 usage 就入库
    record_usage(state, label, usage.as_ref());

    match outcome {
        Ok(nr) => Ok(NormalizeResult {
            schema: nr.schema,
            usage: nr.usage.as_ref().map(NormalizeUsage::from),
        }),
        Err(e) => Err(fail(app, e)),
    }
}

/// 用量入库（厂商未返回时**不写**）。
fn record_usage(state: &AppState, model_label: &str, usage: Option<&Usage>) {
    let Some(u) = usage else {
        return;
    };
    let row = usage_to_row(&NormalizeUsage::from(u), model_label);
    if let Err(e) = state.db.insert_usage(&row) {
        // 用量写失败不该让用户的生成结果作废 —— 只记日志
        civilcalc_core::log::w("Commands", "用量入库失败", Some(&e.to_string()));
    }
}

/// 统一失败出口：发 `ai://error` 并转成 `CommandError`。
fn fail(app: &AppHandle, e: civilcalc_llm::LlmError) -> CommandError {
    let err: CommandError = e.into();
    let _ = app.emit(EVENT_ERROR, &err);
    err
}

/// 当前毫秒时间戳（保留给后续扩展；入库时间由 store 自己取）。
#[allow(dead_code)]
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------
    // 增量差分（事件载荷语义的核心）
    // ---------------------------------------------------------------------

    /// 累积全文 → 只发新增后缀
    #[test]
    fn delta_emits_only_suffix() {
        let mut sent = 0usize;
        assert_eq!(text_delta("A", &mut sent), "A");
        assert_eq!(sent, 1);
        assert_eq!(text_delta("AB", &mut sent), "B");
        assert_eq!(text_delta("ABC", &mut sent), "C");
        assert_eq!(sent, 3);
    }

    /// 同一文本重复推入 → 空增量（不重复发）
    #[test]
    fn delta_empty_when_unchanged() {
        let mut sent = 0usize;
        assert_eq!(text_delta("AB", &mut sent), "AB");
        assert_eq!(text_delta("AB", &mut sent), "");
        assert_eq!(text_delta("A", &mut sent), "", "长度回退也不重发");
    }

    /// 空文本不产生事件
    #[test]
    fn delta_empty_for_empty_text() {
        let mut sent = 0usize;
        assert_eq!(text_delta("", &mut sent), "");
        assert_eq!(sent, 0);
    }

    /// 🔴 中文（多字节）差分必须落在字符边界，不能切碎 UTF-8
    #[test]
    fn delta_is_utf8_safe() {
        let mut sent = 0usize;
        assert_eq!(text_delta("①②", &mut sent), "①②");
        assert_eq!(text_delta("①②③", &mut sent), "③");
        assert_eq!(text_delta("①②③④", &mut sent), "④");
        // 逐段拼回必须等于原文（前端是 `+=`，这条保证累加正确）
        let mut rebuilt = String::new();
        let mut s2 = 0usize;
        for frame in ["①②", "①②③", "①②③④"] {
            rebuilt.push_str(&text_delta(frame, &mut s2));
        }
        assert_eq!(rebuilt, "①②③④");
    }

    /// 模拟前端 `+=` 累加：所有增量拼起来 == 最终全文
    #[test]
    fn concatenated_deltas_equal_full_text() {
        let mut sent = 0usize;
        let frames = ["圆柱", "圆柱体积", "圆柱体积 V", "圆柱体积 V=πr²h"];
        let mut rebuilt = String::new();
        for f in frames {
            rebuilt.push_str(&text_delta(f, &mut sent));
        }
        assert_eq!(rebuilt, "圆柱体积 V=πr²h");
    }

    // ---------------------------------------------------------------------
    // 用量 DTO
    // ---------------------------------------------------------------------

    #[test]
    fn usage_dto_normalizes_cross_protocol() {
        // /responses 形态：只有 input/output/total
        let u = Usage {
            input_tokens: Some(100),
            output_tokens: Some(20),
            total_tokens: 120,
            ..Default::default()
        };
        let dto = NormalizeUsage::from(&u);
        assert_eq!(dto.prompt_tokens, 100, "input → prompt");
        assert_eq!(dto.completion_tokens, 20);
        assert_eq!(dto.total_tokens, 120);
    }

    #[test]
    fn usage_dto_carries_cached_and_reasoning() {
        let u = Usage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            prompt_cache_hit_tokens: Some(60),
            prompt_cache_miss_tokens: Some(40),
            completion_tokens_details: Some(civilcalc_llm::TokenDetails {
                cached_tokens: None,
                reasoning_tokens: Some(30),
            }),
            ..Default::default()
        };
        let dto = NormalizeUsage::from(&u);
        assert_eq!(dto.cached_tokens, 60);
        assert_eq!(dto.reasoning_tokens, 30);
    }

    /// 入库行字段与 DTO 一一对应（别在搬运时错位）
    #[test]
    fn usage_dto_to_row() {
        let dto = NormalizeUsage {
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3,
            cached_tokens: 4,
            reasoning_tokens: 5,
        };
        let row = usage_to_row(&dto, "GLM-4");
        assert_eq!(row.model_label, "GLM-4");
        assert_eq!(row.prompt_tokens, 1);
        assert_eq!(row.completion_tokens, 2);
        assert_eq!(row.total_tokens, 3);
        assert_eq!(row.cached_tokens, 4);
        assert_eq!(row.reasoning_tokens, 5);
    }

    #[test]
    fn usage_dto_serializes_camel_case() {
        let dto = NormalizeUsage {
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3,
            cached_tokens: 4,
            reasoning_tokens: 5,
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["promptTokens"], 1);
        assert_eq!(v["reasoningTokens"], 5);
        assert!(v.get("prompt_tokens").is_none(), "不得漏成 snake_case");
    }

    // ---------------------------------------------------------------------
    // 事件常量
    // ---------------------------------------------------------------------

    /// 事件名与 `docs/08` §4 逐字一致（大小写敏感、无通配）
    #[test]
    fn event_names_match_contract() {
        assert_eq!(EVENT_THINKING, "ai://thinking");
        assert_eq!(EVENT_CHUNK, "ai://chunk");
        assert_eq!(EVENT_DONE, "ai://done");
        assert_eq!(EVENT_ERROR, "ai://error");
    }

}
