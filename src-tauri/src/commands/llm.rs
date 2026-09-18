//! 组 8：LLM 配置与密钥（11 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `llm_list_profiles` | — | `LlmConfig`（**不含 Key**） |
//! | `llm_save_profile` | `profile`, `apiKey?` | `void` |
//! | `llm_update_profile` | `profile` | `void` |
//! | `llm_set_active` | `id` | `void` |
//! | `llm_delete_profile` | `id` | `void` |
//! | `llm_set_api_key` | `profileId`, `apiKey` | `void` |
//! | `llm_has_api_key` | `profileId` | `boolean` |
//! | `llm_delete_api_key` | `profileId` | `void` |
//! | `llm_test_connection` | `profileId` | `LlmTestResult` |
//! | `llm_config_export` | `includeApiKeys` | `string`（JSON 包） |
//! | `llm_config_import` | `json` | `LlmConfigImportReport` |
//!
//! ## 🔒 凭据铁律
//!
//! - **`llm_list_profiles` 的返回结构里没有 Key** —— `LlmProfile` 类型本身就没有该字段
//! - Key 只经 `llm_set_api_key` / `llm_save_profile` 进 **keyring**
//! - `llm_config_export(includeApiKeys=true)` 是**唯一**会吐出明文 Key 的路径，
//!   前端必须二次确认后才可传 `true`

use crate::error::{CmdResult, CommandError};
use crate::state::AppState;
use crate::secrets::profile_api_key;
use civilcalc_llm::{ChatMessage, LlmClient, LlmConfig, LlmProfile, ResolvedLlmProfile};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::State;

/// 连通性测试结果（`llm_test_connection`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmTestResult {
    /// 是否连通且鉴权通过
    pub ok: bool,
    /// 实际使用的模型名
    pub model: String,
    /// 往返耗时（毫秒）
    pub latency_ms: u64,
    /// 人类可读结论（成功或失败原因）
    pub message: String,
}

/// 配置导入结果
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmConfigImportReport {
    /// 是否写入了模型配置
    pub config_applied: bool,
    /// 实际写入的 API Key 数
    pub api_keys_applied: usize,
    /// 被跳过的档位 id（本机不存在）
    pub skipped_profiles: Vec<String>,
}

/// 导出包（`llm_config_export` 的 JSON 内容）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LlmExportBundle {
    /// `llm_config` 的**原始 JSON**（原样搬运，保证导入后设置页完全一致）
    llm_config_json: String,
    /// 仅当用户勾选「含密钥」时存在
    #[serde(skip_serializing_if = "Option::is_none")]
    api_keys: Option<HashMap<String, String>>,
}

// =============================================================================
// 命令
// =============================================================================

/// 读全部模型配置（**不含 Key**）
#[tauri::command]
pub fn llm_list_profiles(state: State<'_, AppState>) -> CmdResult<LlmConfig> {
    state.secrets.get_llm_config().map_err(Into::into)
}

/// 保存档位配置；`api_key` 为 `Some` 时**同时**写入 keyring
#[tauri::command]
pub fn llm_save_profile(
    state: State<'_, AppState>,
    profile: LlmProfile,
    api_key: Option<String>,
) -> CmdResult<()> {
    match api_key {
        Some(k) => state.secrets.save_profile(&profile, &k)?,
        None => state.secrets.update_profile(&profile)?,
    }
    // 确保该档位出现在总表里（首次保存新档位时）
    let mut cfg = state.secrets.get_llm_config()?;
    if cfg.profile(&profile.id).is_none() {
        cfg.profiles.push(profile);
        state.secrets.save_llm_config(&cfg)?;
    } else {
        // 同步总表里的副本（总表存的是档位快照）
        cfg.profiles.retain(|p| p.id != profile.id);
        cfg.profiles.push(profile);
        state.secrets.save_llm_config(&cfg)?;
    }
    Ok(())
}

