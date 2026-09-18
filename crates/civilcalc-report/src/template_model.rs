//! 计算书章节模型。
//!
//! 源：`core/report/TemplateModel.kt`（107 行）
//! 契约：`docs/04-数据契约.md` §5.2
//!
//! ## 为什么「模板」还在，但界面上没有模板管理
//!
//! 源项目已用**导出选项**取代「模板 CRUD」（源码注释：「用户判定无存在价值」，
//! 建模板＝建文件夹）。PC 端同理（ADR-023）：只实现 [`crate::export_options::ExportOptions`]，
//! 但 **`ReportTemplate` / `TemplateSection` 模型保留** —— 它是
//! `ExportOptions::to_template()` 的返回载体，也是 `DocxGenerator` 的输入口径。
//!
//! 这两个类型**不落库**（源项目 `report_templates` 是死表，PC 端不建，ADR-024）。

use serde::{Deserialize, Serialize};

/// 默认免责声明 —— **逐字**照抄源项目，改一个字都算回归。
///
/// 源：`TemplateModel.kt:44`。注意「AI全能计算器」与「注册工程师」之间没有空格。
pub const DEFAULT_DISCLAIMER: &str = "本计算书由AI全能计算器自动生成，结果需经注册工程师复核。";

/// 参数表默认列（源：`TemplateSection.ParamsTable.columns` 的默认值）
pub const DEFAULT_PARAMS_COLUMNS: [&str; 4] = ["symbol", "desc", "value", "unit"];

fn default_true() -> bool {
    true
}

fn default_sections() -> Vec<TemplateSection> {
    vec![
        TemplateSection::formula_info(),
        TemplateSection::params_table(),
        TemplateSection::result_block(),
        TemplateSection::step_results(),
        TemplateSection::notes(),
    ]
}

fn default_params_columns() -> Vec<String> {
    DEFAULT_PARAMS_COLUMNS.iter().map(|s| (*s).to_string()).collect()
}

/// 计算书模板。**不落库**，只作渲染输入。
///
/// `#[serde(default)]` 在**结构体**上：反序列化时缺失字段取 `Default` 里的值，
/// 这样老 JSON（少字段）不会解析失败。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ReportTemplate {
    pub id: String,
    pub name: String,
    pub is_builtin: bool,
    pub sections: Vec<TemplateSection>,
    pub cover: CoverConfig,
    pub footer: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Default for ReportTemplate {
    fn default() -> Self {
        let now = civilcalc_core::now_ms();
        Self {
            id: String::new(),
            name: String::new(),
            is_builtin: false,
            sections: default_sections(),
            cover: CoverConfig::default(),
            footer: String::new(),
            created_at: now,
            updated_at: now,
        }
    }
}

/// 封面配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CoverConfig {
    /// 是否显示「工程计算书」主标题
    pub show_project_name: bool,
    /// 是否显示计算器名（或 [`CoverConfig::calculator_name_alias`]）
    pub show_calculator_name: bool,
    /// 是否显示 logo（**一期不渲染** —— 源项目该字段也未真正使用）
    pub show_logo: bool,
    /// logo 路径（同上，一期保留字段不渲染）
    pub logo_uri: Option<String>,
    /// 计算器名别名；`None` 时回落到 `schema.resultName`
    pub calculator_name_alias: Option<String>,
    /// 自定义封面字段（渲染成「标签：____」下划线填空行）
    pub custom_fields: Vec<CoverField>,
}

impl Default for CoverConfig {
    fn default() -> Self {
        Self {
            show_project_name: true,
            show_calculator_name: true,
            show_logo: false,
            logo_uri: None,
            calculator_name_alias: None,
            custom_fields: Vec::new(),
        }
    }
}

/// 封面上的自定义填空字段。
///
/// ⚠️ 三个字段**都没有默认值**（对齐源项目 `data class CoverField`）——
/// 缺 `key` 的字段是无意义的，宁可反序列化失败也不静默补空串。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverField {
    pub key: String,
    pub label: String,
    pub required: bool,
}

/// 分步样式。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StepStyle {
    pub numbered: bool,
    pub show_substitution: bool,
    pub show_result: bool,
}

