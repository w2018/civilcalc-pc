//! 备份包的数据模型。
//!
//! 源：`civilcalc-android-v2/core/backup/BackupModel.kt`（178 行）
//!
//! ## 🔴 ADR-009：JSON 形状必须与 Android 端逐字节兼容
//!
//! 用户可能已有云端备份。字段名、枚举名、默认值都要对齐：
//!
//! - **字段名 camelCase**（kotlinx 默认）→ `#[serde(rename_all = "camelCase")]`
//! - **`BackupSection` 是 `SCREAMING_SNAKE_CASE`**（`"FORMULAS"` / `"LLM_CONFIG"` …）
//!   —— 这是枚举**变体名**，不是字段名，所以 `rename_all` 要单独写
//! - **缺字段按默认值处理**（旧备份缺某类数据也要能导入）→ 每个字段 `#[serde(default)]`
//!
//! ## `sections` 为什么必须存在
//!
//! `manifest.sections` 记录「这个包**到底带没带**这类数据」。
//! 空列表与没带是两回事：「完整还原」只清空包里确实含有的类别，
//! 否则会把本机其它数据误删。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 当前备份格式版本。**只在不兼容变更时 +1**。
pub const BACKUP_FORMAT_VERSION: i32 = 1;

/// 备份包里的一个数据类别（写进 [`BackupManifest::sections`]）。
///
/// ⚠️ serde 表示是 `SCREAMING_SNAKE_CASE`，与源 Kotlin 枚举名逐字一致 ——
/// 这个字符串会进备份包，**不能改**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BackupSection {
    Formulas,
    History,
    Preferences,
    UsageStats,
    /// 真实生效的 AI 模型配置（地址/模型/思考强度/协议/活跃模型）
    LlmConfig,
    /// 模型密钥：仅在用户勾选时写入
    ApiKeys,
    Images,
    Background,
}

/// 备份包清单：写明格式版本与来源，导入时先看这个再决定怎么做。
///
/// `counts` 是**给人看的**（确认弹窗里列清单），不参与导入。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupManifest {
    #[serde(default = "default_format_version")]
    pub backup_format_version: i32,
    #[serde(default)]
    pub app_version_code: i64,
    #[serde(default)]
    pub app_version_name: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub counts: BTreeMap<String, i32>,
    /// 本包实际包含的类别（导入时判断该清空/还原哪些）
    #[serde(default)]
    pub sections: Vec<BackupSection>,
}

fn default_format_version() -> i32 {
    BACKUP_FORMAT_VERSION
}

impl Default for BackupManifest {
    fn default() -> Self {
        Self {
            backup_format_version: BACKUP_FORMAT_VERSION,
            app_version_code: 0,
            app_version_name: String::new(),
            created_at: 0,
            counts: BTreeMap::new(),
            sections: Vec::new(),
        }
    }
}

// =============================================================================
// 各表行（字段与源 Room 实体一一对应，导入时按主键 REPLACE）
// =============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormulaRow {
    pub id: String,
    pub schema_json: String,
    #[serde(default)]
    pub favorite: i32,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRow {
    pub id: i64,
    pub formula_id: String,
    pub formula_snapshot_json: String,
    pub inputs_json: String,
    pub result_json: String,
    #[serde(default)]
    pub thinking_content: Option<String>,
    #[serde(default)]
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteRow {
    pub formula_id: String,
    #[serde(default)]
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormulaVersionRow {
    pub formula_id: String,
    pub version: String,
    #[serde(default)]
    pub parent_version: Option<String>,
    pub schema_json: String,
    pub change_type: String,
    #[serde(default)]
    pub change_log: String,
    pub editor: String,
    #[serde(default)]
    pub created_at: i64,
    /// ⚠️ **0/1 而不是 bool**（源 `FormulaVersionRow.verified: Int`，备份包是跨端契约）
    #[serde(default)]
    pub verified: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmUsageStatsRow {
    #[serde(default)]
    pub id: i64,
    pub model_label: String,
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default)]
    pub cached_tokens: i64,
    #[serde(default)]
    pub reasoning_tokens: i64,
    #[serde(default)]
    pub created_at: i64,
}

/// 各表的集合；**缺字段按空处理**（旧备份缺某类数据也能导入）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupTables {
    #[serde(default)]
    pub formulas: Vec<FormulaRow>,
    #[serde(default)]
    pub history: Vec<HistoryRow>,
    #[serde(default)]
    pub favorites: Vec<FavoriteRow>,
    #[serde(default)]
    pub versions: Vec<FormulaVersionRow>,
    #[serde(default)]
    pub usage_stats: Vec<LlmUsageStatsRow>,
}

