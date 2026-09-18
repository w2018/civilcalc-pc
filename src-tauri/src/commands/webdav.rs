//! 组 14：WebDAV 云端备份（11 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `webdav_config_get` | — | `WebDavConfig`（**不含密码**） |
//! | `webdav_config_save` | `config`, `password?` | `void` |
//! | `webdav_has_password` | — | `bool` |
//! | `webdav_clear_password` | — | `void` |
//! | `webdav_test_connection` | `config?`, `password?` | [`WebDavTestResult`] |
//! | `webdav_list` | — | [`RemoteListResult`] |
//! | `webdav_upload` | `selection`, `password?` | [`UploadResult`] |
//! | `webdav_inspect` | `fileName`, `password?` | [`BackupInspectResult`] |
//! | `webdav_download_import` | `fileName`, `password?`, `mode` | [`ImportReport`] |
//! | `webdav_delete` | `fileName` | `void` |
//! | `webdav_cancel` | — | `void` |
//!
//! ## 🔴 两个 `password` 参数含义**不同**，不要混淆
//!
//! | 出现在 | 含义 | 存哪 |
//! |---|---|---|
//! | `webdav_config_save(config, password?)` | **WebDAV 账号密码** | keyring（`webdav_password`） |
//! | `webdav_upload` / `webdav_inspect` / `webdav_download_import` 的 `password?` | **备份包加密密码** | 不存，每次现给 |
//!
//! 前者是「连得上服务器」，后者是「打得开包」。同名不同物，
//! 契约里都是 `password` —— 传错不会报错，只会「连不上」或「打不开」，很难查。
//!
//! ## 本地导出与云端上传**共用打包链路**
//!
//! 两者都调 [`pack_to_tmp`]，所以「本地上传的包」与「云端上传的包」格式完全一致，
//! 加密参数、图片处理、清单内容都不会漂移。
//!
//! ## 下载缓存（避免二次下载）
//!
//! `webdav_inspect` 与 `webdav_download_import` 是**两步**（先看清单再确认导入）。
//! 若各自下载一次，几十 MB 的包要传两遍。所以 `webdav_inspect` 把包落在
//! `<tmp>/webdav-cache-<文件名>`，`webdav_download_import` 命中就直接用。
//!
//! - `webdav_inspect` 开始时**清掉上一次的缓存**（同一时刻只可能有一个待导入的包）
//! - `webdav_download_import` 结束（成败皆然）**删掉缓存**
//! - 命中判据是「文件存在且非空」；失败/取消留下的半包会被下一次 inspect 清掉
//!
//! ⚠️ 缓存只在**同一会话、间隔数秒**内有效。若远端文件在此期间被别处改动，
//! 用的是旧包 —— 但导入前仍会解析 manifest，不会把坏包写进库。

use std::path::PathBuf;

use civilcalc_backup::model::BackupSelection;
use civilcalc_backup::webdav_config::WebDavConfig;
use civilcalc_backup::webdav_repo::{self, RemoteListResult, UploadResult};
use civilcalc_backup::{BackupError, WebDavClient};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::error::{CmdResult, CommandError};
use crate::paths::DesktopPaths;
use crate::state::AppState;

use super::backup::{
    blocking, import_sync, inspect_sync, pack_to_tmp, parse_mode, BackupInspectResult, ImportReport,
};

// =============================================================================
// 事件
// =============================================================================

/// 上传/下载进度（契约：`{ stage, current, total }`）
pub const EVENT_BACKUP_PROGRESS: &str = "backup://progress";

/// 用户取消（配合 [`webdav_cancel`]）
pub const EVENT_BACKUP_CANCELLED: &str = "backup://cancelled";

/// 进度阶段：打包（本地，0 → 总字节）
const STAGE_PACK: &str = "pack";
/// 进度阶段：上传
const STAGE_UPLOAD: &str = "upload";
/// 进度阶段：下载
const STAGE_DOWNLOAD: &str = "download";
/// 进度阶段：导入（写库）
const STAGE_IMPORT: &str = "import";

