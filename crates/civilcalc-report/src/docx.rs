//! DOCX 计算书生成。
//!
//! 源：`core/report/DocxGenerator.kt`（313 行）
//!
//! ## 技术替换：Apache POI → `docx-rs`（ADR-007）
//!
//! 源项目用 `XWPFDocx`；这里用 `docx_rs::Docx`。验收标准是
//! 「**内容完整 + 结构正确**」，不追求像素级一致。
//!
//! ### ⚠️ 字号单位不同（最容易错的一处）
//!
//! | | 单位 | 28 号字怎么写 |
//! |---|---|---|
//! | POI `run.setFontSize(n)` | **磅**（pt） | `setFontSize(28)` |
//! | OOXML `<w:sz w:val>` / `docx-rs` `Run::size(n)` | **半磅**（half-point） | `size(56)` |
//!
//! 所以本文件里所有字号都写成 `pt(28)` 的形式（[`pt`] 做 ×2），
//! **不要**直接把 POI 的数字搬过来，否则产物字号会小一半。
//!
//! ## 章节序号
//!
//! 标题形如 `一、公式信息` / `二、参数取值`，中文数字到「十」，
//! 之后退回阿拉伯数字（`11、`）。
//!
//! ⚠️ [`TemplateSection::Steps`] **既不渲染、也不占用序号**
//! （源项目该分支是空 `{ }`，连 `nextTitle()` 都没调）。
//!
//! ## 章节完全由模板决定
//!
//! 源注释特别说明：早前这里会把「分步计算」**强制补到末尾**，
//! 导致用户不勾选分步时仍然出现，**已移除**。这里保持「勾了才有」。

use std::collections::HashMap;

use civilcalc_core::schema::{plain_desc, EvalResult, FormulaSchema, SourceKind, StepResult};
use docx_rs::{
    AlignmentType, BreakType, Docx, Paragraph, Run, Table, TableCell, TableRow, WidthType,
};

use civilcalc_core::number_format::format_number;
use crate::report_text::{bold_segments, plain_lines_keeping_emphasis};
use civilcalc_core::result_outputs::{self, ResolvedResult};
use crate::template_model::{ReportTemplate, TemplateSection};
use crate::ReportError;

/// 磅 → 半磅。POI 与 OOXML 的字号单位不同，见模块文档。
const fn pt(points: usize) -> usize {
    points * 2
}

// 字号常量（取值与源项目 POI 调用一一对应）
const SIZE_COVER_TITLE: usize = pt(28);
const SIZE_COVER_NAME: usize = pt(20);
const SIZE_COVER_FIELD: usize = pt(14);
const SIZE_HEADING: usize = pt(14);
const SIZE_SUBHEADING: usize = pt(12);
const SIZE_STEP_HEAD: usize = pt(11);
const SIZE_BRANCH_LABEL: usize = pt(11);
const SIZE_RESULT: usize = pt(16);
const SIZE_BODY: usize = pt(10);
const SIZE_CELL: usize = pt(10);
const SIZE_NOTE: usize = pt(9);

/// 表格宽度：`Pct` 类型下 5000 = 100%（单位是 1/50 个百分点）
const TABLE_WIDTH_PCT: usize = 5000;

/// 封面固定主标题
const COVER_TITLE: &str = "工程计算书";

/// 中文数字（到「十」，之后用阿拉伯数字）
const CN_NUMS: [&str; 10] = ["一", "二", "三", "四", "五", "六", "七", "八", "九", "十"];

/// 分步计算表的表头（6 列，顺序固定）
const STEP_TABLE_HEADERS: [&str; 6] = [
    "序号",
    "表达式",
    "代入数值",
    "结果",
    "Excel 公式",
    "Excel 代入数值",
];

/// 计算书生成器。
///
/// 无状态 —— 所有输入都走参数，便于并发调用（导出多个公式时不必串行）。
#[derive(Debug, Clone, Copy, Default)]
pub struct DocxGenerator;

impl DocxGenerator {
    pub fn new() -> Self {
        Self
    }

    /// 生成 `.docx` 字节。
    ///
    /// # 参数
    ///
    /// - `step_results`：分步求值结果。空列表时「分步计算」章节输出「无」
    ///   （不是省略章节 —— 章节有没有由 `template.sections` 决定）。
    pub fn generate(
        &self,
        schema: &FormulaSchema,
        inputs: &HashMap<String, f64>,
        result: &EvalResult,
        template: &ReportTemplate,
        step_results: &[StepResult],
    ) -> Result<Vec<u8>, ReportError> {
        let doc = build_document(schema, inputs, result, template, step_results);
        pack(doc)
    }
}

/// 把 `Docx` 打成 `.docx` 字节（ZIP）。
fn pack(doc: Docx) -> Result<Vec<u8>, ReportError> {
    let mut buf = std::io::Cursor::new(Vec::new());
    doc.pack(&mut buf)
        .map_err(|e| ReportError::Docx(format!("打包 docx 失败: {e}")))?;
    Ok(buf.into_inner())
}

// =============================================================================
// 文档组装
// =============================================================================

