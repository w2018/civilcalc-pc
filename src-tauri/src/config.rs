//! 应用偏好（替代源项目 `data/prefs/AppPreferences.kt`）。
//!
//! 源项目用 AndroidX DataStore（`civilcalc_prefs`），PC 端存
//! `%APPDATA%\com.w2018.civilcalc.pc\config.json`。
//!
//! ## 为什么是「两个扁平 map」而不是一堆字段
//!
//! 备份包的 `BackupPrefs` 形状就是：
//!
//! ```kotlin
//! data class BackupPrefs(
//!     val strings: Map<String, String> = emptyMap(),
//!     val ints: Map<String, Int> = emptyMap(),
//! )
//! ```
//!
//! 用同样的扁平结构存，**导入导出就是原样搬运**，不需要逐字段映射 ——
//! 新增偏好键时也不会漏改备份逻辑（ADR-009 的"两边各自演进"）。
//! 类型安全由下面的访问器提供。
//!
//! ## ⚠️ 不进备份的键
//!
//! 源项目 `NON_BACKUP_KEYS = {"webdav_last_backup_at", "webdav_backup_count"}`：
//! 这是"本机备份过几次"的本地状态，换机后应从 0 重新开始。
//! 见 [`AppConfig::backup_strings`] / [`AppConfig::backup_ints`]。
//!
//! ## 本模块**不依赖 `tauri`**
//!
//! 只依赖 `std::fs` + serde，因此可用临时目录直接单测（不必等 Tauri 编译）。

use civilcalc_core::CoreError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// =============================================================================
// 偏好键（逐字对齐源项目 AppPreferences.companion 的 key 字符串）
// =============================================================================

/// 主题模式
pub const KEY_THEME: &str = "theme_mode";
/// 用户可改的 Prompt A
pub const KEY_PROMPT_A: &str = "prompt_a";
/// 默认提醒词（含计算书免责声明）
pub const KEY_DEFAULT_REMINDER: &str = "default_reminder";
/// 搜索框草稿
pub const KEY_SEARCH_QUERY: &str = "search_query";
/// 模型测试输入草稿
pub const KEY_MODEL_TEST_INPUT: &str = "model_test_input";
/// 各公式的参数草稿（按 formulaId 的 JSON）
pub const KEY_DRAFTS: &str = "formula_drafts";
/// 是否设置了自定义背景图（`"1"` / 缺省）
pub const KEY_BACKGROUND: &str = "background_image";
/// 背景图透明度百分比
pub const KEY_BG_TRANSPARENCY: &str = "background_transparency";
/// 文字颜色（`AUTO` 或具体色值）
pub const KEY_TEXT_COLOR: &str = "text_color";
/// 自定义页面底色（`#RRGGBB`；缺省 = 用主题默认底色）
///
/// 🔴 这个键是**后补的**。此前 `stores/ui.ts` 的「页面底色」只改了内存与 DOM，
/// 从不落盘 —— 表现为：设了底色，重启就没了；「重置软件」也回不到默认
/// （DOM 上的 `data-bg-contrast` 一直是旧值，文字色看起来"重置失败"）。
pub const KEY_BG_COLOR: &str = "background_color";
/// 提示词区块可见性
pub const KEY_SHOW_PROMPTS: &str = "show_prompts_section";
/// 搜索输入历史（JSON）
pub const KEY_SEARCH_HISTORY: &str = "search_input_history";
/// 微调输入历史（JSON）
pub const KEY_REFINE_HISTORY: &str = "refine_input_history";
/// 模型测试输入历史（JSON）
pub const KEY_MODEL_TEST_HISTORY: &str = "model_test_input_history";
/// 模型测试对话（JSON）
pub const KEY_MODEL_TEST_CONVERSATION: &str = "model_test_conversation";
/// 模型测试上下文容量（tokens）
pub const KEY_TEST_CONTEXT_SIZE: &str = "model_test_context_size";
/// 自动压缩阈值（百分比）
pub const KEY_TEST_COMPRESS_THRESHOLD: &str = "model_test_compress_threshold";
/// 压缩提示开关（0/1）
pub const KEY_TEST_COMPRESS_NOTICE: &str = "model_test_compress_notice";
/// 模型测试独立系统提示词
pub const KEY_TEST_SYSTEM_PROMPT: &str = "model_test_system_prompt";
/// 生成公式时是否一并输出「详解」（0/1）
pub const KEY_GENERATE_EXPLANATION: &str = "generate_explanation";
/// 计算书导出选项（JSON，取代原「模板」）
pub const KEY_EXPORT_OPTIONS: &str = "export_options";
/// 上次 WebDAV 备份时间（Unix 毫秒）——**不进备份**
pub const KEY_WEBDAV_LAST_BACKUP: &str = "webdav_last_backup_at";
/// 累计 WebDAV 备份次数——**不进备份**
pub const KEY_WEBDAV_BACKUP_COUNT: &str = "webdav_backup_count";

/// 不进备份包的键（源项目 `NON_BACKUP_KEYS`，逐字对齐）
pub const NON_BACKUP_KEYS: &[&str] = &[KEY_WEBDAV_LAST_BACKUP, KEY_WEBDAV_BACKUP_COUNT];