/// `backup://progress` 的载荷。
///
/// ⚠️ `docs/05` 写的是 `{ stage, sent, total }`，`docs/08`（IPC 契约）写
/// `{ stage, current, total }`。**以 `docs/08` 为准** —— 它与 `export://progress`
/// 同形，前端可以共用一个进度组件。
///
/// `total` 在服务端不给 `Content-Length` 时为 **0**（下载进度未知）——
/// 前端据此显示「已传输 x MB」而不是百分比。不用 `-1` 是因为
/// `current/total` 都是无符号数，负数反而要额外约定。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackupProgress {
    pub stage: String,
    pub current: u64,
    pub total: u64,
}

/// 发进度事件。**失败只记日志** —— 进度丢了不该让备份失败。
fn emit_progress(app: &AppHandle, stage: &str, current: u64, total: u64) {
    let payload = BackupProgress {
        stage: stage.to_string(),
        current,
        total,
    };
    if let Err(e) = app.emit(EVENT_BACKUP_PROGRESS, payload) {
        civilcalc_core::log::w("Commands", &format!("发送备份进度失败: {e}"), None);
    }
}

/// 发取消事件。同样**失败只记日志**。
fn emit_cancelled(app: &AppHandle) {
    if let Err(e) = app.emit(EVENT_BACKUP_CANCELLED, ()) {
        civilcalc_core::log::w("Commands", &format!("发送备份取消事件失败: {e}"), None);
    }
}

// =============================================================================
// IPC 类型
// =============================================================================

/// 连通性测试结果。
///
/// ## 为什么不用 `Result<(), _>` 表达
///
/// 与 `llm_test_connection` 同一口径，**区分两类失败**：
///
/// | 情形 | 表达 |
/// |---|---|
/// | 本地配置问题（地址不合规 / 没密码） | `Err`（`invalidArgument` / `unauthorized`） |
/// | 连不上（网络、鉴权被拒、证书） | `Ok { ok: false, .. }` |
///
/// 都塞进 `Err` 的话，前端只能显示一句红字 ——
/// 用户不知道该去改配置还是查网络。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebDavTestResult {
    /// 是否连通且鉴权通过
    pub ok: bool,
    /// 归一化后的远端目录 URL（**成败都给**，便于用户核对到底连的是哪）
    pub dir_url: String,
    /// 人类可读结论（成功或失败原因）
    pub message: String,
}

// =============================================================================
// 配置与密码
// =============================================================================

/// 读 WebDAV 配置（**不含密码**）。未配置时返回坚果云默认值。
#[tauri::command]
pub fn webdav_config_get(state: State<'_, AppState>) -> CmdResult<WebDavConfig> {
    Ok(state.secrets.get_webdav_config()?)
}

/// 保存 WebDAV 配置。
///
/// ## ⚠️ `password` 为空 = **不改动已有密码**（不是「设成空密码」）
///
/// 前端提交后**立即清空输入框**（契约要求「密码只存 keyring」），
/// 所以「留空」是常态。若把空串当新密码写进去，
/// 用户每次改服务器地址都会**静默把密码抹掉** —— 之后所有操作报「密码未设置」。
#[tauri::command]
pub fn webdav_config_save(
    state: State<'_, AppState>,
    config: WebDavConfig,
    password: Option<String>,
) -> CmdResult<()> {
    let pw = password.as_deref().filter(|p| !p.is_empty());
    state.secrets.save_webdav_config(&config, pw)?;
    Ok(())
}

/// 是否已保存 WebDAV 密码（**不返回密码本身**）
#[tauri::command]
pub fn webdav_has_password(state: State<'_, AppState>) -> CmdResult<bool> {
    Ok(state.secrets.has_webdav_password())
}

/// 清除已保存的 WebDAV 密码（配置保留）
#[tauri::command]
pub fn webdav_clear_password(state: State<'_, AppState>) -> CmdResult<()> {
    state.secrets.delete(crate::secrets::KEY_WEBDAV_PASSWORD)?;
    Ok(())
}

// =============================================================================
// 测试连接
// =============================================================================