/// DataStore 偏好：按值类型分开存（当前只有 String 与 Int 两种键）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupPrefs {
    #[serde(default)]
    pub strings: BTreeMap<String, String>,
    #[serde(default)]
    pub ints: BTreeMap<String, i32>,
}

/// 模型密钥（**仅在用户勾选「含 API Key」时写入**；包内是明文）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSecrets {
    #[serde(default)]
    pub api_keys: BTreeMap<String, String>,
}

/// 包内一张图片的登记信息；字节内容在 `images/<fileName>` 条目里（按内容哈希命名，与本地一致）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupImageRecord {
    pub id: String,
    pub file_name: String,
    #[serde(default = "default_mime")]
    pub mime_type: String,
    #[serde(default)]
    pub refs: Vec<String>,
    #[serde(default)]
    pub created_at: i64,
}

fn default_mime() -> String {
    "image/jpeg".to_string()
}

/// 一个备份包的全部内容（**不含图片字节**；图片逐条流式读写）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupContent {
    #[serde(default)]
    pub manifest: BackupManifest,
    #[serde(default)]
    pub tables: BackupTables,
    #[serde(default)]
    pub prefs: BackupPrefs,
    /// 真实生效的 AI 模型配置（加密存储里 `llm_config` 的**原始 JSON**，原样存取）。
    ///
    /// 存原始字符串而不是解析成结构体：这份 JSON 里还有思考强度/协议/视觉等字段，
    /// 原样搬运才能保证「导入后设置页与生成链路完全一致」，也不会因为将来加字段而丢配置。
    /// **不含密钥**（密钥单独在 [`BackupSecrets`]）。
    #[serde(default)]
    pub llm_config_json: Option<String>,
    #[serde(default)]
    pub secrets: Option<BackupSecrets>,
    #[serde(default)]
    pub images: Vec<BackupImageRecord>,
    /// 包内是否带背景图：读入时按实际条目填充；**写出时以 `background_bytes` 参数为准**（不读这个标志）
    #[serde(default)]
    pub has_background: bool,
}

/// 上传时勾选的备份范围。
///
/// 图片三档结论：**被公式引用的默认勾**（不勾则公式里的内联图恢复后是「已删除」占位），
/// 被对话/历史引用的与未被引用的默认不勾 —— 按需选择，包体更小。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSelection {
    /// 公式 + 收藏 + 版本链
    pub formulas: bool,
    pub history: bool,
    /// 偏好（主题 / 提示词 / 提醒词 / 导出选项 / 输入历史 / 模型测试对话等）
    pub preferences: bool,
    pub usage_stats: bool,
    /// 真实生效的 AI 模型配置（**不含密钥**）
    pub llm_config: bool,
    /// 模型密钥：默认不勾，勾选后**明文**写入备份包
    pub include_api_keys: bool,
    pub include_formula_images: bool,
    pub include_history_images: bool,
    pub include_unreferenced_images: bool,
}

impl Default for BackupSelection {
    /// 全量（图片只含公式引用的那档）—— 源 `BackupSelection.ALL`。
    fn default() -> Self {
        Self {
            formulas: true,
            history: true,
            preferences: true,
            usage_stats: true,
            llm_config: true,
            include_api_keys: false,
            include_formula_images: true,
            include_history_images: false,
            include_unreferenced_images: false,
        }
    }
}

impl BackupSelection {
    /// 全量（与 `Default` 同义，供调用方表意）。
    #[must_use]
    pub fn all() -> Self {
        Self::default()
    }
}

// =============================================================================
// 备份摘要（确认弹窗与结果提示用）
// =============================================================================

/// 一次备份/导入涉及的数据量。
///
/// 源 `core/backup/WebDavRepository.kt` 的 `BackupSummary`。
/// **既是导出时的「我放了什么」，也是导入后的「我写了什么」** —— 同一个类型两处用，
/// 所以 `of(content)` 与「实际写入条数」都能构造出它。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSummary {
    #[serde(default)]
    pub formulas: i32,
    #[serde(default)]
    pub history: i32,
    #[serde(default)]
    pub favorites: i32,
    #[serde(default)]
    pub versions: i32,
    #[serde(default)]
    pub usage_stats: i32,
    /// 偏好项数（`strings.len() + ints.len()`）
    #[serde(default)]
    pub preferences: i32,
    /// 是否包含 AI 模型配置
    #[serde(default)]
    pub llm_config: bool,
    #[serde(default)]
    pub api_keys: i32,
    #[serde(default)]
    pub images: i32,
    #[serde(default)]
    pub background: bool,
}

