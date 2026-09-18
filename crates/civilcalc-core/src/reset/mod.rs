//! 重置软件：9 类可重置项 + 真实存量统计 + 二次确认文案。
//!
//! 源：`civilcalc-android-v2/core/reset/AppReset.kt`（214 行）
//!
//! ## 9 个类别（`ResetSection`）
//!
//! `FORMULAS` / `HISTORY` / `USAGE_STATS` / `IMAGES` / `APPEARANCE` /
//! `PROMPTS` / `INPUT_HISTORY` / `LLM_CONFIG` / `WEBDAV_CONFIG`
//!
//! ## 默认勾选
//!
//! 前 7 项默认勾选；**`LLM_CONFIG` 与 `WEBDAV_CONFIG` 默认不勾**
//! （源项目注释："要清得自己伸手"）
//!
//! ## 文案要求
//!
//! `confirm_lines()` 逐项写清将删除什么，并注明
//! "不可撤销、云端备份不受影响"

use serde::{Deserialize, Serialize};

/// 「重置软件」能清空/回退的类别。
///
/// 顺序就是设置页弹窗里的展示顺序：先把本机数据排前面，配置类（模型、WebDAV）排最后 ——
/// 它们默认不勾，要清得用户自己伸手。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResetSection {
    /// 公式 / 收藏 / 版本链（三张表一起，公式没了收藏与版本链就没有意义）
    Formulas,
    /// 计算历史
    History,
    /// Token 用量记录
    UsageStats,
    /// 图片缓存（公式与历史引用的插图会变成「图片已删除」占位）
    Images,
    /// 外观：背景图、主题、文字颜色、透明度
    Appearance,
    /// 默认提示词与提醒词（回退到内置文案）
    Prompts,
    /// 输入历史、公式草稿、模型测试对话
    InputHistory,
    /// AI 模型配置与密钥（回到内置三家预设）
    LlmConfig,
    /// WebDAV 服务器地址 / 账号 / 密码
    WebDavConfig,
}

impl ResetSection {
    /// 展示顺序（也是执行顺序）。
    pub const ALL_ORDER: [ResetSection; 9] = [
        ResetSection::Formulas,
        ResetSection::History,
        ResetSection::UsageStats,
        ResetSection::Images,
        ResetSection::Appearance,
        ResetSection::Prompts,
        ResetSection::InputHistory,
        ResetSection::LlmConfig,
        ResetSection::WebDavConfig,
    ];
}

/// 重置范围。`formulas`…`input_history` 是数据类（默认勾），`llm_config` 与 `web_dav_config` 是配置类（默认不勾）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetSelection {
    pub formulas: bool,
    pub history: bool,
    pub usage_stats: bool,
    pub images: bool,
    pub appearance: bool,
    pub prompts: bool,
    pub input_history: bool,
    pub llm_config: bool,
    pub web_dav_config: bool,
}

impl Default for ResetSelection {
    fn default() -> Self {
        // 前 7 项默认勾选；LLM/WebDAV 配置默认不勾
        Self {
            formulas: true,
            history: true,
            usage_stats: true,
            images: true,
            appearance: true,
            prompts: true,
            input_history: true,
            llm_config: false,
            web_dav_config: false,
        }
    }
}

impl ResetSelection {
    /// 已勾选的类别（执行顺序即此顺序）。
    pub fn sections(&self) -> Vec<ResetSection> {
        let mut out = Vec::new();
        if self.formulas {
            out.push(ResetSection::Formulas);
        }
        if self.history {
            out.push(ResetSection::History);
        }
        if self.usage_stats {
            out.push(ResetSection::UsageStats);
        }
        if self.images {
            out.push(ResetSection::Images);
        }
        if self.appearance {
            out.push(ResetSection::Appearance);
        }
        if self.prompts {
            out.push(ResetSection::Prompts);
        }
        if self.input_history {
            out.push(ResetSection::InputHistory);
        }
        if self.llm_config {
            out.push(ResetSection::LlmConfig);
        }
        if self.web_dav_config {
            out.push(ResetSection::WebDavConfig);
        }
        out
    }

