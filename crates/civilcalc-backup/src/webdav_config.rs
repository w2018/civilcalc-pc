//! WebDAV 连接配置。
//!
//! 源：`civilcalc-android-v2/core/backup/WebDavConfig.kt`
//!
//! ## ⚠️ 凭据审计（源项目 BUG-27）
//!
//! [`WebDavConfig`] **不含密码** —— 密码单独进 keyring（`webdav_password`）。
//! 带密码的 [`WebDavConnection`] 只在内存里传递，**不序列化、不落盘**。

use serde::{Deserialize, Serialize};

use crate::webdav_path::{base_url_error, normalize_base_url, remote_dir_url};

/// 坚果云 WebDAV 入口。
///
/// 账号 = 登录邮箱；密码需在「账户信息 → 安全选项 → 添加应用密码」生成
/// （**不是登录密码**）。
pub const DEFAULT_BASE_URL: &str = "https://dav.jianguoyun.com/dav";

/// 远端文件夹名（不存在时自动创建）
pub const DEFAULT_REMOTE_DIR: &str = "civilcalc";

/// 备份记录最多展示条数（按时间倒序取最新）
pub const LIST_LIMIT: usize = 20;

/// 远端服务预设：默认坚果云，也可自定义任意 WebDAV 地址。
///
/// ⚠️ serde 表示为 `SCREAMING_SNAKE_CASE`，与源项目 Kotlin 枚举名逐字一致 ——
/// 这个字符串会随 `webdav_config` 进备份包，**不能改**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WebDavPreset {
    #[default]
    Nutstore,
    Custom,
}

impl WebDavPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            WebDavPreset::Nutstore => "NUTSTORE",
            WebDavPreset::Custom => "CUSTOM",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "CUSTOM" => WebDavPreset::Custom,
            _ => WebDavPreset::Nutstore,
        }
    }
}

/// 预设对应的默认服务器地址；自定义预设留空由用户填写。
pub fn default_base_url_of(preset: WebDavPreset) -> &'static str {
    match preset {
        WebDavPreset::Nutstore => DEFAULT_BASE_URL,
        WebDavPreset::Custom => "",
    }
}

/// WebDAV 连接配置（**不含密码**）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebDavConfig {
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub username: String,
    #[serde(default = "default_remote_dir")]
    pub remote_dir: String,
    #[serde(default)]
    pub preset: WebDavPreset,
}

fn default_base_url() -> String {
    DEFAULT_BASE_URL.to_string()
}

fn default_remote_dir() -> String {
    DEFAULT_REMOTE_DIR.to_string()
}

impl Default for WebDavConfig {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            username: String::new(),
            remote_dir: default_remote_dir(),
            preset: WebDavPreset::Nutstore,
        }
    }
}

impl WebDavConfig {
    /// 归一化后的 base URL（去空白、去末尾 `/`）
    pub fn normalized_base_url(&self) -> String {
        normalize_base_url(&self.base_url)
    }

    /// 远端目录 URL（末尾带 `/`）
    pub fn dir_url(&self) -> String {
        remote_dir_url(&self.base_url, &self.remote_dir)
    }

    /// 配置是否可用。
    ///
    /// ⚠️ **只认 https**（与源一致）—— 早期版本写成「http 或 https」，
    /// 与 `WebDavPath.baseUrlError` 的约束矛盾。备份包里含 API Key 与 WebDAV 密码，
    /// 明文 HTTP 会把凭据暴露在链路上。
    ///
    /// 判据直接复用 [`base_url_error`]，避免两处规则漂移。
    pub fn is_usable(&self) -> bool {
        base_url_error(&self.base_url).is_none()
    }

    /// 配置不可用时的中文原因（供设置页直接展示）
    pub fn base_url_error(&self) -> Option<String> {
        base_url_error(&self.base_url)
    }

