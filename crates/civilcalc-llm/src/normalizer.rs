//! 唯一 AI 解析入口：把「公式文本 / 自然语言描述」用 Prompt A 一步归一化为 `FormulaSchema`。
//!
//! 源：`civilcalc-android-v2/core/llm/LlmNormalizer.kt`（231 行）
//!
//! ## 职责
//!
//! 1. **视觉前置校验**：带图但模型未开视觉 → 直接拒绝，**不发请求**（省一次计费）
//! 2. 流式调用（思考过程实时回调）
//! 3. **JSON 解析 + 归一化 + 校验**，失败带上下文**自纠重试**（最多 [`MAX_JSON_RETRY`] 次）
//! 4. 无论成功/失败/异常，**都保证回调用量**（含自纠重试的用量）
//!
//! ## 🔴 四条强制归一化（AI 输出不可信）
//!
//! | # | 动作 | 不做会怎样 |
//! |---|---|---|
//! | 1 | `source = {kind: AI, verified: false, ref: null}` | AI 自填权威出处 → 冒充规范来源 |
//! | 2 | `schemaVersion = 3` | 触发多余的 Excel 字段迁移 |
//! | 3 | 续写时**换新 id** | AI 无法感知库内 id，同名概念复用旧 id → **整行覆盖既有公式**（真机出现过两条公式 `revisedFrom` 互指成环） |
//! | 4 | `resultOutputs = ResultOutputs::sanitize` | 声明项对不上分号段 → 带错标签入库 |
//!
//! ## 🔴 校验失败也走自纠重试（不只是 JSON 解析失败）
//!
//! 只重试 JSON 解析的话，「格式正确但内容错」（方程没写成闭式解、引用了未声明符号）
//! 这类输出会直接失败，用户只能整条重来。所以校验失败走**同一条自纠链路**。
//!
//! ## 🔴 补壳/归一化必须在重试循环**内部**
//!
//! 模型自纠后的新输出同样要过一遍 `normalize` —— 否则会绕过 id 重写与清单修复。

use std::sync::atomic::AtomicBool;

use civilcalc_core::excel::validator::validate as validate_excel;
use civilcalc_core::log;
use civilcalc_core::result_outputs::sanitize as sanitize_result_outputs;
use civilcalc_core::schema::{
    validate_schema, FormulaSchema, FormulaSource, SourceKind,
};
use uuid::Uuid;

use crate::client::LlmClient;
use crate::config::ResolvedLlmProfile;
use crate::error::{LlmError, LlmErrorCode};
use crate::prompts::PROMPT_A;
use crate::protocol::ChatMessage;
use crate::stream::Signal;
use crate::usage::{merge_usage, Usage};

/// JSON 解析失败最大重试次数（**不含**首次）。
pub const MAX_JSON_RETRY: usize = 2;

/// 日志标签（与源 `CivilLog` 的 tag 一致）。
const LOG_TAG: &str = "LlmNormalizer";

/// Schema 示例（与 Prompt A 的输出契约一致）；自纠重试时注入提示词让模型对齐结构。
///
/// ⚠️ 源里的 `schemaVersion` 示例值是 `2`，但归一化会**强制改写成 3** ——
/// 这是源的既有状态（示例没跟着升版），**照抄不改**，避免与源产物不一致。
const SCHEMA_EXAMPLE: &str = r#"{
  "id": "usr:example_001",
  "resultName": "示例公式",
  "resultSymbol": "R",
  "resultUnit": "m",
  "resultOutputs": [],
  "expression": "a + b",
  "sourceEquations": [],
  "altExpressions": [],
  "constants": {},
  "variables": [{"symbol": "a", "desc": "边长a", "unit": "m", "required": true}, {"symbol": "b", "desc": "边长b", "unit": "m", "required": true}],
  "domain": "适用条件",
  "referenceBasis": "经验公式/教材通用式",
  "tags": [],
  "source": {"kind": "AI", "ref": null, "verified": false},
  "stepsTemplate": [],
  "designNotes": "设计要点归纳",
  "explanation": {"summary": "需求理解（①②③分点）", "solution": "解决方式（①②③分点）", "steps": [{"title": "步骤名", "expression": "a + b", "detail": "计算依据与预期结果"}]},
  "schemaVersion": 2,
  "createdAt": 0, "updatedAt": 0
}"#;

