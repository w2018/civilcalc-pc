//! LLM 模型配置。
//!
//! 源：`civilcalc-android-v2/core/llm/LlmConfig.kt`（66 行）
//!
//! ## ⚠️ 必须保留的「实测知识」（重写最易丢失的资产）
//!
//! 源项目注释里记着 2026-09 对各家的实测结论，**逐字保留**：
//!
//! - **DeepSeek**（chat/completions）：`thinking.type=disabled` 可真关闭；
//!   `reasoning_effort` low/high/max（默认 high）
//! - **DeepSeek**（responses）：`reasoning.effort` none/low/high/max
//! - **GLM glm-5.3/5.3-flash**：**强制思考**，发 `thinking.type=disabled` 直接
//!   400（错误码 1210），只能靠 `reasoning_effort` low/high/max（默认 max）降本
//!   → **本应用把「关闭」降级为 low**
//! - **小米 MiMo**（chat/completions）：`thinking.type` 开/关；**无档位控制**
//!
//! 这三个结论决定 [`supports_reasoning_effort`] / [`forces_thinking`] 的实现，
//! 改它们之前先读 [`crate::client`] 的协议拼装。

use serde::{Deserialize, Serialize};

/// 思考强度。
///
/// 各家控制方式不统一（实测 2026-09）：
///
/// | 厂商 | 协议 | 关闭 | 档位 |
/// |---|---|---|---|
/// | DeepSeek | chat/completions | `thinking.type=disabled` 有效 | `reasoning_effort` low/high/max（默认 high） |
/// | DeepSeek | responses | — | `reasoning.effort` none/low/high/max |
/// | GLM glm-5.3/5.3-flash | chat/completions | ❌ **强制思考**，发 disabled 直接 400（1210） | `reasoning_effort` low/high/max（默认 max） |
/// | 小米 MiMo | chat/completions | `thinking.type` 开/关 | ❌ 无档位控制 |
///
/// ⚠️ serde 表示为 `SCREAMING_SNAKE_CASE`，与源项目 Kotlin 枚举名逐字一致 ——
/// 这个字符串会随 `llm_config` 进备份包，**不能改**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ThinkingLevel {
    /// 跟随厂商默认（不显式下发控制参数）
    #[default]
    Auto,
    /// 关闭思考。GLM 上会**降级为 low**（见 [`forces_thinking`]）
    Off,
    Low,
    High,
    Max,
}

impl ThinkingLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            ThinkingLevel::Auto => "AUTO",
            ThinkingLevel::Off => "OFF",
            ThinkingLevel::Low => "LOW",
            ThinkingLevel::High => "HIGH",
            ThinkingLevel::Max => "MAX",
        }
    }

    /// 解析；未知值回落 `Auto`（不报错）
    pub fn parse(s: &str) -> Self {
        match s {
            "OFF" => ThinkingLevel::Off,
            "LOW" => ThinkingLevel::Low,
            "HIGH" => ThinkingLevel::High,
            "MAX" => ThinkingLevel::Max,
            _ => ThinkingLevel::Auto,
        }
    }

    /// 是否需要在请求里显式下发思考控制参数
    pub fn is_explicit(self) -> bool {
        self != ThinkingLevel::Auto
    }
}

/// 接入协议。
///
/// `Auto` 保持历史行为：**DeepSeek + 联网走 `/responses`，其余走 `/chat/completions`**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApiProtocol {
    #[default]
    Auto,
    ChatCompletions,
    Responses,
}

impl ApiProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            ApiProtocol::Auto => "AUTO",
            ApiProtocol::ChatCompletions => "CHAT_COMPLETIONS",
            ApiProtocol::Responses => "RESPONSES",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "CHAT_COMPLETIONS" => ApiProtocol::ChatCompletions,
            "RESPONSES" => ApiProtocol::Responses,
            _ => ApiProtocol::Auto,
        }
    }
}

/// 一个模型配置档（**不含 API Key**）。
///
/// ⚠️ API Key **从不**存在这个结构里，也不落 SQLite —— 只进 keyring
/// （源项目 BUG-27 凭据审计）。本类型会序列化进 `llm_config` 与备份包。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmProfile {
    /// 档位 id（同时是 keyring 条目的键：`profile_{id}` / `profile_{id}_key`）
    pub id: String,
    /// 展示名
    pub label: String,
    /// Base URL
    pub base_url: String,
    /// 模型名
    pub model: String,
    /// 是否启用
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 是否联网（走 web_search 工具）
    #[serde(default)]
    pub web_search: bool,
    /// 思考强度
    #[serde(default)]
    pub thinking_level: ThinkingLevel,
    /// 模型是否支持视觉（图片识别），由用户在设置中判断开启
    #[serde(default)]
    pub vision: bool,
    /// 接入协议
    #[serde(default)]
    pub api_protocol: ApiProtocol,
}

