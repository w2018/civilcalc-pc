//! HTML 预览渲染（ADR-018）。
//!
//! ## 为什么有这个东西
//!
//! 源项目**没有**导出前预览 —— 只能先导出 `.docx` 再打开看，不满意就重导。
//! PC 端可以低成本提供：复用同一份 [`TemplateSection`] 渲染逻辑，只把输出
//! 从 docx 换成 HTML。
//!
//! ## ⚠️ 两套渲染的视觉一致性风险（ADR-018 已记录）
//!
//! docx 与 HTML 是两套渲染代码，存在**长得不一样**的风险。缓解措施：
//!
//! 1. **共享同一份章节顺序与标题常量** —— 章节顺序由 [`ReportTemplate::sections`]
//!    决定，标题取 [`TemplateSection::title`]，两处都从同一来源读
//! 2. **共享结果口径** —— 都用 [`civilcalc_core::result_outputs::resolve`]
//! 3. **预览页显式标注**「预览仅供参考，实际以导出文件为准」
//!
//! ## 自包含
//!
//! 产物是**单个 HTML 字符串**（样式内联，无外部资源）—— 因为它要在
//! Tauri webview 里直接渲染，且应用的 CSP 不允许 `unsafe-eval`、
//! 也不该为预览放行外部域名。
//!
//! ## 转义
//!
//! 所有来自 schema / 用户输入的内容都经 [`escape_html`]。
//! 公式名、变量说明、AI 详解都是**不可信文本**（AI 可能产出 `<script>`）。

use std::collections::HashMap;

use civilcalc_core::schema::{plain_desc, EvalResult, FormulaSchema, SourceKind, StepResult};

use civilcalc_core::number_format::format_number;
use crate::report_text::{bold_segments, plain_lines_keeping_emphasis};
use civilcalc_core::result_outputs::{self, ResolvedResult};
use crate::template_model::{NotesConfig, ReportTemplate, TemplateSection};

/// 封面固定主标题（与 docx 一致）
const COVER_TITLE: &str = "工程计算书";

/// 中文数字（到「十」，之后用阿拉伯数字）—— 与 docx 同一套
const CN_NUMS: [&str; 10] = ["一", "二", "三", "四", "五", "六", "七", "八", "九", "十"];

/// 分步计算表表头（6 列，顺序与 docx 一致）
const STEP_TABLE_HEADERS: [&str; 6] = [
    "序号",
    "表达式",
    "代入数值",
    "结果",
    "Excel 公式",
    "Excel 代入数值",
];

/// 渲染计算书 HTML 预览。
///
/// 返回**完整 HTML 文档**（含 `<!DOCTYPE html>`），可直接塞进 `iframe`
/// 的 `srcdoc` 或用 `v-html` 挂到容器里。
pub fn render_html(
    schema: &FormulaSchema,
    inputs: &HashMap<String, f64>,
    result: &EvalResult,
    template: &ReportTemplate,
    step_results: &[StepResult],
) -> String {
    let mut body = String::with_capacity(4096);

    render_cover(&mut body, schema, template);

    let mut idx = 0usize;
    for section in &template.sections {
        match section {
            TemplateSection::FormulaInfo {
                show_source,
                show_verification,
                ..
            } => render_formula_info(
                &mut body,
                schema,
                *show_source,
                *show_verification,
                &next_title(&mut idx),
            ),
            TemplateSection::ParamsTable { columns } => {
                render_params_table(&mut body, schema, inputs, columns, &next_title(&mut idx))
            }
            // ⚠️ `Steps` 不渲染、也不占序号（与 docx 一致）
            TemplateSection::Steps { .. } => {}
            TemplateSection::StepResults { .. } => {
                render_step_results(&mut body, step_results, &next_title(&mut idx))
            }
            TemplateSection::ResultBlock { .. } => {
                render_result_block(&mut body, schema, result, &next_title(&mut idx))
            }
            TemplateSection::Explanation => {
                render_explanation(&mut body, schema, &next_title(&mut idx))
            }
            TemplateSection::Notes { config } => {
                render_notes(&mut body, schema, config, &next_title(&mut idx))
            }
        }
    }

    wrap_document(&body)
}