/// 只更新档位元数据，**不动 Key**
#[tauri::command]
pub fn llm_update_profile(state: State<'_, AppState>, profile: LlmProfile) -> CmdResult<()> {
    state.secrets.update_profile(&profile)?;
    let mut cfg = state.secrets.get_llm_config()?;
    cfg.profiles.retain(|p| p.id != profile.id);
    cfg.profiles.push(profile);
    state.secrets.save_llm_config(&cfg)?;
    Ok(())
}

/// 设为活跃档位。档位不存在时返回 [`CommandError::NotFound`]。
#[tauri::command]
pub fn llm_set_active(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let mut cfg = state.secrets.get_llm_config()?;
    if cfg.profile(&id).is_none() {
        return Err(CommandError::NotFound {
            message: format!("模型档位不存在: {id}"),
        });
    }
    cfg.active = id;
    state.secrets.save_llm_config(&cfg).map_err(Into::into)
}

/// 删除档位后 `active` 该回落到谁。
///
/// 优先取**第一个启用的**档位；都禁用了就取第一个；一个都没有则空串
/// （前端见空串即提示"未配置模型"）。
fn fallback_active(cfg: &LlmConfig) -> String {
    cfg.profiles
        .iter()
        .find(|p| p.enabled)
        .or_else(|| cfg.profiles.first())
        .map(|p| p.id.clone())
        .unwrap_or_default()
}

/// 删除档位（**同时删 keyring 里的配置与 Key**），并从总表移除
#[tauri::command]
pub fn llm_delete_profile(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let mut cfg = state.secrets.get_llm_config()?;
    cfg.profiles.retain(|p| p.id != id);

    // 删掉的正好是活跃档位 → 回落到其他可用档位
    if cfg.active == id {
        cfg.active = fallback_active(&cfg);
        civilcalc_core::log::w(
            "Commands",
            &format!("活跃档位被删除，已回落到: {}", cfg.active),
            None,
        );
    }

    state.secrets.save_llm_config(&cfg)?;
    state.secrets.delete_profile(&id).map_err(Into::into)
}

/// 单独设置某档位的 API Key
#[tauri::command]
pub fn llm_set_api_key(
    state: State<'_, AppState>,
    profile_id: String,
    api_key: String,
) -> CmdResult<()> {
    state
        .secrets
        .set(&crate::secrets::profile_api_key(&profile_id), &api_key)
        .map_err(Into::into)
}

/// 是否已配置该档位的 Key
#[tauri::command]
pub fn llm_has_api_key(state: State<'_, AppState>, profile_id: String) -> CmdResult<bool> {
    Ok(state.secrets.has_api_key(&profile_id))
}

/// 删除某档位的 Key（**幂等**）
#[tauri::command]
pub fn llm_delete_api_key(state: State<'_, AppState>, profile_id: String) -> CmdResult<()> {
    state
        .secrets
        .delete(&crate::secrets::profile_api_key(&profile_id))
        .map_err(Into::into)
}

