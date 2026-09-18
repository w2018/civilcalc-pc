//! 计算书导出选项。
//!
//! 源：`core/report/ExportOptions.kt`（58 行）
//! 契约：`docs/04-数据契约.md` §5.1
//!
//! ## 为什么是「导出选项」而不是「模板」
//!
//! 源项目原本有模板 CRUD。实际用下来：**建模板＝建文件夹**，
//! 只是给一组章节起个名字，没有额外能力，用户判定无存在价值。
//! 于是改成「导出时直接勾选 + 结果持久化」，不再需要先建模板。
//!
//! 章节模型仍复用 [`ReportTemplate`] / [`TemplateSection`]，
//! [`ExportOptions::to_template`] 就是两者的桥。
//!
//! ## 落点
//!
//! 7 个勾选 + 免责声明**持久化到 `AppConfig`**（不是独立表）——
//! 源项目 `report_templates` 表已是死表，PC 端不建（ADR-024）。

use crate::template_model::{CoverConfig, ReportTemplate, TemplateSection};
use civilcalc_core::schema::FormulaSchema;
use serde::{Deserialize, Serialize};

/// 内置模板 id（[`ExportOptions::to_template`] 产物固定用它）
pub const TEMPLATE_ID: &str = "export-options";

/// 计算书导出选项。
///
/// 7 个勾选默认**全开**（用户不选也能导出一份完整计算书）；
/// `disclaimer` 默认空串 = 用内置默认免责声明。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExportOptions {
    /// 封面（主标题 + 计算器名）
    pub show_cover: bool,
    pub show_formula_info: bool,
    pub show_params_table: bool,
    pub show_result_block: bool,
    /// 分步计算（**不是** `TemplateSection::Steps`，那是样式开关）
    pub show_calculation_steps: bool,
    pub show_explanation: bool,
    pub show_notes: bool,
    /// 免责声明；空 = 用内置默认（见 [`crate::template_model::DEFAULT_DISCLAIMER`]）
    pub disclaimer: String,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            show_cover: true,
            show_formula_info: true,
            show_params_table: true,
            show_result_block: true,
            show_calculation_steps: true,
            show_explanation: true,
            show_notes: true,
            disclaimer: String::new(),
        }
    }
}

impl ExportOptions {
    /// 展开为固定顺序的计算书章节（`DocxGenerator` 的输入口径）。
    ///
    /// ## ⚠️ 顺序是**写死**的，与结构体字段顺序无关
    ///
    /// 源项目用 `buildList { if (…) add(…) }` 按固定次序追加。
    /// 前端勾选顺序**不影响**文档章节顺序 —— 否则同一份配置在不同
    /// 操作路径下会产出不同的文档，对不上「计算书要可复现」的要求。
    ///
    /// 固定顺序：`FormulaInfo → ParamsTable → ResultBlock → StepResults
    /// → Explanation → Notes`
    pub fn to_template(&self) -> ReportTemplate {
        let mut sections = Vec::with_capacity(6);

        if self.show_formula_info {
            sections.push(TemplateSection::formula_info());
        }
        if self.show_params_table {
            sections.push(TemplateSection::params_table());
        }
        if self.show_result_block {
            sections.push(TemplateSection::result_block());
        }
        if self.show_calculation_steps {
            sections.push(TemplateSection::step_results());
        }
        if self.show_explanation {
            sections.push(TemplateSection::explanation());
        }
        if self.show_notes {
            sections.push(TemplateSection::notes_with_disclaimer(&self.disclaimer));
        }

        ReportTemplate {
            id: TEMPLATE_ID.to_string(),
            name: "导出选项".to_string(),
            is_builtin: true,
            sections,
            // 封面两个开关跟随 showCover（源项目 `CoverConfig(showProjectName = showCover, showCalculatorName = showCover)`）
            cover: CoverConfig {
                show_project_name: self.show_cover,
                show_calculator_name: self.show_cover,
                ..CoverConfig::default()
            },
            // footer / createdAt / updatedAt 走 ReportTemplate 的默认值
            ..ReportTemplate::default()
        }
    }

    /// 是否**一个章节都没勾**（导出会得到只有封面的空文档）。
    ///
    /// 导出前应据此提示用户 —— 一份只有封面的「计算书」几乎肯定是误操作。
    pub fn is_empty_selection(&self) -> bool {
        !(self.show_formula_info
            || self.show_params_table
            || self.show_result_block
            || self.show_calculation_steps
            || self.show_explanation
            || self.show_notes)
    }
}