/// 下一个章节序号（与 docx 同规则）
fn next_title(idx: &mut usize) -> String {
    let i = *idx;
    *idx += 1;
    match CN_NUMS.get(i) {
        Some(cn) => format!("{cn}、"),
        None => format!("{}、", i + 1),
    }
}

// =============================================================================
// 文档外壳
// =============================================================================

fn wrap_document(body: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
:root {{
  --pv-text: #1f1f1f;
  --pv-muted: #6b6b6b;
  --pv-bg: #ffffff;
  --pv-surface: #f7f7f7;
  --pv-line: #e0e0e0;
  --pv-accent: #07c160;
}}
@media (prefers-color-scheme: dark) {{
  :root {{
    --pv-text: #e8e8e8;
    --pv-muted: #9a9a9a;
    --pv-bg: #1a1a1a;
    --pv-surface: #242424;
    --pv-line: #3a3a3a;
  }}
}}
* {{ box-sizing: border-box; }}
body {{
  margin: 0;
  padding: 24px 28px 48px;
  background: var(--pv-bg);
  color: var(--pv-text);
  font-family: system-ui, "Microsoft YaHei", sans-serif;
  font-size: 14px;
  line-height: 1.7;
}}
.pv-note {{
  margin-bottom: 20px;
  padding: 8px 12px;
  border-left: 3px solid var(--pv-accent);
  background: var(--pv-surface);
  color: var(--pv-muted);
  font-size: 12px;
}}
.pv-cover {{ text-align: center; padding: 32px 0 40px; }}
.pv-cover h1 {{ font-size: 26px; margin: 0 0 12px; }}
.pv-cover .pv-calc {{ font-size: 18px; margin: 0 0 8px; }}
.pv-cover .pv-field {{ color: var(--pv-muted); margin: 4px 0; }}
h2 {{ font-size: 16px; margin: 28px 0 10px; }}
h3 {{ font-size: 14px; margin: 16px 0 6px; }}
table {{ width: 100%; border-collapse: collapse; margin: 8px 0 4px; }}
th, td {{
  border: 1px solid var(--pv-line);
  padding: 6px 10px;
  text-align: left;
  font-size: 13px;
  vertical-align: top;
}}
th {{ background: var(--pv-surface); font-weight: 500; }}
.pv-result {{
  text-align: center;
  font-size: 18px;
  font-weight: 500;
  margin: 12px 0;
}}
.pv-empty {{ color: var(--pv-muted); font-style: italic; }}
.pv-notes {{ color: var(--pv-muted); font-size: 12px; }}
.pv-notes p {{ margin: 4px 0; }}
.pv-step-title {{ font-weight: 500; margin: 10px 0 2px; }}
.pv-step-expr {{ color: var(--pv-muted); font-family: ui-monospace, Consolas, monospace; font-size: 12px; }}
.pv-strike {{ color: var(--pv-muted); }}
</style>
</head>
<body>
<p class="pv-note">预览仅供参考，实际以导出的 Word 文件为准。</p>
{body}
</body>
</html>
"#,
        title = escape_html(COVER_TITLE),
        body = body
    )
}

// =============================================================================
// 各章节
// =============================================================================

fn render_cover(out: &mut String, schema: &FormulaSchema, template: &ReportTemplate) {
    let cover = &template.cover;
    out.push_str("<div class=\"pv-cover\">\n");

    if cover.show_project_name {
        out.push_str(&format!("<h1>{}</h1>\n", escape_html(COVER_TITLE)));
    }
    if cover.show_calculator_name {
        let name = cover
            .calculator_name_alias
            .as_deref()
            .unwrap_or(schema.result_name.as_str());
        out.push_str(&format!(
            "<p class=\"pv-calc\">{}</p>\n",
            escape_html(name)
        ));
    }
    for field in &cover.custom_fields {
        out.push_str(&format!(
            "<p class=\"pv-field\">{}：________________</p>\n",
            escape_html(&field.label)
        ));
    }

    out.push_str("</div>\n");
}