fn build_document(
    schema: &FormulaSchema,
    inputs: &HashMap<String, f64>,
    result: &EvalResult,
    template: &ReportTemplate,
    step_results: &[StepResult],
) -> Docx {
    let mut doc = render_cover(Docx::new(), schema, template);

    let mut section_idx = 0usize;
    for section in &template.sections {
        // ⚠️ `Steps` 不调 `next_title` —— 它不占序号（对齐源项目的空分支）
        doc = match section {
            TemplateSection::FormulaInfo {
                show_source,
                show_verification,
                ..
            } => render_formula_info(doc, schema, *show_source, *show_verification, &next_title(&mut section_idx)),

            TemplateSection::ParamsTable { columns } => {
                render_params_table(doc, schema, inputs, columns, &next_title(&mut section_idx))
            }

            TemplateSection::Steps { .. } => doc,

            TemplateSection::StepResults { .. } => {
                render_step_results(doc, step_results, &next_title(&mut section_idx))
            }

            TemplateSection::ResultBlock { .. } => {
                render_result_block(doc, schema, result, &next_title(&mut section_idx))
            }

            TemplateSection::Explanation => {
                render_explanation(doc, schema, &next_title(&mut section_idx))
            }

            TemplateSection::Notes { config } => {
                render_notes(doc, schema, config, &next_title(&mut section_idx))
            }
        };
    }

    doc
}

/// 下一个章节序号：`一、` … `十、`，第 11 个起是 `11、`。
fn next_title(idx: &mut usize) -> String {
    let i = *idx;
    *idx += 1;
    match CN_NUMS.get(i) {
        Some(cn) => format!("{cn}、"),
        None => format!("{}、", i + 1),
    }
}

// =============================================================================
// 元素构造助手
// =============================================================================

fn run(text: &str, size: usize, bold: bool) -> Run {
    let r = Run::new().add_text(text).size(size);
    if bold {
        r.bold()
    } else {
        r
    }
}

fn line(text: &str, size: usize, bold: bool) -> Paragraph {
    Paragraph::new().add_run(run(text, size, bold))
}

fn centered(text: &str, size: usize, bold: bool) -> Paragraph {
    line(text, size, bold).align(AlignmentType::Center)
}

/// 章节标题（一级）：加粗 + [`SIZE_HEADING`]
fn heading(text: &str) -> Paragraph {
    line(text, SIZE_HEADING, true)
}

/// 空行
fn blank() -> Paragraph {
    Paragraph::new()
}

fn cell(text: &str, bold: bool) -> TableCell {
    TableCell::new().add_paragraph(line(text, SIZE_CELL, bold))
}

fn table_row(cells: &[(String, bool)]) -> TableRow {
    TableRow::new(cells.iter().map(|(t, b)| cell(t, *b)).collect())
}

/// 等宽两列表（用于「结果 / 数值」这类 header + 数据行）
fn table_from(rows: Vec<Vec<(String, bool)>>) -> Table {
    Table::new(rows.into_iter().map(|r| table_row(&r)).collect())
        .width(TABLE_WIDTH_PCT, WidthType::Pct)
}

// =============================================================================
// 封面
// =============================================================================

fn render_cover(mut doc: Docx, schema: &FormulaSchema, template: &ReportTemplate) -> Docx {
    let cover = &template.cover;

    // 顶部留白（源项目 `repeat(6) { doc.createParagraph() }`）
    for _ in 0..6 {
        doc = doc.add_paragraph(blank());
    }

    if cover.show_project_name {
        doc = doc.add_paragraph(centered(COVER_TITLE, SIZE_COVER_TITLE, true));
    }

    doc = doc.add_paragraph(blank());

    if cover.show_calculator_name {
        // 别名优先；没配就用公式名（源项目 `cover.calculatorNameAlias ?: schema.resultName`）
        let name = cover
            .calculator_name_alias
            .as_deref()
            .unwrap_or(schema.result_name.as_str());
        doc = doc.add_paragraph(centered(name, SIZE_COVER_NAME, true));
    }

    for field in &cover.custom_fields {
        doc = doc.add_paragraph(centered(
            &format!("{}：________________", field.label),
            SIZE_COVER_FIELD,
            false,
        ));
    }

    // 封面收尾：一个带换行的空段落（源项目 `createParagraph().createRun().addBreak()`）
    doc.add_paragraph(Paragraph::new().add_run(Run::new().add_break(BreakType::TextWrapping)))
}

// =============================================================================
// 各章节
// =============================================================================

/// `公式信息`：两列信息表。
///
/// ⚠️ `TemplateSection::FormulaInfo.show_version` **未使用** ——
/// 源项目该分支只检查 `showSource` 与 `showVerification`，这里保持一致
/// （版本号在「备注」章节输出）。
fn render_formula_info(
    mut doc: Docx,
    schema: &FormulaSchema,
    show_source: bool,
    show_verification: bool,
    title: &str,
) -> Docx {
    doc = doc.add_paragraph(heading(&format!("{title}公式信息")));

    let mut rows: Vec<Vec<(String, bool)>> = vec![
        vec![
            ("公式名称".to_string(), true),
            (schema.result_name.clone(), false),
        ],
        vec![
            ("数学表达式".to_string(), true),
            (schema.expression.clone(), false),
        ],
    ];

    if show_source {
        let kind = source_kind_label(schema.source.kind);
        let text = match schema.source.ref_.as_deref() {
            Some(r) if !r.is_empty() => format!("{kind}（{r}）"),
            _ => kind.to_string(),
        };
        rows.push(vec![("来源".to_string(), true), (text, false)]);
    }

    if show_verification {
        let basis = schema.reference_basis.clone().unwrap_or_else(|| {
            if schema.source.verified {
                "已验证".to_string()
            } else {
                "未验证".to_string()
            }
        });
        rows.push(vec![("公式依据".to_string(), true), (basis, false)]);
    }

    doc = doc.add_table(table_from(rows));
    doc.add_paragraph(blank())
}

fn source_kind_label(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Standard => "国家标准规范",
        SourceKind::Ai => "AI 生成",
        SourceKind::Custom => "自定义",
        SourceKind::Derived => "推导公式",
    }
}