fn default_true() -> bool {
    true
}

impl LlmProfile {
    /// 便捷构造（其余字段走默认值）
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            base_url: base_url.into(),
            model: model.into(),
            enabled: true,
            web_search: false,
            thinking_level: ThinkingLevel::Auto,
            vision: false,
            api_protocol: ApiProtocol::Auto,
        }
    }

    /// 带视觉能力
    pub fn with_vision(mut self) -> Self {
        self.vision = true;
        self
    }

    /// keyring 里存配置的键名
    pub fn config_key(&self) -> String {
        format!("profile_{}", self.id)
    }

    /// keyring 里存 API Key 的键名
    pub fn api_key_key(&self) -> String {
        format!("profile_{}_key", self.id)
    }
}

/// 全部模型配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmConfig {
    #[serde(default = "default_profiles")]
    pub profiles: Vec<LlmProfile>,
    /// 活跃档位 id
    #[serde(default = "default_active")]
    pub active: String,
}

fn default_active() -> String {
    "deepseek".to_string()
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            profiles: default_profiles(),
            active: default_active(),
        }
    }
}

/// 三家预设（1:1 对齐源项目 `defaultProfiles()`）。
///
/// | id | label | baseUrl | model |
/// |---|---|---|---|
/// | `deepseek` | DeepSeek | `https://api.deepseek.com/v1` | `deepseek-flash` |
/// | `mimo` | MiniMax Mimo | `https://api.xiaomimimo.com/v1` | `mimo-v2.5` |
/// | `glm` | 智谱 GLM | `https://open.bigmodel.cn/api/paas/v4` | `glm-5.3-flash` |
///
/// 三家**都预置为支持视觉**（源项目 `vision = true`）。
pub fn default_profiles() -> Vec<LlmProfile> {
    vec![
        LlmProfile::new(
            "deepseek",
            "DeepSeek",
            "https://api.deepseek.com/v1",
            "deepseek-flash",
        )
        .with_vision(),
        LlmProfile::new(
            "mimo",
            "MiniMax Mimo",
            "https://api.xiaomimimo.com/v1",
            "mimo-v2.5",
        )
        .with_vision(),
        LlmProfile::new(
            "glm",
            "智谱 GLM",
            "https://open.bigmodel.cn/api/paas/v4",
            "glm-5.3-flash",
        )
        .with_vision(),
    ]
}

impl LlmConfig {
    /// 按 id 找档位
    pub fn profile(&self, id: &str) -> Option<&LlmProfile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    /// 取活跃档位（可能为 `None` —— 配置损坏时）
    pub fn active_profile(&self) -> Option<&LlmProfile> {
        self.profile(&self.active)
    }
}

/// **已解析**的模型配置：把档位与 API Key 合到一起，供调用链直接使用。
///
/// 只在内存里短暂存在，**绝不序列化落盘**（含明文 Key）——
/// 因此**刻意不派生 `Serialize`/`Deserialize`**。
///
/// `Debug` 是**手写的**：直接派生会把明文 Key 打进日志。
#[derive(Clone, PartialEq)]
pub struct ResolvedLlmProfile {
    pub base_url: String,
    pub model: String,
    /// ⚠️ 明文 Key。只进内存与请求头，**不进日志、不进错误文案**。
    pub api_key: String,
    pub web_search: bool,
    pub thinking_level: ThinkingLevel,
    pub vision: bool,
    pub api_protocol: ApiProtocol,
}

impl std::fmt::Debug for ResolvedLlmProfile {
    /// 手写实现：**Key 只输出脱敏形式**。
    ///
    /// 派生 `Debug` 是凭据泄漏最常见的路径（`tracing::debug!("{profile:?}")`）。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedLlmProfile")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &mask_secret(&self.api_key))
            .field("web_search", &self.web_search)
            .field("thinking_level", &self.thinking_level)
            .field("vision", &self.vision)
            .field("api_protocol", &self.api_protocol)
            .finish()
    }
}