impl Default for StepStyle {
    fn default() -> Self {
        Self {
            numbered: true,
            show_substitution: true,
            show_result: true,
        }
    }
}

/// 备注章节配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotesConfig {
    /// 免责声明正文。空串时由 `ExportOptions::to_template()` 回落到 [`DEFAULT_DISCLAIMER`]
    pub default_disclaimer: String,
    pub allow_user_override: bool,
    /// 是否输出「公式版本：{id}」
    pub show_version: bool,
    /// 是否输出「参考依据：{ref}」
    pub show_source_ref: bool,
}

impl Default for NotesConfig {
    fn default() -> Self {
        Self {
            default_disclaimer: DEFAULT_DISCLAIMER.to_string(),
            allow_user_override: true,
            show_version: true,
            show_source_ref: true,
        }
    }
}

/// 计算书章节。**7 变体**，`#[serde(tag = "type")]` 非破坏可扩展。
///
/// ## ⚠️ 变体名保持 PascalCase（不加 `rename_all`）
///
/// 契约 `docs/04` §5.2 写的是 `#[serde(tag = "type")]`，即变体序列化为
/// `"FormulaInfo"` / `"ParamsTable"` …。字段则用 `rename_all_fields` 转 camelCase。
///
/// 本项目其余类型一律 camelCase，这里是**刻意的不一致** —— 章节名是
/// 「数据契约里已冻结的枚举值」，与业务字段命名规则不同源。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum TemplateSection {
    /// 公式信息表（名称 / 表达式 / 来源 / 依据）
    FormulaInfo {
        #[serde(default = "default_true")]
        show_source: bool,
        #[serde(default = "default_true")]
        show_version: bool,
        #[serde(default = "default_true")]
        show_verification: bool,
    },

    /// 参数取值表
    ParamsTable {
        #[serde(default = "default_params_columns")]
        columns: Vec<String>,
    },

    /// 分步样式开关。
    ///
    /// ⚠️ **`DocxGenerator` 不渲染该章节**（源项目 `is TemplateSection.Steps -> { }`）。
    /// 真正的分步表格由 [`TemplateSection::StepResults`] 输出。
    /// 保留变体是为了契约完整与「非破坏可扩展」。
    Steps {
        #[serde(default)]
        style: StepStyle,
    },

    /// 分步计算表（6 列：序号 / 表达式 / 代入数值 / 结果 / Excel 公式 / Excel 代入数值）
    StepResults {
        #[serde(default = "default_true")]
        show_expression: bool,
        #[serde(default = "default_true")]
        show_substituted: bool,
        #[serde(default = "default_true")]
        show_excel_formula: bool,
        #[serde(default = "default_true")]
        show_unit: bool,
    },

    /// 计算结果块（单结果居中加粗；多结果成表；分支结果表）
    ResultBlock {
        #[serde(default = "default_true")]
        with_unit_conversion: bool,
    },

    /// 公式详解（内容取自 `schema.explanation`，无字段）
    Explanation,

    /// 备注（免责声明 / 版本 / 参考依据）
    Notes {
        #[serde(default)]
        config: NotesConfig,
    },
}

impl TemplateSection {
    // ------------------------------------------------------------ 默认构造器
    //
    // Rust 的枚举变体没有「字段默认值」概念，源项目可以直接写
    // `TemplateSection.FormulaInfo()`。这里用构造器把默认值集中在一处 ——
    // 否则每个调用点都要把 3~4 个 bool 全写一遍，改默认值时容易漏。

    pub fn formula_info() -> Self {
        Self::FormulaInfo {
            show_source: true,
            show_version: true,
            show_verification: true,
        }
    }

    pub fn params_table() -> Self {
        Self::ParamsTable {
            columns: default_params_columns(),
        }
    }

    pub fn steps() -> Self {
        Self::Steps {
            style: StepStyle::default(),
        }
    }

    pub fn step_results() -> Self {
        Self::StepResults {
            show_expression: true,
            show_substituted: true,
            show_excel_formula: true,
            show_unit: true,
        }
    }

    pub fn result_block() -> Self {
        Self::ResultBlock {
            with_unit_conversion: true,
        }
    }

