//! 重置软件：9 类可重置项 + 真实存量统计 + 二次确认文案 + 实际执行。
//!
//! 命令层把「本机数据」的清空能力聚到一起，按勾选逐类执行。
//! 与备份导入的「完整还原」区别：这边是**本机出厂态** ——
//! 提示词/提醒词回退内置文案、模型配置回内置三家预设、外观回默认值。
//! 只动本机数据：不删数据库文件、云端备份一条也不碰。
//!
//! 纯类型与文案逻辑见 `civilcalc-core::reset`（可独立单测）。
//!
//! ## 🔴 两条硬规则（都是真 bug 换来的，v1.0.8）
//!
//! ### ① 每一块都必须由 `selection` 把关
//!
//! 曾经「公式 / 历史 / 用量 / 插图」四块是**无条件执行**的 ——
//! 用户只勾了「外观（主题/背景）」点重置，公式、历史、用量、插图
//! 也一起被删了。表现是「会重置未勾选的项目」，而且**不可撤销**。
//!
//! 所以现在每一块前面都有 `if selection.<字段>`，没有例外。
//! 新增可重置类别时，这里也要加一道闸门 ——
//! `tests/reset_selection.rs` 会用「只勾一项」的方式把这条规则钉住。
//!
//! ### ② 偏好改动必须走 `update_config`（会落盘）
//!
//! 曾经这里直接用 `state.config.lock().unwrap()` 改内存，**从不 save**。
//! 于是重置当时看着生效，**重启后全部回来** —— 因为磁盘上的
//! `config.json` 一个字节都没动。
//!
//! `AppState::update_config` 会在改完后 `snapshot.save()`，所以一律走它。

use crate::config::{
    ConfigSection, KEY_DRAFTS, KEY_MODEL_TEST_HISTORY, KEY_MODEL_TEST_INPUT, KEY_REFINE_HISTORY,
    KEY_SEARCH_HISTORY, KEY_SEARCH_QUERY,
};
use crate::error::{CmdResult, CommandError};
use crate::state::AppState;
use civilcalc_core::reset::{ResetCounts, ResetSelection, ResetSummary};
use civilcalc_core::source::builtin_loader;
use civilcalc_llm::LlmConfig;
use civilcalc_store::{BackgroundStore, LlmImageStore};
use tauri::State;

/// 「输入历史与草稿」精确到**键**的清单。
///
/// 为什么不直接 `reset_section(ModelTest)`：那个区块里还放着模型测试的
/// **上下文容量、压缩阈值、专属系统提示词**。用户只勾了「输入历史」，
/// 不该顺手把这些也回默认 —— 那又变成「重置了没勾的东西」。
const INPUT_HISTORY_KEYS: &[&str] = &[
    KEY_SEARCH_QUERY,
    KEY_SEARCH_HISTORY,
    KEY_REFINE_HISTORY,
    KEY_MODEL_TEST_INPUT,
    KEY_MODEL_TEST_HISTORY,
    KEY_DRAFTS,
];

/// 本机存量（用于弹窗副标题与二次确认）。
#[tauri::command]
pub fn reset_counts(state: State<'_, AppState>) -> CmdResult<ResetCounts> {
    let db = &state.db;
    let secrets = &state.secrets;

    // 不用 `get_llm_config()`：它在空存储时会返回内置三家预设，
    // 让「本机有没有模型配置」显示失真。这里只看**原始 JSON 在不在**。
    let profiles = secrets
        .export_llm_config_json()
        .map_err(core_err)?
        .and_then(|raw| serde_json::from_str::<LlmConfig>(&raw).ok())
        .map(|c| c.profiles.len() as i64)
        .unwrap_or(0);

    let api_keys = secrets.export_api_keys().map_err(core_err)?.len() as i64;

    let webdav = secrets.get_webdav_config().map_err(core_err)?;
    let web_dav_configured =
        !webdav.username.trim().is_empty() && secrets.has_webdav_password();

    let image_store = open_image_store(&state)?;
    let images = image_store.list().len() as i64;
    let image_bytes = image_store.total_size_bytes();

    let background = BackgroundStore::new(state.paths.background_path());

    Ok(ResetCounts {
        formulas: db.list_formulas().map_err(core_err)?.len() as i64,
        favorites: db.list_favorite_ids().map_err(core_err)?.len() as i64,
        versions: db.list_all_versions().map_err(core_err)?.len() as i64,
        history: db.history_count().map_err(core_err)?,
        usage_stats: db.usage_count().map_err(core_err)?,
        images,
        image_bytes,
        llm_profiles: profiles,
        api_keys,
        web_dav_configured,
        has_background: background.exists(),
    })
}