/// `参数取值`：按 `columns` 决定的列表格。
///
/// 取值优先级：**本次输入 → 变量默认值 → `—`**。
fn render_params_table(
    mut doc: Docx,
    schema: &FormulaSchema,
    inputs: &HashMap<String, f64>,
    columns: &[String],
    title: &str,
) -> Docx {
    doc = doc.add_paragraph(heading(&format!("{title}参数取值")));

    let header: Vec<(String, bool)> = columns
        .iter()
        .map(|c| (param_column_label(c).to_string(), true))
        .collect();
    let mut rows = vec![header];

    for v in &schema.variables {
        let row: Vec<(String, bool)> = columns
            .iter()
            .map(|c| {
                let text = match c.as_str() {
                    "symbol" => v.symbol.clone(),
                    "desc" => plain_desc(v),
                    "value" => inputs
                        .get(&v.symbol)
                        .map(|x| format_number(*x))
                        .or_else(|| v.default.map(format_number))
                        .unwrap_or_else(|| "—".to_string()),
                    "unit" => v.unit.clone().unwrap_or_default(),
                    // 未知列名原样当表头用，数据留空（不 panic）
                    _ => String::new(),
                };
                (text, false)
            })
            .collect();
        rows.push(row);
    }

    doc = doc.add_table(table_from(rows));
    doc.add_paragraph(blank())
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

/// `分步计算`：6 列表。空列表输出「无」（章节仍在）。
fn render_step_results(
    mut doc: Docx,
    steps: &[StepResult],
    title: &str,
) -> Docx {
    doc = doc.add_paragraph(heading(&format!("{title}分步计算")));

    if steps.is_empty() {
        doc = doc.add_paragraph(line("无", SIZE_BODY, false));
        return doc.add_paragraph(blank());
    }

    let mut rows = vec![STEP_TABLE_HEADERS
        .iter()
        .map(|h| ((*h).to_string(), true))
        .collect::<Vec<_>>()];

    for (i, step) in steps.iter().enumerate() {
        let value_text = format_number(step.value);
        let with_unit = if step.unit.is_empty() {
            value_text
        } else {
            format!("{value_text} {}", step.unit)
        };
        // 代入数值为空时回落到原表达式（源项目 `ifBlank { step.expression }`）
        let substituted = if step.substituted_expression.trim().is_empty() {
            step.expression.clone()
        } else {
            step.substituted_expression.clone()
        };

        rows.push(vec![
            ((i + 1).to_string(), false),
            (step.expression.clone(), false),
            (substituted, false),
            (with_unit, false),
            (step.excel_formula.clone(), false),
            (step.excel_value_formula.clone(), false),
        ]);
    }

    doc = doc.add_table(table_from(rows));
    doc.add_paragraph(blank())
}

/// `计算结果`：单结果居中大字；多结果成表；分支只列**适用**的。
fn render_result_block(
    mut doc: Docx,
    schema: &FormulaSchema,
    result: &EvalResult,
    title: &str,
) -> Docx {
    doc = doc.add_paragraph(heading(&format!("{title}计算结果")));

    let outputs = result_outputs::resolve(schema, result);

    if result_outputs::is_multi(&outputs) {
        let mut rows = vec![vec![
            ("结果".to_string(), true),
            ("数值".to_string(), true),
        ]];
        for o in &outputs {
            rows.push(vec![(o.label(), true), (o.value_with_unit(), false)]);
        }
        doc = doc.add_table(table_from(rows));
    } else {
        // 单结果：`名称 = 数值 单位` 居中加粗
        let unit = schema
            .result_unit
            .as_deref()
            .map(|u| format!(" {u}"))
            .unwrap_or_default();
        doc = doc.add_paragraph(centered(
            &format!(
                "{} = {}{unit}",
                schema.result_name,
                format_number(result.primary)
            ),
            SIZE_RESULT,
            true,
        ));
    }

    // 分支结果：**只列适用的**（不适用的显示出来只会让用户困惑）
    let applicable: Vec<&civilcalc_core::schema::EvalBranch> =
        result.branches.iter().filter(|b| b.applicable).collect();

    if !applicable.is_empty() {
        doc = doc.add_paragraph(blank());
        doc = doc.add_paragraph(line("分支结果：", SIZE_BRANCH_LABEL, true));

        let unit_suffix = schema
            .result_unit
            .as_deref()
            .map(|u| format!(" {u}"))
            .unwrap_or_default();
        let rows = applicable
            .iter()
            .map(|b| {
                let value = match b.value {
                    Some(v) => format!("{}{unit_suffix}", format_number(v)),
                    None => "—".to_string(),
                };
                vec![(b.label.clone(), true), (value, false)]
            })
            .collect();
        doc = doc.add_table(table_from(rows));
    }

    doc.add_paragraph(blank())
}

/// `公式详解`：理解需求 / 解决方式 / 分步依据。
///
/// 无内容时输出一行斜体说明，**不省略章节**（让读者知道这里本该有内容）。
fn render_explanation(mut doc: Docx, schema: &FormulaSchema, title: &str) -> Docx {
    doc = doc.add_paragraph(heading(&format!("{title}公式详解")));

    let Some(e) = schema.explanation.as_ref() else {
        return explanation_placeholder(doc);
    };
    if !schema.has_explanation_content() {
        return explanation_placeholder(doc);
    }

    doc = write_sub_section(doc, "理解需求", &e.summary);
    doc = write_sub_section(doc, "解决方式", &e.solution);

    if !e.steps.is_empty() {
        doc = doc.add_paragraph(line("分步依据", SIZE_SUBHEADING, true));
        for (i, step) in e.steps.iter().enumerate() {
            doc = doc.add_paragraph(line(
                &format!("{}. {}", i + 1, step.title),
                SIZE_STEP_HEAD,
                true,
            ));
            if let Some(expr) = step.expression.as_deref().filter(|x| !x.trim().is_empty()) {
                doc = doc.add_paragraph(line(expr, SIZE_BODY, false));
            }
            doc = write_body_lines(doc, &step.detail);
        }
    }

    doc.add_paragraph(blank())
}

fn explanation_placeholder(doc: Docx) -> Docx {
    let p = Paragraph::new().add_run(
        Run::new()
            .add_text("本公式没有可导出的详解内容（生成时未包含「计算公式详解」）")
            .size(SIZE_BODY)
            .italic(),
    );
    doc.add_paragraph(p).add_paragraph(blank())
}

/// 小标题 + 正文（正文为空则整块跳过）
fn write_sub_section(mut doc: Docx, heading_text: &str, body: &str) -> Docx {
    if body.trim().is_empty() {
        return doc;
    }
    doc = doc.add_paragraph(line(heading_text, SIZE_SUBHEADING, true));
    write_body_lines(doc, body)
}

/// 把一段详解正文按行写入：`**强调**` 片段按加粗 run 输出。
///
/// ⚠️ 取行必须用 [`plain_lines_keeping_emphasis`]（**保留 `**`**）——
/// 源项目用 `plainLines`（已删标记）再 `boldSegments`，那段是死代码，
/// 详解正文从来没有加粗过。契约 §5.4 要求按段加粗，故这里修正。
fn write_body_lines(mut doc: Docx, body: &str) -> Docx {
    for text_line in plain_lines_keeping_emphasis(body) {
        let segments = bold_segments(&text_line);
        // 过滤后为空的情况：整行只有标记（如 `**`）—— 跳过不建段落
        if segments.is_empty() {
            continue;
        }
        let mut p = Paragraph::new();
        for (text, bold) in segments {
            p = p.add_run(run(&text, SIZE_BODY, bold));
        }
        doc = doc.add_paragraph(p);
    }
    doc
}

/// `备注`：免责声明 / 公式版本 / 参考依据。
///
/// ⚠️ 本分支**不追加尾部空段落**（源项目如此）—— 它是最后一个章节。
fn render_notes(
    mut doc: Docx,
    schema: &FormulaSchema,
    config: &crate::template_model::NotesConfig,
    title: &str,
) -> Docx {
    doc = doc.add_paragraph(heading(&format!("{title}备注")));

    if !config.default_disclaimer.trim().is_empty() {
        let p = Paragraph::new().add_run(
            Run::new()
                .add_text(config.default_disclaimer.clone())
                .size(SIZE_NOTE)
                .italic(),
        );
        doc = doc.add_paragraph(p);
    }

    if config.show_version {
        doc = doc.add_paragraph(line(&format!("公式版本：{}", schema.id), SIZE_NOTE, false));
    }

    if config.show_source_ref {
        if let Some(r) = schema.source.ref_.as_deref().filter(|x| !x.is_empty()) {
            doc = doc.add_paragraph(line(&format!("参考依据：{r}"), SIZE_NOTE, false));
        }
    }

    doc
}

// =============================================================================
// 供测试与外部复用的入口
// =============================================================================

/// 生成后的纯文本骨架（排障 / 日志用）。
///
/// ⚠️ 这不是「另一种渲染」—— 只是把 `ResolvedResult` 拼成一行，便于日志里
/// 一眼看出结果对不对（不需要解包 docx）。
pub fn result_summary_line(outputs: &[ResolvedResult]) -> String {
    result_outputs::build_inline_text(outputs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export_options::ExportOptions;
    use crate::template_model::{CoverConfig, CoverField, NotesConfig};
    use civilcalc_core::schema::{
        EvalBranch, EvalOutput, ExplanationStep, FormulaExplanation, ResultOutput, StepTemplate,
    };
    use std::io::Read;

    // ------------------------------------------------------------ 测试助手

    fn schema(json: &str) -> FormulaSchema {
        serde_json::from_str(json).expect("schema 应能解析")
    }

    /// 源项目 `ExportOptionsTest.schemaWithExplanation()`
    fn schema_with_explanation() -> FormulaSchema {
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

    /// 解包 `.docx`（ZIP）取出 `word/document.xml` 文本
    fn document_xml(bytes: &[u8]) -> String {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .expect("产物应是合法 ZIP");
        let mut f = zip
            .by_name("word/document.xml")
            .expect("docx 里应有 word/document.xml");
        let mut s = String::new();
        f.read_to_string(&mut s).expect("document.xml 应是 UTF-8");
        s
    }

    fn generate(s: &FormulaSchema, opts: &ExportOptions) -> String {
        let bytes = DocxGenerator::new()
            .generate(
                s,
                &inputs(&[("b", 2.0), ("h", 3.0)]),
                &EvalResult::primary_only(6.0),
                &opts.to_template(),
                &[],
            )
            .expect("生成应成功");
        assert!(bytes.starts_with(b"PK"), "产物应是 ZIP");
        document_xml(&bytes)
    }

    // ------------------------------------------------------------ 源项目用例

    /// 源 `docxContainsExplanationTextAndOmitsUncheckedSections`（前半）
    #[test]
    fn docx_contains_explanation_text() {
        let xml = generate(&schema_with_explanation(), &ExportOptions::default());
        assert!(xml.contains("公式详解"), "详解章节缺失");
        assert!(xml.contains("求矩形面积"), "理解需求内容缺失");
        assert!(xml.contains("取长"), "分步依据缺失");
        assert!(xml.contains("复核"), "免责声明缺失");
    }

    /// 源 `docxContainsExplanationTextAndOmitsUncheckedSections`（后半）
    #[test]
    fn docx_omits_unchecked_sections() {
        let opts = ExportOptions {
            show_explanation: false,
            show_calculation_steps: false,
            ..ExportOptions::default()
        };
        let xml = generate(&schema_with_explanation(), &opts);
        assert!(!xml.contains("公式详解"), "未勾选详解仍被写入");
        assert!(!xml.contains("分步计算"), "未勾选分步仍被写入");
    }

    // ------------------------------------------------------------ 结构

    #[test]
    fn all_default_sections_present() {
        let xml = generate(&schema_with_explanation(), &ExportOptions::default());
        for t in ["公式信息", "参数取值", "计算结果", "分步计算", "公式详解", "备注"] {
            assert!(xml.contains(t), "缺少章节: {t}");
        }
    }

    /// 章节序号用中文数字，且**连续**（Steps 不占号）
    #[test]
    fn chinese_section_numbering_is_continuous() {
        let xml = generate(&schema_with_explanation(), &ExportOptions::default());
        assert!(xml.contains("一、公式信息"));
        assert!(xml.contains("二、参数取值"));
        assert!(xml.contains("三、计算结果"));
        assert!(xml.contains("四、分步计算"));
        assert!(xml.contains("五、公式详解"));
        assert!(xml.contains("六、备注"));
    }

    /// `Steps` 章节既不出标题也不占序号
    #[test]
    fn steps_section_is_skipped_entirely() {
        let mut t = ExportOptions::default().to_template();
        t.sections.insert(0, TemplateSection::steps());
        let bytes = DocxGenerator::new()
            .generate(
                &schema_with_explanation(),
                &inputs(&[("b", 2.0), ("h", 3.0)]),
                &EvalResult::primary_only(6.0),
                &t,
                &[],
            )
            .unwrap();
        let xml = document_xml(&bytes);
        assert!(xml.contains("一、公式信息"), "Steps 不该占掉序号");
        assert!(xml.contains("二、参数取值"));
        assert!(!xml.contains("分步样式"), "Steps 不该出标题");
    }

    /// 序号到「十」之后退回阿拉伯数字
    #[test]
    fn numbering_falls_back_after_ten() {
        let mut idx = 0;
        let titles: Vec<String> = (0..12).map(|_| next_title(&mut idx)).collect();
        assert_eq!(titles[0], "一、");
        assert_eq!(titles[9], "十、");
        assert_eq!(titles[10], "11、");
        assert_eq!(titles[11], "12、");
    }

    // ------------------------------------------------------------ 封面

    #[test]
    fn cover_contains_title_and_calculator_name() {
        let xml = generate(&schema_with_explanation(), &ExportOptions::default());
        assert!(xml.contains(COVER_TITLE), "封面主标题缺失");
        assert!(xml.contains("测试公式"), "计算器名（回落公式名）缺失");
    }

    #[test]
    fn cover_hidden_when_show_cover_off() {
        let opts = ExportOptions {
            show_cover: false,
            ..ExportOptions::default()
        };
        let xml = generate(&schema_with_explanation(), &opts);
        assert!(!xml.contains(COVER_TITLE), "封面应被关闭");
    }

    #[test]
    fn cover_alias_and_custom_fields() {
        let mut t = ExportOptions::default().to_template();
        t.cover = CoverConfig {
            show_project_name: true,
            show_calculator_name: true,
            calculator_name_alias: Some("结构计算工具箱".to_string()),
            custom_fields: vec![
                CoverField {
                    key: "project".to_string(),
                    label: "工程名称".to_string(),
                    required: true,
                },
                CoverField {
                    key: "author".to_string(),
                    label: "计算人".to_string(),
                    required: false,
                },
            ],
            ..CoverConfig::default()
        };

        let bytes = DocxGenerator::new()
            .generate(
                &schema_with_explanation(),
                &inputs(&[("b", 2.0), ("h", 3.0)]),
                &EvalResult::primary_only(6.0),
                &t,
                &[],
            )
            .unwrap();
        let xml = document_xml(&bytes);

        assert!(xml.contains("结构计算工具箱"), "别名应取代公式名");
        assert!(xml.contains("工程名称：________________"));
        assert!(xml.contains("计算人：________________"));
    }

    // ------------------------------------------------------------ 公式信息

    #[test]
    fn formula_info_rows() {
        let xml = generate(&schema_with_explanation(), &ExportOptions::default());
        assert!(xml.contains("公式名称"));
        assert!(xml.contains("数学表达式"));
        assert!(xml.contains("A = b*h"));
        assert!(xml.contains("来源"));
        assert!(xml.contains("自定义"), "SourceKind::Custom → 自定义");
        assert!(xml.contains("公式依据"));
        assert!(xml.contains("未验证"), "verified=false → 未验证");
    }

    /// `show_source = false` 时「来源」行消失
    #[test]
    fn formula_info_respects_show_source() {
        let mut t = ExportOptions::default().to_template();
        for s in &mut t.sections {
            if let TemplateSection::FormulaInfo {
                show_source,
                show_verification,
                ..
            } = s
            {
                *show_source = false;
                *show_verification = false;
            }
        }
        let bytes = DocxGenerator::new()
            .generate(
                &schema_with_explanation(),
                &inputs(&[("b", 2.0), ("h", 3.0)]),
                &EvalResult::primary_only(6.0),
                &t,
                &[],
            )
            .unwrap();
        let xml = document_xml(&bytes);
        assert!(!xml.contains("公式依据"), "showVerification=false 应隐藏该行");
        assert!(xml.contains("公式名称"), "其余行仍在");
    }

    /// `reference_basis` 优先于「已验证 / 未验证」
    #[test]
    fn reference_basis_wins_over_verified_flag() {
        let mut s = schema_with_explanation();
        s.reference_basis = Some("GB 50010-2010 第 6.2 节".to_string());
        let xml = generate(&s, &ExportOptions::default());
        assert!(xml.contains("GB 50010-2010 第 6.2 节"));
        assert!(!xml.contains("未验证"));
    }

    /// 标准来源 + `ref` → 「国家标准规范（GB …）」
    #[test]
    fn standard_source_with_ref() {
        let mut s = schema_with_explanation();
        s.source = civilcalc_core::schema::FormulaSource {
            kind: SourceKind::Standard,
            ref_: Some("GB 50010-2010".to_string()),
            verified: true,
            model: None,
            created_by: None,
        };
        let xml = generate(&s, &ExportOptions::default());
        assert!(xml.contains("国家标准规范（GB 50010-2010）"));
    }

    // ------------------------------------------------------------ 参数表

    #[test]
    fn params_table_uses_inputs_defaults_and_placeholder() {
        let mut s = schema_with_explanation();
        s.variables = vec![
            var("b", "宽度", "mm", Some(100.0)),
            var("h", "高度", "mm", Some(200.0)),
            var("x", "无值", "mm", None),
        ];
        // 只给 b 输入 → b 用输入、h 用默认、x 用占位符
        let bytes = DocxGenerator::new()
            .generate(
                &s,
                &inputs(&[("b", 2.5)]),
                &EvalResult::primary_only(6.0),
                &ExportOptions::default().to_template(),
                &[],
            )
            .unwrap();
        let xml = document_xml(&bytes);

        assert!(xml.contains("符号"), "表头应有「符号」");
        assert!(xml.contains("宽度"));
        assert!(xml.contains("2.5"), "b 应用输入值");
        assert!(xml.contains("200"), "h 应用默认值");
        assert!(xml.contains("—"), "x 无值应显示占位符");
    }

    /// 参数说明里的 Markdown 标记要被去掉
    #[test]
    fn params_desc_has_no_markdown() {
        let mut s = schema_with_explanation();
        s.variables = vec![var("b", "**梁宽**", "mm", None)];
        let xml = generate(&s, &ExportOptions::default());
        assert!(xml.contains("梁宽"));
        assert!(!xml.contains("**"), "参数说明不该出现 Markdown 标记");
    }

    /// 自定义列（未知列名）不 panic，且按原名列头
    #[test]
    fn custom_param_column_is_tolerated() {
        let mut t = ExportOptions::default().to_template();
        for s in &mut t.sections {
            if let TemplateSection::ParamsTable { columns } = s {
                *columns = vec!["symbol".to_string(), "note".to_string()];
            }
        }
        let mut sch = schema_with_explanation();
        sch.variables = vec![var("b", "宽度", "mm", None)];

        let bytes = DocxGenerator::new()
            .generate(
                &sch,
                &HashMap::new(),
                &EvalResult::primary_only(6.0),
                &t,
                &[],
            )
            .unwrap();
        let xml = document_xml(&bytes);
        assert!(xml.contains("note"), "未知列名原样作表头");
        assert!(xml.contains("b"));
    }

    // ------------------------------------------------------------ 分步计算

    #[test]
    fn step_results_empty_shows_placeholder() {
        let xml = generate(&schema_with_explanation(), &ExportOptions::default());
        assert!(xml.contains("分步计算"));
        assert!(xml.contains("无"), "空分步应显示「无」而不是省略章节");
    }

    #[test]
    fn step_results_table_content() {
        let step = StepResult {
            symbol: "A".to_string(),
            label: "面积".to_string(),
            group: None,
            value: 6.0,
            unit: "m2".to_string(),
            expression: "b*h".to_string(),
            substituted_expression: "2*3".to_string(),
            excel_formula: "=A1*B1".to_string(),
            excel_value_formula: "=2*3".to_string(),
        };
        let bytes = DocxGenerator::new()
            .generate(
                &schema_with_explanation(),
                &inputs(&[("b", 2.0), ("h", 3.0)]),
                &EvalResult::primary_only(6.0),
                &ExportOptions::default().to_template(),
                &[step],
            )
            .unwrap();
        let xml = document_xml(&bytes);

        for h in STEP_TABLE_HEADERS {
            assert!(xml.contains(h), "缺少表头: {h}");
        }
        assert!(xml.contains("b*h"));
        assert!(xml.contains("2*3"));
        assert!(xml.contains("=A1*B1"));
        assert!(xml.contains("6 m2"), "结果应带单位");
    }

    /// `substitutedExpression` 为空时回落到原表达式
    #[test]
    fn step_substituted_falls_back_to_expression() {
        let step = StepResult {
            symbol: "A".to_string(),
            label: "面积".to_string(),
            group: None,
            value: 6.0,
            unit: String::new(),
            expression: "b*h".to_string(),
            substituted_expression: String::new(),
            excel_formula: String::new(),
            excel_value_formula: String::new(),
        };
        let bytes = DocxGenerator::new()
            .generate(
                &schema_with_explanation(),
                &inputs(&[("b", 2.0), ("h", 3.0)]),
                &EvalResult::primary_only(6.0),
                &ExportOptions::default().to_template(),
                &[step],
            )
            .unwrap();
        let xml = document_xml(&bytes);
        // 表达式列与代入列都是 `b*h`
        assert!(xml.matches("b*h").count() >= 2, "代入列应回落到原表达式");
        // 单位空 → 不带空格后缀
        assert!(xml.contains(">6<"));
    }

    // ------------------------------------------------------------ 计算结果

    #[test]
    fn single_result_is_centered_bold_line() {
        let xml = generate(&schema_with_explanation(), &ExportOptions::default());
        assert!(xml.contains("测试公式 = 6"));
        assert!(xml.contains("center"), "应居中");
    }

    /// 单结果带单位
    #[test]
    fn single_result_with_unit() {
        let mut s = schema_with_explanation();
        s.result_unit = Some("m2".to_string());
        let xml = generate(&s, &ExportOptions::default());
        assert!(xml.contains("测试公式 = 6 m2"));
    }

    #[test]
    fn multi_result_renders_table() {
        let mut s = schema_with_explanation();
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

        let bytes = DocxGenerator::new()
            .generate(
                &s,
                &inputs(&[("b", 2.0), ("h", 3.0)]),
                &r,
                &ExportOptions::default().to_template(),
                &[],
            )
            .unwrap();
        let xml = document_xml(&bytes);

        assert!(xml.contains("x（X 坐标）"));
        assert!(xml.contains("y（Y 坐标）"));
        assert!(xml.contains("1 m"));
        assert!(xml.contains("2 m"));
        assert!(!xml.contains("测试公式 = 2"), "多结果不该走单值行");
    }

    /// 分支结果只列适用的
    #[test]
    fn only_applicable_branches_are_rendered() {
        let mut r = EvalResult::primary_only(6.0);
        r.branches = vec![
            EvalBranch {
                label: "适用分支".to_string(),
                value: Some(10.0),
                applicable: true,
                condition: Some("a>0".to_string()),
            },
            EvalBranch {
                label: "不适用分支".to_string(),
                value: Some(20.0),
                applicable: false,
                condition: Some("a<0".to_string()),
            },
        ];
        let bytes = DocxGenerator::new()
            .generate(
                &schema_with_explanation(),
                &inputs(&[("b", 2.0), ("h", 3.0)]),
                &r,
                &ExportOptions::default().to_template(),
                &[],
            )
            .unwrap();
        let xml = document_xml(&bytes);

        assert!(xml.contains("分支结果："));
        assert!(xml.contains("适用分支"));
        assert!(!xml.contains("不适用分支"), "不适用的分支不该出现");
    }

    /// 分支值为 `None` → 占位符（不写 NaN）
    #[test]
    fn branch_without_value_uses_placeholder() {
        let mut r = EvalResult::primary_only(6.0);
        r.branches = vec![EvalBranch {
            label: "分支".to_string(),
            value: None,
            applicable: true,
            condition: None,
        }];
        let bytes = DocxGenerator::new()
            .generate(
                &schema_with_explanation(),
                &inputs(&[("b", 2.0), ("h", 3.0)]),
                &r,
                &ExportOptions::default().to_template(),
                &[],
            )
            .unwrap();
        let xml = document_xml(&bytes);
        assert!(xml.contains("—"));
        assert!(!xml.contains("NaN"));
    }

    /// 没有适用分支时不出「分支结果：」
    #[test]
    fn no_applicable_branch_no_section() {
        let mut r = EvalResult::primary_only(6.0);
        r.branches = vec![EvalBranch {
            label: "不适用".to_string(),
            value: Some(1.0),
            applicable: false,
            condition: None,
        }];
        let bytes = DocxGenerator::new()
            .generate(
                &schema_with_explanation(),
                &inputs(&[("b", 2.0), ("h", 3.0)]),
                &r,
                &ExportOptions::default().to_template(),
                &[],
            )
            .unwrap();
        assert!(!document_xml(&bytes).contains("分支结果："));
    }

    // ------------------------------------------------------------ 公式详解

    /// 无详解内容 → 斜体说明（章节仍在）
    #[test]
    fn explanation_placeholder_when_empty() {
        let s = schema(
            r#"{
                "id":"x","resultName":"无详解","resultSymbol":"A",
                "expression":"b*h","source":{"kind":"CUSTOM"}
            }"#,
        );
        let xml = generate(&s, &ExportOptions::default());
        assert!(xml.contains("公式详解"), "章节应仍在");
        assert!(xml.contains("没有可导出的详解内容"));
    }

    /// 详解的加粗标记转成多个 run（`**` 不出现在正文里）
    #[test]
    fn explanation_emphasis_becomes_runs() {
        let xml = generate(&schema_with_explanation(), &ExportOptions::default());
        assert!(xml.contains("理解需求"));
        assert!(xml.contains("解决方式"));
        assert!(xml.contains("分步依据"));
        assert!(xml.contains("取长"));
        assert!(xml.contains("从图纸量取"));
        assert!(!xml.contains("**"), "正文里不该残留 Markdown 标记");
    }

    /// 圆圈数字被拆成多行（每行一个段落）
    #[test]
    fn explanation_circled_points_split_into_paragraphs() {
        let xml = generate(&schema_with_explanation(), &ExportOptions::default());
        // solution = "① 取长；② 取宽；③ 相乘。" → 三行
        for t in ["① 取长；", "② 取宽；", "③ 相乘。"] {
            assert!(xml.contains(t), "缺少分点: {t}");
        }
    }

    /// 空的分量（summary 空）不产生小标题
    #[test]
    fn blank_explanation_component_skipped() {
        let mut s = schema_with_explanation();
        s.explanation = Some(FormulaExplanation {
            summary: String::new(),
            solution: "只有解决方式".to_string(),
            steps: vec![],
        });
        let xml = generate(&s, &ExportOptions::default());
        assert!(!xml.contains("理解需求"), "空分量不该出小标题");
        assert!(xml.contains("解决方式"));
    }

    // ------------------------------------------------------------ 备注

    #[test]
    fn notes_contain_disclaimer_version_and_ref() {
        let mut s = schema_with_explanation();
        s.source = civilcalc_core::schema::FormulaSource {
            kind: SourceKind::Standard,
            ref_: Some("GB 50010-2010".to_string()),
            verified: true,
            model: None,
            created_by: None,
        };
        let xml = generate(&s, &ExportOptions::default());
        assert!(xml.contains(crate::template_model::DEFAULT_DISCLAIMER));
        assert!(xml.contains("公式版本：test-explain"));
        assert!(xml.contains("参考依据：GB 50010-2010"));
    }

    #[test]
    fn custom_disclaimer_replaces_default() {
        let opts = ExportOptions {
            disclaimer: "本计算书仅供内部复核使用。".to_string(),
            ..ExportOptions::default()
        };
        let xml = generate(&schema_with_explanation(), &opts);
        assert!(xml.contains("本计算书仅供内部复核使用。"));
        assert!(!xml.contains(crate::template_model::DEFAULT_DISCLAIMER));
    }

    /// 开关关掉时不输出对应行
    #[test]
    fn notes_respect_switches() {
        let mut t = ExportOptions::default().to_template();
        for s in &mut t.sections {
            if let TemplateSection::Notes { config } = s {
                *config = NotesConfig {
                    default_disclaimer: String::new(),
                    show_version: false,
                    show_source_ref: false,
                    ..NotesConfig::default()
                };
            }
        }
        let bytes = DocxGenerator::new()
            .generate(
                &schema_with_explanation(),
                &inputs(&[("b", 2.0), ("h", 3.0)]),
                &EvalResult::primary_only(6.0),
                &t,
                &[],
            )
            .unwrap();
        let xml = document_xml(&bytes);
        assert!(!xml.contains("公式版本："));
        assert!(!xml.contains(crate::template_model::DEFAULT_DISCLAIMER));
    }

    // ------------------------------------------------------------ 边界

    /// 全部章节关闭 → 只有封面，仍是合法 docx
    #[test]
    fn cover_only_document_is_valid() {
        let opts = ExportOptions {
            show_formula_info: false,
            show_params_table: false,
            show_result_block: false,
            show_calculation_steps: false,
            show_explanation: false,
            show_notes: false,
            ..ExportOptions::default()
        };
        let xml = generate(&schema_with_explanation(), &opts);
        assert!(xml.contains(COVER_TITLE));
        assert!(!xml.contains("公式信息"));
    }

    /// 无变量、无分步的极简公式不 panic
    #[test]
    fn minimal_formula_does_not_panic() {
        let s = schema(
            r#"{
                "id":"m","resultName":"一加一","resultSymbol":"V",
                "expression":"1+1","source":{"kind":"CUSTOM"}
            }"#,
        );
        let bytes = DocxGenerator::new()
            .generate(
                &s,
                &HashMap::new(),
                &EvalResult::primary_only(2.0),
                &ExportOptions::default().to_template(),
                &[],
            )
            .unwrap();
        assert!(bytes.starts_with(b"PK"));
    }

    /// 空 schema 列表（没有 sections）不 panic
    #[test]
    fn empty_sections_does_not_panic() {
        let mut t = ExportOptions::default().to_template();
        t.sections.clear();
        let bytes = DocxGenerator::new()
            .generate(
                &schema_with_explanation(),
                &HashMap::new(),
                &EvalResult::primary_only(1.0),
                &t,
                &[],
            )
            .unwrap();
        assert!(bytes.starts_with(b"PK"));
    }

    /// 结果摘要行（排障用）
    #[test]
    fn result_summary_line_formats() {
        let items = vec![
            ResolvedResult {
                symbol: "x".to_string(),
                name: "X".to_string(),
                value: Some(1.0),
                unit: "m".to_string(),
            },
            ResolvedResult {
                symbol: "y".to_string(),
                name: "Y".to_string(),
                value: Some(2.0),
                unit: String::new(),
            },
        ];
        assert_eq!(result_summary_line(&items), "x = 1 m, y = 2");
    }

    // ------------------------------------------------------------ 助手

    fn var(symbol: &str, desc: &str, unit: &str, default: Option<f64>) -> civilcalc_core::schema::FormulaVar {
        civilcalc_core::schema::FormulaVar {
            symbol: symbol.to_string(),
            desc: desc.to_string(),
            unit: Some(unit.to_string()),
            default,
            min: None,
            max: None,
            required: true,
        }
    }

    /// `StepTemplate` 在测试里未直接用到，但确认 core 类型可构造
    /// （`result_display_name` 依赖它）
    #[test]
    fn step_template_type_is_available() {
        let s = StepTemplate {
            symbol: "x".to_string(),
            label: "X 坐标".to_string(),
            group: None,
            expression: "a+1".to_string(),
            unit: "m".to_string(),
            note: None,
        };
        let mut sch = schema_with_explanation();
        sch.steps_template = Some(vec![s]);
        assert_eq!(
            civilcalc_core::schema::result_display_name(&sch, "x"),
            "X 坐标"
        );
    }
}