/// 归一化请求。
///
/// 源把这一串参数平铺在 `normalizeFromPaste` 上（8 个），Rust 侧收成结构 ——
/// 既避开「参数过多」，也让调用点自解释（`req.rename_to` 比第 5 个位置参数清楚）。
#[derive(Debug, Clone)]
pub struct NormalizeRequest<'a> {
    /// 原始输入（公式文本 / 自然语言描述）
    pub raw: &'a str,
    /// 自定义 Prompt A；`None` 或**空白** → 用内置 [`PROMPT_A`]
    pub prompt_a: Option<&'a str>,
    /// 附图 data URL 列表（仅用户消息携带；上限由调用方保证）
    pub images: &'a [String],
    /// 续写微调：新名称（`Some` 即触发换新 id）
    pub rename_to: Option<&'a str>,
    /// 续写微调：父公式 id（`Some` 即触发换新 id）
    pub revised_from_id: Option<&'a str>,
}

impl<'a> NormalizeRequest<'a> {
    /// 最小请求：只给原始输入，其余取默认（无图、无自定义提示词、非续写）。
    #[must_use]
    pub fn new(raw: &'a str) -> Self {
        Self {
            raw,
            prompt_a: None,
            images: &[],
            rename_to: None,
            revised_from_id: None,
        }
    }

    #[must_use]
    pub fn with_images(mut self, images: &'a [String]) -> Self {
        self.images = images;
        self
    }

    #[must_use]
    pub fn with_prompt(mut self, prompt_a: Option<&'a str>) -> Self {
        self.prompt_a = prompt_a;
        self
    }

    /// 续写微调（`rename_to` / `revised_from_id` 任一为 `Some` → 强制换新 id）。
    #[must_use]
    pub fn as_refine(mut self, rename_to: Option<&'a str>, revised_from_id: Option<&'a str>) -> Self {
        self.rename_to = rename_to;
        self.revised_from_id = revised_from_id;
        self
    }
}

/// 归一化产物。
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizeResult {
    pub schema: FormulaSchema,
    /// 本次调用的用量（含自纠重试的合并值）；厂商未返回时 `None`
    pub usage: Option<Usage>,
}

/// AI 解析器。
#[derive(Debug, Clone)]
pub struct LlmNormalizer {
    client: LlmClient,
    max_json_retry: usize,
}

impl LlmNormalizer {
    #[must_use]
    pub fn new(client: LlmClient) -> Self {
        Self {
            client,
            max_json_retry: MAX_JSON_RETRY,
        }
    }

    /// 自定义重试次数（测试用；生产用 [`MAX_JSON_RETRY`]）。
    #[must_use]
    pub fn with_max_retry(mut self, n: usize) -> Self {
        self.max_json_retry = n;
        self
    }

    #[must_use]
    pub fn client(&self) -> &LlmClient {
        &self.client
    }

    /// 把「公式文本 / 自然语言描述」归一化为 [`FormulaSchema`]。
    ///
    /// - `on_thinking`：思考过程（**累积全文**）
    /// - `on_usage`：无论成功/失败/异常都会回调一次（含自纠重试的合并用量）
    ///
    /// # Errors
    ///
    /// - `VISION_UNSUPPORTED`：带图但 `profile.vision == false`
    /// - 透传流式调用的错误
    /// - `SCHEMA_PARSE_ERROR`：重试耗尽后仍无法解析/校验
    pub async fn normalize_from_paste(
        &self,
        profile: &ResolvedLlmProfile,
        req: &NormalizeRequest<'_>,
        cancel: Option<&AtomicBool>,
        on_thinking: &mut (dyn FnMut(String) + Send),
        on_usage: &mut (dyn FnMut(Usage) + Send),
    ) -> Result<NormalizeResult, LlmError> {
        let mut captured: Option<Usage> = None;
        let mut extra: Option<Usage> = None;

        let result = self
            .run(profile, req, cancel, on_thinking, &mut captured, &mut extra)
            .await;

        // ★ 优先级最高：任何路径都回调 usage（成功、失败、异常）；含自纠重试的用量
        if let Some(u) = merge_usage(captured.as_ref(), extra.as_ref()) {
            on_usage(u);
        }
        result
    }