/// 测试连接（MKCOL + PROPFIND）。
///
/// ## `config` / `password` 都是**可选**的（与契约的「无入参」不同，有意）
///
/// `docs/05` 的用户流程是「填地址 → **测试连接** → 保存」，也就是
/// **保存之前**就要能测。所以两个参数都可传：传了就用传进来的（弹窗里当前填的值），
/// 没传就回落已保存的配置与 keyring 里的密码。
///
/// 不传参时行为与契约完全一致 —— 这是**纯增量**，不影响无参调用。
#[tauri::command]
pub async fn webdav_test_connection(
    state: State<'_, AppState>,
    config: Option<WebDavConfig>,
    password: Option<String>,
) -> CmdResult<WebDavTestResult> {
    // ① 取配置：入参优先，否则读已保存的
    let cfg = match config {
        Some(c) => c,
        None => state.secrets.get_webdav_config()?,
    };

    // ② 地址合规性（**本地**校验，不联网）→ 属于「配置问题」，走 `Err`
    if let Some(msg) = cfg.base_url_error() {
        return Err(CommandError::InvalidArgument { message: msg });
    }

    // ③ 取密码：非空入参优先，否则读 keyring
    let pw = match password.as_deref().filter(|p| !p.is_empty()) {
        Some(p) => p.to_string(),
        None => state
            .secrets
            .get(crate::secrets::KEY_WEBDAV_PASSWORD)?
            .filter(|p| !p.trim().is_empty())
            .ok_or_else(|| CommandError::Unauthorized {
                message: "WebDAV 密码未设置".to_string(),
            })?,
    };

    let conn = civilcalc_backup::webdav_config::WebDavConnection::new(cfg, pw);
    let dir_url = conn.dir_url();
    let client = WebDavClient::new();

    // ④ 真的连一次：建目录（405 视为已存在）+ 列一次
    Ok(match webdav_repo::test_connection(&client, &conn).await {
        Ok(message) => WebDavTestResult {
            ok: true,
            dir_url,
            message,
        },
        Err(e) => {
            // 细节只进日志；给用户的是 `user_message()`（源文案逐字对齐）
            let user = e.user_message().to_string();
            civilcalc_core::log::w("Commands", "WebDAV 连通性测试失败", Some(&e.detail()));
            WebDavTestResult {
                ok: false,
                dir_url,
                message: user,
            }
        }
    })
}

// =============================================================================
// 列表 / 上传 / 下载 / 删除 / 取消
// =============================================================================

/// 列远端备份（最多 [`civilcalc_backup::webdav_config::LIST_LIMIT`] 条）。
///
/// ## ⚠️ 返回 [`RemoteListResult`] 而不是契约写的 `RemoteEntry[]`
///
/// 因为 `RemoteEntry` 有两个问题：
/// 1. 它是 **PROPFIND 原始条目**（含目录项、含别家的文件），前端还得自己过滤；
/// 2. 契约的 `RemoteEntry[]` **丢了 `total`** —— 而界面要提示「还有 N 条更早的」。
///
/// `RemoteListResult` 已经做过「过滤 → 排序 → 截断」并带上截断前的总数，
/// 是命令层该给的形状。
#[tauri::command]
pub async fn webdav_list(state: State<'_, AppState>) -> CmdResult<RemoteListResult> {
    let conn = state.secrets.resolve_webdav_connection()?;
    let client = WebDavClient::new();
    Ok(webdav_repo::list_remote(&client, &conn).await?)
}

