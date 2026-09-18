//! 密钥与安全配置存储（替代源项目 `data/security/KeystoreBackedPrefs.kt`）。
//!
//! ## 源项目的做法与 PC 端的对应
//!
//! | Android | PC |
//! |---|---|
//! | `MasterKeys.getOrCreate(AES256_GCM_SPEC)` + `EncryptedSharedPreferences` | **Windows 凭据管理器**（`keyring` crate，ADR-006） |
//! | 服务名 = 应用包名 | 服务名 = `com.w2018.civilcalc.pc` |
//!
//! ## 存了哪些键（逐字对齐源项目）
//!
//! | 键 | 内容 |
//! |---|---|
//! | `llm_config` | `LlmConfig` 的 JSON（档位列表 + 活跃 id） |
//! | `profile_{id}` | `LlmProfile` 的 JSON |
//! | `profile_{id}_key` | **该档位的 API Key（明文，仅存于此）** |
//! | `webdav_config` | `WebDavConfig` 的 JSON |
//! | `webdav_password` | **WebDAV 密码（明文，仅存于此）** |
//!
//! ## ⚠️ 凭据审计（源项目 BUG-27）
//!
//! - **API Key 与 WebDAV 密码绝不进 SQLite、绝不进日志、绝不进错误文案**
//! - `llm_profiles` / `webdav_config` 这类结构里**没有**凭据字段
//!   （`LlmProfile` / `WebDavConfig` 的测试里各有一条断言守着）
//! - 只有 `export_api_keys()` 会取出明文，且**仅当用户在备份弹窗里显式勾选**时才写进备份包
//!
//! ## 为什么把存储抽成 trait
//!
//! 直接调 `keyring` 的话，单测会往**用户真实的 Windows 凭据管理器**里写垃圾。
//! [`SecretStore`] 让测试用 [`MemoryStore`]，生产用 [`KeyringStore`]，
//! 17 个业务方法因此全部可测。

use civilcalc_backup::{WebDavConfig, WebDavConnection};
use civilcalc_core::CoreError;
use civilcalc_llm::{LlmConfig, LlmProfile, ResolvedLlmProfile};
use std::collections::HashMap;
use std::sync::Mutex;

/// keyring 服务名（= 应用 `identifier`，见 ADR-017）
pub const SERVICE_NAME: &str = "com.w2018.civilcalc.pc";

/// 键：模型配置总表
pub const KEY_LLM_CONFIG: &str = "llm_config";
/// 键：WebDAV 配置
pub const KEY_WEBDAV_CONFIG: &str = "webdav_config";
/// 键：WebDAV 密码
pub const KEY_WEBDAV_PASSWORD: &str = "webdav_password";

/// 单个档位的配置键
pub fn profile_key(id: &str) -> String {
    format!("profile_{id}")
}

/// 单个档位的 API Key 键
pub fn profile_api_key(id: &str) -> String {
    format!("profile_{id}_key")
}

// =============================================================================
// 存储抽象
// =============================================================================

/// 密钥存储后端。
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, CoreError>;
    fn set(&self, key: &str, value: &str) -> Result<(), CoreError>;
    fn delete(&self, key: &str) -> Result<(), CoreError>;
}

/// 生产实现：Windows 凭据管理器（macOS 钥匙串 / Linux Secret Service 同源）。
pub struct KeyringStore {
    service: String,
}

impl KeyringStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }
}

impl Default for KeyringStore {
    fn default() -> Self {
        Self::new(SERVICE_NAME)
    }
}

impl KeyringStore {
    fn entry(&self, key: &str) -> Result<keyring::Entry, CoreError> {
        keyring::Entry::new(&self.service, key).map_err(|e| CoreError::Storage {
            message: format!("访问系统凭据存储失败: {e}"),
        })
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, key: &str) -> Result<Option<String>, CoreError> {
        match self.entry(key)?.get_password() {
            Ok(v) => Ok(Some(v)),
            // 没有这一条 → 不是错误
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(CoreError::Storage {
                message: format!("读取凭据失败（{key}）: {e}"),
            }),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), CoreError> {
        self.entry(key)?
            .set_password(value)
            .map_err(|e| CoreError::Storage {
                message: format!("写入凭据失败（{key}）: {e}"),
            })
    }

    fn delete(&self, key: &str) -> Result<(), CoreError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) => Ok(()),
            // 本来就没有 → 幂等成功
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(CoreError::Storage {
                message: format!("删除凭据失败（{key}）: {e}"),
            }),
        }
    }
}