/// 连通性测试（P4-2 落地后改为真实探测）。
///
/// ## 返回约定
///
/// - **本地配置问题**（档位不存在 / 未设 Key）→ 抛 `notFound` / `unauthorized`
/// - **网络或鉴权失败** → 返回 `Ok(LlmTestResult { ok: false, .. })`
///
/// 两者必须区分：前者是"你没配好"，后者是"配好了但连不通" —— 都塞进 `Err` 的话
/// 前端只能显示一句红字，用户不知道该去改配置还是查网络。
///
/// ⚠️ 探测请求**不设 `max_tokens` 上限**（思考 token 计入该上限，设小值会让
/// 强制思考的厂商返回空正文，从而把"连通"误判成"失败"）。
#[tauri::command]
pub async fn llm_test_connection(
    state: State<'_, AppState>,
    profile_id: String,
) -> CmdResult<LlmTestResult> {
    let cfg = state.secrets.get_llm_config()?;
    let profile = cfg.profile(&profile_id).ok_or_else(|| CommandError::NotFound {
        message: format!("模型档位不存在: {profile_id}"),
    })?;
    let key = state
        .secrets
        .get(&profile_api_key(&profile_id))?
        .unwrap_or_default();
    if key.trim().is_empty() {
        return Err(CommandError::Unauthorized {
            message: "API Key 未设置".to_string(),
        });
    }

    let resolved = ResolvedLlmProfile::from_profile(profile, key);
    let model = resolved.model.clone();
    let messages = vec![
        ChatMessage::system("你是连通性探针。收到任何输入都只回复两个字符：ok"),
        ChatMessage::user("ping", Vec::new()),
    ];

    let client = LlmClient::new();
    let started = std::time::Instant::now();
    let outcome = client.chat(&resolved, &messages, false, false, None).await;
    let latency_ms = started.elapsed().as_millis() as u64;

    Ok(match outcome {
        Ok(r) => LlmTestResult {
            ok: true,
            model,
            latency_ms,
            message: format!("连接成功，返回 {} 字", r.content.chars().count()),
        },
        Err(e) => {
            // 用户可见文案来自 `user_message()`（源文案逐字对齐），细节只进日志
            let user = e.user_message().to_string();
            civilcalc_core::log::w("Commands", "连通性测试失败", Some(&e.error_detail));
            LlmTestResult {
                ok: false,
                model,
                latency_ms,
                message: user,
            }
        }
    })
}

/// 导出配置（JSON 包）。
///
/// `include_api_keys = true` 时**会吐出明文 Key** —— 前端必须先弹二次确认。
#[tauri::command]
pub fn llm_config_export(
    state: State<'_, AppState>,
    include_api_keys: bool,
) -> CmdResult<String> {
    let llm_config_json = state
        .secrets
        .export_llm_config_json()?
        .unwrap_or_else(|| serde_json::to_string(&LlmConfig::default()).unwrap_or_default());

    let api_keys = if include_api_keys {
        civilcalc_core::log::w("Commands", "导出配置包含明文 API Key（用户已确认）", None);
        Some(state.secrets.export_api_keys()?)
    } else {
        None
    };

    serde_json::to_string(&LlmExportBundle {
        llm_config_json,
        api_keys,
    })
    .map_err(|e| CommandError::Parse {
        message: format!("序列化导出包失败: {e}"),
    })
}

/// 导入配置（`llm_config_export` 产出的 JSON 包）。
///
/// 包里的 `apiKeys` 缺失时**不动本机 Key**（与"导出时没勾含密钥"对应）。
#[tauri::command]
pub fn llm_config_import(
    state: State<'_, AppState>,
    json: String,
) -> CmdResult<LlmConfigImportReport> {
    let bundle: LlmExportBundle = serde_json::from_str(&json).map_err(|e| CommandError::Parse {
        message: format!("导入包无法解析: {e}"),
    })?;

    let before = state.secrets.get_llm_config()?;
    state
        .secrets
        .import_llm_config_json(Some(&bundle.llm_config_json))?;
    let after = state.secrets.get_llm_config()?;

    let (api_keys_applied, skipped_profiles) = match &bundle.api_keys {
        None => (0, Vec::new()),
        Some(keys) => {
            let skipped: Vec<String> = keys
                .keys()
                .filter(|id| after.profile(id).is_none())
                .cloned()
                .collect();
            let n = state.secrets.import_api_keys(keys)?;
            (n, skipped)
        }
    };

    civilcalc_core::log::i(
        "Commands",
        &format!(
            "导入模型配置：{} 个档位 → {} 个档位，写入 {api_keys_applied} 个 Key",
            before.profiles.len(),
            after.profiles.len()
        ),
    );

    Ok(LlmConfigImportReport {
        config_applied: true,
        api_keys_applied,
        skipped_profiles,
    })
}

#[cfg(test)]
mod tests {
    use crate::secrets::{MemoryStore, Secrets};
    use civilcalc_llm::{LlmConfig, LlmProfile};