    /// 切换预设时**同步默认地址**（切到 `CUSTOM` 保留用户已填的地址）
    pub fn apply_preset(&mut self, preset: WebDavPreset) {
        if preset == WebDavPreset::Nutstore {
            self.base_url = DEFAULT_BASE_URL.to_string();
        }
        self.preset = preset;
    }
}

/// 带密码的连接信息：**只在内存里传递**，不序列化、不落盘。
///
/// `Debug` 是手写的 —— 直接派生会把明文密码打进日志。
#[derive(Clone, PartialEq)]
pub struct WebDavConnection {
    pub config: WebDavConfig,
    /// ⚠️ 明文密码。只进内存与请求头，**不进日志、不进错误文案**。
    pub password: String,
}

impl std::fmt::Debug for WebDavConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebDavConnection")
            .field("config", &self.config)
            .field("password", &"(已隐藏)")
            .finish()
    }
}

impl WebDavConnection {
    pub fn new(config: WebDavConfig, password: impl Into<String>) -> Self {
        Self {
            config,
            password: password.into(),
        }
    }

    /// 归一化后的 base URL
    pub fn base_url(&self) -> String {
        self.config.normalized_base_url()
    }

    /// 远端目录 URL
    pub fn dir_url(&self) -> String {
        self.config.dir_url()
    }