/// 测试实现：进程内内存表。
///
/// **不要在生产用** —— 密钥会随进程消失。
#[derive(Default)]
pub struct MemoryStore {
    data: Mutex<HashMap<String, String>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前键数（测试断言用）
    pub fn len(&self) -> usize {
        self.data.lock().map(|d| d.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn keys(&self) -> Vec<String> {
        self.data
            .lock()
            .map(|d| {
                let mut v: Vec<String> = d.keys().cloned().collect();
                v.sort();
                v
            })
            .unwrap_or_default()
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>, CoreError> {
        Ok(self
            .data
            .lock()
            .map_err(|_| CoreError::Storage {
                message: "内存密钥表锁中毒".to_string(),
            })?
            .get(key)
            .cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), CoreError> {
        self.data
            .lock()
            .map_err(|_| CoreError::Storage {
                message: "内存密钥表锁中毒".to_string(),
            })?
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), CoreError> {
        self.data
            .lock()
            .map_err(|_| CoreError::Storage {
                message: "内存密钥表锁中毒".to_string(),
            })?
            .remove(key);
        Ok(())
    }
}

// =============================================================================
// Secrets
// =============================================================================

/// 安全存储门面。业务方法 1:1 对齐源项目 `SecurityRepository` 的 17 个方法。
pub struct Secrets<S: SecretStore = KeyringStore> {
    store: S,
}

impl Default for Secrets<KeyringStore> {
    fn default() -> Self {
        Self::new(KeyringStore::default())
    }
}

impl<S: SecretStore> Secrets<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// 取底层存储（测试用）
    pub fn store(&self) -> &S {
        &self.store
    }

    // ---------------------------------------------------------------- 底层读写

    /// 读原始值
    pub fn get(&self, key: &str) -> Result<Option<String>, CoreError> {
        self.store.get(key)
    }

    /// 写原始值
    pub fn set(&self, key: &str, value: &str) -> Result<(), CoreError> {
        self.store.set(key, value)
    }

    /// 删原始值（幂等）
    pub fn delete(&self, key: &str) -> Result<(), CoreError> {
        self.store.delete(key)
    }

    /// 读 JSON 并反序列化；键不存在时返回 `None`
    fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        key: &str,
        what: &str,
    ) -> Result<Option<T>, CoreError> {
        match self.store.get(key)? {
            None => Ok(None),
            Some(raw) if raw.trim().is_empty() => Ok(None),
            Some(raw) => serde_json::from_str(&raw)
                .map(Some)
                .map_err(|e| CoreError::Parse {
                    message: format!("解析{what}失败: {e}"),
                }),
        }
    }

    fn set_json<T: serde::Serialize>(
        &self,
        key: &str,
        value: &T,
        what: &str,
    ) -> Result<(), CoreError> {
        let raw = serde_json::to_string(value).map_err(|e| CoreError::Parse {
            message: format!("序列化{what}失败: {e}"),
        })?;
        self.store.set(key, &raw)
    }

    // ================================================================
    // 1. saveProfile
    // ================================================================

    /// 保存档位配置**并同时写入 API Key**（源项目 `saveProfile`）
    pub fn save_profile(&self, profile: &LlmProfile, api_key: &str) -> Result<(), CoreError> {
        self.set_json(&profile_key(&profile.id), profile, "模型配置")?;
        self.store.set(&profile_api_key(&profile.id), api_key)
    }

    // ================================================================
    // 2. updateProfile
    // ================================================================

    /// 只更新档位配置，**不动 API Key**（源项目 `updateProfile`）
    pub fn update_profile(&self, profile: &LlmProfile) -> Result<(), CoreError> {
        self.set_json(&profile_key(&profile.id), profile, "模型配置")
    }

    // ================================================================
    // 3. hasApiKey
    // ================================================================

    /// 是否已配置该档位的 API Key（空白视为未配置；读取失败也返回 `false`，不抛）
    pub fn has_api_key(&self, id: &str) -> bool {
        matches!(self.store.get(&profile_api_key(id)), Ok(Some(k)) if !k.trim().is_empty())
    }

    // ================================================================
    // 4. resolveActiveProfile
    // ================================================================

    /// 解析活跃档位 + API Key，供调用链直接使用。
    ///
    /// 错误文案逐字对齐源项目 `resolveActiveProfile`。
    pub fn resolve_active_profile(&self) -> Result<ResolvedLlmProfile, CoreError> {
        let config: LlmConfig = self
            .get_json(KEY_LLM_CONFIG, "模型配置")?
            .ok_or_else(|| CoreError::NotFound {
                message: "未配置 AI 模型".to_string(),
            })?;

        if config.active.trim().is_empty() {
            return Err(CoreError::NotFound {
                message: "未设置活跃模型".to_string(),
            });
        }

        let profile: LlmProfile = self
            .get_json(&profile_key(&config.active), "模型档位")?
            .ok_or_else(|| CoreError::NotFound {
                message: format!("活跃模型不存在: {}", config.active),
            })?;

        let api_key = self
            .store
            .get(&profile_api_key(&config.active))?
            .filter(|k| !k.is_empty())
            .ok_or_else(|| CoreError::Unauthorized {
                message: "API Key 未设置".to_string(),
            })?;

        if !profile.enabled {
            return Err(CoreError::InvalidArgument {
                message: "模型已禁用，请先在设置中启用".to_string(),
            });
        }

        Ok(ResolvedLlmProfile::from_profile(&profile, api_key))
    }

    // ================================================================
    // 5. deleteProfile
    // ================================================================

    /// 删除档位配置**与其 API Key**
    pub fn delete_profile(&self, id: &str) -> Result<(), CoreError> {
        self.store.delete(&profile_key(id))?;
        self.store.delete(&profile_api_key(id))
    }

    // ================================================================
    // 6. getLlmConfig
    // ================================================================

    /// 读模型配置总表；**未配置时返回内置三家预设**（源项目行为）
    pub fn get_llm_config(&self) -> Result<LlmConfig, CoreError> {
        Ok(self
            .get_json::<LlmConfig>(KEY_LLM_CONFIG, "模型配置")?
            .unwrap_or_default())
    }

    // ================================================================
    // 7. saveLlmConfig
    // ================================================================

    pub fn save_llm_config(&self, config: &LlmConfig) -> Result<(), CoreError> {
        self.set_json(KEY_LLM_CONFIG, config, "模型配置")
    }

    // ================================================================
    // 8. getWebDavConfig
    // ================================================================

    /// 读 WebDAV 配置；**未配置时返回坚果云默认值**（源项目行为）
    pub fn get_webdav_config(&self) -> Result<WebDavConfig, CoreError> {
        Ok(self
            .get_json::<WebDavConfig>(KEY_WEBDAV_CONFIG, "WebDAV 配置")?
            .unwrap_or_default())
    }

    // ================================================================
    // 9. saveWebDavConfig
    // ================================================================

    /// 保存 WebDAV 配置。`password = None` 表示**不改动已有密码**。
    pub fn save_webdav_config(
        &self,
        config: &WebDavConfig,
        password: Option<&str>,
    ) -> Result<(), CoreError> {
        self.set_json(KEY_WEBDAV_CONFIG, config, "WebDAV 配置")?;
        if let Some(pw) = password {
            self.store.set(KEY_WEBDAV_PASSWORD, pw)?;
        }
        Ok(())
    }

    // ================================================================
    // 10. resolveWebDavConnection
    // ================================================================

    /// 解析出带密码的连接信息（**只在内存里**）
    pub fn resolve_webdav_connection(&self) -> Result<WebDavConnection, CoreError> {
        let config = self.get_webdav_config()?;
        if !config.is_usable() {
            return Err(CoreError::InvalidArgument {
                message: "WebDAV 服务器地址未配置或格式不正确".to_string(),
            });
        }
        let password = self
            .store
            .get(KEY_WEBDAV_PASSWORD)?
            .filter(|p| !p.is_empty())
            .ok_or_else(|| CoreError::Unauthorized {
                message: "WebDAV 密码未设置".to_string(),
            })?;
        Ok(WebDavConnection::new(config, password))
    }

    // ================================================================
    // 11. hasWebDavPassword
    // ================================================================

    pub fn has_webdav_password(&self) -> bool {
        matches!(self.store.get(KEY_WEBDAV_PASSWORD), Ok(Some(p)) if !p.trim().is_empty())
    }

    // ================================================================
    // 12. exportLlmConfigJson
    // ================================================================

    /// 导出 `llm_config` 的**原始 JSON**（备份包用 `llmConfigJson` 字段原样搬运）。
    ///
    /// 为什么存原始字符串而不是解析后重新序列化：见 `BackupContent.llmConfigJson`
    /// 的源项目注释 —— 「原样搬运才能保证导入后设置页与生成链路完全一致」。
    pub fn export_llm_config_json(&self) -> Result<Option<String>, CoreError> {
        match self.store.get(KEY_LLM_CONFIG)? {
            Some(s) if !s.trim().is_empty() => Ok(Some(s)),
            _ => Ok(None),
        }
    }

    // ================================================================
    // 13. importLlmConfigJson
    // ================================================================

    /// 导入 `llm_config` 原始 JSON。`None` / 空串表示"包里没带"，**不改动本机**。
    ///
    /// 会先校验能被解析为 `LlmConfig`，避免把坏数据写进去。
    pub fn import_llm_config_json(&self, json: Option<&str>) -> Result<(), CoreError> {
        let Some(raw) = json.filter(|s| !s.trim().is_empty()) else {
            return Ok(());
        };
        // 先试解析（坏数据不落盘）
        let _: LlmConfig = serde_json::from_str(raw).map_err(|e| CoreError::Parse {
            message: format!("备份包中的模型配置无法解析: {e}"),
        })?;
        self.store.set(KEY_LLM_CONFIG, raw)
    }

    // ================================================================
    // 14. exportApiKeys
    // ================================================================

    /// 导出全部档位的 API Key（`{档位id: key}`）。
    ///
    /// ⚠️ **只在用户勾选「含 API Key」时才调用** —— 返回值是明文。
    pub fn export_api_keys(&self) -> Result<HashMap<String, String>, CoreError> {
        let config = self.get_llm_config()?;
        let mut out = HashMap::new();
        for p in &config.profiles {
            if let Some(k) = self.store.get(&profile_api_key(&p.id))? {
                if !k.is_empty() {
                    out.insert(p.id.clone(), k);
                }
            }
        }
        Ok(out)
    }

    // ================================================================
    // 15. importApiKeys
    // ================================================================

    /// 导入 API Key 表（备份包 `secrets.apiKeys`）。
    ///
    /// 只写入**本机已存在的档位**，避免备份包带进未知档位；
    /// 空值跳过（不覆盖本机已有的 Key）。
    pub fn import_api_keys(&self, keys: &HashMap<String, String>) -> Result<usize, CoreError> {
        let config = self.get_llm_config()?;
        let mut n = 0;
        for (id, key) in keys {
            if key.is_empty() {
                continue;
            }
            if config.profile(id).is_none() {
                civilcalc_core::log::w(
                    "Secrets",
                    &format!("备份包含未知档位的 API Key，已跳过: {id}"),
                    None,
                );
                continue;
            }
            self.store.set(&profile_api_key(id), key)?;
            n += 1;
        }
        Ok(n)
    }

    // ================================================================
    // 16. clearLlmConfig
    // ================================================================

    /// 清空模型配置**与全部 API Key**（"重置软件"用）。
    ///
    /// 返回删除的键数。
    pub fn clear_llm_config(&self) -> Result<usize, CoreError> {
        // 先按已记录的档位删，再删总表（顺序无关，但先删具体项更稳）
        let mut n = 0;
        if let Ok(config) = self.get_llm_config() {
            for p in &config.profiles {
                self.store.delete(&profile_key(&p.id))?;
                self.store.delete(&profile_api_key(&p.id))?;
                n += 2;
            }
        }
        self.store.delete(KEY_LLM_CONFIG)?;
        Ok(n + 1)
    }

    // ================================================================
    // 17. clearWebDavConfig
    // ================================================================

    /// 清空 WebDAV 配置**与密码**
    pub fn clear_webdav_config(&self) -> Result<usize, CoreError> {
        self.store.delete(KEY_WEBDAV_CONFIG)?;
        self.store.delete(KEY_WEBDAV_PASSWORD)?;
        Ok(2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secrets() -> Secrets<MemoryStore> {
        Secrets::new(MemoryStore::new())
    }

    fn profile(id: &str) -> LlmProfile {
        LlmProfile::new(id, "标签", "https://api.deepseek.com/v1", "m")
    }

    // ---------------- 底层读写 ----------------

    #[test]
    fn store_roundtrip_and_delete_is_idempotent() {
        let s = secrets();
        assert_eq!(s.get("k").unwrap(), None);

        s.set("k", "v").unwrap();
        assert_eq!(s.get("k").unwrap().as_deref(), Some("v"));

        s.delete("k").unwrap();
        assert_eq!(s.get("k").unwrap(), None);
        // 幂等
        s.delete("k").unwrap();
    }

    // ---------------- 1/2/3: profile 保存与 Key ----------------

    #[test]
    fn save_profile_writes_both_config_and_key() {
        let s = secrets();
        s.save_profile(&profile("deepseek"), "sk-xxx").unwrap();

        assert_eq!(s.store().keys(), vec!["profile_deepseek", "profile_deepseek_key"]);
        assert!(s.has_api_key("deepseek"));
    }

    #[test]
    fn update_profile_does_not_touch_key() {
        let s = secrets();
        s.save_profile(&profile("d"), "sk-original").unwrap();

        let mut p = profile("d");
        p.model = "新模型".to_string();
        s.update_profile(&p).unwrap();

        // Key 未被改动
        assert_eq!(
            s.get(&profile_api_key("d")).unwrap().as_deref(),
            Some("sk-original")
        );
        // 配置已更新
        let got: LlmProfile = s.get_json(&profile_key("d"), "配置").unwrap().unwrap();
        assert_eq!(got.model, "新模型");
    }

    #[test]
    fn has_api_key_false_for_missing_and_blank() {
        let s = secrets();
        assert!(!s.has_api_key("nope"), "不存在 → false");

        s.save_profile(&profile("d"), "").unwrap();
        assert!(!s.has_api_key("d"), "空串 → false");

        s.save_profile(&profile("d"), "   ").unwrap();
        assert!(!s.has_api_key("d"), "全空白 → false");
    }

    #[test]
    fn delete_profile_removes_both() {
        let s = secrets();
        s.save_profile(&profile("d"), "sk").unwrap();
        s.delete_profile("d").unwrap();
        assert!(s.store().is_empty());
    }

    // ---------------- 4: resolveActiveProfile ----------------

    #[test]
    fn resolve_active_profile_success() {
        let s = secrets();
        s.save_profile(&profile("deepseek"), "sk-abcdefghijklmnop").unwrap();
        s.save_llm_config(&LlmConfig::default()).unwrap();

        let r = s.resolve_active_profile().unwrap();
        assert_eq!(r.model, "m");
        assert_eq!(r.api_key, "sk-abcdefghijklmnop");
        assert_eq!(r.base_url, "https://api.deepseek.com/v1");
    }

    #[test]
    fn resolve_active_profile_error_messages_match_source() {
        // ① 未配置模型
        let s = secrets();
        let e = s.resolve_active_profile().unwrap_err();
        assert_eq!(e.to_string(), "未配置 AI 模型");

        // ② 活跃 id 指向不存在的档位
        let s = secrets();
        let cfg = LlmConfig {
            active: "ghost".to_string(),
            ..Default::default()
        };
        s.save_llm_config(&cfg).unwrap();
        let e = s.resolve_active_profile().unwrap_err();
        assert_eq!(e.to_string(), "活跃模型不存在: ghost");

        // ③ 档位存在但没 Key
        let s = secrets();
        s.update_profile(&profile("deepseek")).unwrap();
        s.save_llm_config(&LlmConfig::default()).unwrap();
        let e = s.resolve_active_profile().unwrap_err();
        assert_eq!(e.to_string(), "API Key 未设置");
        assert_eq!(e.code(), "UNAUTHORIZED");

        // ④ 档位被禁用
        let s = secrets();
        let mut p = profile("deepseek");
        p.enabled = false;
        s.save_profile(&p, "sk-abcdefghijklmnop").unwrap();
        s.save_llm_config(&LlmConfig::default()).unwrap();
        let e = s.resolve_active_profile().unwrap_err();
        assert_eq!(e.to_string(), "模型已禁用，请先在设置中启用");
    }

    #[test]
    fn resolve_active_profile_requires_non_blank_active_id() {
        let s = secrets();
        let cfg = LlmConfig {
            active: "   ".to_string(),
            ..Default::default()
        };
        s.save_llm_config(&cfg).unwrap();
        let e = s.resolve_active_profile().unwrap_err();
        assert_eq!(e.to_string(), "未设置活跃模型");
    }

    /// **凭据审计**：解析出的结构 `Debug` 不得含明文 Key
    #[test]
    fn resolved_profile_debug_masks_key() {
        let s = secrets();
        s.save_profile(&profile("deepseek"), "sk-super-secret-value").unwrap();
        s.save_llm_config(&LlmConfig::default()).unwrap();

        let r = s.resolve_active_profile().unwrap();
        let dbg = format!("{r:?}");
        assert!(!dbg.contains("sk-super-secret-value"), "Debug 泄漏 Key: {dbg}");
        assert!(dbg.contains("sk-s"), "应保留脱敏前缀");
    }

    // ---------------- 6/7: llm_config ----------------

    #[test]
    fn get_llm_config_defaults_to_three_presets() {
        let s = secrets();
        let c = s.get_llm_config().unwrap();
        assert_eq!(c.profiles.len(), 3);
        assert_eq!(c.active, "deepseek");
    }

    #[test]
    fn llm_config_roundtrip() {
        let s = secrets();
        let c = LlmConfig {
            active: "glm".to_string(),
            ..Default::default()
        };
        s.save_llm_config(&c).unwrap();
        assert_eq!(s.get_llm_config().unwrap().active, "glm");
    }

    // ---------------- 8/9/10/11: WebDAV ----------------

    #[test]
    fn get_webdav_config_defaults_to_nutstore() {
        let s = secrets();
        let c = s.get_webdav_config().unwrap();
        assert_eq!(c.base_url, "https://dav.jianguoyun.com/dav");
        assert_eq!(c.remote_dir, "civilcalc");
    }

    #[test]
    fn save_webdav_config_none_keeps_existing_password() {
        let s = secrets();
        let cfg = WebDavConfig::default();
        s.save_webdav_config(&cfg, Some("pw1")).unwrap();
        assert!(s.has_webdav_password());

        // password = None → 不改密码
        s.save_webdav_config(&cfg, None).unwrap();
        assert_eq!(
            s.get(KEY_WEBDAV_PASSWORD).unwrap().as_deref(),
            Some("pw1")
        );
    }

    #[test]
    fn resolve_webdav_connection_success() {
        let s = secrets();
        let cfg = WebDavConfig {
            username: "me@example.com".to_string(),
            ..Default::default()
        };
        s.save_webdav_config(&cfg, Some("app-pw")).unwrap();

        let c = s.resolve_webdav_connection().unwrap();
        assert_eq!(c.username(), "me@example.com");
        assert_eq!(c.password, "app-pw");
        assert_eq!(c.dir_url(), "https://dav.jianguoyun.com/dav/civilcalc/");
    }

    #[test]
    fn resolve_webdav_connection_errors() {
        // 地址不可用
        let s = secrets();
        let cfg = WebDavConfig {
            base_url: "not-a-url".to_string(),
            ..Default::default()
        };
        s.save_webdav_config(&cfg, Some("pw")).unwrap();
        assert_eq!(
            s.resolve_webdav_connection().unwrap_err().to_string(),
            "WebDAV 服务器地址未配置或格式不正确"
        );

        // 缺密码
        let s = secrets();
        s.save_webdav_config(&WebDavConfig::default(), None).unwrap();
        let e = s.resolve_webdav_connection().unwrap_err();
        assert_eq!(e.to_string(), "WebDAV 密码未设置");
        assert_eq!(e.code(), "UNAUTHORIZED");
    }

    #[test]
    fn has_webdav_password_false_when_blank() {
        let s = secrets();
        assert!(!s.has_webdav_password());
        s.save_webdav_config(&WebDavConfig::default(), Some("  "))
            .unwrap();
        assert!(!s.has_webdav_password(), "全空白视为未设置");
    }

    // ---------------- 12/13: llm_config JSON 原样搬运 ----------------

    #[test]
    fn export_llm_config_json_is_verbatim() {
        let s = secrets();
        // 故意写一个字段顺序非字典序、含未知字段的 JSON
        let raw = r#"{"active":"deepseek","profiles":[],"zzz":1}"#;
        s.set(KEY_LLM_CONFIG, raw).unwrap();

        assert_eq!(
            s.export_llm_config_json().unwrap().as_deref(),
            Some(raw),
            "必须逐字节原样导出（备份包原样搬运）"
        );
    }

    #[test]
    fn export_llm_config_json_none_when_absent_or_blank() {
        let s = secrets();
        assert_eq!(s.export_llm_config_json().unwrap(), None);

        s.set(KEY_LLM_CONFIG, "   ").unwrap();
        assert_eq!(s.export_llm_config_json().unwrap(), None);
    }

    #[test]
    fn import_llm_config_json_none_is_noop() {
        let s = secrets();
        s.set(KEY_LLM_CONFIG, "keep-me").unwrap();

        s.import_llm_config_json(None).unwrap();
        s.import_llm_config_json(Some("")).unwrap();
        s.import_llm_config_json(Some("   ")).unwrap();

        assert_eq!(s.get(KEY_LLM_CONFIG).unwrap().as_deref(), Some("keep-me"));
    }

    #[test]
    fn import_llm_config_json_rejects_bad_data_without_writing() {
        let s = secrets();
        s.set(KEY_LLM_CONFIG, "keep-me").unwrap();

        let e = s.import_llm_config_json(Some("{ 坏 JSON")).unwrap_err();
        assert_eq!(e.code(), "PARSE_ERROR");
        assert_eq!(
            s.get(KEY_LLM_CONFIG).unwrap().as_deref(),
            Some("keep-me"),
            "坏数据不得覆盖本机配置"
        );
    }

    #[test]
    fn import_llm_config_json_accepts_valid() {
        let s = secrets();
        let raw = r#"{"active":"glm","profiles":[]}"#;
        s.import_llm_config_json(Some(raw)).unwrap();
        assert_eq!(s.get(KEY_LLM_CONFIG).unwrap().as_deref(), Some(raw));
        assert_eq!(s.get_llm_config().unwrap().active, "glm");
    }

    // ---------------- 14/15: API Key 导出/导入 ----------------

    #[test]
    fn export_api_keys_only_non_empty() {
        let s = secrets();
        s.save_profile(&profile("deepseek"), "sk-1").unwrap();
        s.save_profile(&profile("mimo"), "").unwrap();
        s.save_profile(&profile("glm"), "sk-3").unwrap();
        s.save_llm_config(&LlmConfig::default()).unwrap();

        let keys = s.export_api_keys().unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys.get("deepseek").map(String::as_str), Some("sk-1"));
        assert_eq!(keys.get("glm").map(String::as_str), Some("sk-3"));
        assert!(!keys.contains_key("mimo"), "空 Key 不导出");
    }

    #[test]
    fn import_api_keys_skips_unknown_profiles_and_blanks() {
        let s = secrets();
        s.save_llm_config(&LlmConfig::default()).unwrap();

        let keys = HashMap::from([
            ("deepseek".to_string(), "sk-new".to_string()),
            ("ghost".to_string(), "sk-ghost".to_string()),
            ("glm".to_string(), String::new()),
        ]);
        let n = s.import_api_keys(&keys).unwrap();
        assert_eq!(n, 1, "只应写入已存在的档位且非空");

        assert_eq!(
            s.get(&profile_api_key("deepseek")).unwrap().as_deref(),
            Some("sk-new")
        );
        assert_eq!(
            s.get(&profile_api_key("ghost")).unwrap(),
            None,
            "未知档位不得写入"
        );
    }

    #[test]
    fn import_api_keys_does_not_overwrite_with_empty() {
        let s = secrets();
        s.save_llm_config(&LlmConfig::default()).unwrap();
        s.save_profile(&profile("deepseek"), "sk-original").unwrap();

        let keys = HashMap::from([("deepseek".to_string(), String::new())]);
        s.import_api_keys(&keys).unwrap();
        assert_eq!(
            s.get(&profile_api_key("deepseek")).unwrap().as_deref(),
            Some("sk-original"),
            "空值不应覆盖本机已有 Key"
        );
    }

    // ---------------- 16/17: 清空 ----------------

    #[test]
    fn clear_llm_config_removes_profiles_and_keys() {
        let s = secrets();
        s.save_profile(&profile("deepseek"), "sk").unwrap();
        s.save_llm_config(&LlmConfig::default()).unwrap();
        assert!(!s.store().is_empty());

        s.clear_llm_config().unwrap();

        assert_eq!(s.get(KEY_LLM_CONFIG).unwrap(), None);
        assert_eq!(s.get(&profile_key("deepseek")).unwrap(), None);
        assert_eq!(s.get(&profile_api_key("deepseek")).unwrap(), None);
        assert!(!s.has_api_key("deepseek"));
    }

    #[test]
    fn clear_webdav_config_removes_config_and_password() {
        let s = secrets();
        s.save_webdav_config(&WebDavConfig::default(), Some("pw"))
            .unwrap();

        s.clear_webdav_config().unwrap();

        assert_eq!(s.get(KEY_WEBDAV_CONFIG).unwrap(), None);
        assert_eq!(s.get(KEY_WEBDAV_PASSWORD).unwrap(), None);
        assert!(!s.has_webdav_password());
    }

    /// 清空 LLM 配置**不应**影响 WebDAV（反之亦然）
    #[test]
    fn clears_are_isolated() {
        let s = secrets();
        s.save_profile(&profile("deepseek"), "sk").unwrap();
        s.save_llm_config(&LlmConfig::default()).unwrap();
        s.save_webdav_config(&WebDavConfig::default(), Some("pw"))
            .unwrap();

        s.clear_llm_config().unwrap();
        assert!(s.has_webdav_password(), "清 LLM 不该动 WebDAV");
        assert!(s.get(KEY_WEBDAV_CONFIG).unwrap().is_some());

        s.clear_webdav_config().unwrap();
        assert_eq!(s.get(KEY_WEBDAV_PASSWORD).unwrap(), None);
    }

    // ---------------- 键名对齐源项目 ----------------

    #[test]
    fn key_names_match_source() {
        assert_eq!(KEY_LLM_CONFIG, "llm_config");
        assert_eq!(KEY_WEBDAV_CONFIG, "webdav_config");
        assert_eq!(KEY_WEBDAV_PASSWORD, "webdav_password");
        assert_eq!(profile_key("deepseek"), "profile_deepseek");
        assert_eq!(profile_api_key("deepseek"), "profile_deepseek_key");
        assert_eq!(SERVICE_NAME, "com.w2018.civilcalc.pc");
    }
}