    /// 是否至少勾选了一项。
    pub fn any(&self) -> bool {
        self.sections().is_empty()
    }

    /// 该类别是否勾选。
    pub fn is_on(&self, section: ResetSection) -> bool {
        self.sections().contains(&section)
    }

    /// 切换某类别的勾选状态。
    pub fn with(&self, section: ResetSection, on: bool) -> Self {
        let mut s = self.clone();
        match section {
            ResetSection::Formulas => s.formulas = on,
            ResetSection::History => s.history = on,
            ResetSection::UsageStats => s.usage_stats = on,
            ResetSection::Images => s.images = on,
            ResetSection::Appearance => s.appearance = on,
            ResetSection::Prompts => s.prompts = on,
            ResetSection::InputHistory => s.input_history = on,
            ResetSection::LlmConfig => s.llm_config = on,
            ResetSection::WebDavConfig => s.web_dav_config = on,
        }
        s
    }

    /// 全部勾选。
    pub const ALL: ResetSelection = ResetSelection {
        formulas: true,
        history: true,
        usage_stats: true,
        images: true,
        appearance: true,
        prompts: true,
        input_history: true,
        llm_config: true,
        web_dav_config: true,
    };

    /// 全不勾。
    pub const NONE: ResetSelection = ResetSelection {
        formulas: false,
        history: false,
        usage_stats: false,
        images: false,
        appearance: false,
        prompts: false,
        input_history: false,
        llm_config: false,
        web_dav_config: false,
    };
}

/// 本机现在的存量（重置弹窗每行的副标题，以及二次确认里的「将删除 N 条」）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetCounts {
    pub formulas: i64,
    pub favorites: i64,
    pub versions: i64,
    pub history: i64,
    pub usage_stats: i64,
    pub images: i64,
    pub image_bytes: i64,
    pub llm_profiles: i64,
    pub api_keys: i64,
    pub web_dav_configured: bool,
    pub has_background: bool,
}

impl ResetCounts {
    /// 某一行右边显示的存量说明；数量为 0 时给「暂无」而不是「0 条」。
    pub fn subtitle_of(&self, section: ResetSection) -> String {
        match section {
            ResetSection::Formulas => {
                let parts = [
                    (self.formulas, "公式"),
                    (self.favorites, "收藏"),
                    (self.versions, "版本"),
                ];
                let shown: Vec<String> = parts
                    .iter()
                    .filter(|(v, _)| *v > 0)
                    .map(|(v, n)| format!("{v} {n}"))
                    .collect();
                if shown.is_empty() {
                    "暂无公式数据".to_string()
                } else {
                    shown.join(" · ")
                }
            }
            ResetSection::History => {
                if self.history == 0 {
                    "暂无计算历史".to_string()
                } else {
                    format!("{} 记录", self.history)
                }
            }
            ResetSection::UsageStats => {
                if self.usage_stats == 0 {
                    "暂无用量记录".to_string()
                } else {
                    format!("{} 记录", self.usage_stats)
                }
            }
            ResetSection::Images => {
                if self.images == 0 {
                    "暂无插图".to_string()
                } else {
                    format!("{} 张 · {}", self.images, format_bytes(self.image_bytes))
                }
            }
            ResetSection::Appearance => {
                if self.has_background {
                    "已设背景图（并重置主题 / 文字颜色 / 透明度）".to_string()
                } else {
                    "重置主题 / 文字颜色 / 透明度".to_string()
                }
            }
            ResetSection::Prompts => "回退到内置提示词与提醒词".to_string(),
            ResetSection::InputHistory => "搜索 / 精炼 / 模型测试的输入历史与草稿".to_string(),
            ResetSection::LlmConfig => {
                if self.llm_profiles == 0 {
                    "回到内置三家预设".to_string()
                } else {
                    format!(
                        "{} 个模型{}（回到内置三家预设）",
                        self.llm_profiles,
                        if self.api_keys > 0 {
                            format!(" · {} 个密钥", self.api_keys)
                        } else {
                            String::new()
                        },
                    )
                }
            }
            ResetSection::WebDavConfig => {
                if self.web_dav_configured {
                    "已配置（地址 / 账号 / 密码一并清除）".to_string()
                } else {
                    "当前未配置".to_string()
                }
            }
        }
    }