impl BackupSummary {
    /// 五张表的行数合计（不含偏好/密钥/图片）
    #[must_use]
    pub fn rows(&self) -> i32 {
        self.formulas + self.history + self.favorites + self.versions + self.usage_stats
    }

    /// 从包内容统计（导出前展示与导入后核对都用它）
    #[must_use]
    pub fn of(content: &BackupContent) -> Self {
        Self {
            formulas: content.tables.formulas.len() as i32,
            history: content.tables.history.len() as i32,
            favorites: content.tables.favorites.len() as i32,
            versions: content.tables.versions.len() as i32,
            usage_stats: content.tables.usage_stats.len() as i32,
            preferences: (content.prefs.strings.len() + content.prefs.ints.len()) as i32,
            llm_config: content
                .llm_config_json
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty()),
            api_keys: content
                .secrets
                .as_ref()
                .map_or(0, |s| s.api_keys.len() as i32),
            images: content.images.len() as i32,
            background: content.has_background,
        }
    }

    /// 一行摘要：`公式 65 · 历史 12 · 图片 8 张`
    ///
    /// ⚠️ **`favorites` 不参与**（源同）—— 收藏在界面上是公式的一个标记，
    /// 单独列一行「收藏 3」会让用户以为那是一份独立数据。别「顺手补上」。
    #[must_use]
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.formulas > 0 {
            parts.push(format!("公式 {}", self.formulas));
        }
        if self.history > 0 {
            parts.push(format!("历史 {}", self.history));
        }
        if self.versions > 0 {
            parts.push(format!("版本 {}", self.versions));
        }
        if self.usage_stats > 0 {
            parts.push(format!("用量记录 {}", self.usage_stats));
        }
        if self.llm_config {
            parts.push("AI 模型配置".to_string());
        }
        if self.preferences > 0 {
            parts.push(format!("设置 {} 项", self.preferences));
        }
        if self.api_keys > 0 {
            parts.push(format!("密钥 {}", self.api_keys));
        }
        if self.images > 0 {
            parts.push(format!("图片 {} 张", self.images));
        }
        if self.background {
            parts.push("背景图".to_string());
        }
        if parts.is_empty() {
            "空备份".to_string()
        } else {
            parts.join(" · ")
        }
    }
}

#[cfg(test)]
mod summary_tests {
    use super::*;
    use std::collections::BTreeMap;

    /// 夹具：各类条数（用结构体而不是 11 个位置参数 —— 位置参数接反了看不出来）
    #[derive(Default)]
    struct Fixture {
        formulas: usize,
        history: usize,
        favorites: usize,
        versions: usize,
        usage: usize,
        strings: usize,
        ints: usize,
        llm_config: Option<String>,
        api_keys: usize,
        images: usize,
        background: bool,
    }

    fn content_with(f: Fixture) -> BackupContent {
        let Fixture {
            formulas,
            history,
            favorites,
            versions,
            usage,
            strings,
            ints,
            llm_config,
            api_keys,
            images,
            background,
        } = f;
        let mut c = BackupContent::default();
        c.tables.formulas = (0..formulas)
            .map(|i| FormulaRow {
                id: format!("usr:{i}"),
                schema_json: "{}".into(),
                favorite: 0,
                created_at: 0,
                updated_at: 0,
            })
            .collect();
        c.tables.history = (0..history)
            .map(|i| HistoryRow {
                id: i as i64,
                formula_id: "usr:1".into(),
                formula_snapshot_json: "{}".into(),
                inputs_json: "{}".into(),
                result_json: "{}".into(),
                thinking_content: None,
                created_at: 0,
            })
            .collect();
        c.tables.favorites = (0..favorites)
            .map(|i| FavoriteRow {
                formula_id: format!("usr:{i}"),
                created_at: 0,
            })
            .collect();
        c.tables.versions = (0..versions)
            .map(|i| FormulaVersionRow {
                formula_id: "usr:1".into(),
                version: format!("v{i}.0"),
                parent_version: None,
                schema_json: "{}".into(),
                change_type: "create".into(),
                change_log: String::new(),
                editor: "user".into(),
                created_at: 0,
                verified: 0,
            })
            .collect();
        c.tables.usage_stats = (0..usage)
            .map(|i| LlmUsageStatsRow {
                id: i as i64,
                model_label: "m".into(),
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                cached_tokens: 0,
                reasoning_tokens: 0,
                created_at: 0,
            })
            .collect();
        for i in 0..strings {
            c.prefs.strings.insert(format!("k{i}"), "v".into());
        }
        for i in 0..ints {
            c.prefs.ints.insert(format!("i{i}"), 1);
        }
        c.llm_config_json = llm_config;
        if api_keys > 0 {
            let mut keys = BTreeMap::new();
            for i in 0..api_keys {
                keys.insert(format!("p{i}"), "sk".into());
            }
            c.secrets = Some(BackupSecrets { api_keys: keys });
        }
        c.images = (0..images)
            .map(|i| BackupImageRecord {
                id: format!("img{i}"),
                file_name: format!("{i}.jpg"),
                mime_type: "image/jpeg".into(),
                refs: vec![],
                created_at: 0,
            })
            .collect();
        c.has_background = background;
        c
    }