/// 打包并上传到 WebDAV。
///
/// `password` 是**备份包加密密码**（不是 WebDAV 密码，见模块文档）。
///
/// ## 流程
///
/// ```text
/// ① MKCOL 确保远端目录存在（405 视为已存在）
/// ② 打包到 tmp（可选加密）      → backup://progress { stage: "pack" }
/// ③ PUT 上传（逐块读，可取消）  → backup://progress { stage: "upload" }
/// ④ 删 tmp；记 lastBackupAt / backupCount
/// ```
///
/// ⚠️ 取消时**必须**让前端拿到 `cancelled` 错误：只 `await` 而没监听
/// `backup://cancelled` 的前端会把「用户取消」显示成「上传成功」——
/// 那是**假成功**，比多一个错误更糟。
#[tauri::command]
pub async fn webdav_upload(
    state: State<'_, AppState>,
    app: AppHandle,
    selection: BackupSelection,
    password: Option<String>,
) -> CmdResult<UploadResult> {
    let conn = state.secrets.resolve_webdav_connection()?;
    let db = state.db.clone();
    let secrets = state.secrets.clone();
    let paths = state.paths.clone();
    let config = state.config.clone();

    // 🔴 先清标志：上次的取消不能秒杀这次任务
    let cancel = state.net.begin_webdav();

    // ① 打包（阻塞 IO 进 spawn_blocking；几十 MB 的 gzip/加密不能占住 async 线程）
    let packed = blocking(move || {
        pack_to_tmp(
            &db,
            &secrets,
            &config,
            &paths,
            &selection,
            password.as_deref(),
        )
    })
    .await?;

    // 打包是纯本地动作，没有中间进度 —— 一次性报满
    emit_progress(&app, STAGE_PACK, packed.size_bytes, packed.size_bytes);

    // ② 上传
    let client = WebDavClient::new();
    let app_for_progress = app.clone();
    let uploaded = webdav_repo::upload_backup(
        &client,
        &conn,
        &packed.name,
        &packed.path,
        move |done, total| emit_progress(&app_for_progress, STAGE_UPLOAD, done, total),
        Some(cancel.clone()),
    )
    .await;

    // ③ tmp 一律删（成败、取消都一样）
    let _ = std::fs::remove_file(&packed.path);

    match uploaded {
        Ok(result) => {
            record_backup_stats(&state)?;
            Ok(result)
        }
        Err(BackupError::Cancelled) => {
            emit_cancelled(&app);
            Err(CommandError::Cancelled {
                message: "上传已取消".to_string(),
            })
        }
        Err(e) => Err(e.into()),
    }
}

/// 只解析云端备份包的清单（**不导入**，供二次确认）。
///
/// 实现上会**真的把包下载到临时目录**并留在那里，供随后的
/// [`webdav_download_import`] 复用（见模块文档的「下载缓存」）。
///
/// `password` 是**备份包加密密码**。加密包没给密码时返回
/// `decoded = false`（**不是错误**）—— 前端据此弹「请输入密码」并重调。
#[tauri::command]
pub async fn webdav_inspect(
    state: State<'_, AppState>,
    app: AppHandle,
    file_name: String,
    password: Option<String>,
) -> CmdResult<BackupInspectResult> {
    // 文件名来自远端列表 → 必须校验（目录里可能混着用户自己的文件）
    webdav_repo::ensure_backup_name(&file_name)?;

    let conn = state.secrets.resolve_webdav_connection()?;
    let paths = state.paths.clone();

    // 清掉上一次的缓存（同一时刻只可能有一个待导入的包）
    clear_cache(&paths);
    let cached = cache_path(&paths, &file_name);

    let cancel = state.net.begin_webdav();
    let client = WebDavClient::new();
    let app_for_progress = app.clone();
    let target = cached.clone();
    let name = file_name.clone();
    let downloaded = webdav_repo::download_backup(
        &client,
        &conn,
        &name,
        &target,
        move |done, total| {
            // 服务端不给 `Content-Length` 时 total = -1 → 报 0（「进度未知」）
            let total = if total < 0 { 0 } else { total as u64 };
            emit_progress(&app_for_progress, STAGE_DOWNLOAD, done, total);
        },
        Some(&cancel),
    )
    .await;

    match downloaded {
        Ok(_) => {}
        Err(BackupError::Cancelled) => {
            let _ = std::fs::remove_file(&cached);
            emit_cancelled(&app);
            return Err(CommandError::Cancelled {
                message: "下载已取消".to_string(),
            });
        }
        Err(e) => {
            // 半包不留
            let _ = std::fs::remove_file(&cached);
            return Err(e.into());
        }
    }

    let pw = password.clone();
    blocking(move || inspect_sync(&cached, pw.as_deref())).await
}