fn render_formula_info(
    out: &mut String,
    schema: &FormulaSchema,
    show_source: bool,
    show_verification: bool,
    title: &str,
) {
    out.push_str(&format!("<h2>{}公式信息</h2>\n<table>\n", escape_html(title)));

    row_2col(out, "公式名称", &schema.result_name);
    row_2col(out, "数学表达式", &schema.expression);

    if show_source {
        let kind = source_kind_label(schema.source.kind);
        let text = match schema.source.ref_.as_deref() {
            Some(r) if !r.is_empty() => format!("{kind}（{r}）"),
            _ => kind.to_string(),
        };
        row_2col(out, "来源", &text);
    }

    if show_verification {
        let basis = schema.reference_basis.clone().unwrap_or_else(|| {
            if schema.source.verified {
                "已验证".to_string()
            } else {
                "未验证".to_string()
            }
        });
        row_2col(out, "公式依据", &basis);
    }

    out.push_str("</table>\n");
}

fn row_2col(out: &mut String, key: &str, value: &str) {
    out.push_str(&format!(
        "<tr><th>{}</th><td>{}</td></tr>\n",
        escape_html(key),
        escape_html(value)
    ));
}

fn source_kind_label(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Standard => "国家标准规范",
        SourceKind::Ai => "AI 生成",
        SourceKind::Custom => "自定义",
        SourceKind::Derived => "推导公式",
    }
}

fn render_params_table(
    out: &mut String,
    schema: &FormulaSchema,
    inputs: &HashMap<String, f64>,
    columns: &[String],
    title: &str,
) {
    out.push_str(&format!(
        "<h2>{}参数取值</h2>\n<table>\n<thead><tr>",
        escape_html(title)
    ));
    for c in columns {
        out.push_str(&format!("<th>{}</th>", escape_html(param_column_label(c))));
    }
    out.push_str("</tr></thead>\n<tbody>\n");

    for v in &schema.variables {
        out.push_str("<tr>");
        for c in columns {
            let text = match c.as_str() {
                "symbol" => v.symbol.clone(),
                "desc" => plain_desc(v),
                "value" => inputs
                    .get(&v.symbol)
                    .map(|x| format_number(*x))
                    .or_else(|| v.default.map(format_number))
                    .unwrap_or_else(|| "—".to_string()),
                "unit" => v.unit.clone().unwrap_or_default(),
                _ => String::new(),
            };
            out.push_str(&format!("<td>{}</td>", escape_html(&text)));
        }
        out.push_str("</tr>\n");
    }

    out.push_str("</tbody>\n</table>\n");
}

fn param_column_label(col: &str) -> &str {
    match col {
        "symbol" => "符号",
        "desc" => "说明",
        "value" => "取值",
        "unit" => "单位",
        other => other,
    }
}

fn render_step_results(out: &mut String, steps: &[StepResult], title: &str) {
    out.push_str(&format!("<h2>{}分步计算</h2>\n", escape_html(title)));

    if steps.is_empty() {
        out.push_str("<p class=\"pv-empty\">无</p>\n");
        return;
    }

    out.push_str("<table>\n<thead><tr>");
    for h in STEP_TABLE_HEADERS {
        out.push_str(&format!("<th>{}</th>", escape_html(h)));
    }
    out.push_str("</tr></thead>\n<tbody>\n");

    for (i, step) in steps.iter().enumerate() {
        let value_text = format_number(step.value);
        let with_unit = if step.unit.is_empty() {
            value_text
        } else {
            format!("{value_text} {}", step.unit)
        };
        let substituted = if step.substituted_expression.trim().is_empty() {
            step.expression.clone()
        } else {
            step.substituted_expression.clone()
        };

        out.push_str("<tr>");
        for cell in [
            (i + 1).to_string(),
            step.expression.clone(),
            substituted,
            with_unit,
            step.excel_formula.clone(),
            step.excel_value_formula.clone(),
        ] {
            out.push_str(&format!("<td>{}</td>", escape_html(&cell)));
        }
        out.push_str("</tr>\n");
    }

    out.push_str("</tbody>\n</table>\n");
}

