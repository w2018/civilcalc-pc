//! 按需为**既有**公式生成「计算公式详解」（Prompt Explain，旧公式补齐用）。
//!
//! 源：`civilcalc-android-v2/core/llm/FormulaExplainer.kt`（135 行）
//!
//! 与 [`crate::normalizer`] 的关系：normalizer 是「从需求生成**整条公式**」，
//! 本模块是「给**已有公式**补一份详解」。两者都走流式 + 自纠重试，但：
//!
//! | | normalizer | explainer |
//! |---|---|---|
//! | system 提示 | Prompt A（可带详解段） | Prompt Explain |
//! | 解析目标 | `FormulaSchema` | `FormulaExplanation` |
//! | 重试次数 | 2（可配） | **1**（源 `repeat(2)`） |
//! | 修正提示 | 附 schema 示例 | **不附**（输出结构简单） |
//!
//! ## 🔴 附图序号契约
//!
//! 带图时 user 消息要追加「本次附图 N 张，序号 1~N」——模型据此才敢在详解里写
//! `{{img:N}}`。`images` 顺序**必须**与 `schema.imageIds` 一致，否则序号错位。

use std::sync::atomic::AtomicBool;

use civilcalc_core::log;
use civilcalc_core::result_outputs::build_declared_summary;
use civilcalc_core::schema::{FormulaExplanation, FormulaSchema};
use civilcalc_core::schema::image_marker;

use crate::client::LlmClient;
use crate::config::ResolvedLlmProfile;
use crate::error::{LlmError, LlmErrorCode};
use crate::normalizer::strip_markdown_fences;
use crate::prompts::PROMPT_EXPLAIN;
use crate::protocol::ChatMessage;
use crate::stream::Signal;
use crate::usage::{merge_usage, Usage};

/// 日志标签（与源 `CivilLog` 的 tag 一致）。
const LOG_TAG: &str = "FormulaExplainer";

/// 详解解析重试次数（**不含**首次）—— 源是 `repeat(2)`，即最多 2 次尝试。
pub const MAX_EXPLAIN_RETRY: usize = 1;

/// 按需生成详解。
#[derive(Debug, Clone)]
pub struct LlmExplainer {
    client: LlmClient,
}

impl LlmExplainer {
    #[must_use]
    pub fn new(client: LlmClient) -> Self {
        Self { client }
    }

    #[must_use]
    pub fn client(&self) -> &LlmClient {
        &self.client
    }

    /// 为既有公式生成详解。
    ///
    /// `user_text` 一般由 [`build_user_text`] 生成；`images` 是该公式当初生成时所依据的
    /// 附图（data URL，**顺序与 `schema.imageIds` 一致**）。
    ///
    /// `on_usage`：无论成功/失败/异常都回调一次（含自纠重试的合并用量）；
    /// 厂商未返回 usage 时**不回调**（不写本地估算值）。
    ///
    /// # Errors
    ///
    /// - 透传流式调用的错误
    /// - `UNKNOWN`：AI 响应为空
    /// - `SCHEMA_PARSE_ERROR`：重试后仍无法解析为 [`FormulaExplanation`]
    pub async fn explain(
        &self,
        profile: &ResolvedLlmProfile,
        user_text: &str,
        images: &[String],
        cancel: Option<&AtomicBool>,
        on_usage: &mut (dyn FnMut(Usage) + Send),
    ) -> Result<FormulaExplanation, LlmError> {
        let mut captured: Option<Usage> = None;
        let mut extra: Option<Usage> = None;

        let result = self
            .run(profile, user_text, images, cancel, &mut captured, &mut extra)
            .await;

        if let Some(u) = merge_usage(captured.as_ref(), extra.as_ref()) {
            on_usage(u);
        }
        result
    }