/// 二次确认文案：逐项写清将删除什么（只列勾选的类别，本机没有的也照实写出来）。
#[tauri::command]
pub fn reset_confirm_lines(
    state: State<'_, AppState>,
    selection: ResetSelection,
) -> CmdResult<Vec<String>> {
    let counts = reset_counts(state)?;
    Ok(counts.confirm_lines(&selection))
}

/// 按勾选执行重置；返回实际清掉的量供提示。
///
/// ⚠️ **不可撤销**：执行后无法退回，云端备份不受影响。
#[tauri::command]
pub fn reset_execute(
    state: State<'_, AppState>,
    selection: ResetSelection,
) -> CmdResult<ResetSummary> {
    run_reset(&state, &selection)
}

/// [`reset_execute`] 的实现体。
///
/// 单独拆出来是为了**可回归测试**：命令签名收 `State<'_, AppState>`，
/// 测试里造不出来；而这里只要一个 `&AppState`。
///
/// ⚠️ 必须是 `pub`（不是 `pub(crate)`）—— 集成测试在**另一个 crate** 里，
/// `pub(crate)` 对它不可见。见 `src-tauri/tests/reset_selection.rs`。
pub fn run_reset(state: &AppState, selection: &ResetSelection) -> CmdResult<ResetSummary> {
    if selection.sections().is_empty() {
        return Ok(ResetSummary::default());
    }

    let db = &state.db;
    let secrets = &state.secrets;

    // ---- 公式 / 收藏 / 版本（闸门：selection.formulas）----
    let mut formulas = 0i64;
    let mut favorites = 0i64;
    let mut versions = 0i64;
    if selection.formulas {
        formulas = db.list_formulas().map_err(core_err)?.len() as i64;
        favorites = db.list_favorite_ids().map_err(core_err)?.len() as i64;
        versions = db.list_all_versions().map_err(core_err)?.len() as i64;
        db.clear_formulas().map_err(core_err)?;

        // 🔴 内置库与用户公式**同表**，而播种只在表空时发生。
        // 不在这里立刻补播的话，本次会话里公式库是空的、检索索引里
        // 还留着已删的条目 —— 要等下次启动才恢复。
        if let Ok(loaded) = builtin_loader::load_embedded() {
            if let Err(e) = db.seed_builtins_if_empty(&loaded.valid) {
                civilcalc_core::log::w("Reset", "内置库补播失败", Some(&e.to_string()));
            }
        }
        // 索引里不能留已删公式
        state.rebuild_index()?;
    }

    // ---- 历史（闸门：selection.history）----
    let mut history = 0i64;
    if selection.history {
        history = db.history_count().map_err(core_err)?;
        db.clear_history().map_err(core_err)?;
    }

    // ---- 用量（闸门：selection.usage_stats）----
    let mut usage_stats = 0i64;
    if selection.usage_stats {
        usage_stats = db.usage_count().map_err(core_err)?;
        db.clear_usage().map_err(core_err)?;
    }

    // ---- 图片（闸门：selection.images）----
    let mut images = 0i64;
    if selection.images {
        let store = open_image_store(state)?;
        let ids: Vec<String> = store.list().into_iter().map(|i| i.id).collect();
        if !ids.is_empty() {
            images = store.delete(&ids) as i64;
        }
    }

    // ---- 背景图文件（闸门：selection.appearance）----
    // 文件 IO 放在偏好事务之外，失败也不影响偏好重置
    let mut background = false;
    if selection.appearance {
        background = BackgroundStore::new(state.paths.background_path()).clear();
    }

    // ---- 偏好类：一次 `update_config` 批量重置并**落盘** ----
    //
    // 走 `update_config` 而不是裸锁：它改完会 `save()`。
    // 这里也刻意把多个区块**合成一次提交** —— 每次提交都会整份写盘，
    // 分四次写纯属浪费。
    let mut to_reset: Vec<ConfigSection> = Vec::new();
    if selection.appearance {
        to_reset.push(ConfigSection::Appearance);
    }
    if selection.prompts {
        to_reset.push(ConfigSection::Prompts);
    }

    let mut preferences = if to_reset.is_empty() {
        0i64
    } else {
        state
            .update_config(|c| {
                to_reset
                    .iter()
                    .map(|s| c.reset_section(*s))
                    .sum::<usize>()
            })
            .map_err(|e| CommandError::Storage {
                message: e.to_string(),
            })? as i64
    };

    // 「输入历史与草稿」按**键**删（见 `INPUT_HISTORY_KEYS` 的说明）
    if selection.input_history {
        preferences += state
            .update_config(|c| {
                let mut n = 0i64;
                for k in INPUT_HISTORY_KEYS {
                    let had = c.get_str(k).is_some() || c.get_int(k).is_some();
                    c.remove(k);
                    if had {
                        n += 1;
                    }
                }
                n
            })
            .map_err(|e| CommandError::Storage {
                message: e.to_string(),
            })?;
    }

    // ---- LLM 配置：回到内置三家预设（闸门：selection.llm_config）----
    let mut llm_config = false;
    let mut api_keys = 0i64;
    if selection.llm_config {
        api_keys = secrets.export_api_keys().map_err(core_err)?.len() as i64;
        llm_config = secrets
            .export_llm_config_json()
            .map_err(core_err)?
            .is_some();
        secrets.clear_llm_config().map_err(core_err)?;
    }

    // ---- WebDAV 配置：清地址 / 账号 / 密码（闸门：selection.web_dav_config）----
    let mut web_dav_config = false;
    if selection.web_dav_config {
        let webdav = secrets.get_webdav_config().map_err(core_err)?;
        web_dav_config = secrets.has_webdav_password() || !webdav.username.trim().is_empty();
        secrets.clear_webdav_config().map_err(core_err)?;
    }

    Ok(ResetSummary {
        formulas,
        favorites,
        versions,
        history,
        usage_stats,
        images,
        background,
        preferences,
        llm_config,
        api_keys,
        web_dav_config,
    })
}