impl ResolvedLlmProfile {
    /// 由档位 + 明文 Key 构造
    pub fn from_profile(p: &LlmProfile, api_key: impl Into<String>) -> Self {
        Self {
            base_url: p.base_url.clone(),
            model: p.model.clone(),
            api_key: api_key.into(),
            web_search: p.web_search,
            thinking_level: p.thinking_level,
            vision: p.vision,
            api_protocol: p.api_protocol,
        }
    }

    /// 脱敏展示（日志用）：只保留 Key 的首尾各 4 位
    pub fn masked_api_key(&self) -> String {
        mask_secret(&self.api_key)
    }

    /// 实际要下发的思考档位（已按厂商能力归一化，见 [`effective_thinking_level`]）
    pub fn effective_thinking_level(&self) -> ThinkingLevel {
        effective_thinking_level(&self.base_url, self.thinking_level)
    }
}

/// 把密钥脱敏成 `abcd…wxyz` 形式。短于 12 位则全遮蔽。
///
/// ⚠️ 日志与错误文案**必须**用它，不要直接打印 Key。
pub fn mask_secret(s: &str) -> String {
    let n = s.chars().count();
    if n == 0 {
        return "(空)".to_string();
    }
    if n < 12 {
        return "***".to_string();
    }
    let head: String = s.chars().take(4).collect();
    let tail: String = s.chars().skip(n - 4).collect();
    format!("{head}…{tail}")
}

// =============================================================================
// 厂商能力判定（1:1 对齐源项目三个顶层函数）
// =============================================================================

/// 该 Base URL 是否支持 `web_search` 联网。
///
/// 源项目：DeepSeek 走 Responses API，小米 Mimo、GLM(bigmodel) 走 chat/completions。
pub fn supports_web_search(base_url: &str) -> bool {
    base_url.contains("deepseek") || base_url.contains("bigmodel") || base_url.contains("xiaomimimo")
}

/// 该厂商是否**强制思考**（GLM glm-5.3 系列无法关闭思考）。
///
/// 为 `true` 时，[`ThinkingLevel::Off`] 必须**降级为 [`ThinkingLevel::Low`]** ——
/// 否则会收到 400（错误码 1210）。
pub fn forces_thinking(base_url: &str) -> bool {
    base_url.contains("bigmodel")
}

/// 该厂商在 chat/completions 上是否支持 `reasoning_effort` 档位
/// （小米 MiMo 仅支持开关）。
pub fn supports_reasoning_effort(base_url: &str) -> bool {
    !base_url.contains("xiaomimimo")
}