    async fn run(
        &self,
        profile: &ResolvedLlmProfile,
        user_text: &str,
        images: &[String],
        cancel: Option<&AtomicBool>,
        captured: &mut Option<Usage>,
        extra: &mut Option<Usage>,
    ) -> Result<FormulaExplanation, LlmError> {
        let user_content = if images.is_empty() {
            user_text.to_string()
        } else {
            format!(
                "{user_text}（本次附图 {} 张，序号 1~{}，按附图给出顺序）",
                images.len(),
                images.len()
            )
        };
        let messages = vec![
            ChatMessage::system(PROMPT_EXPLAIN),
            ChatMessage::user(user_content, images.to_vec()),
        ];

        let mut on_signal = |sig: Signal| {
            if let Signal::Usage(u) = sig {
                *captured = Some(u);
            }
        };

        // max_tokens 不设上限：思考 token 计入该上限，设小值会截断思考致正文为空
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

        let content = chat.content;
        if content.trim().is_empty() {
            return Err(LlmError::new(LlmErrorCode::Unknown, "AI 响应为空", None, None));
        }
        self.parse_with_retry(&strip_markdown_fences(&content), profile, extra)
            .await
    }

    /// 解析失败时把原输出 + 错误拼进修正提示词，用**非流式**调用自纠重试 1 次。
    async fn parse_with_retry(
        &self,
        text: &str,
        profile: &ResolvedLlmProfile,
        extra: &mut Option<Usage>,
    ) -> Result<FormulaExplanation, LlmError> {
        let mut last_error: Option<String> = None;
        let mut current_text = text.to_string();

        for attempt in 0..=MAX_EXPLAIN_RETRY {
            match serde_json::from_str::<FormulaExplanation>(&current_text) {
                Ok(parsed) => return Ok(parsed),
                Err(e) => {
                    last_error = Some(e.to_string());
                    log::w(
                        LOG_TAG,
                        &format!(
                            "JSON 解析失败（第 {} 次）: {e}；长度={} 开头={}",
                            attempt + 1,
                            current_text.chars().count(),
                            head_chars(&current_text, 300)
                        ),
                        None,
                    );
                }
            }

            if attempt == 0 {
                let fix_messages = vec![
                    ChatMessage::system(PROMPT_EXPLAIN),
                    ChatMessage::user(
                        format!(
                            "上次输出解析失败，请严格按照输出 Schema 修正后重新输出，只输出 JSON，\
                             不要 markdown 代码块，不要解释文字：\n\n\
                             上次输出：\n{current_text}\n\n\
                             错误信息：{}",
                            last_error.as_deref().unwrap_or("")
                        ),
                        Vec::new(),
                    ),
                ];
                // 非流式自纠（maxTokens 不设上限，截断同样会导致修正失败）；
                // ⚠️ webSearch 传 false（源未传该参数，用默认值）
                match self.client.chat(profile, &fix_messages, true, false, None).await {
                    Ok(r) => {
                        if let Some(u) = r.usage.as_ref() {
                            *extra = merge_usage(extra.as_ref(), Some(u));
                        }
                        if !r.content.trim().is_empty() {
                            current_text = strip_markdown_fences(&r.content);
                        }
                    }
                    Err(e) => log::w(LOG_TAG, "JSON 自纠重试调用失败", Some(&e.error_detail)),
                }
            }
        }

        Err(LlmError::new(
            LlmErrorCode::SchemaParseError,
            format!(
                "JSON 解析失败（重试 {MAX_EXPLAIN_RETRY} 次后）: {}",
                last_error.as_deref().unwrap_or("")
            ),
            None,
            None,
        ))
    }
}