// =============================================================================
// 内部
// =============================================================================

fn open_image_store(state: &AppState) -> CmdResult<LlmImageStore> {
    LlmImageStore::new(state.paths.images_dir()).map_err(|e| CommandError::Storage {
        message: format!("图片目录不可用: {e}"),
    })
}

fn core_err(e: civilcalc_core::CoreError) -> CommandError {
    CommandError::Storage {
        message: e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 「输入历史」的键清单不能漏、也不能混进别的设置。
    ///
    /// 漏了 → 用户勾了「输入历史」却清不掉；混进别的 → 又变成「重置了没勾的」。
    #[test]
    fn input_history_keys_are_exact() {
        assert_eq!(INPUT_HISTORY_KEYS.len(), 6);
        for k in INPUT_HISTORY_KEYS {
            assert!(!k.is_empty());
        }
        // 模型测试的**设置**类键绝不能在里面（那是「提示词 / AI 行为」的范畴）
        assert!(!INPUT_HISTORY_KEYS.contains(&"model_test_context_size"));
        assert!(!INPUT_HISTORY_KEYS.contains(&"model_test_system_prompt"));
        assert!(!INPUT_HISTORY_KEYS.contains(&"model_test_compress_threshold"));
        // 模型测试**对话**属于输入历史的范畴（它是记忆，不是设置）
        assert!(INPUT_HISTORY_KEYS.contains(&KEY_MODEL_TEST_HISTORY));
    }
}