    async fn run(
        &self,
        profile: &ResolvedLlmProfile,
        req: &NormalizeRequest<'_>,
        cancel: Option<&AtomicBool>,
        on_thinking: &mut (dyn FnMut(String) + Send),
        captured: &mut Option<Usage>,
        extra: &mut Option<Usage>,
    ) -> Result<NormalizeResult, LlmError> {
        // 视觉校验：携带图片但当前模型未开启视觉 → 直接拒绝，不发起请求
        if !req.images.is_empty() && !profile.vision {
            return Err(LlmError::new(
                LlmErrorCode::VisionUnsupported,
                format!(
                    "模型 {} 未开启视觉能力但请求携带 {} 张图片",
                    profile.model,
                    req.images.len()
                ),
                None,
                None,
            ));
        }

        let effective_prompt = match req.prompt_a {
            Some(p) if !p.trim().is_empty() => p,
            _ => PROMPT_A,
        };

        // 附图序号契约：详解里的 {{img:N}} 按附图给出顺序从 1 起；
        // 显式声明张数可显著减少序号臆造
        let user_content = if req.images.is_empty() {
            req.raw.to_string()
        } else {
            format!(
                "{}\n（本次附图 {} 张，序号 1~{}，按附图给出顺序）",
                req.raw,
                req.images.len(),
                req.images.len()
            )
        };

        let messages = vec![
            ChatMessage::system(effective_prompt),
            ChatMessage::user(user_content, req.images.to_vec()),
        ];

        let mut on_signal = |sig: Signal| match sig {
            Signal::Thinking(t) => on_thinking(t),
            Signal::Usage(u) => *captured = Some(u),
            _ => {}
        };

        // max_tokens 不设上限：思考 token 计入该上限（三家一致），复杂需求思考即需 ~10K，
        // 设上限会截断思考致正文为空/不完整 → 不发送该字段，由服务端按模型最大输出
        let chat = self
            .client
            .chat_stream(
                profile,
                &messages,
                true,
                profile.web_search,
                cancel,
                &mut on_signal,
            )
            .await?;

        let resp = chat.content;
        if resp.trim().is_empty() {
            return Err(LlmError::new(LlmErrorCode::Unknown, "AI 响应为空", None, None));
        }

        // JSON 解析（带重试）；补壳/归一化随重试一起做
        let json_text = strip_markdown_fences(&resp);
        let normalize =
            |s: &FormulaSchema| -> FormulaSchema { normalize_schema(s, req.rename_to, req.revised_from_id) };
        let normalized_schema = self
            .safe_parse_json(&json_text, effective_prompt, profile, &normalize, extra)
            .await?;

        // 校验 AI 生成的 Excel 公式（失败仅记录日志，不阻断入库、不污染思考流）
        let excel_validation = validate_excel(&normalized_schema);
        if !excel_validation.ok {
            log::w(
                LOG_TAG,
                "Excel 公式校验警告",
                excel_validation.message.as_deref(),
            );
        }

        Ok(NormalizeResult {
            schema: normalized_schema,
            usage: captured.clone(),
        })
    }