/// 拼装按需生成详解的 user 消息。
///
/// 只取**关键信息**（不整包序列化 `FormulaSchema`）—— 否则 Excel 字段等冗余会干扰模型。
#[must_use]
pub fn build_user_text(schema: &FormulaSchema) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "公式名称：{}（结果符号 {}，单位 {}）\n",
        schema.result_name,
        schema.result_symbol,
        schema.result_unit.as_deref().unwrap_or("")
    ));
    out.push_str(&format!("主表达式：{}\n", schema.expression));

    // 多结果（方程组等）：详解要按每个输出分别讲解，缺了这行模型会只当成单个结果
    if !schema.result_outputs.is_empty() {
        out.push_str(&format!(
            "结果输出：{}\n",
            build_declared_summary(&schema.result_outputs)
        ));
    }

    if !schema.alt_expressions.is_empty() {
        let joined = schema
            .alt_expressions
            .iter()
            .map(|alt| {
                let cond = alt
                    .condition
                    .as_ref()
                    .map(|c| format!("（条件：{c}）"))
                    .unwrap_or_default();
                format!("{}:{}{}", alt.label, alt.expression, cond)
            })
            .collect::<Vec<_>>()
            .join("；");
        out.push_str(&format!("分支公式：{joined}\n"));
    }

    if !schema.variables.is_empty() {
        let joined = schema
            .variables
            .iter()
            .map(|v| {
                let unit = v
                    .unit
                    .as_deref()
                    .filter(|u| !u.trim().is_empty())
                    .map(|u| format!("（{u}）"))
                    .unwrap_or_default();
                format!("{}={}{unit}", v.symbol, v.desc)
            })
            .collect::<Vec<_>>()
            .join("；");
        out.push_str(&format!("变量：{joined}\n"));
    }

    if !schema.constants.is_empty() {
        // 与 refine 同因：HashMap 无序 → 排序保证前缀稳定（见 refine 模块文档）
        let mut items: Vec<(&str, f64)> = schema
            .constants
            .iter()
            .map(|(k, v)| (k.as_str(), *v))
            .collect();
        items.sort_by(|a, b| a.0.cmp(b.0));
        let joined = items
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("；");
        out.push_str(&format!("常量：{joined}\n"));
    }

    if let Some(steps) = schema.steps_template.as_ref().filter(|s| !s.is_empty()) {
        let joined = steps
            .iter()
            .map(|s| format!("{}={}（{}）", s.symbol, s.expression, s.label))
            .collect::<Vec<_>>()
            .join("；");
        out.push_str(&format!("计算步骤：{joined}\n"));
    }

    if !schema.domain.trim().is_empty() {
        out.push_str(&format!("适用条件：{}\n", schema.domain));
    }
    if let Some(b) = schema.reference_basis.as_deref().filter(|b| !b.trim().is_empty()) {
        out.push_str(&format!("依据：{b}\n"));
    }
    if let Some(n) = schema.design_notes.as_deref().filter(|n| !n.trim().is_empty()) {
        out.push_str(&format!("设计要点：{n}\n"));
    }
    out
}