/// 公式是否带可导出的「详解」内容。
///
/// 详解来自 AI 分析（理解需求 / 解决方式 / 分步依据任一非空）。
/// 用于：`show_explanation` 勾着但公式没有详解时，导出对话框应给出提示，
/// 而不是导出一个写着「本公式没有可导出的详解内容」的章节。
///
/// ⚠️ 判定用 `trim` 后非空 —— 源项目用 `isNotBlank()`，
/// 只有空白字符的字段不算「有内容」。
pub fn has_explanation_content(schema: &FormulaSchema) -> bool {
    let Some(e) = schema.explanation.as_ref() else {
        return false;
    };

    if !e.summary.trim().is_empty() || !e.solution.trim().is_empty() {
        return true;
    }

    e.steps.iter().any(|s| {
        !s.title.trim().is_empty()
            || !s.detail.trim().is_empty()
            || s.expression
                .as_ref()
                .is_some_and(|x| !x.trim().is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use civilcalc_core::schema::{
        EvalResult, ExplanationStep, FormulaExplanation, FormulaSource, SourceKind,
    };

    /// 最小可解析的 Schema JSON（只有 5 个必填字段，其余走 serde 默认）
    const MINIMAL: &str = r#"{
        "id": "test-explain",
        "resultName": "测试公式",
        "resultSymbol": "A",
        "expression": "A = b*h",
        "source": { "kind": "CUSTOM" }
    }"#;

    fn base_schema() -> FormulaSchema {
        serde_json::from_str(MINIMAL).expect("最小 JSON 应能解析")
    }

    fn schema_with_explanation() -> FormulaSchema {
        let mut s = base_schema();
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

    /// 章节的判别名（测试里用来断言顺序）
    fn kind_names(t: &ReportTemplate) -> Vec<&'static str> {
        t.sections
            .iter()
            .map(|s| match s {
                TemplateSection::FormulaInfo { .. } => "FormulaInfo",
                TemplateSection::ParamsTable { .. } => "ParamsTable",
                TemplateSection::Steps { .. } => "Steps",
                TemplateSection::StepResults { .. } => "StepResults",
                TemplateSection::ResultBlock { .. } => "ResultBlock",
                TemplateSection::Explanation => "Explanation",
                TemplateSection::Notes { .. } => "Notes",
            })
            .collect()
    }

    // ------------------------------------------------------------ 源项目用例

    /// 源 `allSectionsOnByDefaultInFixedOrder`
    #[test]
    fn all_sections_on_by_default_in_fixed_order() {
        assert_eq!(
            kind_names(&ExportOptions::default().to_template()),
            [
                "FormulaInfo",
                "ParamsTable",
                "ResultBlock",
                "StepResults",
                "Explanation",
                "Notes"
            ]
        );
    }

    /// 源 `uncheckedSectionsAreDropped`
    #[test]
    fn unchecked_sections_are_dropped() {
        let opts = ExportOptions {
            show_explanation: false,
            show_calculation_steps: false,
            show_notes: false,
            ..ExportOptions::default()
        };
        assert_eq!(
            kind_names(&opts.to_template()),
            ["FormulaInfo", "ParamsTable", "ResultBlock"]
        );
    }

    /// 源 `coverFlagsFollowShowCover`
    #[test]
    fn cover_flags_follow_show_cover() {
        let off = ExportOptions {
            show_cover: false,
            ..ExportOptions::default()
        }
        .to_template();
        assert!(!off.cover.show_project_name);
        assert!(!off.cover.show_calculator_name);

        let on = ExportOptions::default().to_template();
        assert!(on.cover.show_project_name);
        assert!(on.cover.show_calculator_name);
    }

    /// 源 `disclaimerFallsBackToBuiltinWhenBlank`
    #[test]
    fn disclaimer_falls_back_to_builtin_when_blank() {
        let blank = ExportOptions::default().to_template();
        assert_eq!(
            notes_config(&blank).default_disclaimer,
            crate::template_model::DEFAULT_DISCLAIMER
        );

        let custom = ExportOptions {
            disclaimer: "本计算书仅供内部复核使用。".to_string(),
            ..ExportOptions::default()
        }
        .to_template();
        assert_eq!(
            notes_config(&custom).default_disclaimer,
            "本计算书仅供内部复核使用。"
        );
    }

    /// 源 `explanationContentDetection`
    #[test]
    fn explanation_content_detection() {
        assert!(has_explanation_content(&schema_with_explanation()));

        let mut none = base_schema();
        none.explanation = None;
        assert!(!has_explanation_content(&none));

        let mut empty = base_schema();
        empty.explanation = Some(FormulaExplanation::default());
        assert!(!has_explanation_content(&empty));
    }

    /// 源 `sectionsSurviveJsonRoundTrip`（PC 端不落库，但保留以钉住 7 变体可序列化）
    #[test]
    fn sections_survive_json_round_trip() {
        let sections = ExportOptions::default().to_template().sections;
        let json = serde_json::to_string(&sections).unwrap();
        let back: Vec<TemplateSection> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, sections, "往返失败: {json}");
    }

    fn notes_config(t: &ReportTemplate) -> crate::template_model::NotesConfig {
        t.sections
            .iter()
            .find_map(|s| match s {
                TemplateSection::Notes { config } => Some(config.clone()),
                _ => None,
            })
            .expect("应有 Notes 章节")
    }

    // ------------------------------------------------------------ 顺序无关性

    /// 勾选顺序不影响文档章节顺序（顺序写死）
    #[test]
    fn selection_order_does_not_change_section_order() {
        // 只勾 ResultBlock 与 FormulaInfo —— 无论怎么点，产出顺序固定
        let opts = ExportOptions {
            show_formula_info: true,
            show_params_table: false,
            show_result_block: true,
            show_calculation_steps: false,
            show_explanation: false,
            show_notes: false,
            ..ExportOptions::default()
        };
        assert_eq!(kind_names(&opts.to_template()), ["FormulaInfo", "ResultBlock"]);
    }

    // ------------------------------------------------------------ 边界

    /// 全部不勾 → 只有封面
    #[test]
    fn all_off_yields_cover_only() {
        let opts = ExportOptions {
            show_formula_info: false,
            show_params_table: false,
            show_result_block: false,
            show_calculation_steps: false,
            show_explanation: false,
            show_notes: false,
            ..ExportOptions::default()
        };
        let t = opts.to_template();
        assert!(t.sections.is_empty(), "不该有章节");
        assert!(t.cover.show_project_name, "封面仍显示");
        assert!(opts.is_empty_selection());
    }

    #[test]
    fn default_selection_is_not_empty() {
        assert!(!ExportOptions::default().is_empty_selection());
    }

    /// 模板元信息
    #[test]
    fn template_meta() {
        let t = ExportOptions::default().to_template();
        assert_eq!(t.id, TEMPLATE_ID);
        assert_eq!(t.name, "导出选项");
        assert!(t.is_builtin);
        assert_eq!(t.footer, "");
        assert!(t.created_at > 0 && t.updated_at > 0);
    }

    // ------------------------------------------------------------ 详解判定

    /// 只有空白字符的字段不算「有内容」
    #[test]
    fn whitespace_only_explanation_is_empty() {
        let mut s = base_schema();
        s.explanation = Some(FormulaExplanation {
            summary: "  \n\t ".to_string(),
            solution: " ".to_string(),
            steps: vec![ExplanationStep {
                title: "  ".to_string(),
                expression: Some("   ".to_string()),
                detail: "\n".to_string(),
            }],
        });
        assert!(!has_explanation_content(&s), "全空白应判为无内容");
    }

    /// 任一分量非空即为「有内容」—— 逐个分量单独验证
    #[test]
    fn any_single_component_counts() {
        let cases: Vec<FormulaExplanation> = vec![
            FormulaExplanation {
                summary: "有".to_string(),
                ..Default::default()
            },
            FormulaExplanation {
                solution: "有".to_string(),
                ..Default::default()
            },
            FormulaExplanation {
                steps: vec![ExplanationStep {
                    title: "有".to_string(),
                    expression: None,
                    detail: String::new(),
                }],
                ..Default::default()
            },
            FormulaExplanation {
                steps: vec![ExplanationStep {
                    title: String::new(),
                    expression: None,
                    detail: "有".to_string(),
                }],
                ..Default::default()
            },
            FormulaExplanation {
                steps: vec![ExplanationStep {
                    title: String::new(),
                    expression: Some("x+1".to_string()),
                    detail: String::new(),
                }],
                ..Default::default()
            },
        ];

        for (i, e) in cases.into_iter().enumerate() {
            let mut s = base_schema();
            s.explanation = Some(e);
            assert!(has_explanation_content(&s), "第 {i} 个分量非空应判为有内容");
        }
    }

    // ------------------------------------------------------------ 序列化

    /// 导出选项走 camelCase（要落 `AppConfig`，是 IPC 可见的）
    #[test]
    fn options_serde_is_camel_case() {
        let v = serde_json::to_value(ExportOptions::default()).unwrap();
        assert!(v.get("showCover").is_some());
        assert!(v.get("showCalculationSteps").is_some());
        assert!(v.get("disclaimer").is_some());
        assert!(v.get("show_cover").is_none(), "不得泄漏 snake_case");
    }

    /// 缺字段的老配置应能解析（全走默认）
    #[test]
    fn options_tolerate_missing_fields() {
        let o: ExportOptions = serde_json::from_str(r#"{"showNotes":false}"#).unwrap();
        assert!(!o.show_notes);
        assert!(o.show_cover, "其余走默认 true");
        assert_eq!(o.disclaimer, "");

        let o: ExportOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(o, ExportOptions::default());
    }

    /// 用真实 Schema 类型做一次端到端（防止 core 类型改名后这里悄悄失效）
    #[test]
    fn works_with_real_schema_type() {
        let mut s = schema_with_explanation();
        s.source = FormulaSource {
            kind: SourceKind::Ai,
            ref_: None,
            verified: false,
            model: Some("test-model".to_string()),
            created_by: None,
        };
        assert!(has_explanation_content(&s));

        // EvalResult 只是确认 core 类型可用（P3-10 的渲染输入）
        let r = EvalResult::primary_only(6.0);
        assert_eq!(r.primary, 6.0);
    }
}