/// 下载云端备份并导入。
///
/// `password` 是**备份包加密密码**；`mode` 是 `"merge"` / `"replace"`。
///
/// 若 [`webdav_inspect`] 刚下过同一个包（缓存命中），**跳过下载**直接导入。
#[tauri::command]
pub async fn webdav_download_import(
    state: State<'_, AppState>,
    app: AppHandle,
    file_name: String,
    password: Option<String>,
    mode: String,
) -> CmdResult<ImportReport> {
    webdav_repo::ensure_backup_name(&file_name)?;
    let clear_local_first = parse_mode(&mode)?;

    let paths = state.paths.clone();
    let cached = cache_path(&paths, &file_name);

    // 缓存命中：文件存在且非空 → 跳过下载
    let hit = std::fs::metadata(&cached).is_ok_and(|m| m.len() > 0);
    if !hit {
        let conn = state.secrets.resolve_webdav_connection()?;
        let cancel = state.net.begin_webdav();
        let client = WebDavClient::new();
        let app_for_progress = app.clone();
        let target = cached.clone();
        let name = file_name.clone();
        let downloaded = webdav_repo::download_backup(
            &client,
            &conn,
            &name,
            &target,
            move |done, total| {
                let total = if total < 0 { 0 } else { total as u64 };
                emit_progress(&app_for_progress, STAGE_DOWNLOAD, done, total);
            },
            Some(&cancel),
        )
        .await;

        match downloaded {
            Ok(_) => {}
            Err(BackupError::Cancelled) => {
                let _ = std::fs::remove_file(&cached);
                emit_cancelled(&app);
                return Err(CommandError::Cancelled {
                    message: "下载已取消".to_string(),
                });
            }
            Err(e) => {
                let _ = std::fs::remove_file(&cached);
                return Err(e.into());
            }
        }
    }

    // 导入（阻塞 IO：解包 + 逐条写库）
    emit_progress(&app, STAGE_IMPORT, 0, 0);
    let db = state.db.clone();
    let secrets = state.secrets.clone();
    let config = state.config.clone();
    let paths2 = state.paths.clone();
    let path = cached.clone();
    let result = blocking(move || {
        import_sync(
            &db,
            &secrets,
            &config,
            &paths2,
            &path,
            password.as_deref(),
            clear_local_first,
        )
    })
    .await;

    // 缓存一律删（成败皆然）—— 留着只会占空间，且下次 inspect 也会清
    let _ = std::fs::remove_file(&cached);
    result
}

/// 删除远端备份（**已不存在也算成功**，源同）
#[tauri::command]
pub async fn webdav_delete(
    state: State<'_, AppState>,
    file_name: String,
) -> CmdResult<()> {
    let conn = state.secrets.resolve_webdav_connection()?;
    let client = WebDavClient::new();
    webdav_repo::delete_remote(&client, &conn, &file_name).await?;
    // 删掉的正好是待导入的包 → 缓存也清掉（免得之后「导入」到已删的包）
    clear_cache(&state.paths);
    Ok(())
}

/// 取消当前上传/下载（**立即断流**，不是「跑完再丢」）
///
/// 置位后网络读循环每读一个分块检查一次，命中即 drop 响应体（连接随之关闭）。
#[tauri::command]
pub fn webdav_cancel(state: State<'_, AppState>) -> CmdResult<()> {
    state.net.cancel_webdav();
    Ok(())
}

// =============================================================================
// 辅助
// =============================================================================

/// 下载缓存的文件名前缀。
///
/// 用前缀（而不是固定名）是因为要按**备份文件名**区分 ——
/// 但实际同一时刻只会有一个人待导入，所以 `clear_cache` 直接按前缀全清。
const CACHE_PREFIX: &str = "webdav-cache-";

/// 某个备份包在本机的缓存路径
fn cache_path(paths: &DesktopPaths, file_name: &str) -> PathBuf {
    paths.tmp_dir().join(format!("{CACHE_PREFIX}{file_name}"))
}