    /// 二次确认弹窗里逐项列的处理说明，**只列勾选的类别**（本机没有的也照实写出来）。
    pub fn confirm_lines(&self, selection: &ResetSelection) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        if selection.formulas {
            if self.formulas + self.favorites + self.versions == 0 {
                lines.push("公式数据（本机暂无）".to_string());
            } else {
                if self.formulas > 0 {
                    lines.push(format!("公式 {} 条", self.formulas));
                }
                if self.favorites > 0 {
                    lines.push(format!("收藏 {} 条", self.favorites));
                }
                if self.versions > 0 {
                    lines.push(format!("版本 {} 条", self.versions));
                }
            }
        }
        if selection.history {
            lines.push(Self::count_line(self.history, "计算历史"));
        }
        if selection.usage_stats {
            lines.push(Self::count_line(self.usage_stats, "用量记录"));
        }
        if selection.images {
            if self.images == 0 {
                lines.push("插图（本机暂无）".to_string());
            } else {
                lines.push(format!(
                    "插图 {} 张（{}）",
                    self.images,
                    format_bytes(self.image_bytes)
                ));
            }
        }
        if selection.appearance {
            lines.push(if self.has_background {
                "删除背景图，主题 / 文字颜色 / 透明度回默认".to_string()
            } else {
                "主题 / 文字颜色 / 透明度回默认".to_string()
            });
        }
        if selection.prompts {
            lines.push("默认提示词与提醒词回退内置文案".to_string());
        }
        if selection.input_history {
            lines.push("清空输入历史、公式草稿与模型测试对话".to_string());
        }
        if selection.llm_config {
            if self.llm_profiles + self.api_keys == 0 {
                lines.push("AI 模型配置与密钥（本机暂无，回到内置三家预设）".to_string());
            } else {
                lines.push(format!(
                    "清空 {} 个模型与 {} 个密钥（回到内置三家预设）",
                    self.llm_profiles, self.api_keys
                ));
            }
        }
        if selection.web_dav_config {
            if self.web_dav_configured {
                lines.push("清除 WebDAV 服务器地址 / 账号 / 密码".to_string());
            } else {
                lines.push("WebDAV 配置（本机未配置）".to_string());
            }
        }
        lines
    }

    fn count_line(value: i64, label: &str) -> String {
        if value > 0 {
            format!("{label} {value} 条")
        } else {
            format!("{label}（本机暂无）")
        }
    }
}

/// 一次重置实际清掉的量。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetSummary {
    pub formulas: i64,
    pub favorites: i64,
    pub versions: i64,
    pub history: i64,
    pub usage_stats: i64,
    pub images: i64,
    pub background: bool,
    /// 被重置（回退默认值）的偏好项数
    pub preferences: i64,
    pub llm_config: bool,
    pub api_keys: i64,
    pub web_dav_config: bool,
}

impl ResetSummary {
    /// 一行摘要：公式 65 · 历史 12 · 插图 8 张 · 设置 4 项
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.formulas > 0 {
            parts.push(format!("公式 {} 条", self.formulas));
        }
        if self.favorites > 0 {
            parts.push(format!("收藏 {} 条", self.favorites));
        }
        if self.versions > 0 {
            parts.push(format!("版本 {} 条", self.versions));
        }
        if self.history > 0 {
            parts.push(format!("历史 {} 条", self.history));
        }
        if self.usage_stats > 0 {
            parts.push(format!("用量记录 {} 条", self.usage_stats));
        }
        if self.images > 0 {
            parts.push(format!("插图 {} 张", self.images));
        }
        if self.background {
            parts.push("背景图".to_string());
        }
        if self.preferences > 0 {
            parts.push(format!("设置 {} 项", self.preferences));
        }
        if self.llm_config {
            parts.push("AI 模型配置".to_string());
        }
        if self.api_keys > 0 {
            parts.push(format!("密钥 {} 个", self.api_keys));
        }
        if self.web_dav_config {
            parts.push("WebDAV 配置".to_string());
        }
        if parts.is_empty() {
            "没有可重置的内容".to_string()
        } else {
            format!("已重置：{}", parts.join(" · "))
        }
    }
}