// ---- 默认值（逐字对齐源项目常量） ----

/// 主题：跟随系统
pub const THEME_SYSTEM: &str = "SYSTEM";
/// 主题：亮色
pub const THEME_LIGHT: &str = "LIGHT";
/// 主题：暗色
pub const THEME_DARK: &str = "DARK";

/// 文字颜色：自动（随主题）
pub const TEXT_COLOR_AUTO: &str = "AUTO";

/// 背景透明度默认值（源项目 `DEFAULT_BG_TRANSPARENCY = 22`）
pub const DEFAULT_BG_TRANSPARENCY: i64 = 22;
/// 模型测试上下文容量默认值（源项目 `DEFAULT_MODEL_TEST_CONTEXT_SIZE = 200_000`）
pub const DEFAULT_TEST_CONTEXT_SIZE: i64 = 200_000;
/// 自动压缩阈值默认值（源项目 `DEFAULT_MODEL_TEST_COMPRESS_THRESHOLD = 80`）
pub const DEFAULT_TEST_COMPRESS_THRESHOLD: i64 = 80;

/// 主题模式（`theme_mode` 的取值）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

impl ThemeMode {
    /// 存储值（与源项目 `THEME_*` 常量一致）
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeMode::System => THEME_SYSTEM,
            ThemeMode::Light => THEME_LIGHT,
            ThemeMode::Dark => THEME_DARK,
        }
    }

    /// 解析存储值。**未知值回落到 `System`**（源项目 `?: THEME_SYSTEM` 的行为），
    /// 不报错 —— 偏好文件被外部改坏时也要能启动。
    pub fn parse(s: &str) -> Self {
        match s {
            THEME_LIGHT => ThemeMode::Light,
            THEME_DARK => ThemeMode::Dark,
            _ => ThemeMode::System,
        }
    }
}

// =============================================================================
// AppConfig
// =============================================================================

/// 偏好的**区块**划分（供 `config_reset_section` 用）。
///
/// 每个区块拥有若干键；重置某区块 = 删掉这些键（下次读取即回落默认值）。
/// **删键而不是写默认值** —— 这样"用户从未设置过"与"设置成默认值"可区分。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConfigSection {
    /// 外观：主题 / 背景图 / 透明度 / 文字色
    Appearance,
    /// 提示词：Prompt A / 默认提醒词 / 区块可见性 / 是否生成详解
    Prompts,
    /// 导出选项
    Export,
    /// 搜索：草稿与输入历史
    Search,
    /// 公式参数草稿
    Drafts,
    /// 模型测试：草稿 / 历史 / 对话 / 上下文设置 / 独立系统提示词
    ModelTest,
    /// 微调输入历史
    Refine,
    /// WebDAV 备份统计（本机状态）
    ///
    /// 🔴 **必须显式 `rename`**：`SCREAMING_SNAKE_CASE` 会把 `WebDavStats`
    /// 拆成 `WEB_DAV_STATS`，而手写的 [`ConfigSection::as_str`] /
    /// [`ConfigSection::parse`] 用的是 `WEBDAV_STATS`（前端 `types/system.ts`
    /// 也写 `WEBDAV_STATS`）。不写这一行就会出现「serde 与 as_str 不一致」——
    /// 命令层因为走 `parse()` 而侥幸能用，但任何把它序列化的地方都会错。
    #[serde(rename = "WEBDAV_STATS")]
    WebDavStats,
}

impl ConfigSection {
    /// 解析区块名。**未知值返回 `None`**（由命令层转成 `InvalidArgument`，
    /// 不要静默当成某个区块 —— 那样会删错东西）。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "APPEARANCE" => Some(ConfigSection::Appearance),
            "PROMPTS" => Some(ConfigSection::Prompts),
            "EXPORT" => Some(ConfigSection::Export),
            "SEARCH" => Some(ConfigSection::Search),
            "DRAFTS" => Some(ConfigSection::Drafts),
            "MODEL_TEST" => Some(ConfigSection::ModelTest),
            "REFINE" => Some(ConfigSection::Refine),
            "WEBDAV_STATS" => Some(ConfigSection::WebDavStats),
            _ => None,
        }
    }

    /// 线上表示（与 serde 一致）
    pub fn as_str(self) -> &'static str {
        match self {
            ConfigSection::Appearance => "APPEARANCE",
            ConfigSection::Prompts => "PROMPTS",
            ConfigSection::Export => "EXPORT",
            ConfigSection::Search => "SEARCH",
            ConfigSection::Drafts => "DRAFTS",
            ConfigSection::ModelTest => "MODEL_TEST",
            ConfigSection::Refine => "REFINE",
            ConfigSection::WebDavStats => "WEBDAV_STATS",
        }
    }

    /// 该区块拥有的键
    pub fn keys(self) -> &'static [&'static str] {
        match self {
            ConfigSection::Appearance => &[
                KEY_THEME,
                KEY_BACKGROUND,
                KEY_BG_TRANSPARENCY,
                KEY_TEXT_COLOR,
                KEY_BG_COLOR,
            ],
            ConfigSection::Prompts => &[
                KEY_PROMPT_A,
                KEY_DEFAULT_REMINDER,
                KEY_SHOW_PROMPTS,
                KEY_GENERATE_EXPLANATION,
            ],
            ConfigSection::Export => &[KEY_EXPORT_OPTIONS],
            ConfigSection::Search => &[KEY_SEARCH_QUERY, KEY_SEARCH_HISTORY],
            ConfigSection::Drafts => &[KEY_DRAFTS],
            ConfigSection::ModelTest => &[
                KEY_MODEL_TEST_INPUT,
                KEY_MODEL_TEST_HISTORY,
                KEY_MODEL_TEST_CONVERSATION,
                KEY_TEST_CONTEXT_SIZE,
                KEY_TEST_COMPRESS_THRESHOLD,
                KEY_TEST_COMPRESS_NOTICE,
                KEY_TEST_SYSTEM_PROMPT,
            ],
            ConfigSection::Refine => &[KEY_REFINE_HISTORY],
            ConfigSection::WebDavStats => &[KEY_WEBDAV_LAST_BACKUP, KEY_WEBDAV_BACKUP_COUNT],
        }
    }

    /// 全部区块（供"重置软件"列出）
    pub fn all() -> &'static [ConfigSection] {
        &[
            ConfigSection::Appearance,
            ConfigSection::Prompts,
            ConfigSection::Export,
            ConfigSection::Search,
            ConfigSection::Drafts,
            ConfigSection::ModelTest,
            ConfigSection::Refine,
            ConfigSection::WebDavStats,
        ]
    }
}