/// 清掉 `tmp/` 下所有下载缓存。
///
/// 只删**带前缀的**文件 —— `tmp/` 里还有计算书导出的临时文件，
/// 一律清空会打断正在进行的导出。
fn clear_cache(paths: &DesktopPaths) {
    let Ok(entries) = std::fs::read_dir(paths.tmp_dir()) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with(CACHE_PREFIX) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// 记录「上次备份时间 + 累计次数」。
///
/// ⚠️ 这两个键在 [`crate::config::NON_BACKUP_KEYS`] 里（**不进备份包**）——
/// 它们是「本机备份过几次」的本地状态，换机后应当从 0 重新开始。
///
/// 计数**只在上传成功后**增加：失败或取消不该让用户看到「累计 3 次」却只有 2 个包。
fn record_backup_stats(state: &AppState) -> CmdResult<()> {
    let now = super::backup::now_ms();
    state.update_config(|c| {
        let n = c.get_int(crate::config::KEY_WEBDAV_BACKUP_COUNT).unwrap_or(0);
        c.set_int(crate::config::KEY_WEBDAV_BACKUP_COUNT, n + 1);
        c.set_int(crate::config::KEY_WEBDAV_LAST_BACKUP, now);
    })?;
    Ok(())
}

// =============================================================================
// 测试
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use civilcalc_backup::webdav_config::{WebDavConfig, WebDavPreset};

    fn test_paths(tag: &str) -> DesktopPaths {
        let root = std::env::temp_dir().join(format!("civilcalc-webdav-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("tmp")).unwrap();
        DesktopPaths::for_test(root)
    }

    // ---- 缓存路径与清理 ----

    #[test]
    fn cache_path_is_under_tmp_with_prefix() {
        let paths = test_paths("cache-path");
        let p = cache_path(&paths, "civilcalc_backup_20260915_143012.tar.gz");
        assert_eq!(p.parent().unwrap(), paths.tmp_dir());
        assert!(p
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(CACHE_PREFIX));
        // 原文件名必须完整保留（否则 import 时按名字取会错）
        assert!(p.to_string_lossy().contains("civilcalc_backup_20260915_143012.tar.gz"));
    }

    /// 🔴 `clear_cache` **只删带前缀的** —— `tmp/` 里还有计算书导出的临时文件
    #[test]
    fn clear_cache_only_removes_prefixed_files() {
        let paths = test_paths("clear-cache");
        let tmp = paths.tmp_dir();
        let cached = cache_path(&paths, "civilcalc_backup_20260915_143012.tar.gz");
        let other = tmp.join("计算书.docx.tmp-abc");
        std::fs::write(&cached, b"cached").unwrap();
        std::fs::write(&other, b"report").unwrap();

        clear_cache(&paths);

        assert!(!cached.exists(), "缓存该被删");
        assert!(other.exists(), "无关的临时文件不能被删");
    }

    #[test]
    fn clear_cache_is_safe_when_tmp_missing() {
        let paths = test_paths("clear-missing");
        let _ = std::fs::remove_dir_all(paths.tmp_dir());
        // 不该 panic
        clear_cache(&paths);
    }

    // ---- 进度载荷 ----

    #[test]
    fn progress_payload_is_camel_case() {
        let p = BackupProgress {
            stage: STAGE_UPLOAD.to_string(),
            current: 100,
            total: 200,
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["stage"], "upload");
        assert_eq!(v["current"], 100);
        assert_eq!(v["total"], 200);
        assert!(v.get("sent").is_none(), "docs/05 的 `sent` 不是契约字段");
    }

    /// `docs/08` 是权威：字段名是 `current`，不是 `docs/05` 的 `sent`
    #[test]
    fn progress_stage_names_are_stable() {
        assert_eq!(STAGE_PACK, "pack");
        assert_eq!(STAGE_UPLOAD, "upload");
        assert_eq!(STAGE_DOWNLOAD, "download");
        assert_eq!(STAGE_IMPORT, "import");
    }

    // ---- 测试结果类型 ----

    #[test]
    fn test_result_serializes_camel_case() {
        let r = WebDavTestResult {
            ok: true,
            dir_url: "https://dav.jianguoyun.com/dav/civilcalc".to_string(),
            message: "连接成功".to_string(),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["dirUrl"], "https://dav.jianguoyun.com/dav/civilcalc");
        assert!(v.get("dir_url").is_none(), "不得泄漏 snake_case");
    }

    /// 失败也要给 `dirUrl` —— 用户要能核对「到底连的是哪个地址」
    #[test]
    fn test_result_failure_still_carries_dir_url() {
        let r = WebDavTestResult {
            ok: false,
            dir_url: "https://x.com/dav/civilcalc".to_string(),
            message: "网络连接失败".to_string(),
        };
        assert!(!r.ok);
        assert!(!r.dir_url.is_empty());
    }

    // ---- 配置合规性（本地校验，不联网） ----

    /// 🔴 只认 https：明文 HTTP 会把凭据暴露在链路上
    #[test]
    fn config_rejects_plain_http_before_network() {
        let cfg = WebDavConfig {
            base_url: "http://dav.example.com/dav".to_string(),
            ..Default::default()
        };
        assert!(cfg.base_url_error().is_some(), "http 必须被本地拦下");
        assert!(!cfg.is_usable());
    }

    #[test]
    fn config_accepts_https() {
        let cfg = WebDavConfig {
            base_url: "https://dav.jianguoyun.com/dav".to_string(),
            ..Default::default()
        };
        assert!(cfg.base_url_error().is_none());
        assert!(cfg.is_usable());
    }

    /// 空地址也要被拦下（否则会去连一个空 URL）
    #[test]
    fn config_rejects_blank_url() {
        let cfg = WebDavConfig {
            base_url: "   ".to_string(),
            ..Default::default()
        };
        assert!(cfg.base_url_error().is_some());
    }

    /// 预设默认值：坚果云地址正确且可用
    #[test]
    fn nutstore_preset_is_usable() {
        let mut cfg = WebDavConfig::default();
        cfg.apply_preset(WebDavPreset::Nutstore);
        assert_eq!(cfg.base_url, "https://dav.jianguoyun.com/dav");
        assert!(cfg.is_usable());
    }

    /// 切到自定义预设时**保留**用户已填地址（源同）
    #[test]
    fn custom_preset_keeps_user_url() {
        let mut cfg = WebDavConfig {
            base_url: "https://my.example.com/dav".to_string(),
            ..Default::default()
        };
        cfg.apply_preset(WebDavPreset::Custom);
        assert_eq!(cfg.base_url, "https://my.example.com/dav");
    }

    /// 归一化：去空白、去末尾斜杠（`dir_url` 拼接依赖它）
    ///
    /// ⚠️ `dir_url()` **有意以 `/` 结尾** —— PROPFIND / MKCOL 都按集合处理，
    /// 少了斜杠部分服务端会 301 或直接 404。别"顺手"去掉。
    #[test]
    fn dir_url_normalizes_base() {
        let cfg = WebDavConfig {
            base_url: "  https://x.com/dav/  ".to_string(),
            remote_dir: "civilcalc".to_string(),
            ..Default::default()
        };
        assert_eq!(cfg.normalized_base_url(), "https://x.com/dav");
        assert_eq!(cfg.dir_url(), "https://x.com/dav/civilcalc/");
    }

    /// 多级 `remote_dir` 里的斜杠会被清理（源同：整段作为一个集合名）
    #[test]
    fn dir_url_trims_dir_slashes() {
        let cfg = WebDavConfig {
            base_url: "https://x.com/dav".to_string(),
            remote_dir: "/civilcalc/".to_string(),
            ..Default::default()
        };
        assert_eq!(cfg.dir_url(), "https://x.com/dav/civilcalc/");
    }

    // ---- 导入方式解析 ----

    #[test]
    fn parse_mode_maps_both_modes() {
        assert!(!parse_mode("merge").unwrap());
        assert!(parse_mode("replace").unwrap());
    }

    #[test]
    fn parse_mode_rejects_unknown() {
        let e = parse_mode("overwrite").unwrap_err();
        match e {
            CommandError::InvalidArgument { message } => {
                assert!(message.contains("overwrite"), "实际: {message}");
                assert!(message.contains("merge"), "要告诉用户支持什么");
            }
            other => panic!("期望 invalidArgument，实际: {other:?}"),
        }
    }

    /// 大小写敏感：`Merge` 不是合法值（避免前端随手传驼峰）
    #[test]
    fn parse_mode_is_case_sensitive() {
        assert!(parse_mode("Merge").is_err());
        assert!(parse_mode("REPLACE").is_err());
    }
}