/// 把用户选的思考强度**归一化为实际要下发的档位**。
///
/// 这是把上面三个能力函数组合起来的**唯一入口**，调用方不要自己拼判断。
///
/// | 输入 | 厂商 | 输出 | 说明 |
/// |---|---|---|---|
/// | `Off` | GLM | `Low` | 强制思考，关闭会 400 → 降级 |
/// | `Off` | MiMo | `Off` | 只支持开关，可关 |
/// | `Off` | DeepSeek | `Off` | 可关 |
/// | `Low/High/Max` | MiMo | `Auto` | 无档位控制 → 不显式下发 |
/// | `Low/High/Max` | 其他 | 原样 | |
/// | `Auto` | 任意 | `Auto` | |
pub fn effective_thinking_level(base_url: &str, want: ThinkingLevel) -> ThinkingLevel {
    match want {
        ThinkingLevel::Auto => ThinkingLevel::Auto,
        ThinkingLevel::Off => {
            if forces_thinking(base_url) {
                ThinkingLevel::Low
            } else {
                ThinkingLevel::Off
            }
        }
        // MiMo 不支持档位，只能开/关 —— 想调档时退化为"不显式下发"
        ThinkingLevel::Low | ThinkingLevel::High | ThinkingLevel::Max => {
            if supports_reasoning_effort(base_url) {
                want
            } else {
                ThinkingLevel::Auto
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------- 默认预设 ----------------

    #[test]
    fn default_profiles_match_source() {
        let p = default_profiles();
        assert_eq!(p.len(), 3);

        assert_eq!(p[0].id, "deepseek");
        assert_eq!(p[0].label, "DeepSeek");
        assert_eq!(p[0].base_url, "https://api.deepseek.com/v1");
        assert_eq!(p[0].model, "deepseek-flash");

        assert_eq!(p[1].id, "mimo");
        assert_eq!(p[1].label, "MiniMax Mimo");
        assert_eq!(p[1].base_url, "https://api.xiaomimimo.com/v1");
        assert_eq!(p[1].model, "mimo-v2.5");

        assert_eq!(p[2].id, "glm");
        assert_eq!(p[2].label, "智谱 GLM");
        assert_eq!(p[2].base_url, "https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(p[2].model, "glm-5.3-flash");

        // 三家都预置视觉
        assert!(p.iter().all(|x| x.vision));
        // 默认启用、不联网、思考 AUTO、协议 AUTO
        assert!(p.iter().all(|x| x.enabled));
        assert!(p.iter().all(|x| !x.web_search));
        assert!(p
            .iter()
            .all(|x| x.thinking_level == ThinkingLevel::Auto));
        assert!(p.iter().all(|x| x.api_protocol == ApiProtocol::Auto));
    }

    #[test]
    fn default_config_uses_deepseek_active() {
        let c = LlmConfig::default();
        assert_eq!(c.active, "deepseek");
        assert_eq!(c.profiles.len(), 3);
        assert_eq!(c.active_profile().unwrap().label, "DeepSeek");
        assert!(c.profile("nope").is_none());
    }

    // ---------------- 枚举 serde（进备份包，不能改） ----------------

    #[test]
    fn thinking_level_serde_is_screaming_snake() {
        for (v, s) in [
            (ThinkingLevel::Auto, "\"AUTO\""),
            (ThinkingLevel::Off, "\"OFF\""),
            (ThinkingLevel::Low, "\"LOW\""),
            (ThinkingLevel::High, "\"HIGH\""),
            (ThinkingLevel::Max, "\"MAX\""),
        ] {
            assert_eq!(serde_json::to_string(&v).unwrap(), s);
            assert_eq!(ThinkingLevel::parse(v.as_str()), v);
        }
        // 未知值回落 Auto
        assert_eq!(ThinkingLevel::parse("BOGUS"), ThinkingLevel::Auto);
    }

    #[test]
    fn api_protocol_serde_is_screaming_snake() {
        for (v, s) in [
            (ApiProtocol::Auto, "\"AUTO\""),
            (ApiProtocol::ChatCompletions, "\"CHAT_COMPLETIONS\""),
            (ApiProtocol::Responses, "\"RESPONSES\""),
        ] {
            assert_eq!(serde_json::to_string(&v).unwrap(), s);
            assert_eq!(ApiProtocol::parse(v.as_str()), v);
        }
        assert_eq!(ApiProtocol::parse("BOGUS"), ApiProtocol::Auto);
    }

    // ---------------- 厂商能力 ----------------

    #[test]
    fn web_search_support_matches_source() {
        assert!(supports_web_search("https://api.deepseek.com/v1"));
        assert!(supports_web_search("https://open.bigmodel.cn/api/paas/v4"));
        assert!(supports_web_search("https://api.xiaomimimo.com/v1"));
        assert!(!supports_web_search("https://api.openai.com/v1"));
        assert!(!supports_web_search("http://localhost:11434/v1"));
    }

    #[test]
    fn only_glm_forces_thinking() {
        assert!(forces_thinking("https://open.bigmodel.cn/api/paas/v4"));
        assert!(!forces_thinking("https://api.deepseek.com/v1"));
        assert!(!forces_thinking("https://api.xiaomimimo.com/v1"));
    }

    #[test]
    fn only_mimo_lacks_reasoning_effort() {
        assert!(!supports_reasoning_effort("https://api.xiaomimimo.com/v1"));
        assert!(supports_reasoning_effort("https://api.deepseek.com/v1"));
        assert!(supports_reasoning_effort("https://open.bigmodel.cn/api/paas/v4"));
    }

    // ---------------- 思考强度归一化（关键业务规则） ----------------

    /// **GLM 强制思考**：Off 必须降级为 Low（否则 400 / 错误码 1210）
    #[test]
    fn glm_off_is_downgraded_to_low() {
        let glm = "https://open.bigmodel.cn/api/paas/v4";
        assert_eq!(
            effective_thinking_level(glm, ThinkingLevel::Off),
            ThinkingLevel::Low,
            "GLM 发 thinking.type=disabled 会 400，必须降级为 low"
        );
    }

    #[test]
    fn deepseek_off_stays_off() {
        let ds = "https://api.deepseek.com/v1";
        assert_eq!(
            effective_thinking_level(ds, ThinkingLevel::Off),
            ThinkingLevel::Off
        );
    }

    #[test]
    fn mimo_off_stays_off_but_levels_collapse_to_auto() {
        let mimo = "https://api.xiaomimimo.com/v1";
        // MiMo 只支持开关 → Off 可保留
        assert_eq!(
            effective_thinking_level(mimo, ThinkingLevel::Off),
            ThinkingLevel::Off
        );
        // 但不支持档位 → Low/High/Max 退化为 Auto（不显式下发）
        for lv in [ThinkingLevel::Low, ThinkingLevel::High, ThinkingLevel::Max] {
            assert_eq!(
                effective_thinking_level(mimo, lv),
                ThinkingLevel::Auto,
                "MiMo 无档位控制，{lv:?} 应退化为 Auto"
            );
        }
    }

    #[test]
    fn levels_pass_through_for_capable_vendors() {
        let ds = "https://api.deepseek.com/v1";
        for lv in [ThinkingLevel::Low, ThinkingLevel::High, ThinkingLevel::Max] {
            assert_eq!(effective_thinking_level(ds, lv), lv);
        }
        assert_eq!(
            effective_thinking_level(ds, ThinkingLevel::Auto),
            ThinkingLevel::Auto
        );
    }

    #[test]
    fn glm_levels_pass_through() {
        let glm = "https://open.bigmodel.cn/api/paas/v4";
        for lv in [ThinkingLevel::Low, ThinkingLevel::High, ThinkingLevel::Max] {
            assert_eq!(effective_thinking_level(glm, lv), lv);
        }
    }

    // ---------------- 脱敏 ----------------

    #[test]
    fn mask_secret_keeps_only_ends() {
        assert_eq!(mask_secret("sk-1234567890abcdef"), "sk-1…cdef");
        assert_eq!(mask_secret(""), "(空)");
        assert_eq!(mask_secret("short"), "***");
        assert_eq!(mask_secret("12345678901"), "***", "11 位也全遮蔽");
        assert_eq!(mask_secret("123456789012"), "1234…9012");
    }

    #[test]
    fn masked_key_never_leaks_full_secret() {
        let p = LlmProfile::new("x", "X", "https://api.deepseek.com/v1", "m");
        let r = ResolvedLlmProfile::from_profile(&p, "sk-abcdefghijklmnop");
        let m = r.masked_api_key();
        assert!(!m.contains("abcdefghijklmnop"), "脱敏串不得含完整 Key");
        assert!(m.starts_with("sk-a"));
        assert!(m.ends_with("mnop"));
    }

    // ---------------- 序列化形状（进备份包 llm_config） ----------------

    #[test]
    fn profile_serde_is_camel_case_and_has_no_api_key() {
        let p = LlmProfile::new("d", "D", "https://api.deepseek.com/v1", "m");
        let v = serde_json::to_value(&p).unwrap();

        assert_eq!(v["baseUrl"], serde_json::json!("https://api.deepseek.com/v1"));
        assert_eq!(v["thinkingLevel"], serde_json::json!("AUTO"));
        assert_eq!(v["apiProtocol"], serde_json::json!("AUTO"));
        assert_eq!(v["webSearch"], serde_json::json!(false));
        assert!(v.get("base_url").is_none(), "不得泄漏 snake_case");
        // ⚠️ 凭据审计：结构里绝不能有 Key 字段
        for k in ["apiKey", "api_key", "key", "token"] {
            assert!(v.get(k).is_none(), "LlmProfile 不得含凭据字段 {k}");
        }
    }

    #[test]
    fn config_roundtrip() {
        let c = LlmConfig::default();
        let s = serde_json::to_string(&c).unwrap();
        let back: LlmConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back, c);
    }

    /// 旧配置缺字段时要能用默认值（`coerceInputValues` 语义）
    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let j = r#"{"id":"x","label":"X","baseUrl":"u","model":"m"}"#;
        let p: LlmProfile = serde_json::from_str(j).unwrap();
        assert!(p.enabled, "缺 enabled 应默认 true");
        assert!(!p.web_search);
        assert_eq!(p.thinking_level, ThinkingLevel::Auto);
        assert!(!p.vision);
        assert_eq!(p.api_protocol, ApiProtocol::Auto);
    }

    #[test]
    fn keyring_key_names_match_source() {
        let p = LlmProfile::new("deepseek", "D", "u", "m");
        assert_eq!(p.config_key(), "profile_deepseek");
        assert_eq!(p.api_key_key(), "profile_deepseek_key");
    }
}