/// 取前 `n` 个**字符**（日志截断中文不能切碎 UTF-8）。
fn head_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// 供测试/调用方复用的图片标记剥离（避免各处重复引 `core::schema`）。
#[must_use]
pub fn strip_image_markers(text: &str) -> String {
    image_marker::strip_all(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use civilcalc_core::schema::{
        AltExpression, FormulaSource, FormulaVar, ResultOutput, SourceKind, StepTemplate,
    };
    use std::collections::HashMap;

    fn base_schema() -> FormulaSchema {
        FormulaSchema {
            id: "usr:1".into(),
            result_name: "矩形面积".into(),
            result_symbol: "A".into(),
            result_unit: Some("m²".into()),
            result_outputs: Vec::new(),
            expression: "b*h".into(),
            source_equations: Vec::new(),
            alt_expressions: Vec::new(),
            constants: HashMap::new(),
            variables: Vec::new(),
            domain: String::new(),
            tags: Vec::new(),
            reference_basis: None,
            design_notes: None,
            explanation: None,
            image_ids: Vec::new(),
            revised_from: None,
            source: FormulaSource {
                kind: SourceKind::Ai,
                ref_: None,
                verified: false,
                model: None,
                created_by: None,
            },
            steps_template: None,
            doc_template_id: None,
            excel_expression: None,
            excel_alt_expressions: None,
            excel_steps_template: None,
            excel_function_docs: None,
            schema_version: 3,
            created_at: 0,
            updated_at: 0,
        }
    }

    // ---------------------------------------------------------------------
    // build_user_text
    // ---------------------------------------------------------------------

    #[test]
    fn user_text_header() {
        let t = build_user_text(&base_schema());
        assert!(t.starts_with("公式名称：矩形面积（结果符号 A，单位 m²）\n"));
        assert!(t.contains("\n主表达式：b*h\n"));
    }

    #[test]
    fn user_text_empty_unit() {
        let mut s = base_schema();
        s.result_unit = None;
        assert!(build_user_text(&s).starts_with("公式名称：矩形面积（结果符号 A，单位 ）\n"));
    }

    /// 变量用**全角括号**包单位（与 refine 的「，单位」写法不同 —— 源如此）
    #[test]
    fn user_text_variables_use_parens() {
        let mut s = base_schema();
        s.variables = vec![
            FormulaVar {
                symbol: "b".into(),
                desc: "宽".into(),
                unit: Some("m".into()),
                default: None,
                min: None,
                max: None,
                required: true,
            },
            FormulaVar {
                symbol: "k".into(),
                desc: "系数".into(),
                unit: None,
                default: None,
                min: None,
                max: None,
                required: true,
            },
        ];
        let t = build_user_text(&s);
        assert!(t.contains("变量：b=宽（m）；k=系数\n"), "实际：{t}");
    }

    #[test]
    fn user_text_blank_unit_omitted() {
        let mut s = base_schema();
        s.variables = vec![FormulaVar {
            symbol: "b".into(),
            desc: "宽".into(),
            unit: Some("  ".into()),
            default: None,
            min: None,
            max: None,
            required: true,
        }];
        assert!(build_user_text(&s).contains("变量：b=宽\n"));
    }

    #[test]
    fn user_text_constants_sorted() {
        let mut s = base_schema();
        s.constants = HashMap::from([("beta".to_string(), 2.5), ("alpha".to_string(), 206000.0)]);
        let a = build_user_text(&s);
        assert_eq!(a, build_user_text(&s), "两次必须一致（缓存前缀稳定）");
        assert!(a.contains("常量：alpha=206000；beta=2.5\n"), "实际：{a}");
    }

    #[test]
    fn user_text_steps_and_outputs() {
        let mut s = base_schema();
        s.expression = "x = a+b; y = a-b".into();
        s.result_outputs = vec![
            ResultOutput { symbol: "x".into(), name: "X".into(), unit: "m".into() },
        ];
        s.steps_template = Some(vec![StepTemplate {
            symbol: "A".into(),
            label: "底面积".into(),
            group: None,
            expression: "pi*r^2".into(),
            unit: "m²".into(),
            note: None,
        }]);
        let t = build_user_text(&s);
        assert!(t.contains("结果输出："));
        assert!(t.contains("计算步骤：A=pi*r^2（底面积）\n"), "实际：{t}");
    }

    #[test]
    fn user_text_alt_expressions() {
        let mut s = base_schema();
        s.alt_expressions = vec![AltExpression {
            label: "短柱".into(),
            expression: "a+b".into(),
            condition: Some("h/b<=4".into()),
        }];
        assert!(build_user_text(&s).contains("分支公式：短柱:a+b（条件：h/b<=4）\n"));
    }

    #[test]
    fn user_text_optional_lines_absent_when_blank() {
        let t = build_user_text(&base_schema());
        for absent in ["结果输出：", "分支公式：", "变量：", "常量：", "计算步骤：", "适用条件：", "依据：", "设计要点："] {
            assert!(!t.contains(absent), "不该出现 `{absent}`：{t}");
        }
    }

    #[test]
    fn user_text_includes_domain_basis_notes() {
        let mut s = base_schema();
        s.domain = "矩形".into();
        s.reference_basis = Some("教材通用式".into());
        s.design_notes = Some("按净尺寸".into());
        let t = build_user_text(&s);
        assert!(t.contains("适用条件：矩形\n"));
        assert!(t.contains("依据：教材通用式\n"));
        assert!(t.contains("设计要点：按净尺寸\n"));
    }

    /// **不**序列化 Excel 字段（源注释：避免 Excel 冗余干扰）
    #[test]
    fn user_text_excludes_excel_fields() {
        let mut s = base_schema();
        s.excel_expression = Some("=B1*C1".into());
        assert!(!build_user_text(&s).contains("=B1*C1"));
    }

    // ---------------------------------------------------------------------
    // 常量与工具
    // ---------------------------------------------------------------------

    #[test]
    fn retry_budget_is_one() {
        assert_eq!(MAX_EXPLAIN_RETRY, 1, "源 repeat(2) = 最多 2 次尝试");
    }

    #[test]
    fn head_chars_is_char_safe() {
        assert_eq!(head_chars("甲乙丙", 2), "甲乙");
    }

    #[test]
    fn strip_image_markers_works() {
        assert_eq!(strip_image_markers("看{{img:1}}这里"), "看这里");
    }

    #[test]
    fn explainer_constructs() {
        let e = LlmExplainer::new(LlmClient::new());
        let _ = e.client();
    }
}
