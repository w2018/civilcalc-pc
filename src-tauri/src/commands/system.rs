//! 组 1：系统与配置（8 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `init_db` | — | `void` |
//! | `app_info` | — | [`AppInfo`] |
//! | `config_get` | — | [`ConfigSnapshot`] |
//! | `config_save` | `snapshot` | `void` |
//! | `config_reset_section` | `section: String` | `number`（删除的键数） |
//! | `compile_cache_stats` / `compile_cache_clear` | — | 编译缓存 |
//! | `eula_status` | — | `boolean`（是否已同意用户协议） |
//! | `eula_accept` | — | `void` |
//! | `app_exit` | — | `void`（不返回，直接退出进程） |

use crate::config::{ConfigSection, ConfigSnapshot};
use crate::error::{CmdResult, CommandError};
use crate::paths::DesktopPaths;
use crate::state::AppState;
use crate::AppInfo;
use tauri::State;

/// 应用信息（"关于"页 + 排障）。
///
/// ⚠️ 数据库查询失败**不返回错误**，而是把 `formulaCount` 记 0、
/// `dbSchemaVersion` 记 -1，让"关于"页仍能打开（否则用户无法自助排障）。
/// 失败细节进日志。
#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    let paths: &DesktopPaths = &state.paths;

    let (formula_count, db_schema_version) = match (
        state.db.list_formulas(),
        state.db.user_version(),
    ) {
        (Ok(list), Ok(v)) => (list.len(), v),
        (f, v) => {
            civilcalc_core::log::e(
                "Commands",
                &format!(
                    "读取数据库状态失败：公式列表={:?} user_version={:?}",
                    f.as_ref().err().map(|e| e.to_string()),
                    v.as_ref().err().map(|e| e.to_string())
                ),
                None,
            );
            (f.map(|l| l.len()).unwrap_or(0), v.unwrap_or(-1))
        }
    };

    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        product_name: "AI全能计算器".to_string(),
        identifier: "com.w2018.civilcalc.pc".to_string(),
        db_path: paths.db_path().to_string_lossy().to_string(),
        app_data_dir: paths.app_data_dir.to_string_lossy().to_string(),
        app_cache_dir: paths.app_cache_dir.to_string_lossy().to_string(),
        logs_dir: paths.logs_dir().to_string_lossy().to_string(),
        builtin_formula_count: crate::commands::builtin_formula_count(),
        formula_count,
        db_schema_version,
    }
}

/// 初始化数据库。
///
/// **实际初始化在 `setup()` 里已经做完**（见 `lib.rs` 的启动时序）——
/// 应用能显示窗口就说明库已就绪。此命令保留是为了：
///
/// 1. 前端有一个**明确的同步点**：await 它成功即代表可以开始调其他数据命令
/// 2. 未来若改为"延迟初始化"，契约不用变
///
/// 因此它是**幂等且恒成功**的（只做一次健康检查）。
#[tauri::command]
pub fn init_db(state: State<'_, AppState>) -> CmdResult<i32> {
    // 一次真实查询，确认库确实可用（而不是只返回 Ok）
    state.db.user_version().map_err(Into::into)
}

/// 读全部偏好（快照形状，与备份包 `BackupPrefs` 同形）
#[tauri::command]
pub fn config_get(state: State<'_, AppState>) -> CmdResult<ConfigSnapshot> {
    state.with_config(|c| c.snapshot())
}

/// **整体替换**全部偏好。
///
/// ⚠️ 是替换不是合并：快照里没有的键会被清掉。
/// 前端应当"先 `config_get` → 改 → 再 `config_save`"，不要只传要改的键。
#[tauri::command]
pub fn config_save(state: State<'_, AppState>, snapshot: ConfigSnapshot) -> CmdResult<()> {
    state.update_config(move |c| c.replace_with(snapshot))
}