/// 落盘结构（与备份包 `BackupPrefs` 同形）。
///
/// **对前端也直接暴露**：`config_get` / `config_save` 用的就是这个形状。
/// 让前端拿到与备份包一致的结构，避免在 Rust 与 TS 两侧各维护一份 22 字段的映射。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigSnapshot {
    #[serde(default)]
    pub strings: BTreeMap<String, String>,
    #[serde(default)]
    pub ints: BTreeMap<String, i64>,
}

/// 应用偏好。
///
/// 用 `BTreeMap` 而非 `HashMap`：落盘时键有序，**文件可 diff**，
/// 备份包对比也稳定。
#[derive(Debug, Clone)]
pub struct AppConfig {
    path: PathBuf,
    data: ConfigSnapshot,
}

impl AppConfig {
    // ---------------------------------------------------------------- 载入/保存

    /// 从文件载入。**文件不存在 → 全默认值**（首次启动）。
    ///
    /// 文件损坏（JSON 坏）→ 返回 `Err`，由调用方决定（启动时应记日志后走默认值，
    /// 而不是拒绝启动 —— 偏好丢失不该让用户打不开应用）。
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, CoreError> {
        let path = path.into();
        if !path.exists() {
            return Ok(Self {
                path,
                data: ConfigSnapshot::default(),
            });
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| CoreError::Storage { message: format!("读取偏好文件失败: {e}") })?;
        let data: ConfigSnapshot = serde_json::from_str(&text)
            .map_err(|e| CoreError::Parse { message: format!("解析偏好文件失败: {e}") })?;
        Ok(Self { path, data })
    }

    /// 内存态（测试用，不落盘）
    pub fn in_memory() -> Self {
        Self {
            path: PathBuf::new(),
            data: ConfigSnapshot::default(),
        }
    }