fn render_result_block(
    out: &mut String,
    schema: &FormulaSchema,
    result: &EvalResult,
    title: &str,
) {
    out.push_str(&format!("<h2>{}计算结果</h2>\n", escape_html(title)));

    let outputs = result_outputs::resolve(schema, result);

    if result_outputs::is_multi(&outputs) {
        out.push_str("<table>\n<thead><tr><th>结果</th><th>数值</th></tr></thead>\n<tbody>\n");
        for o in &outputs {
            out.push_str(&format!(
                "<tr><td>{}</td><td>{}</td></tr>\n",
                escape_html(&o.label()),
                escape_html(&o.value_with_unit())
            ));
        }
        out.push_str("</tbody>\n</table>\n");
    } else {
        let unit = schema
            .result_unit
            .as_deref()
            .map(|u| format!(" {u}"))
            .unwrap_or_default();
        out.push_str(&format!(
            "<p class=\"pv-result\">{} = {}{}</p>\n",
            escape_html(&schema.result_name),
            escape_html(&format_number(result.primary)),
            escape_html(&unit)
        ));
    }

    // 分支结果：只列适用的（与 docx 一致）
    let applicable: Vec<&civilcalc_core::schema::EvalBranch> =
        result.branches.iter().filter(|b| b.applicable).collect();

    if !applicable.is_empty() {
        let unit_suffix = schema
            .result_unit
            .as_deref()
            .map(|u| format!(" {u}"))
            .unwrap_or_default();
        out.push_str("<h3>分支结果</h3>\n<table>\n<tbody>\n");
        for b in applicable {
            let value = match b.value {
                Some(v) => format!("{}{unit_suffix}", format_number(v)),
                None => "—".to_string(),
            };
            out.push_str(&format!(
                "<tr><th>{}</th><td>{}</td></tr>\n",
                escape_html(&b.label),
                escape_html(&value)
            ));
        }
        out.push_str("</tbody>\n</table>\n");
    }
}

fn render_explanation(out: &mut String, schema: &FormulaSchema, title: &str) {
    out.push_str(&format!("<h2>{}公式详解</h2>\n", escape_html(title)));

    let Some(e) = schema.explanation.as_ref() else {
        out.push_str("<p class=\"pv-empty\">本公式没有可导出的详解内容（生成时未包含「计算公式详解」）</p>\n");
        return;
    };
    if !schema.has_explanation_content() {
        out.push_str("<p class=\"pv-empty\">本公式没有可导出的详解内容（生成时未包含「计算公式详解」）</p>\n");
        return;
    }

    write_sub_section(out, "理解需求", &e.summary);
    write_sub_section(out, "解决方式", &e.solution);

    if !e.steps.is_empty() {
        out.push_str("<h3>分步依据</h3>\n");
        for (i, step) in e.steps.iter().enumerate() {
            out.push_str(&format!(
                "<p class=\"pv-step-title\">{}、{}</p>\n",
                i + 1,
                escape_html(&step.title)
            ));
            if let Some(expr) = step.expression.as_deref().filter(|x| !x.trim().is_empty()) {
                out.push_str(&format!(
                    "<p class=\"pv-step-expr\">{}</p>\n",
                    escape_html(expr)
                ));
            }
            write_body_lines(out, &step.detail);
        }
    }
}

fn write_sub_section(out: &mut String, heading_text: &str, body: &str) {
    if body.trim().is_empty() {
        return;
    }
    out.push_str(&format!("<h3>{}</h3>\n", escape_html(heading_text)));
    write_body_lines(out, body);
}