/// 按区块重置偏好（删键 → 下次读取回落默认值）。
///
/// 返回删除的键数。`section` 未知时返回 [`CommandError::InvalidArgument`]
/// （**不静默当成某个区块** —— 那样会删错东西）。
#[tauri::command]
pub fn config_reset_section(state: State<'_, AppState>, section: String) -> CmdResult<usize> {
    let sec = ConfigSection::parse(&section).ok_or_else(|| CommandError::InvalidArgument {
        message: format!(
            "未知的偏好区块: {section}（可用值: {}）",
            ConfigSection::all()
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" / ")
        ),
    })?;
    state.update_config(move |c| c.reset_section(sec))
}

/// 表达式编译缓存统计（排障用）。
///
/// 正常使用下 `hitRate` 应明显 > 0 —— 用户在参数表里改值时表达式不变，
/// 每次求值都应命中缓存。
#[tauri::command]
pub fn compile_cache_stats() -> civilcalc_core::engine::cache::CacheStats {
    civilcalc_core::engine::compile_cache_stats()
}

/// 清空表达式编译缓存（排障用；也会重置统计）。
#[tauri::command]
pub fn compile_cache_clear() {
    civilcalc_core::engine::clear_compile_cache();
    civilcalc_core::log::i("Commands", "已清空表达式编译缓存");
}

// =============================================================================
// 首次运行的「用户协议」
// =============================================================================

/// 用户是否已同意用户协议。
///
/// 前端在启动时问一次：`false` 就弹不可关闭的协议弹窗。
#[tauri::command]
pub fn eula_status(state: State<'_, AppState>) -> CmdResult<bool> {
    let g = state.config.lock().map_err(|_| CommandError::Storage {
        message: "偏好数据锁已中毒，请重启应用".to_string(),
    })?;
    Ok(g.eula_accepted())
}

/// 记下「用户已同意用户协议」。
///
/// ⚠️ 走 `state.update_config(...)` —— 它会在改完后 `save()`。
/// 直接改 `state.config.lock()` 里的内存**不落盘**，
/// 表现是「这次同意了，下次启动又被弹」。
#[tauri::command]
pub fn eula_accept(state: State<'_, AppState>) -> CmdResult<()> {
    state.update_config(|c| c.set_eula_accepted(true))
}

/// 退出应用（用户在协议弹窗里点「不同意」时用）。
///
/// ⚠️ **不能在命令里直接 `app.exit()`**：那会在 IPC 响应发回去之前就把进程
/// 干掉，前端那次 `await` 永远等不到结果。这里挪到另一个线程、延迟一小会儿
/// 再退，让响应先落地（时序上干净，日志也能写完）。
#[tauri::command]
pub fn app_exit(app: tauri::AppHandle) {
    civilcalc_core::log::i("Commands", "用户选择不同意用户协议，退出应用");
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(120));
        app.exit(0);
    });
}

#[cfg(test)]
mod tests {
    use crate::config::{ConfigSection, KEY_PROMPT_A, KEY_THEME, THEME_DARK};

    /// `config_reset_section` 的错误分支：未知区块必须被拒
    #[test]
    fn unknown_section_is_rejected() {
        assert!(ConfigSection::parse("NOPE").is_none());
        // 而所有合法值都能解析（与命令层拼给用户的提示一致）
        for s in ConfigSection::all() {
            assert!(ConfigSection::parse(s.as_str()).is_some());
        }
    }

    /// 重置区块后确实回落默认值
    #[test]
    fn reset_section_restores_defaults() {
        let mut c = crate::config::AppConfig::in_memory();
        c.set_str(KEY_THEME, THEME_DARK);
        c.set_str(KEY_PROMPT_A, "自定义");

        c.reset_section(ConfigSection::Appearance);

        assert_eq!(c.get_str(KEY_THEME), None, "键应被删除而非写成默认值");
        assert_eq!(c.get_str(KEY_PROMPT_A), Some("自定义"), "其他区块不受影响");
    }
}