    /// 带重试的「JSON 解析 + 归一化 + 校验」。
    ///
    /// 失败时把「原始输出 + 错误信息 + Schema 示例」拼成修正提示词，
    /// 再调一次**非流式** LLM 让模型自我纠正（该次用量经 `extra` 上报，不丢统计）。
    async fn safe_parse_json(
        &self,
        json_text: &str,
        system_prompt: &str,
        profile: &ResolvedLlmProfile,
        normalize: &(dyn Fn(&FormulaSchema) -> FormulaSchema + Send + Sync),
        extra: &mut Option<Usage>,
    ) -> Result<FormulaSchema, LlmError> {
        let mut last_error: Option<String> = None;
        let mut current_text = json_text.to_string();

        for attempt in 0..=self.max_json_retry {
            let parsed: Option<FormulaSchema> = match serde_json::from_str(&current_text) {
                Ok(v) => Some(v),
                Err(e) => {
                    last_error = Some(e.to_string());
                    // 记录原始返回片段与长度，便于线上定位解析失败根因
                    log::w(
                        LOG_TAG,
                        &format!(
                            "JSON 解析失败（第 {} 次）: {e}；长度={} 开头={}",
                            attempt + 1,
                            current_text.chars().count(),
                            head_chars(&current_text, 400)
                        ),
                        None,
                    );
                    None
                }
            };

            if let Some(parsed) = parsed {
                let candidate = normalize(&parsed);
                match validate_schema(&candidate) {
                    Ok(()) => return Ok(candidate),
                    Err(e) => {
                        last_error = Some(e.error_detail.clone());
                        log::w(
                            LOG_TAG,
                            &format!("Schema 校验失败（第 {} 次）: {}", attempt + 1, e.error_detail),
                            None,
                        );
                    }
                }
            }

            if attempt < self.max_json_retry {
                // 用本次实际生效的 system（而非硬编码默认值），
                // 保证与首次请求同一前缀、便于命中 KV cache
                let fix_messages = vec![
                    ChatMessage::system(system_prompt),
                    ChatMessage::user(
                        format!(
                            "上次输出解析失败，请严格按照 schema 修正后重新输出，只输出 JSON：\n\n\
                             上次输出：\n{current_text}\n\n\
                             错误信息：{}\n\n\
                             正确 schema 示例：\n{SCHEMA_EXAMPLE}",
                            last_error.as_deref().unwrap_or("")
                        ),
                        Vec::new(),
                    ),
                ];
                // 非流式自纠修正（maxTokens 不设上限，截断同样会导致修正失败）
                // ⚠️ 这里 webSearch 传 false（源未传该参数，用默认值）
                match self
                    .client
                    .chat(profile, &fix_messages, true, false, None)
                    .await
                {
                    Ok(r) => {
                        if let Some(u) = r.usage.as_ref() {
                            *extra = merge_usage(extra.as_ref(), Some(u));
                        }
                        if !r.content.trim().is_empty() {
                            current_text = strip_markdown_fences(&r.content);
                        }
                    }
                    // 修正失败，下次重试用原始文本
                    Err(e) => log::w(
                        LOG_TAG,
                        &format!("JSON 自纠重试调用失败（第 {} 次）", attempt + 1),
                        Some(&e.error_detail),
                    ),
                }
            }
        }

        Err(LlmError::new(
            LlmErrorCode::SchemaParseError,
            format!(
                "JSON 解析/校验失败（重试 {} 次后）: {}",
                self.max_json_retry,
                last_error.as_deref().unwrap_or("")
            ),
            None,
            None,
        ))
    }
}

/// 四条强制归一化（见模块文档）。抽成自由函数便于单测。
#[must_use]
pub fn normalize_schema(
    s: &FormulaSchema,
    rename_to: Option<&str>,
    revised_from_id: Option<&str>,
) -> FormulaSchema {
    let mut out = s.clone();
    out.id = new_id_if_refine(&s.id, rename_to, revised_from_id);
    out.source = FormulaSource {
        kind: SourceKind::Ai,
        // AI 不得自填权威出处
        ref_: None,
        verified: false,
        ..s.source.clone()
    };
    // 强制标记为 v3（AI 的 Excel 字段按「第 1 行横向」契约生成）
    out.schema_version = 3;
    // 续写微调：版本化名称 + 指向父公式
    if let Some(name) = rename_to.filter(|n| !n.trim().is_empty()) {
        out.result_name = name.to_string();
    }
    if let Some(rid) = revised_from_id {
        out.revised_from = Some(rid.to_string());
    }
    // 多结果清单：先剔除对不上分号段的声明项，避免带错标签入库
    out.result_outputs = sanitize_result_outputs(s);
    out
}