    /// 用户名（来自 config）
    pub fn username(&self) -> &str {
        &self.config.username
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------- 默认值（对齐源项目 companion 常量） ----------------

    #[test]
    fn default_config_matches_source() {
        let c = WebDavConfig::default();
        assert_eq!(c.base_url, "https://dav.jianguoyun.com/dav");
        assert_eq!(c.remote_dir, "civilcalc");
        assert_eq!(c.username, "");
        assert_eq!(c.preset, WebDavPreset::Nutstore);
    }

    #[test]
    fn constants_match_source() {
        assert_eq!(DEFAULT_BASE_URL, "https://dav.jianguoyun.com/dav");
        assert_eq!(DEFAULT_REMOTE_DIR, "civilcalc");
        assert_eq!(LIST_LIMIT, 20);
    }

    #[test]
    fn default_base_url_of_preset() {
        assert_eq!(default_base_url_of(WebDavPreset::Nutstore), DEFAULT_BASE_URL);
        assert_eq!(default_base_url_of(WebDavPreset::Custom), "");
    }

    // ---------------- 预设枚举 serde（进备份包） ----------------

    #[test]
    fn preset_serde_is_screaming_snake() {
        assert_eq!(
            serde_json::to_string(&WebDavPreset::Nutstore).unwrap(),
            "\"NUTSTORE\""
        );
        assert_eq!(
            serde_json::to_string(&WebDavPreset::Custom).unwrap(),
            "\"CUSTOM\""
        );
        assert_eq!(WebDavPreset::parse("CUSTOM"), WebDavPreset::Custom);
        assert_eq!(WebDavPreset::parse("NUTSTORE"), WebDavPreset::Nutstore);
        // 未知值回落 NUTSTORE
        assert_eq!(WebDavPreset::parse("BOGUS"), WebDavPreset::Nutstore);
    }

    // ---------------- URL 派生 ----------------

    #[test]
    fn normalized_and_dir_url() {
        let c = WebDavConfig {
            base_url: " https://dav.jianguoyun.com/dav/ ".to_string(),
            ..Default::default()
        };
        assert_eq!(c.normalized_base_url(), "https://dav.jianguoyun.com/dav");
        assert_eq!(
            c.dir_url(),
            "https://dav.jianguoyun.com/dav/civilcalc/"
        );
    }

    #[test]
    fn is_usable_requires_https_scheme() {
        assert!(WebDavConfig::default().is_usable());

        let blank = WebDavConfig {
            base_url: String::new(),
            ..Default::default()
        };
        assert!(!blank.is_usable());

        let no_scheme = WebDavConfig {
            base_url: "dav.jianguoyun.com/dav".to_string(),
            ..Default::default()
        };
        assert!(!no_scheme.is_usable(), "缺 scheme 不可用");

        // 🔴 明文 http **不可用**（与源一致）—— 早期版本这里断言"内网 http 也应可用"，
        //    与 `baseUrlError` 的约束矛盾，已修正。
        let lan = WebDavConfig {
            base_url: "http://192.168.1.10/dav".to_string(),
            ..Default::default()
        };
        assert!(!lan.is_usable(), "明文 http 不可用");
        assert_eq!(
            lan.base_url_error().as_deref(),
            Some("只支持 https 地址（本机安全策略禁止明文流量）")
        );

        // 自签 https 的内网地址是可用的（协议对就行）
        let self_hosted = WebDavConfig {
            base_url: "https://192.168.1.10:8443/dav".to_string(),
            ..Default::default()
        };
        assert!(self_hosted.is_usable());
    }

    #[test]
    fn apply_preset_syncs_base_url() {
        let mut c = WebDavConfig::default();

        // 切到自定义：保留当前地址（由用户改）
        c.apply_preset(WebDavPreset::Custom);
        assert_eq!(c.preset, WebDavPreset::Custom);
        assert_eq!(c.base_url, DEFAULT_BASE_URL, "切 CUSTOM 不清空已有地址");

        // 改地址后再切回坚果云：恢复默认
        c.base_url = "https://my.server/dav".to_string();
        c.apply_preset(WebDavPreset::Nutstore);
        assert_eq!(c.preset, WebDavPreset::Nutstore);
        assert_eq!(c.base_url, DEFAULT_BASE_URL);
    }

    // ---------------- 序列化形状 ----------------

    #[test]
    fn config_serde_is_camel_case_and_has_no_password() {
        let c = WebDavConfig::default();
        let v = serde_json::to_value(&c).unwrap();

        assert_eq!(v["baseUrl"], serde_json::json!(DEFAULT_BASE_URL));
        assert_eq!(v["remoteDir"], serde_json::json!("civilcalc"));
        assert_eq!(v["preset"], serde_json::json!("NUTSTORE"));
        assert!(v.get("base_url").is_none(), "不得泄漏 snake_case");

        // ⚠️ 凭据审计：配置结构里绝不能有密码字段
        for k in ["password", "passwd", "pwd", "token"] {
            assert!(v.get(k).is_none(), "WebDavConfig 不得含凭据字段 {k}");
        }
    }

    #[test]
    fn config_roundtrip() {
        let c = WebDavConfig {
            username: "me@example.com".to_string(),
            preset: WebDavPreset::Custom,
            base_url: "https://x.com/dav".to_string(),
            ..Default::default()
        };

        let s = serde_json::to_string(&c).unwrap();
        let back: WebDavConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let c: WebDavConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c, WebDavConfig::default());
    }

    // ---------------- 连接（含密码） ----------------

    #[test]
    fn connection_delegates_urls() {
        let cfg = WebDavConfig {
            username: "me@example.com".to_string(),
            ..Default::default()
        };
        let conn = WebDavConnection::new(cfg, "app-password");

        assert_eq!(conn.base_url(), DEFAULT_BASE_URL);
        assert_eq!(
            conn.dir_url(),
            "https://dav.jianguoyun.com/dav/civilcalc/"
        );
        assert_eq!(conn.username(), "me@example.com");
    }

    /// **回归**：`Debug` 输出不得含明文密码
    #[test]
    fn connection_debug_hides_password() {
        let conn = WebDavConnection::new(WebDavConfig::default(), "super-secret-pw");
        let s = format!("{conn:?}");
        assert!(!s.contains("super-secret-pw"), "Debug 泄漏了密码: {s}");
        assert!(s.contains("已隐藏"));
        // 但配置信息应保留（排障需要）
        assert!(s.contains("civilcalc"));
    }
}