    pub fn explanation() -> Self {
        Self::Explanation
    }

    pub fn notes() -> Self {
        Self::Notes {
            config: NotesConfig::default(),
        }
    }

    /// 带自定义免责声明的备注章节。空串回落到 [`DEFAULT_DISCLAIMER`]。
    pub fn notes_with_disclaimer(disclaimer: &str) -> Self {
        let text = if disclaimer.trim().is_empty() {
            DEFAULT_DISCLAIMER.to_string()
        } else {
            disclaimer.to_string()
        };
        Self::Notes {
            config: NotesConfig {
                default_disclaimer: text,
                ..NotesConfig::default()
            },
        }
    }

    // ------------------------------------------------------------ 元信息

    /// 章节正文标题（**不含**「一、」这类序号前缀，序号由渲染层拼）。
    ///
    /// 文案**逐字**对齐源项目 `DocxGenerator`：
    /// 标题实际渲染为 `{序号}{本文案}`，如「一、公式信息」。
    pub fn title(&self) -> &'static str {
        match self {
            TemplateSection::FormulaInfo { .. } => "公式信息",
            TemplateSection::ParamsTable { .. } => "参数取值",
            TemplateSection::Steps { .. } => "分步样式",
            TemplateSection::StepResults { .. } => "分步计算",
            TemplateSection::ResultBlock { .. } => "计算结果",
            TemplateSection::Explanation => "公式详解",
            TemplateSection::Notes { .. } => "备注",
        }
    }

    /// 是否由 `DocxGenerator` 渲染出内容。
    ///
    /// [`TemplateSection::Steps`] 返回 `false` —— 它只承载样式开关，
    /// 渲染层会**跳过**它。⚠️ 跳过时**也不占用章节序号**
    /// （源项目该分支是空 `{ }`，连 `nextTitle()` 都没调）——
    /// 所以插一个 `Steps` 不会让后面的章节序号整体后移。
    pub fn is_rendered(&self) -> bool {
        !matches!(self, TemplateSection::Steps { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_disclaimer_is_verbatim() {
        assert_eq!(
            DEFAULT_DISCLAIMER,
            "本计算书由AI全能计算器自动生成，结果需经注册工程师复核。"
        );
        assert_eq!(NotesConfig::default().default_disclaimer, DEFAULT_DISCLAIMER);
    }

    #[test]
    fn default_sections_order_is_frozen() {
        let kinds: Vec<&str> = default_sections()
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
            .collect();
        assert_eq!(
            kinds,
            [
                "FormulaInfo",
                "ParamsTable",
                "ResultBlock",
                "StepResults",
                "Notes"
            ]
        );
    }

    #[test]
    fn cover_defaults() {
        let c = CoverConfig::default();
        assert!(c.show_project_name);
        assert!(c.show_calculator_name);
        assert!(!c.show_logo);
        assert!(c.logo_uri.is_none());
        assert!(c.custom_fields.is_empty());
    }

    #[test]
    fn step_style_and_notes_defaults() {
        let s = StepStyle::default();
        assert!(s.numbered && s.show_substitution && s.show_result);

        let n = NotesConfig::default();
        assert!(n.allow_user_override && n.show_version && n.show_source_ref);
    }

    #[test]
    fn template_default_has_default_sections() {
        let t = ReportTemplate::default();
        assert_eq!(t.sections, default_sections());
        assert!(t.created_at > 0, "时间戳应是真实时间而不是 0");
        assert_eq!(t.created_at, t.updated_at);
    }

    /// 章节标题文案逐字对齐源项目
    #[test]
    fn section_titles_are_verbatim() {
        assert_eq!(TemplateSection::formula_info().title(), "公式信息");
        assert_eq!(TemplateSection::params_table().title(), "参数取值");
        assert_eq!(TemplateSection::step_results().title(), "分步计算");
        assert_eq!(TemplateSection::result_block().title(), "计算结果");
        assert_eq!(TemplateSection::explanation().title(), "公式详解");
        assert_eq!(TemplateSection::notes().title(), "备注");
    }

    /// `Steps` 不渲染 —— 源项目 `is TemplateSection.Steps -> { }`
    #[test]
    fn steps_section_is_not_rendered() {
        assert!(!TemplateSection::steps().is_rendered());
        assert!(TemplateSection::step_results().is_rendered());
        assert!(TemplateSection::explanation().is_rendered());
    }

    /// 序列化形状：判别字段是 `type`，变体名 PascalCase，字段 camelCase
    #[test]
    fn section_serde_shape() {
        let v = serde_json::to_value(TemplateSection::formula_info()).unwrap();
        assert_eq!(v["type"], serde_json::json!("FormulaInfo"));
        assert_eq!(v["showSource"], serde_json::json!(true));
        assert!(v.get("show_source").is_none(), "不得泄漏 snake_case");

        let v = serde_json::to_value(TemplateSection::explanation()).unwrap();
        assert_eq!(v["type"], serde_json::json!("Explanation"));

        let v = serde_json::to_value(TemplateSection::params_table()).unwrap();
        assert_eq!(v["columns"], serde_json::json!(DEFAULT_PARAMS_COLUMNS));
    }

    /// 7 个变体全部可 JSON 往返（漏 `Serialize` 会在这里炸）
    #[test]
    fn all_seven_sections_roundtrip() {
        let all = [
            TemplateSection::formula_info(),
            TemplateSection::params_table(),
            TemplateSection::steps(),
            TemplateSection::step_results(),
            TemplateSection::result_block(),
            TemplateSection::explanation(),
            TemplateSection::notes(),
        ];
        assert_eq!(all.len(), 7);
        for s in &all {
            let json = serde_json::to_string(s).unwrap();
            let back: TemplateSection = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, s, "往返失败: {json}");
        }
    }

    /// 少字段的老 JSON 应能解析（`#[serde(default)]` 的意义）
    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let s: TemplateSection =
            serde_json::from_str(r#"{"type":"FormulaInfo"}"#).unwrap();
        assert_eq!(s, TemplateSection::formula_info());

        let s: TemplateSection = serde_json::from_str(r#"{"type":"Notes"}"#).unwrap();
        match s {
            TemplateSection::Notes { config } => {
                assert_eq!(config.default_disclaimer, DEFAULT_DISCLAIMER);
            }
            other => panic!("应变回 Notes，实际 {other:?}"),
        }

        // 结构体层面：只给 id/name，其余走 Default
        let t: ReportTemplate =
            serde_json::from_str(r#"{"id":"t1","name":"模板"}"#).unwrap();
        assert_eq!(t.sections, default_sections());
        assert_eq!(t.cover, CoverConfig::default());
    }

    #[test]
    fn notes_with_blank_disclaimer_falls_back() {
        for blank in ["", "   ", "\n"] {
            match TemplateSection::notes_with_disclaimer(blank) {
                TemplateSection::Notes { config } => {
                    assert_eq!(config.default_disclaimer, DEFAULT_DISCLAIMER);
                }
                other => panic!("应变回 Notes，实际 {other:?}"),
            }
        }

        match TemplateSection::notes_with_disclaimer("仅供内部复核。") {
            TemplateSection::Notes { config } => {
                assert_eq!(config.default_disclaimer, "仅供内部复核。");
                // 其余开关保持默认
                assert!(config.show_version && config.show_source_ref);
            }
            other => panic!("应变回 Notes，实际 {other:?}"),
        }
    }

    /// `CoverField` 三个字段都没有默认值 —— 缺字段必须解析失败
    #[test]
    fn cover_field_requires_all_fields() {
        assert!(serde_json::from_str::<CoverField>(r#"{"key":"k"}"#).is_err());
        let ok: CoverField =
            serde_json::from_str(r#"{"key":"k","label":"l","required":false}"#).unwrap();
        assert_eq!(ok.key, "k");
        assert!(!ok.required);
    }

    #[test]
    fn template_serde_is_camel_case() {
        let t = ReportTemplate::default();
        let v = serde_json::to_value(&t).unwrap();
        assert!(v.get("isBuiltin").is_some());
        assert!(v.get("createdAt").is_some());
        assert!(v.get("is_builtin").is_none());
    }
}