/// 续写微调（`rename_to` / `revised_from_id` 任一存在）时强制返回**全新 id**。
///
/// AI 无法感知库内既有 id，同名概念续写会复用旧 id 导致**覆盖既有公式**。
/// id 必须满足校验器的 `usr:` 前缀规则，否则校验直接拒绝。
/// 普通 AI 搜索保持 AI 的 id 不变（同 id 视为重新解析同一公式，沿用收藏）。
#[must_use]
pub fn new_id_if_refine(ai_id: &str, rename_to: Option<&str>, revised_from_id: Option<&str>) -> String {
    if rename_to.is_some() || revised_from_id.is_some() {
        format!("usr:{}", Uuid::new_v4())
    } else {
        ai_id.to_string()
    }
}

/// 去掉模型常见的 Markdown 围栏（```json … ```）。
///
/// 顺序与源一致：先 `trim`，再去 ```` ```json ````、去 ```` ``` ````、去尾部 ```` ``` ````，最后再 `trim`。
#[must_use]
pub fn strip_markdown_fences(text: &str) -> String {
    let t = text.trim();
    let t = t.strip_prefix("```json").unwrap_or(t);
    let t = t.strip_prefix("```").unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t);
    t.trim().to_string()
}

/// 取前 `n` 个**字符**（不是字节）—— 日志里截断中文不能切碎 UTF-8。
fn head_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use civilcalc_core::schema::{ResultOutput, CURRENT_SCHEMA_VERSION};
    use std::collections::HashMap;

    fn schema(expression: &str, outputs: Vec<ResultOutput>) -> FormulaSchema {
        FormulaSchema {
            id: "usr:from_ai".to_string(),
            result_name: "AI 公式".to_string(),
            result_symbol: "R".to_string(),
            result_unit: Some("m".to_string()),
            result_outputs: outputs,
            expression: expression.to_string(),
            source_equations: Vec::new(),
            alt_expressions: Vec::new(),
            constants: HashMap::new(),
            variables: Vec::new(),
            domain: "结构".to_string(),
            tags: Vec::new(),
            reference_basis: None,
            design_notes: None,
            explanation: None,
            image_ids: Vec::new(),
            revised_from: None,
            source: FormulaSource {
                kind: SourceKind::Custom,
                ref_: Some("GB 50010-2010".to_string()),
                verified: true,
                model: Some("glm-4".to_string()),
                created_by: Some("user".to_string()),
            },
            steps_template: None,
            doc_template_id: None,
            excel_expression: None,
            excel_alt_expressions: None,
            excel_steps_template: None,
            excel_function_docs: None,
            schema_version: 2,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn out(symbol: &str) -> ResultOutput {
        ResultOutput {
            symbol: symbol.to_string(),
            name: format!("{symbol} 名"),
            unit: "m".to_string(),
        }
    }

    // ---------------------------------------------------------------------
    // strip_markdown_fences
    // ---------------------------------------------------------------------

    #[test]
    fn fences_json_block() {
        assert_eq!(strip_markdown_fences("```json\n{\"a\":1}\n```"), "{\"a\":1}");
    }

    #[test]
    fn fences_bare_block() {
        assert_eq!(strip_markdown_fences("```\n{\"a\":1}\n```"), "{\"a\":1}");
    }

    #[test]
    fn fences_without_block_passthrough() {
        assert_eq!(strip_markdown_fences("  {\"a\":1}  "), "{\"a\":1}");
    }

    #[test]
    fn fences_only_opening() {
        assert_eq!(strip_markdown_fences("```json\n{\"a\":1}"), "{\"a\":1}");
    }

    /// 只删**一层**围栏（源用 removePrefix，不做循环剥离）
    #[test]
    fn fences_only_one_layer() {
        assert_eq!(
            strip_markdown_fences("```json\n```json\n{\"a\":1}"),
            "```json\n{\"a\":1}",
            "第二层围栏保留（与源 removePrefix 行为一致）"
        );
    }

    #[test]
    fn fences_empty_and_whitespace() {
        assert_eq!(strip_markdown_fences(""), "");
        assert_eq!(strip_markdown_fences("   \n  "), "");
        assert_eq!(strip_markdown_fences("```"), "");
    }

    // ---------------------------------------------------------------------
    // new_id_if_refine
    // ---------------------------------------------------------------------

    #[test]
    fn plain_search_keeps_ai_id() {
        assert_eq!(new_id_if_refine("usr:abc", None, None), "usr:abc");
    }

    /// 🔴 续写（任一参数存在）必须换新 id，且满足 `usr:` 前缀
    #[test]
    fn refine_always_mints_new_id() {
        let a = new_id_if_refine("usr:abc", Some("新名"), None);
        let b = new_id_if_refine("usr:abc", None, Some("usr:parent"));
        assert_ne!(a, "usr:abc");
        assert_ne!(b, "usr:abc");
        assert!(a.starts_with("usr:"), "必须满足校验器的 usr: 前缀规则");
        assert!(b.starts_with("usr:"));
        assert_ne!(a, b, "每次都应不同");
    }

    /// 空白的 `rename_to` 也算「续写」（源判据是 `!= null`，不是「非空」）
    #[test]
    fn blank_rename_still_counts_as_refine() {
        let id = new_id_if_refine("usr:abc", Some("   "), None);
        assert_ne!(id, "usr:abc", "源判据是 null 而非 blank");
    }

    // ---------------------------------------------------------------------
    // normalize_schema：四条强制归一化
    // ---------------------------------------------------------------------

    /// 🔴 强制 AI 来源：`verified=false` + `ref=null`，且**保留** model/created_by
    #[test]
    fn forces_ai_source_and_clears_ref() {
        let s = schema("a+b", vec![]);
        let n = normalize_schema(&s, None, None);
        assert_eq!(n.source.kind, SourceKind::Ai);
        assert!(!n.source.verified, "AI 产出一律 verified=false");
        assert_eq!(n.source.ref_, None, "必须清掉 AI 自填的权威出处");
        assert_eq!(n.source.model.as_deref(), Some("glm-4"), "其余字段保留");
        assert_eq!(n.source.created_by.as_deref(), Some("user"));
    }

    /// 🔴 强制 schemaVersion = 3
    #[test]
    fn forces_schema_version_three() {
        let s = schema("a+b", vec![]);
        assert_eq!(s.schema_version, 2);
        assert_eq!(normalize_schema(&s, None, None).schema_version, 3);
        assert_ne!(CURRENT_SCHEMA_VERSION, 0, "core 的常量仍可用");
    }

    /// 普通搜索不改名、不设 revisedFrom、不改 id
    #[test]
    fn plain_search_keeps_identity_fields() {
        let s = schema("a+b", vec![]);
        let n = normalize_schema(&s, None, None);
        assert_eq!(n.id, "usr:from_ai");
        assert_eq!(n.result_name, "AI 公式");
        assert_eq!(n.revised_from, None);
    }

    /// 续写：改名 + 指向父公式 + 换 id
    #[test]
    fn refine_sets_name_parent_and_new_id() {
        let s = schema("a+b", vec![]);
        let n = normalize_schema(&s, Some("矩形面积-v2.0"), Some("usr:parent"));
        assert_eq!(n.result_name, "矩形面积-v2.0");
        assert_eq!(n.revised_from.as_deref(), Some("usr:parent"));
        assert_ne!(n.id, "usr:from_ai");
        assert!(n.id.starts_with("usr:"));
    }

    /// 空白 rename 不改名（但 id 已换 —— 两条判据不同，源如此）
    #[test]
    fn blank_rename_keeps_original_name() {
        let s = schema("a+b", vec![]);
        let n = normalize_schema(&s, Some("  "), Some("usr:p"));
        assert_eq!(n.result_name, "AI 公式", "空白名不覆盖原名");
        assert_ne!(n.id, "usr:from_ai", "但 id 仍换新");
    }

    /// 🔴 多结果清单被清洗：对不上分号段的声明项剔除
    #[test]
    fn sanitizes_result_outputs() {
        // 单段表达式声明了 x/y 两个输出 → 全部剔除（源 sanitize 规则）
        let s = schema("a+b", vec![out("x"), out("y")]);
        let n = normalize_schema(&s, None, None);
        assert!(
            n.result_outputs.is_empty(),
            "单段表达式不该保留多结果声明，实际 {:?}",
            n.result_outputs
        );
    }

    /// 多段表达式且声明与段符号一致 → 保留
    #[test]
    fn sanitize_keeps_matching_declarations() {
        let s = schema("x = a+b; y = a-b", vec![out("x"), out("y")]);
        let n = normalize_schema(&s, None, None);
        let syms: Vec<&str> = n.result_outputs.iter().map(|o| o.symbol.as_str()).collect();
        assert_eq!(syms, ["x", "y"]);
    }

    /// 清洗用的是**原始** schema（与源 `sanitize(raw)` 一致）
    #[test]
    fn sanitize_reads_original_schema() {
        let s = schema("x = a+b; y = a-b", vec![out("x")]);
        let n = normalize_schema(&s, Some("新名"), Some("usr:p"));
        assert_eq!(n.result_outputs.len(), 1);
        assert_eq!(n.result_outputs[0].symbol, "x");
    }

    /// 归一化不改表达式
    #[test]
    fn expression_untouched() {
        let s = schema("sqrt(a^2+b^2)", vec![]);
        assert_eq!(normalize_schema(&s, None, None).expression, "sqrt(a^2+b^2)");
    }

    // ---------------------------------------------------------------------
    // 常量与工具
    // ---------------------------------------------------------------------

    #[test]
    fn max_json_retry_is_two() {
        assert_eq!(MAX_JSON_RETRY, 2, "源 maxJsonRetry = 2");
    }

    #[test]
    fn schema_example_is_valid_json() {
        let v: serde_json::Value =
            serde_json::from_str(SCHEMA_EXAMPLE).expect("示例必须是合法 JSON（会被塞进提示词）");
        assert_eq!(v["source"]["kind"], "AI");
        assert_eq!(v["source"]["verified"], false);
    }

    /// 示例里的 `schemaVersion` 是 2（源如此），但归一化会强制 3 —— 两者独立
    #[test]
    fn schema_example_keeps_source_version_two() {
        let v: serde_json::Value = serde_json::from_str(SCHEMA_EXAMPLE).unwrap();
        assert_eq!(
            v["schemaVersion"], 2,
            "源示例写的是 2（没跟着升版），照抄不改"
        );
    }

    #[test]
    fn head_chars_is_char_safe() {
        // 3 个汉字 = 9 字节；按字符截取不应切碎
        assert_eq!(head_chars("甲乙丙丁", 2), "甲乙");
        assert_eq!(head_chars("abc", 10), "abc");
        assert_eq!(head_chars("", 5), "");
    }

    #[test]
    fn normalizer_constructs() {
        let n = LlmNormalizer::new(LlmClient::new());
        assert_eq!(n.max_json_retry, MAX_JSON_RETRY);
        let n2 = LlmNormalizer::new(LlmClient::new()).with_max_retry(5);
        assert_eq!(n2.max_json_retry, 5);
        let _ = n.client();
    }
}