/// 逐行写入正文，`**强调**` 转 `<strong>`
///
/// ⚠️ 取行用 [`plain_lines_keeping_emphasis`]（保留 `**`）——
/// 用 `plain_lines` 会把标记删掉，`<strong>` 永远出不来（源项目的同款 bug）。
fn write_body_lines(out: &mut String, body: &str) {
    for line in plain_lines_keeping_emphasis(body) {
        let segments = bold_segments(&line);
        if segments.is_empty() {
            continue;
        }
        let mut html = String::new();
        for (text, bold) in segments {
            if bold {
                html.push_str(&format!("<strong>{}</strong>", escape_html(&text)));
            } else {
                html.push_str(&escape_html(&text));
            }
        }
        out.push_str(&format!("<p>{html}</p>\n"));
    }
}

fn render_notes(
    out: &mut String,
    schema: &FormulaSchema,
    config: &NotesConfig,
    title: &str,
) {
    out.push_str(&format!(
        "<h2>{}备注</h2>\n<div class=\"pv-notes\">\n",
        escape_html(title)
    ));

    if !config.default_disclaimer.trim().is_empty() {
        out.push_str(&format!("<p>{}</p>\n", escape_html(&config.default_disclaimer)));
    }
    if config.show_version {
        out.push_str(&format!(
            "<p>公式版本：{}</p>\n",
            escape_html(&schema.id)
        ));
    }
    if config.show_source_ref {
        if let Some(r) = schema.source.ref_.as_deref().filter(|x| !x.is_empty()) {
            out.push_str(&format!("<p>参考依据：{}</p>\n", escape_html(r)));
        }
    }

    out.push_str("</div>\n");
}

// =============================================================================
// 转义
// =============================================================================