/// 字节数给人看的形式（与设置页其它地方的写法一致）。
pub fn format_bytes(bytes: i64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / 1024.0 / 1024.0 / 1024.0)
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts() -> ResetCounts {
        ResetCounts {
            formulas: 65,
            favorites: 12,
            versions: 8,
            history: 30,
            usage_stats: 4,
            images: 3,
            image_bytes: 12_345,
            llm_profiles: 2,
            api_keys: 1,
            web_dav_configured: true,
            has_background: true,
        }
    }

    #[test]
    fn default_selection_data_on_config_off() {
        let s = ResetSelection::default();
        assert!(s.formulas && s.history && s.usage_stats && s.images && s.appearance && s.prompts && s.input_history);
        assert!(!s.llm_config && !s.web_dav_config);
    }

    #[test]
    fn sections_keep_order() {
        let s = ResetSelection::default();
        assert_eq!(s.sections().len(), 7);
        assert_eq!(s.sections()[0], ResetSection::Formulas);
        assert_eq!(s.sections()[6], ResetSection::InputHistory);
    }

    #[test]
    fn with_toggles_and_is_on() {
        let s = ResetSelection::NONE.with(ResetSection::LlmConfig, true);
        assert!(s.is_on(ResetSection::LlmConfig));
        assert!(!s.is_on(ResetSection::Formulas));
        let s2 = s.with(ResetSection::LlmConfig, false);
        assert!(!s2.is_on(ResetSection::LlmConfig));
    }

    #[test]
    fn subtitle_text() {
        let c = counts();
        assert_eq!(
            c.subtitle_of(ResetSection::Formulas),
            "65 公式 · 12 收藏 · 8 版本"
        );
        assert_eq!(c.subtitle_of(ResetSection::Images), "3 张 · 12 KB");
        assert_eq!(
            c.subtitle_of(ResetSection::LlmConfig),
            "2 个模型 · 1 个密钥（回到内置三家预设）"
        );
        assert_eq!(c.subtitle_of(ResetSection::WebDavConfig), "已配置（地址 / 账号 / 密码一并清除）");
    }

    #[test]
    fn confirm_lines_only_selected() {
        let c = counts();
        // 只勾公式 + LLM（公式块分条列出公式/收藏/版本）
        let sel = ResetSelection::NONE
            .with(ResetSection::Formulas, true)
            .with(ResetSection::LlmConfig, true);
        let lines = c.confirm_lines(&sel);
        assert_eq!(
            lines,
            vec![
                "公式 65 条".to_string(),
                "收藏 12 条".to_string(),
                "版本 8 条".to_string(),
                "清空 2 个模型与 1 个密钥（回到内置三家预设）".to_string(),
            ]
        );
    }

    #[test]
    fn confirm_lines_empty_selection_still_lists_nothing_present() {
        let c = ResetCounts::default();
        let sel = ResetSelection::NONE.with(ResetSection::Formulas, true);
        assert_eq!(c.confirm_lines(&sel), vec!["公式数据（本机暂无）"]);
    }

    #[test]
    fn summary_describe_joins_parts() {
        let s = ResetSummary {
            formulas: 65,
            history: 12,
            images: 3,
            preferences: 4,
            ..Default::default()
        };
        assert_eq!(
            s.describe(),
            "已重置：公式 65 条 · 历史 12 条 · 插图 3 张 · 设置 4 项"
        );
        assert_eq!(ResetSummary::default().describe(), "没有可重置的内容");
    }

    #[test]
    fn format_bytes_tiers() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2 KB");
        assert_eq!(format_bytes(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.0 GB");
    }
}