    /// 原子保存：先写 `config.json.tmp` 再 rename。
    ///
    /// **不直接覆盖原文件** —— 写到一半断电会留下半个文件，用户的偏好就没了。
    pub fn save(&self) -> Result<(), CoreError> {
        if self.path.as_os_str().is_empty() {
            return Ok(()); // 内存态
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::Storage { message: format!("创建配置目录失败: {e}") })?;
        }
        let text = serde_json::to_string_pretty(&self.data)
            .map_err(|e| CoreError::Parse { message: format!("序列化偏好失败: {e}") })?;

        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text.as_bytes())
            .map_err(|e| CoreError::Storage { message: format!("写入偏好临时文件失败: {e}") })?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| CoreError::Storage { message: format!("替换偏好文件失败: {e}") })?;
        Ok(())
    }

    /// 偏好文件路径
    pub fn path(&self) -> &Path {
        &self.path
    }

    // ---------------------------------------------------------------- 通用读写

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.data.strings.get(key).map(String::as_str)
    }

    /// 空串视为**未设置**（对齐源项目 `.orEmpty()` 后再判空的用法）
    pub fn get_str_non_empty(&self, key: &str) -> Option<&str> {
        self.get_str(key).filter(|s| !s.is_empty())
    }

    pub fn set_str(&mut self, key: &str, value: impl Into<String>) {
        self.data.strings.insert(key.to_string(), value.into());
    }

    pub fn get_int(&self, key: &str) -> Option<i64> {
        self.data.ints.get(key).copied()
    }

    /// 取整数并**夹紧到 `[min, max]`**；缺省时用 `default`
    pub fn get_int_clamped(&self, key: &str, default: i64, min: i64, max: i64) -> i64 {
        self.get_int(key).unwrap_or(default).clamp(min, max)
    }

    pub fn set_int(&mut self, key: &str, value: i64) {
        self.data.ints.insert(key.to_string(), value);
    }

    pub fn get_bool(&self, key: &str) -> bool {
        self.get_int(key).unwrap_or(0) != 0
    }

    pub fn set_bool(&mut self, key: &str, value: bool) {
        self.set_int(key, if value { 1 } else { 0 });
    }

    /// 删除一个键（清空某类草稿时用）
    pub fn remove(&mut self, key: &str) {
        self.data.strings.remove(key);
        self.data.ints.remove(key);
    }

    /// 所有键名（排障用）
    pub fn keys(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self
            .data
            .strings
            .keys()
            .chain(self.data.ints.keys())
            .map(String::as_str)
            .collect();
        v.sort_unstable();
        v
    }

    // ---------------------------------------------------------------- 类型化访问器

    /// 主题模式（缺省 `SYSTEM`）
    pub fn theme_mode(&self) -> ThemeMode {
        ThemeMode::parse(self.get_str(KEY_THEME).unwrap_or(THEME_SYSTEM))
    }

    pub fn set_theme_mode(&mut self, mode: ThemeMode) {
        self.set_str(KEY_THEME, mode.as_str());
    }

    /// 背景透明度百分比，夹紧到 `0..=100`
    pub fn background_transparency(&self) -> i64 {
        self.get_int_clamped(KEY_BG_TRANSPARENCY, DEFAULT_BG_TRANSPARENCY, 0, 100)
    }

    pub fn set_background_transparency(&mut self, percent: i64) {
        self.set_int(KEY_BG_TRANSPARENCY, percent.clamp(0, 100));
    }

    /// 是否设置了自定义背景图（源项目：`KEY_BACKGROUND == "1"`）
    pub fn has_background_image(&self) -> bool {
        self.get_str(KEY_BACKGROUND) == Some("1")
    }

    pub fn set_has_background_image(&mut self, yes: bool) {
        if yes {
            self.set_str(KEY_BACKGROUND, "1");
        } else {
            self.remove(KEY_BACKGROUND);
        }
    }

    /// 文字颜色：`AUTO` 或具体色值
    pub fn text_color(&self) -> &str {
        self.get_str(KEY_TEXT_COLOR).unwrap_or(TEXT_COLOR_AUTO)
    }

    pub fn set_text_color(&mut self, color: impl Into<String>) {
        self.set_str(KEY_TEXT_COLOR, color);
    }

    /// 自定义页面底色（`#RRGGBB`）。未设置 / 空白 → `None`（用主题默认底色）
    pub fn background_color(&self) -> Option<&str> {
        self.get_str(KEY_BG_COLOR).filter(|s| !s.trim().is_empty())
    }

    /// 设置自定义页面底色。传 `None` = 删掉这个键（回到主题默认）
    ///
    /// **删键而不是写默认值** —— 与其它区块一致，
    /// 这样「用户从未设置过」与「设置成默认色」可区分。
    pub fn set_background_color(&mut self, color: Option<&str>) {
        match color.map(str::trim).filter(|s| !s.is_empty()) {
            Some(c) => self.set_str(KEY_BG_COLOR, c),
            None => self.remove(KEY_BG_COLOR),
        }
    }

    /// 模型测试上下文容量（tokens）
    pub fn test_context_size(&self) -> i64 {
        self.get_int_clamped(KEY_TEST_CONTEXT_SIZE, DEFAULT_TEST_CONTEXT_SIZE, 1, 10_000_000)
    }

    /// 自动压缩阈值（百分比）
    pub fn test_compress_threshold(&self) -> i64 {
        self.get_int_clamped(KEY_TEST_COMPRESS_THRESHOLD, DEFAULT_TEST_COMPRESS_THRESHOLD, 1, 100)
    }

    /// 是否生成公式详解（默认**开**）
    pub fn generate_explanation(&self) -> bool {
        self.get_int(KEY_GENERATE_EXPLANATION).unwrap_or(1) != 0
    }

    pub fn set_generate_explanation(&mut self, yes: bool) {
        self.set_bool(KEY_GENERATE_EXPLANATION, yes);
    }

    /// 上次 WebDAV 备份时间（Unix 毫秒）
    pub fn webdav_last_backup_at(&self) -> Option<i64> {
        self.get_int(KEY_WEBDAV_LAST_BACKUP)
    }

    pub fn set_webdav_last_backup_at(&mut self, ms: i64) {
        self.set_int(KEY_WEBDAV_LAST_BACKUP, ms);
    }

    /// 累计 WebDAV 备份次数
    pub fn webdav_backup_count(&self) -> i64 {
        self.get_int(KEY_WEBDAV_BACKUP_COUNT).unwrap_or(0)
    }

    pub fn bump_webdav_backup_count(&mut self) {
        let n = self.webdav_backup_count() + 1;
        self.set_int(KEY_WEBDAV_BACKUP_COUNT, n);
    }

    // ---------------------------------------------------------------- 备份导出/导入

    /// 导出为备份用的字符串 map（**剔除 `NON_BACKUP_KEYS`**）
    pub fn backup_strings(&self) -> BTreeMap<String, String> {
        self.data
            .strings
            .iter()
            .filter(|(k, _)| !NON_BACKUP_KEYS.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// 导出为备份用的整数 map（**剔除 `NON_BACKUP_KEYS`**）
    pub fn backup_ints(&self) -> BTreeMap<String, i64> {
        self.data
            .ints
            .iter()
            .filter(|(k, _)| !NON_BACKUP_KEYS.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), *v))
            .collect()
    }

    /// 用备份包里的偏好**覆盖**本机偏好。
    ///
    /// ## 口径：合并覆盖（ADR-021）
    ///
    /// - 包里有、本机也有 → **用包里的**（覆盖）
    /// - 包里有、本机没有 → 新增
    /// - 本机有、包里没有 → **保留本机值**
    /// - `NON_BACKUP_KEYS` → **永不写入**（哪怕包里带了）
    ///
    /// 返回实际写入的键数，供确认弹窗展示。
    pub fn apply_backup(
        &mut self,
        strings: &BTreeMap<String, String>,
        ints: &BTreeMap<String, i64>,
    ) -> usize {
        let mut n = 0;
        for (k, v) in strings {
            if NON_BACKUP_KEYS.contains(&k.as_str()) {
                continue;
            }
            self.data.strings.insert(k.clone(), v.clone());
            n += 1;
        }
        for (k, v) in ints {
            if NON_BACKUP_KEYS.contains(&k.as_str()) {
                continue;
            }
            self.data.ints.insert(k.clone(), *v);
            n += 1;
        }
        n
    }

    // ---------------------------------------------------------------- 快照（IPC）

    /// 导出全量快照（**含 `NON_BACKUP_KEYS`** —— 这是"本机全量"，不是备份）
    pub fn snapshot(&self) -> ConfigSnapshot {
        self.data.clone()
    }

    /// 用快照**整体替换**（`config_save` 用）。
    ///
    /// 与 [`AppConfig::apply_backup`] 的区别：这里是**替换**，
    /// 快照里没有的键会被清掉；`apply_backup` 是合并覆盖。
    pub fn replace_with(&mut self, snap: ConfigSnapshot) {
        self.data = snap;
    }

    /// 按区块重置（`config_reset_section` 用）。
    ///
    /// 返回被删掉的键数。
    pub fn reset_section(&mut self, section: ConfigSection) -> usize {
        let keys = section.keys();
        let mut n = 0;
        for k in keys {
            let had = self.data.strings.remove(*k).is_some() | self.data.ints.remove(*k).is_some();
            if had {
                n += 1;
            }
        }
        n
    }

    /// 清空所有偏好（"重置软件"用）。**返回是否原本就非空**。
    pub fn clear(&mut self) -> bool {
        let had = !self.data.strings.is_empty() || !self.data.ints.is_empty();
        self.data = ConfigSnapshot::default();
        had
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("civilcalc-cfg-test-{name}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    // ---------------- 默认值 ----------------

    #[test]
    fn defaults_match_source_constants() {
        let c = AppConfig::in_memory();
        assert_eq!(c.theme_mode(), ThemeMode::System);
        assert_eq!(c.background_transparency(), 22, "源项目默认 22");
        assert_eq!(c.text_color(), "AUTO");
        assert_eq!(c.test_context_size(), 200_000, "源项目默认 200k");
        assert_eq!(c.test_compress_threshold(), 80, "源项目默认 80%");
        assert!(c.generate_explanation(), "默认开启详解");
        assert!(!c.has_background_image());
        assert_eq!(c.webdav_backup_count(), 0);
        assert_eq!(c.webdav_last_backup_at(), None);
    }

    #[test]
    fn theme_mode_parse_falls_back_to_system() {
        assert_eq!(ThemeMode::parse("LIGHT"), ThemeMode::Light);
        assert_eq!(ThemeMode::parse("DARK"), ThemeMode::Dark);
        assert_eq!(ThemeMode::parse("SYSTEM"), ThemeMode::System);
        // 未知值不报错，回落 SYSTEM（源项目 `?: THEME_SYSTEM`）
        assert_eq!(ThemeMode::parse("NEON"), ThemeMode::System);
        assert_eq!(ThemeMode::parse(""), ThemeMode::System);
    }

    /// `ConfigSection` 的 **serde 表示必须与 `as_str()` / `parse()` 一致**。
    ///
    /// 曾经不一致：`SCREAMING_SNAKE_CASE` 把 `WebDavStats` 拆成 `WEB_DAV_STATS`，
    /// 而手写的是 `WEBDAV_STATS`。命令层走 `parse()` 所以侥幸能用，
    /// 但任何序列化它的地方都会输出前端不认的字符串。
    /// 这个测试把三者钉在一起（前端侧还有 `tests/contract.rs` 做跨语言比对）。
    #[test]
    fn config_section_serde_matches_as_str() {
        for sec in ConfigSection::all() {
            let json = serde_json::to_value(sec).expect("应能序列化");
            let s = json.as_str().expect("应是字符串");
            assert_eq!(s, sec.as_str(), "{sec:?} 的 serde 表示与 as_str 不一致");
            assert_eq!(
                ConfigSection::parse(s),
                Some(*sec),
                "as_str 产出的 `{s}` 必须能被 parse 回来"
            );
        }
    }

    /// 前端 `types/system.ts` 的取值必须能被 `parse()` 认下（防止再写成 `WEB_DAV_STATS`）。
    #[test]
    fn config_section_parse_accepts_webdav_stats() {
        assert_eq!(
            ConfigSection::parse("WEBDAV_STATS"),
            Some(ConfigSection::WebDavStats)
        );
        assert_eq!(ConfigSection::parse("webdav_stats"), Some(ConfigSection::WebDavStats));
        // 下划线拆开的那种写法**不该**被认下（它从来没被前端用过）
        assert_eq!(ConfigSection::parse("WEB_DAV_STATS"), None);
    }

    #[test]
    fn theme_mode_roundtrip_via_config() {
        let mut c = AppConfig::in_memory();
        for m in [ThemeMode::Light, ThemeMode::Dark, ThemeMode::System] {
            c.set_theme_mode(m);
            assert_eq!(c.theme_mode(), m);
            assert_eq!(c.get_str(KEY_THEME), Some(m.as_str()));
        }
    }

    // ---------------- 夹紧 ----------------

    #[test]
    fn transparency_is_clamped_to_0_100() {
        let mut c = AppConfig::in_memory();
        c.set_int(KEY_BG_TRANSPARENCY, 999);
        assert_eq!(c.background_transparency(), 100);
        c.set_int(KEY_BG_TRANSPARENCY, -5);
        assert_eq!(c.background_transparency(), 0);
        c.set_int(KEY_BG_TRANSPARENCY, 55);
        assert_eq!(c.background_transparency(), 55);
    }

    #[test]
    fn compress_threshold_is_clamped() {
        let mut c = AppConfig::in_memory();
        c.set_int(KEY_TEST_COMPRESS_THRESHOLD, 0);
        assert_eq!(c.test_compress_threshold(), 1);
        c.set_int(KEY_TEST_COMPRESS_THRESHOLD, 500);
        assert_eq!(c.test_compress_threshold(), 100);
    }

    // ---------------- 落盘 ----------------

    #[test]
    fn save_then_load_roundtrip() {
        let dir = tmp_dir("roundtrip");
        let path = dir.join("config.json");

        let mut c = AppConfig::load(&path).unwrap();
        c.set_theme_mode(ThemeMode::Dark);
        c.set_str(KEY_PROMPT_A, "自定义提示词");
        c.set_int(KEY_BG_TRANSPARENCY, 42);
        c.set_bool(KEY_GENERATE_EXPLANATION, false);
        c.save().unwrap();

        let back = AppConfig::load(&path).unwrap();
        assert_eq!(back.theme_mode(), ThemeMode::Dark);
        assert_eq!(back.get_str(KEY_PROMPT_A), Some("自定义提示词"));
        assert_eq!(back.background_transparency(), 42);
        assert!(!back.generate_explanation());
    }

    #[test]
    fn missing_file_yields_defaults() {
        let dir = tmp_dir("missing");
        let c = AppConfig::load(dir.join("nope.json")).unwrap();
        assert_eq!(c.theme_mode(), ThemeMode::System);
        assert!(c.keys().is_empty());
    }

    #[test]
    fn corrupt_file_reports_parse_error() {
        let dir = tmp_dir("corrupt");
        let path = dir.join("config.json");
        std::fs::write(&path, "{ 这不是 JSON").unwrap();

        let err = AppConfig::load(&path).unwrap_err();
        assert_eq!(err.code(), "PARSE_ERROR");
    }

    /// 原子保存：临时文件不残留
    #[test]
    fn save_is_atomic_and_leaves_no_tmp() {
        let dir = tmp_dir("atomic");
        let path = dir.join("config.json");

        let mut c = AppConfig::load(&path).unwrap();
        c.set_theme_mode(ThemeMode::Light);
        c.save().unwrap();

        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists(), "临时文件应已改名");
    }

    #[test]
    fn save_creates_parent_dir() {
        let dir = tmp_dir("mkdir");
        let path = dir.join("nested").join("deep").join("config.json");
        let mut c = AppConfig::load(&path).unwrap();
        c.set_theme_mode(ThemeMode::Dark);
        c.save().unwrap();
        assert!(path.exists());
    }

    // ---------------- 备份导出/导入 ----------------

    #[test]
    fn backup_excludes_non_backup_keys() {
        let mut c = AppConfig::in_memory();
        c.set_str(KEY_PROMPT_A, "P");
        c.set_int(KEY_BG_TRANSPARENCY, 30);
        c.set_int(KEY_WEBDAV_BACKUP_COUNT, 7);
        c.set_int(KEY_WEBDAV_LAST_BACKUP, 1234567890);

        let s = c.backup_strings();
        let i = c.backup_ints();

        assert!(s.contains_key(KEY_PROMPT_A));
        assert!(i.contains_key(KEY_BG_TRANSPARENCY));
        assert!(
            !i.contains_key(KEY_WEBDAV_BACKUP_COUNT),
            "备份次数是本机状态，不进备份"
        );
        assert!(
            !i.contains_key(KEY_WEBDAV_LAST_BACKUP),
            "上次备份时间是本机状态，不进备份"
        );
    }

    #[test]
    fn apply_backup_overwrites_and_adds() {
        let mut c = AppConfig::in_memory();
        c.set_str(KEY_PROMPT_A, "本机旧值");
        c.set_str("only_local", "保留我");

        let mut s = BTreeMap::new();
        s.insert(KEY_PROMPT_A.to_string(), "包里新值".to_string());
        s.insert("only_backup".to_string(), "新增".to_string());
        let mut i = BTreeMap::new();
        i.insert(KEY_BG_TRANSPARENCY.to_string(), 66i64);

        let n = c.apply_backup(&s, &i);
        assert_eq!(n, 3);

        assert_eq!(c.get_str(KEY_PROMPT_A), Some("包里新值"), "覆盖");
        assert_eq!(c.get_str("only_backup"), Some("新增"), "新增");
        assert_eq!(c.get_str("only_local"), Some("保留我"), "本机独有键保留");
        assert_eq!(c.background_transparency(), 66);
    }

    /// **红线**：哪怕备份包里带了 `NON_BACKUP_KEYS`，也不得写入
    #[test]
    fn apply_backup_never_writes_non_backup_keys() {
        let mut c = AppConfig::in_memory();

        let mut s = BTreeMap::new();
        s.insert(KEY_WEBDAV_LAST_BACKUP.to_string(), "999".to_string());
        let mut i = BTreeMap::new();
        i.insert(KEY_WEBDAV_BACKUP_COUNT.to_string(), 999i64);

        let n = c.apply_backup(&s, &i);
        assert_eq!(n, 0, "NON_BACKUP_KEYS 一个都不该写入");
        assert_eq!(c.webdav_backup_count(), 0);
        assert_eq!(c.webdav_last_backup_at(), None);
    }

    // ---------------- 其它 ----------------

    #[test]
    fn keys_are_sorted_and_complete() {
        let mut c = AppConfig::in_memory();
        c.set_str(KEY_PROMPT_A, "p");
        c.set_int(KEY_BG_TRANSPARENCY, 1);
        c.set_str(KEY_THEME, THEME_DARK);

        let k = c.keys();
        assert_eq!(k, vec![KEY_BG_TRANSPARENCY, KEY_PROMPT_A, KEY_THEME]);
    }

    #[test]
    fn remove_deletes_from_both_maps() {
        let mut c = AppConfig::in_memory();
        c.set_str("k", "v");
        assert!(c.get_str("k").is_some());
        c.remove("k");
        assert!(c.get_str("k").is_none());

        c.set_int("n", 1);
        assert!(c.get_int("n").is_some());
        c.remove("n");
        assert!(c.get_int("n").is_none());
    }

    #[test]
    fn clear_reports_whether_it_was_non_empty() {
        let mut c = AppConfig::in_memory();
        assert!(!c.clear(), "本来就空 → false");
        c.set_theme_mode(ThemeMode::Dark);
        assert!(c.clear(), "原本非空 → true");
        assert!(c.keys().is_empty());
    }

    #[test]
    fn webdav_backup_count_increments() {
        let mut c = AppConfig::in_memory();
        assert_eq!(c.webdav_backup_count(), 0);
        c.bump_webdav_backup_count();
        c.bump_webdav_backup_count();
        assert_eq!(c.webdav_backup_count(), 2);
    }

    #[test]
    fn get_str_non_empty_treats_empty_as_unset() {
        let mut c = AppConfig::in_memory();
        c.set_str(KEY_PROMPT_A, "");
        assert_eq!(c.get_str(KEY_PROMPT_A), Some(""));
        assert_eq!(c.get_str_non_empty(KEY_PROMPT_A), None, "空串视为未设置");
        c.set_str(KEY_PROMPT_A, "x");
        assert_eq!(c.get_str_non_empty(KEY_PROMPT_A), Some("x"));
    }

    /// 落盘 JSON 与备份包 `BackupPrefs` 同形（`strings` / `ints` 两个键）
    #[test]
    fn on_disk_shape_matches_backup_prefs() {
        let mut c = AppConfig::in_memory();
        c.set_str(KEY_PROMPT_A, "P");
        c.set_int(KEY_BG_TRANSPARENCY, 22);

        let v = serde_json::to_value(&c.data).unwrap();
        assert!(v.get("strings").is_some(), "须与 BackupPrefs.strings 同名");
        assert!(v.get("ints").is_some(), "须与 BackupPrefs.ints 同名");
    }

    // ---------------- 区块重置 ----------------

    #[test]
    fn section_parse_and_roundtrip() {
        for s in ConfigSection::all() {
            assert_eq!(ConfigSection::parse(s.as_str()), Some(*s));
        }
        // 大小写不敏感
        assert_eq!(
            ConfigSection::parse("appearance"),
            Some(ConfigSection::Appearance)
        );
        // 未知值必须返回 None（不能静默当成某个区块，否则会删错东西）
        assert_eq!(ConfigSection::parse("BOGUS"), None);
        assert_eq!(ConfigSection::parse(""), None);
    }

    #[test]
    fn section_serde_is_screaming_snake() {
        let v = serde_json::to_value(ConfigSection::ModelTest).unwrap();
        assert_eq!(v, serde_json::json!("MODEL_TEST"));
        let back: ConfigSection = serde_json::from_value(v).unwrap();
        assert_eq!(back, ConfigSection::ModelTest);
    }

    #[test]
    fn reset_section_only_removes_its_own_keys() {
        let mut c = AppConfig::in_memory();
        c.set_theme_mode(ThemeMode::Dark); // APPEARANCE
        c.set_background_transparency(50); // APPEARANCE
        c.set_str(KEY_PROMPT_A, "P"); // PROMPTS
        c.set_int(KEY_WEBDAV_BACKUP_COUNT, 3); // WEBDAV_STATS

        let n = c.reset_section(ConfigSection::Appearance);
        assert_eq!(n, 2, "应删掉 2 个键");
        assert_eq!(c.theme_mode(), ThemeMode::System, "重置后回落默认");
        assert_eq!(c.background_transparency(), 22, "回落默认 22");
        assert_eq!(c.get_str(KEY_PROMPT_A), Some("P"), "别的区块不受影响");
        assert_eq!(c.webdav_backup_count(), 3);
    }

    #[test]
    fn reset_section_on_absent_keys_returns_zero() {
        let mut c = AppConfig::in_memory();
        assert_eq!(c.reset_section(ConfigSection::Appearance), 0);
    }

    #[test]
    fn every_section_key_is_a_known_constant() {
        // 防止把键名写错（写错的键永远不会被删掉）
        let known: Vec<&str> = vec![
            KEY_THEME, KEY_PROMPT_A, KEY_DEFAULT_REMINDER, KEY_SEARCH_QUERY,
            KEY_MODEL_TEST_INPUT, KEY_DRAFTS, KEY_BACKGROUND, KEY_BG_TRANSPARENCY,
            KEY_TEXT_COLOR, KEY_BG_COLOR, KEY_SHOW_PROMPTS, KEY_SEARCH_HISTORY,
            KEY_REFINE_HISTORY, KEY_MODEL_TEST_HISTORY, KEY_MODEL_TEST_CONVERSATION,
            KEY_TEST_CONTEXT_SIZE, KEY_TEST_COMPRESS_THRESHOLD, KEY_TEST_COMPRESS_NOTICE,
            KEY_TEST_SYSTEM_PROMPT, KEY_GENERATE_EXPLANATION, KEY_EXPORT_OPTIONS,
            KEY_WEBDAV_LAST_BACKUP, KEY_WEBDAV_BACKUP_COUNT,
        ];
        for s in ConfigSection::all() {
            for k in s.keys() {
                assert!(known.contains(k), "区块 {s:?} 含未知键 {k}");
            }
        }
        // 全部键应被区块划分**完全覆盖**（无遗漏、无重复）
        let mut covered: Vec<&str> = ConfigSection::all().iter().flat_map(|s| s.keys()).copied().collect();
        covered.sort_unstable();
        let before = covered.len();
        covered.dedup();
        assert_eq!(covered.len(), before, "有键被多个区块重复拥有");
        let mut expected = known.clone();
        expected.sort_unstable();
        assert_eq!(
            covered,
            expected,
            "区块划分未覆盖全部 {} 个键",
            known.len()
        );
    }

    // ---------------- 快照（IPC） ----------------

    #[test]
    fn snapshot_roundtrip_via_replace_with() {
        let mut a = AppConfig::in_memory();
        a.set_theme_mode(ThemeMode::Dark);
        a.set_str(KEY_PROMPT_A, "P");
        a.set_int(KEY_BG_TRANSPARENCY, 30);

        let snap = a.snapshot();
        let mut b = AppConfig::in_memory();
        b.replace_with(snap);

        assert_eq!(b.theme_mode(), ThemeMode::Dark);
        assert_eq!(b.get_str(KEY_PROMPT_A), Some("P"));
        assert_eq!(b.background_transparency(), 30);
    }

    /// 快照是**整体替换**：快照里没有的键会被清掉（与 `apply_backup` 的合并覆盖不同）
    #[test]
    fn replace_with_is_replace_not_merge() {
        let mut a = AppConfig::in_memory();
        a.set_theme_mode(ThemeMode::Dark);
        let snap = a.snapshot();

        let mut b = AppConfig::in_memory();
        b.set_str("only_local", "会被清掉");
        b.replace_with(snap);

        assert_eq!(b.get_str("only_local"), None, "替换语义：本机独有键应消失");
        assert_eq!(b.theme_mode(), ThemeMode::Dark);
    }

    /// 快照**含** NON_BACKUP_KEYS（这是本机全量，不是备份）
    #[test]
    fn snapshot_includes_non_backup_keys() {
        let mut c = AppConfig::in_memory();
        c.set_int(KEY_WEBDAV_BACKUP_COUNT, 5);
        let snap = c.snapshot();
        assert!(snap.ints.contains_key(KEY_WEBDAV_BACKUP_COUNT));
        // 而备份导出会剔除它
        assert!(!c.backup_ints().contains_key(KEY_WEBDAV_BACKUP_COUNT));
    }

    #[test]
    fn snapshot_serde_shape() {
        let mut c = AppConfig::in_memory();
        c.set_str(KEY_PROMPT_A, "P");
        c.set_int(KEY_BG_TRANSPARENCY, 22);
        let v = serde_json::to_value(c.snapshot()).unwrap();
        assert!(v.get("strings").is_some());
        assert!(v.get("ints").is_some());
    }
}