/// HTML 文本转义（`&` / `<` / `>` / `"` / `'`）。
///
/// ⚠️ `&` 必须**第一个**替换，否则后面插入的 `&lt;` 会被二次转义成 `&amp;lt;`。
pub fn escape_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// 结果摘要行（与 docx 的 `result_summary_line` 同一口径，便于日志比对）
pub fn result_summary_line(outputs: &[ResolvedResult]) -> String {
    result_outputs::build_inline_text(outputs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export_options::ExportOptions;
    use crate::template_model::{CoverConfig, CoverField};
    use civilcalc_core::schema::{
        EvalBranch, EvalOutput, ExplanationStep, FormulaExplanation, FormulaVar, ResultOutput,
    };

    fn schema(json: &str) -> FormulaSchema {
        serde_json::from_str(json).expect("schema 应能解析")
    }

    fn base() -> FormulaSchema {
        let mut s = schema(
            r#"{
                "id":"test-explain","resultName":"测试公式","resultSymbol":"A",
                "expression":"A = b*h","source":{"kind":"CUSTOM"}
            }"#,
        );
        s.explanation = Some(FormulaExplanation {
            summary: "求矩形面积，**长**与宽相乘。".to_string(),
            solution: "① 取长；② 取宽；③ 相乘。".to_string(),
            steps: vec![ExplanationStep {
                title: "取长".to_string(),
                expression: Some("b".to_string()),
                detail: "从图纸量取".to_string(),
            }],
        });
        s
    }

    fn inputs(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    fn render(s: &FormulaSchema, opts: &ExportOptions) -> String {
        render_html(
            s,
            &inputs(&[("b", 2.0), ("h", 3.0)]),
            &EvalResult::primary_only(6.0),
            &opts.to_template(),
            &[],
        )
    }

    // ------------------------------------------------------------ 转义

    #[test]
    fn escape_covers_all_five() {
        assert_eq!(
            escape_html(r#"<a href="x">&'</a>"#),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;&lt;/a&gt;"
        );
    }

    /// `&` 不能二次转义
    #[test]
    fn escape_is_not_double_applied() {
        assert_eq!(escape_html("&lt;"), "&amp;lt;");
        assert_eq!(escape_html("a & b"), "a &amp; b");
    }

    /// 🔴 AI 产出的 `<script>` 必须被转义（不可信文本）
    #[test]
    fn ai_content_cannot_inject_script() {
        let mut s = base();
        s.result_name = "<script>alert(1)</script>".to_string();
        s.explanation = Some(FormulaExplanation {
            summary: "<img src=x onerror=alert(1)>".to_string(),
            ..Default::default()
        });
        let html = render(&s, &ExportOptions::default());
        // 断言「没有**可执行的**标签/属性」，而不是「没有那段文本」——
        // 转义后 `onerror=alert(1)` 作为**纯文本**出现是安全的，
        // 危险的是 `<` 没被转义。所以判据是尖括号。
        assert!(!html.contains("<script"), "脚本标签不得原样输出");
        assert!(!html.contains("<img"), "img 标签不得原样输出");
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;"));
    }

    // ------------------------------------------------------------ 结构

    #[test]
    fn produces_full_html_document() {
        let html = render(&base(), &ExportOptions::default());
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<meta charset=\"UTF-8\">"));
        assert!(html.ends_with("</html>\n"));
    }

    /// 预览页必须有免责标注（ADR-018 的缓解措施之一）
    #[test]
    fn has_preview_disclaimer() {
        let html = render(&base(), &ExportOptions::default());
        assert!(html.contains("预览仅供参考，实际以导出的 Word 文件为准"));
    }

    #[test]
    fn all_default_sections_present_in_order() {
        let html = render(&base(), &ExportOptions::default());
        let marks = [
            "一、公式信息",
            "二、参数取值",
            "三、计算结果",
            "四、分步计算",
            "五、公式详解",
            "六、备注",
        ];
        let mut last = 0usize;
        for m in marks {
            let at = html.find(m).unwrap_or_else(|| panic!("缺少章节: {m}"));
            assert!(at > last, "章节顺序错乱: {m}");
            last = at;
        }
    }

    #[test]
    fn unchecked_sections_absent() {
        let opts = ExportOptions {
            show_explanation: false,
            show_calculation_steps: false,
            ..ExportOptions::default()
        };
        let html = render(&base(), &opts);
        assert!(!html.contains("公式详解"));
        assert!(!html.contains("分步计算"));
    }

    /// `Steps` 不渲染也不占序号
    #[test]
    fn steps_section_skipped_without_consuming_number() {
        let mut t = ExportOptions::default().to_template();
        t.sections.insert(0, TemplateSection::steps());
        let html = render_html(
            &base(),
            &inputs(&[("b", 2.0), ("h", 3.0)]),
            &EvalResult::primary_only(6.0),
            &t,
            &[],
        );
        assert!(html.contains("一、公式信息"));
        assert!(html.contains("二、参数取值"));
        assert!(!html.contains("分步样式"));
    }

    // ------------------------------------------------------------ 封面

    #[test]
    fn cover_content() {
        let html = render(&base(), &ExportOptions::default());
        assert!(html.contains(COVER_TITLE));
        assert!(html.contains("测试公式"));
    }

    /// ⚠️ 断言 `<h1>` 而不是裸标题 —— `<title>` 里也有「工程计算书」
    #[test]
    fn cover_hidden_when_off() {
        let opts = ExportOptions {
            show_cover: false,
            ..ExportOptions::default()
        };
        let html = render(&base(), &opts);
        assert!(!html.contains("<h1>"), "封面主标题应消失");
        assert!(html.contains("<title>"), "<title> 不受封面开关影响");
    }

    #[test]
    fn cover_alias_and_custom_fields() {
        let mut t = ExportOptions::default().to_template();
        t.cover = CoverConfig {
            calculator_name_alias: Some("结构计算工具箱".to_string()),
            custom_fields: vec![CoverField {
                key: "project".to_string(),
                label: "工程名称".to_string(),
                required: true,
            }],
            ..CoverConfig::default()
        };
        let html = render_html(
            &base(),
            &inputs(&[("b", 2.0), ("h", 3.0)]),
            &EvalResult::primary_only(6.0),
            &t,
            &[],
        );
        assert!(html.contains("结构计算工具箱"));
        assert!(html.contains("工程名称：________________"));
    }

    // ------------------------------------------------------------ 参数表

    /// 取值优先级：**本次输入 → 变量默认值 → 占位符**
    ///
    /// ⚠️ 只给 `b` 传输入（不给 `h`），否则 `h` 会用输入值而不是默认值 ——
    /// 这条测试之前就踩了这个坑。
    #[test]
    fn params_table_with_inputs_and_placeholder() {
        let mut s = base();
        s.variables = vec![
            var("b", "**宽度**", "mm", Some(100.0)),
            var("h", "高度", "mm", Some(200.0)),
            var("x", "无值", "mm", None),
        ];
        let html = render_html(
            &s,
            &inputs(&[("b", 2.5)]),
            &EvalResult::primary_only(6.0),
            &ExportOptions::default().to_template(),
            &[],
        );

        assert!(html.contains("<th>符号</th>"));
        assert!(html.contains("宽度"));
        assert!(!html.contains("**"), "参数说明不该残留 Markdown");
        assert!(html.contains(">2.5</td>"), "b 用输入值");
        assert!(html.contains(">200</td>"), "h 用默认值（未传输入）");
        assert!(html.contains(">—</td>"), "x 用占位符");
    }

    // ------------------------------------------------------------ 结果

    #[test]
    fn single_result_rendered_as_centered_line() {
        let html = render(&base(), &ExportOptions::default());
        assert!(html.contains("class=\"pv-result\""));
        assert!(html.contains("测试公式 = 6"));
    }

    #[test]
    fn multi_result_rendered_as_table() {
        let mut s = base();
        s.result_outputs = vec![
            ResultOutput {
                symbol: "x".to_string(),
                name: "X 坐标".to_string(),
                unit: "m".to_string(),
            },
            ResultOutput {
                symbol: "y".to_string(),
                name: "Y 坐标".to_string(),
                unit: "m".to_string(),
            },
        ];
        let mut r = EvalResult::primary_only(2.0);
        r.outputs = vec![
            EvalOutput {
                symbol: Some("x".to_string()),
                value: 1.0,
            },
            EvalOutput {
                symbol: Some("y".to_string()),
                value: 2.0,
            },
        ];
        let html = render_html(
            &s,
            &inputs(&[("b", 2.0), ("h", 3.0)]),
            &r,
            &ExportOptions::default().to_template(),
            &[],
        );
        assert!(html.contains("x（X 坐标）"));
        assert!(html.contains("1 m"));
        assert!(!html.contains("class=\"pv-result\""));
    }

    #[test]
    fn only_applicable_branches() {
        let mut r = EvalResult::primary_only(6.0);
        r.branches = vec![
            EvalBranch {
                label: "适用".to_string(),
                value: Some(10.0),
                applicable: true,
                condition: None,
            },
            EvalBranch {
                label: "不适用".to_string(),
                value: Some(20.0),
                applicable: false,
                condition: None,
            },
        ];
        let html = render_html(
            &base(),
            &inputs(&[("b", 2.0), ("h", 3.0)]),
            &r,
            &ExportOptions::default().to_template(),
            &[],
        );
        assert!(html.contains("分支结果"));
        assert!(html.contains("适用"));
        assert!(!html.contains("不适用"));
    }

    // ------------------------------------------------------------ 分步

    #[test]
    fn empty_steps_shows_placeholder() {
        let html = render(&base(), &ExportOptions::default());
        assert!(html.contains("class=\"pv-empty\">无</p>"));
    }

    #[test]
    fn steps_table_rendered() {
        let step = StepResult {
            symbol: "A".to_string(),
            label: "面积".to_string(),
            group: None,
            value: 6.0,
            unit: "m2".to_string(),
            expression: "b*h".to_string(),
            substituted_expression: "面积 = b*h = 6 m2".to_string(),
            excel_formula: "=A1*B1".to_string(),
            excel_value_formula: "=2*3".to_string(),
        };
        let html = render_html(
            &base(),
            &inputs(&[("b", 2.0), ("h", 3.0)]),
            &EvalResult::primary_only(6.0),
            &ExportOptions::default().to_template(),
            &[step],
        );
        for h in STEP_TABLE_HEADERS {
            assert!(html.contains(h), "缺少表头 {h}");
        }
        assert!(html.contains("=A1*B1"));
        assert!(html.contains("=2*3"));
        assert!(html.contains("6 m2"));
    }

    // ------------------------------------------------------------ 详解

    #[test]
    fn explanation_content_and_emphasis() {
        let html = render(&base(), &ExportOptions::default());
        assert!(html.contains("<h3>理解需求</h3>"));
        assert!(html.contains("<h3>解决方式</h3>"));
        assert!(html.contains("<h3>分步依据</h3>"));
        assert!(html.contains("<strong>长</strong>"), "**强调** 应转 <strong>");
        assert!(!html.contains("**"));
        assert!(html.contains("① 取长；"));
        assert!(html.contains("② 取宽；"));
    }

    #[test]
    fn explanation_placeholder_when_missing() {
        let s = schema(
            r#"{
                "id":"x","resultName":"无详解","resultSymbol":"A",
                "expression":"b*h","source":{"kind":"CUSTOM"}
            }"#,
        );
        let html = render(&s, &ExportOptions::default());
        assert!(html.contains("没有可导出的详解内容"));
    }

    // ------------------------------------------------------------ 备注

    #[test]
    fn notes_content() {
        let html = render(&base(), &ExportOptions::default());
        assert!(html.contains(crate::template_model::DEFAULT_DISCLAIMER));
        assert!(html.contains("公式版本：test-explain"));
    }

    #[test]
    fn custom_disclaimer() {
        let opts = ExportOptions {
            disclaimer: "仅供内部复核。".to_string(),
            ..ExportOptions::default()
        };
        let html = render(&base(), &opts);
        assert!(html.contains("仅供内部复核。"));
        assert!(!html.contains(crate::template_model::DEFAULT_DISCLAIMER));
    }

    // ------------------------------------------------------------ 边界

    #[test]
    fn cover_only_document() {
        let opts = ExportOptions {
            show_formula_info: false,
            show_params_table: false,
            show_result_block: false,
            show_calculation_steps: false,
            show_explanation: false,
            show_notes: false,
            ..ExportOptions::default()
        };
        let html = render(&base(), &opts);
        assert!(html.contains(COVER_TITLE));
        assert!(!html.contains("公式信息"));
    }

    #[test]
    fn empty_sections_no_panic() {
        let mut t = ExportOptions::default().to_template();
        t.sections.clear();
        let html = render_html(
            &base(),
            &HashMap::new(),
            &EvalResult::primary_only(1.0),
            &t,
            &[],
        );
        assert!(html.starts_with("<!DOCTYPE html>"));
    }

    #[test]
    fn result_summary_line_matches_docx_convention() {
        let items = vec![ResolvedResult {
            symbol: "A".to_string(),
            name: "面积".to_string(),
            value: Some(6.0),
            unit: "m2".to_string(),
        }];
        assert_eq!(result_summary_line(&items), "A = 6 m2");
    }

    fn var(symbol: &str, desc: &str, unit: &str, default: Option<f64>) -> FormulaVar {
        FormulaVar {
            symbol: symbol.to_string(),
            desc: desc.to_string(),
            unit: Some(unit.to_string()),
            default,
            min: None,
            max: None,
            required: true,
        }
    }
}