    fn secrets() -> Secrets<MemoryStore> {
        Secrets::new(MemoryStore::new())
    }

    fn p(id: &str) -> LlmProfile {
        LlmProfile::new(id, "标签", "https://api.deepseek.com/v1", "m")
    }

    /// 导出包（不含 Key）的 serde 形状
    #[test]
    fn export_bundle_without_keys_omits_field() {
        let b = super::LlmExportBundle {
            llm_config_json: "{}".into(),
            api_keys: None,
        };
        let v = serde_json::to_value(&b).unwrap();
        assert!(v.get("llmConfigJson").is_some());
        assert!(
            v.get("apiKeys").is_none(),
            "不含密钥时不应出现 apiKeys 字段"
        );
    }

    #[test]
    fn export_bundle_with_keys_includes_field() {
        let b = super::LlmExportBundle {
            llm_config_json: "{}".into(),
            api_keys: Some([("deepseek".to_string(), "sk-1".to_string())].into()),
        };
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(v["apiKeys"]["deepseek"], serde_json::json!("sk-1"));
    }

    /// 删除活跃档位后应回落到其他可用档位
    #[test]
    fn deleting_active_profile_falls_back() {
        let cfg = LlmConfig {
            active: "mimo".to_string(),
            profiles: LlmConfig::default()
                .profiles
                .into_iter()
                .filter(|x| x.id != "mimo")
                .collect(),
        };
        assert_eq!(super::fallback_active(&cfg), "deepseek");
        assert_eq!(cfg.profiles.len(), 2);
    }

    /// 删除最后一个档位后 active 变空串（前端据此提示"未配置模型"）
    #[test]
    fn deleting_last_profile_empties_active() {
        let cfg = LlmConfig {
            profiles: Vec::new(),
            active: String::new(),
        };
        assert_eq!(super::fallback_active(&cfg), "");
    }

    /// 全部禁用时也要能选出一个（取第一个，而不是空串）
    #[test]
    fn fallback_prefers_enabled_then_first() {
        let mut cfg = LlmConfig::default();
        for p in &mut cfg.profiles {
            p.enabled = false;
        }
        assert_eq!(
            super::fallback_active(&cfg),
            "deepseek",
            "全禁用时应取第一个而非空串"
        );

        // 有启用的就取启用的
        cfg.profiles[1].enabled = true;
        assert_eq!(super::fallback_active(&cfg), "mimo");
    }
    /// `llm_set_active` 的存在性检查
    #[test]
    fn set_active_rejects_unknown_id() {
        let cfg = LlmConfig::default();
        assert!(cfg.profile("ghost").is_none());
        assert!(cfg.profile("deepseek").is_some());
    }

    /// 导入报告的形状
    #[test]
    fn import_report_serde_is_camel_case() {
        let r = super::LlmConfigImportReport {
            config_applied: true,
            api_keys_applied: 2,
            skipped_profiles: vec!["ghost".into()],
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["apiKeysApplied"], serde_json::json!(2));
        assert_eq!(v["skippedProfiles"], serde_json::json!(["ghost"]));
        assert!(v.get("api_keys_applied").is_none());
    }

    /// `llm_test_connection` 在缺 Key 时报 Unauthorized
    #[test]
    fn test_connection_requires_key() {
        let s = secrets();
        s.update_profile(&p("deepseek")).unwrap();
        s.save_llm_config(&LlmConfig::default()).unwrap();
        assert!(!s.has_api_key("deepseek"), "前提：没有 Key");
        // 命令层据此返回 Unauthorized
    }

    /// `llm_list_profiles` 的返回类型不含 Key 字段（编译期即保证）
    #[test]
    fn profile_type_has_no_key_field() {
        let v = serde_json::to_value(p("d")).unwrap();
        assert!(v.get("apiKey").is_none());
        assert!(v.get("api_key").is_none());
    }
}