    #[test]
    fn of_counts_every_category() {
        let c = content_with(Fixture {
            formulas: 2,
            history: 3,
            favorites: 1,
            versions: 4,
            usage: 5,
            strings: 6,
            ints: 7,
            llm_config: Some(r#"{"a":1}"#.to_string()),
            api_keys: 2,
            images: 8,
            background: true,
        });
        let s = BackupSummary::of(&c);
        assert_eq!(s.formulas, 2);
        assert_eq!(s.history, 3);
        assert_eq!(s.favorites, 1);
        assert_eq!(s.versions, 4);
        assert_eq!(s.usage_stats, 5);
        assert_eq!(s.preferences, 13, "strings + ints");
        assert!(s.llm_config);
        assert_eq!(s.api_keys, 2);
        assert_eq!(s.images, 8);
        assert!(s.background);
        assert_eq!(s.rows(), 2 + 3 + 1 + 4 + 5);
    }

    /// 空白 `llmConfigJson` **不算**有模型配置（源用 `isNullOrBlank`）
    #[test]
    fn blank_llm_config_is_not_counted() {
        for raw in [None, Some(String::new()), Some("   ".to_string())] {
            let c = content_with(Fixture {
                llm_config: raw.clone(),
                ..Default::default()
            });
            assert!(!BackupSummary::of(&c).llm_config, "实际: {raw:?}");
        }
        let c = content_with(Fixture {
            llm_config: Some("{}".to_string()),
            ..Default::default()
        });
        assert!(BackupSummary::of(&c).llm_config);
    }

    #[test]
    fn describe_matches_source_order() {
        let c = content_with(Fixture {
            formulas: 65,
            history: 12,
            favorites: 3,
            versions: 2,
            usage: 7,
            strings: 5,
            ints: 1,
            llm_config: Some("{}".to_string()),
            api_keys: 1,
            images: 8,
            background: true,
        });
        assert_eq!(
            BackupSummary::of(&c).describe(),
            "公式 65 · 历史 12 · 版本 2 · 用量记录 7 · AI 模型配置 · 设置 6 项 · 密钥 1 · 图片 8 张 · 背景图"
        );
    }

    /// ⚠️ 收藏不参与 `describe()`（源同）—— 它是公式的标记，不是独立数据
    #[test]
    fn describe_omits_favorites() {
        let s = BackupSummary {
            favorites: 9,
            ..Default::default()
        };
        assert_eq!(s.describe(), "空备份");
        assert_eq!(s.rows(), 9, "但 rows() 算它");
    }

    #[test]
    fn describe_empty_backup() {
        assert_eq!(BackupSummary::default().describe(), "空备份");
    }

    #[test]
    fn describe_single_category_has_no_separator() {
        let s = BackupSummary {
            formulas: 1,
            ..Default::default()
        };
        assert_eq!(s.describe(), "公式 1");
    }

    #[test]
    fn serde_is_camel_case() {
        let s = BackupSummary {
            usage_stats: 3,
            llm_config: true,
            ..Default::default()
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["usageStats"], serde_json::json!(3));
        assert_eq!(v["llmConfig"], serde_json::json!(true));
        assert!(v.get("usage_stats").is_none(), "不得泄漏 snake_case");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------
    // 枚举名必须 SCREAMING_SNAKE_CASE
    // ---------------------------------------------------------------------

    /// 🔴 `BackupSection` 序列化成 `"FORMULAS"` / `"LLM_CONFIG"`（进备份包，不能改）
    #[test]
    fn section_names_are_screaming_snake_case() {
        let cases = [
            (BackupSection::Formulas, "FORMULAS"),
            (BackupSection::History, "HISTORY"),
            (BackupSection::Preferences, "PREFERENCES"),
            (BackupSection::UsageStats, "USAGE_STATS"),
            (BackupSection::LlmConfig, "LLM_CONFIG"),
            (BackupSection::ApiKeys, "API_KEYS"),
            (BackupSection::Images, "IMAGES"),
            (BackupSection::Background, "BACKGROUND"),
        ];
        for (s, expect) in cases {
            assert_eq!(serde_json::to_value(s).unwrap(), serde_json::json!(expect));
            let back: BackupSection = serde_json::from_value(serde_json::json!(expect)).unwrap();
            assert_eq!(back, s);
        }
    }

    /// 反序列化必须是**严格**的：`"LlmConfig"` 这种驼峰名不该被接受
    /// （否则 Android 侧改了枚举名 PC 会静默收下）
    #[test]
    fn section_rejects_camel_case_name() {
        assert!(serde_json::from_value::<BackupSection>(serde_json::json!("LlmConfig")).is_err());
        assert!(serde_json::from_value::<BackupSection>(serde_json::json!("llm_config")).is_err());
    }

    // ---------------------------------------------------------------------
    // 字段名 camelCase
    // ---------------------------------------------------------------------

    #[test]
    fn manifest_serializes_camel_case() {
        let m = BackupManifest {
            sections: vec![BackupSection::Formulas],
            ..Default::default()
        };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["backupFormatVersion"], BACKUP_FORMAT_VERSION);
        assert_eq!(v["appVersionCode"], 0);
        assert_eq!(v["createdAt"], 0);
        assert_eq!(v["sections"][0], "FORMULAS");
        assert!(v.get("backup_format_version").is_none(), "不得漏成 snake_case");
    }

    #[test]
    fn rows_serialize_camel_case() {
        let r = FormulaRow {
            id: "f1".into(),
            schema_json: "{}".into(),
            favorite: 1,
            created_at: 10,
            updated_at: 20,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["schemaJson"], "{}");
        assert_eq!(v["favorite"], 1);
        assert_eq!(v["createdAt"], 10);
        assert_eq!(v["updatedAt"], 20);
    }

    #[test]
    fn version_row_verified_is_int_not_bool() {
        let r = FormulaVersionRow {
            formula_id: "f".into(),
            version: "1.0.0".into(),
            parent_version: None,
            schema_json: "{}".into(),
            change_type: "create".into(),
            change_log: String::new(),
            editor: "user".into(),
            created_at: 0,
            verified: 1,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["verified"], 1, "必须是 0/1 而不是 true/false");
        assert!(v["verified"].is_number());
        assert_eq!(v["parentVersion"], serde_json::Value::Null);
    }

    #[test]
    fn tables_serialize_camel_case() {
        let t = BackupTables::default();
        let v = serde_json::to_value(&t).unwrap();
        assert!(v.get("usageStats").is_some(), "usage_stats → usageStats");
        assert!(v.get("formulas").is_some());
        assert!(v.get("favorites").is_some());
        assert!(v.get("versions").is_some());
    }

    #[test]
    fn content_serializes_camel_case() {
        let c = BackupContent::default();
        let v = serde_json::to_value(&c).unwrap();
        assert!(v.get("llmConfigJson").is_some());
        assert!(v.get("hasBackground").is_some());
        assert!(v.get("images").is_some());
    }

    #[test]
    fn selection_serializes_camel_case() {
        let v = serde_json::to_value(BackupSelection::all()).unwrap();
        assert_eq!(v["includeApiKeys"], false);
        assert_eq!(v["includeFormulaImages"], true);
        assert_eq!(v["includeHistoryImages"], false);
        assert_eq!(v["includeUnreferencedImages"], false);
    }

    // ---------------------------------------------------------------------
    // 缺字段按默认（旧备份兼容）
    // ---------------------------------------------------------------------

    /// 🔴 空对象也能解析成默认值（旧备份缺整类数据）
    #[test]
    fn empty_json_parses_with_defaults() {
        let m: BackupManifest = serde_json::from_str("{}").unwrap();
        assert_eq!(m.backup_format_version, BACKUP_FORMAT_VERSION, "默认格式版本");
        assert!(m.sections.is_empty());

        let t: BackupTables = serde_json::from_str("{}").unwrap();
        assert!(t.formulas.is_empty());
        assert!(t.usage_stats.is_empty());

        let c: BackupContent = serde_json::from_str("{}").unwrap();
        assert_eq!(c, BackupContent::default());
    }

    /// 行记录里除主键外的字段可缺省
    #[test]
    fn row_optional_fields_default() {
        let r: FormulaRow = serde_json::from_str(r#"{"id":"a","schemaJson":"{}"}"#).unwrap();
        assert_eq!(r.favorite, 0);
        assert_eq!(r.created_at, 0);
        assert_eq!(r.updated_at, 0);

        let h: HistoryRow =
            serde_json::from_str(r#"{"id":1,"formulaId":"f","formulaSnapshotJson":"{}","inputsJson":"{}","resultJson":"{}"}"#)
                .unwrap();
        assert_eq!(h.thinking_content, None);
        assert_eq!(h.created_at, 0);

        let v: FormulaVersionRow =
            serde_json::from_str(r#"{"formulaId":"f","version":"1.0.0","schemaJson":"{}","changeType":"create","editor":"u"}"#)
                .unwrap();
        assert_eq!(v.change_log, "");
        assert_eq!(v.verified, 0);
        assert_eq!(v.parent_version, None);
    }

    /// 图片记录的 mimeType 默认是 `image/jpeg`（源默认值）
    #[test]
    fn image_record_default_mime() {
        let r: BackupImageRecord =
            serde_json::from_str(r#"{"id":"i","fileName":"abc.jpg"}"#).unwrap();
        assert_eq!(r.mime_type, "image/jpeg");
        assert!(r.refs.is_empty());
    }

    // ---------------------------------------------------------------------
    // 默认值 / 往返
    // ---------------------------------------------------------------------

    /// `BackupSelection::all()` 的 9 个开关与源逐字一致
    #[test]
    fn selection_all_matches_source() {
        let s = BackupSelection::all();
        assert!(s.formulas && s.history && s.preferences && s.usage_stats && s.llm_config);
        assert!(!s.include_api_keys, "密钥默认不勾");
        assert!(s.include_formula_images, "公式引用的图默认勾");
        assert!(!s.include_history_images, "历史引用的图默认不勾");
        assert!(!s.include_unreferenced_images, "未被引用的图默认不勾");
    }

    /// 完整往返（备份包读写的基本保证）
    #[test]
    fn full_content_roundtrip() {
        let mut c = BackupContent::default();
        c.manifest.sections = vec![BackupSection::Formulas, BackupSection::Images];
        c.manifest.counts.insert("formulas".into(), 3);
        c.tables.formulas.push(FormulaRow {
            id: "f1".into(),
            schema_json: r#"{"id":"f1"}"#.into(),
            favorite: 1,
            created_at: 1,
            updated_at: 2,
        });
        c.prefs.strings.insert("theme".into(), "dark".into());
        c.prefs.ints.insert("fontSize".into(), 14);
        c.llm_config_json = Some(r#"{"active":"a","profiles":[]}"#.into());
        c.secrets = Some(BackupSecrets {
            api_keys: BTreeMap::from([("a".to_string(), "sk-x".to_string())]),
        });
        c.images.push(BackupImageRecord {
            id: "i1".into(),
            file_name: "deadbeef.png".into(),
            mime_type: "image/png".into(),
            refs: vec!["formula:f1".into()],
            created_at: 5,
        });
        c.has_background = true;

        let json = serde_json::to_string(&c).unwrap();
        let back: BackupContent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }

    /// `llmConfigJson` 是**原样字符串**（不解析成结构体，避免将来加字段丢配置）
    #[test]
    fn llm_config_json_is_opaque() {
        let raw = r#"{"active":"a","profiles":[{"id":"a","thinkingLevel":"MAX"}]}"#;
        let c = BackupContent {
            llm_config_json: Some(raw.into()),
            ..Default::default()
        };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["llmConfigJson"], raw, "必须原样搬运");
        assert!(v["llmConfigJson"].is_string(), "是字符串不是对象");
    }

    /// `hasBackground` 写出时**不参与决策**（以 `background_bytes` 参数为准），
    /// 但字段本身要能往返
    #[test]
    fn has_background_roundtrips() {
        for flag in [true, false] {
            let c = BackupContent {
                has_background: flag,
                ..Default::default()
            };
            let back: BackupContent =
                serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
            assert_eq!(back.has_background, flag);
        }
    }
}
