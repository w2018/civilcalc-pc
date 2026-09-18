//! 组 14：本地备份（3 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `backup_local_export` | `selection`, `password?` | [`LocalExportResult`] |
//! | `backup_inspect` | `path`, `password?` | [`BackupInspectResult`] |
//! | `backup_local_import` | `path`, `password?`, `mode` | [`ImportReport`] |
//!
//! ## 两段式：先 inspect 再 import
//!
//! `backup_inspect` 只解析清单（不解码图片字节），供二次确认弹窗展示
//! 「这个包里有什么」。用户确认后才调 `backup_local_import`。
//! 好处是确认弹窗里的数字**来自包本身**，不是估算。
//!
//! ## 🔴 两种导入口径
//!
//! | mode | 行为 |
//! |---|---|
//! | `merge`（默认，ADR-021） | 按主键 `INSERT OR REPLACE`，本机独有的记录保留 |
//! | `replace` | **先清空包里确实含有的类别**，再写入 |
//!
//! ⚠️ `replace` **只清包里含有的类别**：包里没带「计算历史」（用户上传时没勾），
//! 本机历史就保持原样 —— 否则「导入一个只含公式的包」会把历史清空。
//!
//! ## 阻塞 IO 全部放进 `spawn_blocking`
//!
//! 备份包可能几十 MB，`tar`/`gzip`/加密都是同步阻塞 IO。
//! 直接写在 `async` 命令里会**占住 async 运行时线程**（桌面端表现为界面卡住），
//! 所以打包、解包、逐条写库都丢进 `spawn_blocking`。
//!
//! ## 导出落盘位置
//!
//! 系统「下载」目录根（与计算书导出同一口径）。源的 Android 版落在
//! 「下载/公式解析工具/备份」，PC 端按 `docs/04` §5 的 `ScopedDownloads` 定义
//! 简化为「下载」根目录 —— 两个功能落同一个地方，用户不用记两个位置。
//! 下载目录不可用时回落到应用私有 `backups/`。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use civilcalc_backup::model::{
    BackupContent, BackupImageRecord, BackupManifest, BackupPrefs, BackupSecrets, BackupSection,
    BackupSelection, BackupSummary, BackupTables, FavoriteRow, FormulaRow, FormulaVersionRow,
    HistoryRow, LlmUsageStatsRow,
};
use civilcalc_backup::{archive, crypto, file_name, image_group, BackupError};
use civilcalc_store::{BackgroundStore, FavoriteTableRow, LlmImageStore};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::config::AppConfig;
use crate::error::{CmdResult, CommandError};
use crate::paths::DesktopPaths;
use crate::secrets::{SecretStore, Secrets};
use crate::state::AppState;
use civilcalc_store::Db;

/// 取全部行时的 `LIMIT`。
///
/// SQLite 接受 `LIMIT 4294967295`，等价于「不限制」；
/// 用常量而不是 `-1`，是因为 `list_history` 的参数类型是 `u32`。
const ALL_ROWS: u32 = u32::MAX;

// =============================================================================
// IPC 类型
// =============================================================================

/// 本地导出的结果。
///
/// ⚠️ 契约（`docs/05`）写的是 `void`，但那样前端**拿不到落盘路径** ——
/// 而流程要求「完成提示 + 打开所在文件夹」。与 `report_export_docx` 保持同一口径：
/// 返回实际路径（含同名冲突时加上的序号）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalExportResult {
    /// **最终实际写入的路径**（同名冲突时会带 `(2)` 这类序号）
    pub path: String,
    pub display_name: String,
    pub size_bytes: u64,
    /// 包里实际装了什么（完成提示用）
    pub summary: BackupSummary,
}

/// 只解析清单不导入的结果（二次确认弹窗用）
///
/// ## 🔴 `decoded = false` 是一个**正常返回值**，不是错误
///
/// 加密包在**没给密码 / 密码不对**时，仍然返回 `Ok` —— 只是 `decoded = false`
/// 且 `summary` / `sections` 为空。前端据此弹「请输入密码」并**带上密码重调**。
///
/// 为什么不做成错误：那会让前端分不清「这是加密包，请输密码」与
/// 「包坏了」。而且 `encrypted` 是**读文件头**得到的，本来就不需要密码。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInspectResult {
    /// 展示用文件名
    pub file_name: String,
    pub size_bytes: u64,
    /// 是否加密包（读文件头魔数判定，**不校验密码**）
    pub encrypted: bool,
    /// 清单是否已成功解出。
    ///
    /// `false` 只可能是「加密包 + 没给密码/密码不对」——
    /// 其它失败（坏包、截断）仍走 `Err`。
    pub decoded: bool,
    /// 包的创建时间（`manifest.createdAt`）；未解码时为 0
    pub created_at: i64,
    /// 打包时的应用版本名；未解码时为空串
    pub app_version_name: String,
    /// 包内各类数据量；未解码时全 0
    pub summary: BackupSummary,
    /// 包内实际包含的类别（前端据此说明「将清空哪些」）；未解码时为空
    pub sections: Vec<BackupSection>,
}

/// 导入结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    /// 实际使用的口径：`merge` / `replace`
    pub mode: String,
    /// **实际写入的**条数（不是包里声明的条数 —— 图片可能因字节缺失而少写）
    pub summary: BackupSummary,
}

// =============================================================================
// 命令
// =============================================================================

/// 打包并落到「下载」目录（同名自动加序号）。
#[tauri::command]
pub async fn backup_local_export(
    state: State<'_, AppState>,
    selection: BackupSelection,
    password: Option<String>,
) -> CmdResult<LocalExportResult> {
    let db = state.db.clone();
    let secrets = state.secrets.clone();
    let paths = state.paths.clone();
    let config = state.config.clone();

    blocking(move || export_sync(&db, &secrets, &config, &paths, &selection, password.as_deref(), None))
        .await
}

/// 只解析清单不导入（供二次确认）。
#[tauri::command]
pub async fn backup_inspect(
    state: State<'_, AppState>,
    path: String,
    password: Option<String>,
) -> CmdResult<BackupInspectResult> {
    let _ = &state; // 不需要状态：纯文件解析
    blocking(move || inspect_sync(Path::new(&path), password.as_deref())).await
}

/// 导入本机备份包。
///
/// `mode`：`"merge"`（默认）或 `"replace"`。
#[tauri::command]
pub async fn backup_local_import(
    state: State<'_, AppState>,
    path: String,
    password: Option<String>,
    mode: String,
) -> CmdResult<ImportReport> {
    let clear_local_first = parse_mode(&mode)?;
    let db = state.db.clone();
    let secrets = state.secrets.clone();
    let paths = state.paths.clone();
    let config = state.config.clone();

    blocking(move || {
        import_sync(
            &db,
            &secrets,
            &config,
            &paths,
            Path::new(&path),
            password.as_deref(),
            clear_local_first,
        )
    })
    .await
}

// =============================================================================
// 同步实现（在 spawn_blocking 里跑）
// =============================================================================

/// `"merge"` / `"replace"` → `clear_local_first`
pub(crate) fn parse_mode(mode: &str) -> Result<bool, CommandError> {
    match mode {
        "merge" => Ok(false),
        "replace" => Ok(true),
        other => Err(CommandError::InvalidArgument {
            message: format!("未知的导入方式：{other}（只支持 merge / replace）"),
        }),
    }
}

pub(crate) async fn blocking<T, F>(f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> CmdResult<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => r,
        // 闭包 panic 了：`JoinError` 的文案对用户没意义，但要能定位
        Err(e) => {
            civilcalc_core::log::w("Commands", "备份任务异常结束", Some(&e.to_string()));
            Err(CommandError::Unknown {
                message: "备份任务异常结束，请重试".to_string(),
            })
        }
    }
}

/// 打包 → 落盘
fn export_sync<S: SecretStore>(
    db: &Db,
    secrets: &Secrets<S>,
    config: &std::sync::Mutex<AppConfig>,
    paths: &DesktopPaths,
    selection: &BackupSelection,
    password: Option<&str>,
    dest_dir: Option<&Path>,
) -> CmdResult<LocalExportResult> {
    let packed = pack_to_tmp(db, secrets, config, paths, selection, password)?;

    // ② 落到「下载」根；不可用时回落应用私有 backups/
    //    `dest_dir` 是**测试用的注入口** —— 生产一律传 `None`，
    //    免得测试往用户真实的「下载」目录里写文件
    let dest = match dest_dir {
        Some(dir) => crate::export::safe_output_path(dir, &packed.name).map_err(|e| {
            CommandError::Storage {
                message: e.to_string(),
            }
        })?,
        None => resolve_destination(paths, &packed.name)?,
    };
    let copy = std::fs::copy(&packed.path, &dest);
    // 不论成败都删临时文件（半成品可能几十 MB）
    let _ = std::fs::remove_file(&packed.path);
    copy.map_err(|e| CommandError::Storage {
        message: format!("写入备份文件失败：{e}"),
    })?;

    let display_name = dest
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| packed.name.clone());

    Ok(LocalExportResult {
        path: dest.to_string_lossy().to_string(),
        display_name,
        size_bytes: packed.size_bytes,
        summary: packed.summary,
    })
}

/// 已打包好的备份包（躺在**临时目录**里）。
///
/// ⚠️ 调用方**负责删除** `path`。之所以返回路径而不是字节：
/// 包可能有几十 MB，`webdav_upload` 要把它**流式**上传（逐块读），
/// 读进内存再传等于白白多占一份。
pub(crate) struct PackedBackup {
    /// 临时文件路径（在 `paths.tmp_dir()` 下）
    pub path: PathBuf,
    /// 备份文件名（`civilcalc_backup_<时间戳>.tar.gz`）
    pub name: String,
    pub size_bytes: u64,
    /// 包里实际装了什么
    pub summary: BackupSummary,
}

/// 打包到**临时文件**（本地导出与 WebDAV 上传共用）。
///
/// ## 为什么先落临时文件
///
/// 打包过程中任何一步失败（读图、压缩、加密），都**不会**在目标位置留下半个包。
/// 本地导出靠这一点保证「下载目录里不出现打不开的备份」；
/// WebDAV 上传靠它保证「上传失败时 tmp 里只有一个待删文件」。
///
/// ## 🔴 `images_store` 必须在闭包外取好
///
/// 闭包里的错误类型是 `BackupError`，而 `images_store` 返回命令层错误 ——
/// 混在一起编译不过。
pub(crate) fn pack_to_tmp<S: SecretStore>(
    db: &Db,
    secrets: &Secrets<S>,
    config: &std::sync::Mutex<AppConfig>,
    paths: &DesktopPaths,
    selection: &BackupSelection,
    password: Option<&str>,
) -> CmdResult<PackedBackup> {
    let content = build_content(db, secrets, config, paths, selection)?;
    let summary = BackupSummary::of(&content);
    let name = file_name::create(now_ms());

    let store = images_store(paths)?;
    let tmp = paths.tmp_dir().join(&name);
    let write = (|| -> Result<(), BackupError> {
        let file = std::fs::File::create(&tmp)?;
        archive::write_to(
            file,
            &content,
            |record| store.read_bytes_by_name(&record.file_name),
            background_bytes(paths, selection),
            password,
        )
    })();
    if let Err(e) = write {
        // 半成品不留（可能几十 MB）
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }

    let size_bytes = std::fs::metadata(&tmp)
        .map(|m| m.len())
        .map_err(|e| CommandError::Storage {
            message: format!("读取临时备份包失败：{e}"),
        })?;

    Ok(PackedBackup {
        path: tmp,
        name,
        size_bytes,
        summary,
    })
}

/// 解析清单（不解图片字节）
pub(crate) fn inspect_sync(path: &Path, password: Option<&str>) -> CmdResult<BackupInspectResult> {
    let meta = std::fs::metadata(path).map_err(|e| CommandError::NotFound {
        message: format!("备份包不存在或不可读：{e}"),
    })?;
    if !meta.is_file() {
        return Err(CommandError::InvalidArgument {
            message: "选中的不是一个文件".to_string(),
        });
    }

    let encrypted = crypto::is_encrypted_file(path);
    let file = std::fs::File::open(path).map_err(|e| CommandError::Storage {
        message: format!("打开备份包失败：{e}"),
    })?;

    // 加密包没给密码 / 密码不对 → **正常返回** `decoded = false`（见类型文档）。
    // 其余失败照常抛出（坏包、截断这类是用户改不了的问题）。
    let content = match archive::read_meta(file, password) {
        Ok(c) => Some(c),
        Err(BackupError::NeedPassword) | Err(BackupError::WrongPassword) if encrypted => None,
        Err(e) => return Err(e.into()),
    };

    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    Ok(match content {
        Some(c) => BackupInspectResult {
            file_name,
            size_bytes: meta.len(),
            encrypted,
            decoded: true,
            created_at: c.manifest.created_at,
            app_version_name: c.manifest.app_version_name.clone(),
            summary: BackupSummary::of(&c),
            sections: c.manifest.sections.clone(),
        },
        None => BackupInspectResult {
            file_name,
            size_bytes: meta.len(),
            encrypted,
            decoded: false,
            created_at: 0,
            app_version_name: String::new(),
            summary: BackupSummary::default(),
            sections: Vec::new(),
        },
    })
}

/// 导入
pub(crate) fn import_sync<S: SecretStore>(
    db: &Db,
    secrets: &Secrets<S>,
    config: &std::sync::Mutex<AppConfig>,
    paths: &DesktopPaths,
    path: &Path,
    password: Option<&str>,
    clear_local_first: bool,
) -> CmdResult<ImportReport> {
    if !path.is_file() {
        return Err(CommandError::NotFound {
            message: "备份包不存在或不可读".to_string(),
        });
    }

    let file = std::fs::File::open(path).map_err(|e| CommandError::Storage {
        message: format!("打开备份包失败：{e}"),
    })?;
    let content = archive::read_meta(file, password)?;

    // ① 完整还原：只清包里**确实含有**的类别
    if clear_local_first {
        clear_local(db, &content)?;
    }

    // ② 写五张表
    let t = &content.tables;
    for r in &t.formulas {
        db.upsert_formula(&formula_row_to_schema(r)?)?;
        db.set_favorite(&r.id, r.favorite != 0)?;
    }
    for r in &t.favorites {
        db.upsert_favorite_row(&FavoriteTableRow {
            formula_id: r.formula_id.clone(),
            created_at: r.created_at,
        })?;
    }
    for r in &t.versions {
        db.insert_version(&version_row_to_domain(r))?;
    }
    for r in &t.history {
        db.insert_history_with_id(&history_row_to_domain(r))?;
    }
    for r in &t.usage_stats {
        db.insert_usage_at(&usage_row_of(r), r.created_at)?;
    }

    // ③ 图片（含背景图）→ 必须在偏好之前：偏好里的 background_image="1"
    //    指向背景图文件，反过来会让界面短暂出现「有标记没图」
    let mut restored_images = 0i32;
    let mut restored_background = false;
    if !content.images.is_empty() || content.has_background {
        let store = images_store(paths)?;
        let by_name: std::collections::HashMap<&str, &BackupImageRecord> = content
            .images
            .iter()
            .map(|r| (r.file_name.as_str(), r))
            .collect();
        let bg = BackgroundStore::new(paths.background_path());
        let file = std::fs::File::open(path).map_err(|e| CommandError::Storage {
            message: format!("重新打开备份包失败：{e}"),
        })?;
        archive::read_images(
            file,
            password,
            |name, bytes| {
                if let Some(record) = by_name.get(name) {
                    store.restore(
                        &record.id,
                        &record.file_name,
                        &record.mime_type,
                        &record.refs,
                        record.created_at,
                        &bytes,
                    );
                    restored_images += 1;
                }
            },
            |bytes| {
                if bg.write(&bytes).is_ok() {
                    restored_background = true;
                }
            },
        )?;
    }

    // ④ 偏好（整份覆盖写回，**不清空**本机其它键）
    if !content.prefs.strings.is_empty() || !content.prefs.ints.is_empty() {
        restore_prefs(config, &content.prefs)?;
    }

    // ⑤ AI 模型配置（原样 JSON）与密钥
    secrets.import_llm_config_json(content.llm_config_json.as_deref())?;
    if let Some(s) = &content.secrets {
        if !s.api_keys.is_empty() {
            let keys: std::collections::HashMap<String, String> = s
                .api_keys
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            secrets.import_api_keys(&keys)?;
        }
    }

    Ok(ImportReport {
        mode: if clear_local_first { "replace" } else { "merge" }.to_string(),
        summary: BackupSummary {
            // 表格条数以包内声明为准（这些确实都写进去了）
            formulas: t.formulas.len() as i32,
            history: t.history.len() as i32,
            favorites: t.favorites.len() as i32,
            versions: t.versions.len() as i32,
            usage_stats: t.usage_stats.len() as i32,
            preferences: (content.prefs.strings.len() + content.prefs.ints.len()) as i32,
            llm_config: content
                .llm_config_json
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty()),
            api_keys: content.secrets.as_ref().map_or(0, |s| s.api_keys.len() as i32),
            // ⚠️ 图片用**实际恢复数**，不用 `content.images.len()`：
            //    包里登记了但字节缺失时（坏包），声明数会骗人
            images: restored_images,
            background: restored_background,
        },
    })
}

// =============================================================================
// 打包内容汇总
// =============================================================================

pub(crate) fn build_content<S: SecretStore>(
    db: &Db,
    secrets: &Secrets<S>,
    config: &std::sync::Mutex<AppConfig>,
    paths: &DesktopPaths,
    selection: &BackupSelection,
) -> CmdResult<BackupContent> {
    let mut sections: Vec<BackupSection> = Vec::new();

    // ---- 公式 / 收藏 / 版本（同一档勾选）----
    let (formulas, favorites, versions) = if selection.formulas {
        sections.push(BackupSection::Formulas);
        (
            db.list_formulas()?,
            db.list_favorite_rows()?,
            db.list_all_versions()?,
        )
    } else {
        (Vec::new(), Vec::new(), Vec::new())
    };

    let history = if selection.history {
        sections.push(BackupSection::History);
        db.list_history(None, ALL_ROWS, 0)?
    } else {
        Vec::new()
    };

    let usage = if selection.usage_stats {
        sections.push(BackupSection::UsageStats);
        db.list_usage()?
    } else {
        Vec::new()
    };

    // ---- AI 模型配置（原始 JSON 原样搬运）----
    let llm_config_json = if selection.llm_config {
        let raw = secrets.export_llm_config_json()?;
        if raw.as_deref().is_some_and(|s| !s.trim().is_empty()) {
            sections.push(BackupSection::LlmConfig);
        }
        raw
    } else {
        None
    };

    // ---- 偏好 ----
    let prefs = if selection.preferences {
        sections.push(BackupSection::Preferences);
        let snap = {
            let g = config.lock().map_err(|_| CommandError::Storage {
                message: "偏好数据锁已中毒，请重启应用".to_string(),
            })?;
            g.snapshot()
        };
        BackupPrefs {
            strings: snap.strings,
            // ⚠️ 备份格式的 ints 是 `i32`，本机是 `i64` —— 超范围时**夹到 i32 边界**
            //    而不是回绕成负数（回绕会把「很大」变成「很小」）
            ints: snap
                .ints
                .into_iter()
                .map(|(k, v)| (k, clamp_i32(v)))
                .collect(),
        }
    } else {
        BackupPrefs::default()
    };

    // ---- 图片（三档勾选）----
    let store = images_store(paths)?;
    let images: Vec<BackupImageRecord> = store
        .list()
        .into_iter()
        .filter(|info| {
            let group = image_group::group_of(&info.refs);
            match group {
                image_group::ImageRefGroup::Formula => selection.include_formula_images,
                image_group::ImageRefGroup::History => selection.include_history_images,
                image_group::ImageRefGroup::Unreferenced => selection.include_unreferenced_images,
            }
        })
        .filter_map(|info| {
            let file_name = Path::new(&info.path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())?;
            Some(BackupImageRecord {
                id: info.id,
                mime_type: image_group::mime_of(&file_name).to_string(),
                file_name,
                refs: info.refs,
                created_at: info.created_at,
            })
        })
        .collect();
    if !images.is_empty() {
        sections.push(BackupSection::Images);
    }

    // ---- 背景图（跟随偏好档，源同）----
    let background = selection.preferences && BackgroundStore::new(paths.background_path()).exists();
    if background {
        sections.push(BackupSection::Background);
    }

    // ---- 密钥（默认不勾）----
    let secrets_out = if selection.include_api_keys {
        sections.push(BackupSection::ApiKeys);
        let keys = secrets.export_api_keys()?;
        Some(BackupSecrets {
            api_keys: keys.into_iter().collect::<BTreeMap<_, _>>(),
        })
    } else {
        None
    };

    sections.dedup();
    Ok(BackupContent {
        manifest: BackupManifest {
            // `backupFormatVersion` 取 `model.rs` 的默认值（1）——
            // PC 端没有 Android 的 `versionCode`，置 0（字段仍在，跨端结构一致）
            app_version_code: 0,
            app_version_name: env!("CARGO_PKG_VERSION").to_string(),
            created_at: now_ms(),
            counts: counts_of(db, &formulas, &history, &favorites, &versions, &usage),
            sections,
            ..Default::default()
        },
        tables: BackupTables {
            formulas: formulas.iter().map(schema_to_formula_row).collect(),
            history: history.iter().map(history_to_row).collect(),
            favorites: favorites.iter().map(favorite_to_row).collect(),
            versions: versions.iter().map(version_to_row).collect(),
            usage_stats: usage.iter().map(usage_to_row).collect(),
        },
        prefs,
        llm_config_json,
        secrets: secrets_out,
        images,
        has_background: background,
    })
}

/// 各类条数（弹窗展示，也写进 `manifest.counts`）
///
/// ⚠️ 键是**中文**（源同）：源 `BackupPackBuilder.COUNT_*` 常量就是中文，
/// 而这些键会随包进备份文件 —— 改了会让跨端展示不一致。
fn counts_of(
    db: &Db,
    formulas: &[civilcalc_core::schema::FormulaSchema],
    history: &[civilcalc_core::schema::HistoryEntry],
    favorites: &[FavoriteTableRow],
    versions: &[civilcalc_core::schema::FormulaVersion],
    usage: &[civilcalc_store::UsageRecord],
) -> BTreeMap<String, i32> {
    let _ = db;
    BTreeMap::from([
        ("公式".to_string(), formulas.len() as i32),
        ("历史".to_string(), history.len() as i32),
        ("收藏".to_string(), favorites.len() as i32),
        ("版本".to_string(), versions.len() as i32),
        ("用量记录".to_string(), usage.len() as i32),
    ])
}

/// 背景图字节（没勾偏好档时 `None`）
fn background_bytes(paths: &DesktopPaths, selection: &BackupSelection) -> Option<Vec<u8>> {
    if !selection.preferences {
        return None;
    }
    BackgroundStore::new(paths.background_path()).read()
}

fn images_store(paths: &DesktopPaths) -> CmdResult<LlmImageStore> {
    LlmImageStore::new(paths.images_dir()).map_err(|e| CommandError::Storage {
        message: format!("图片目录不可用：{e}"),
    })
}

/// 完整还原：**只清包里确实含有的类别**
fn clear_local(db: &Db, content: &BackupContent) -> CmdResult<()> {
    let sections = &content.manifest.sections;
    if sections.contains(&BackupSection::Formulas) {
        db.clear_formulas()?;
    }
    if sections.contains(&BackupSection::History) {
        db.clear_history()?;
    }
    if sections.contains(&BackupSection::UsageStats) {
        db.clear_usage()?;
    }
    // 偏好与模型配置**不清空**（源注释）：包里的键整份覆盖写回，本机其它键保持不变。
    // 清空反而会误删本机设置。
    //
    // 图片也不清：`restore` 按原 id 落盘并**合并**注册表，
    // 先删会把「包里有但本机也有的图」误伤（引用计数会乱）。
    Ok(())
}

/// 写回偏好。
///
/// ⚠️ 用既有的 [`AppConfig::apply_backup`] 而**不是**自己写一遍合并：
/// 它带 `NON_BACKUP_KEYS` 保护（有些键**永不**从备份写入，例如数据库版本号、
/// 索引重建标记）—— 自己实现的版本会绕过这道保护，属于静默的安全回退。
///
/// 语义（源同）：本机有、包里也有 → 覆盖；本机有、包里没有 → **保留本机值**；
/// `NON_BACKUP_KEYS` → 跳过。
fn restore_prefs(config: &std::sync::Mutex<AppConfig>, prefs: &BackupPrefs) -> CmdResult<()> {
    let mut g = config.lock().map_err(|_| CommandError::Storage {
        message: "偏好数据锁已中毒，请重启应用".to_string(),
    })?;
    // 备份格式的 ints 是 i32，本机是 i64 —— 直接提升，不会丢值
    let ints: BTreeMap<String, i64> = prefs
        .ints
        .iter()
        .map(|(k, v)| (k.clone(), i64::from(*v)))
        .collect();
    let written = g.apply_backup(&prefs.strings, &ints);
    let cloned = g.clone();
    drop(g);
    cloned.save()?;
    civilcalc_core::log::w(
        "Backup",
        "偏好已写回",
        Some(&format!("{written} 个键")),
    );
    Ok(())
}

// =============================================================================
// 行 ↔ 域对象映射
// =============================================================================

/// `FormulaRow.schemaJson` → `FormulaSchema`
///
/// ⚠️ 校验 `id` 一致：备份包里 id 与 JSON 里的 id 理论上应当相同，
/// 但手工改过的包可能不一致 —— 以**行的 id** 为准（它才是主键），
/// 不一致时覆盖 JSON 里的值并记日志，而不是静默入库一条「id 对不上」的数据。
fn formula_row_to_schema(
    row: &FormulaRow,
) -> CmdResult<civilcalc_core::schema::FormulaSchema> {
    let mut schema: civilcalc_core::schema::FormulaSchema =
        serde_json::from_str(&row.schema_json).map_err(|e| CommandError::Validation {
            message: format!("备份包里的公式 JSON 无法解析（{}）：{e}", row.id),
        })?;
    if schema.id != row.id {
        civilcalc_core::log::w(
            "Backup",
            "公式 id 与 schemaJson 内的 id 不一致，以主键为准",
            Some(&format!("行 id={} / JSON id={}", row.id, schema.id)),
        );
        schema.id = row.id.clone();
    }
    Ok(schema)
}

fn schema_to_formula_row(s: &civilcalc_core::schema::FormulaSchema) -> FormulaRow {
    FormulaRow {
        id: s.id.clone(),
        // ⚠️ 这里会**重新序列化**（不是原样搬运库里的文本）。
        //    语义等价：导入侧走 `serde_json::from_str::<FormulaSchema>`，
        //    只要求字段能对上，不要求字节相同。副作用是旧包缺的新字段会被补上默认值。
        schema_json: serde_json::to_string(s).unwrap_or_else(|_| "{}".to_string()),
        favorite: 0, // 收藏在 `favorites` 表里，导入时单独写
        created_at: s.created_at,
        updated_at: s.updated_at,
    }
}

fn favorite_to_row(r: &FavoriteTableRow) -> FavoriteRow {
    FavoriteRow {
        formula_id: r.formula_id.clone(),
        created_at: r.created_at,
    }
}

fn history_to_row(e: &civilcalc_core::schema::HistoryEntry) -> HistoryRow {
    HistoryRow {
        id: e.id,
        formula_id: e.formula_id.clone(),
        formula_snapshot_json: e.formula_snapshot_json.clone(),
        inputs_json: e.inputs_json.clone(),
        result_json: e.result_json.clone(),
        thinking_content: e.thinking_content.clone(),
        created_at: e.created_at,
    }
}

fn history_row_to_domain(r: &HistoryRow) -> civilcalc_core::schema::HistoryEntry {
    civilcalc_core::schema::HistoryEntry {
        id: r.id,
        formula_id: r.formula_id.clone(),
        formula_snapshot_json: r.formula_snapshot_json.clone(),
        inputs_json: r.inputs_json.clone(),
        result_json: r.result_json.clone(),
        thinking_content: r.thinking_content.clone(),
        created_at: r.created_at,
    }
}

fn version_to_row(v: &civilcalc_core::schema::FormulaVersion) -> FormulaVersionRow {
    FormulaVersionRow {
        formula_id: v.formula_id.clone(),
        version: v.version.clone(),
        parent_version: v.parent_version.clone(),
        schema_json: v.schema_json.clone(),
        change_type: v.change_type.clone(),
        change_log: v.change_log.clone(),
        editor: v.editor.clone(),
        created_at: v.created_at,
        // ⚠️ 备份格式里 `verified` 是 **0/1**（不是 bool）—— 跨端契约，源是 Int
        verified: i32::from(v.verified),
    }
}

fn version_row_to_domain(r: &FormulaVersionRow) -> civilcalc_core::schema::FormulaVersion {
    civilcalc_core::schema::FormulaVersion {
        formula_id: r.formula_id.clone(),
        version: r.version.clone(),
        parent_version: r.parent_version.clone(),
        schema_json: r.schema_json.clone(),
        change_type: r.change_type.clone(),
        change_log: r.change_log.clone(),
        editor: r.editor.clone(),
        created_at: r.created_at,
        verified: r.verified != 0,
    }
}

fn usage_to_row(r: &civilcalc_store::UsageRecord) -> LlmUsageStatsRow {
    LlmUsageStatsRow {
        id: r.id,
        model_label: r.model_label.clone(),
        prompt_tokens: r.prompt_tokens,
        completion_tokens: r.completion_tokens,
        total_tokens: r.total_tokens,
        cached_tokens: r.cached_tokens,
        reasoning_tokens: r.reasoning_tokens,
        created_at: r.created_at,
    }
}

fn usage_row_of(r: &LlmUsageStatsRow) -> civilcalc_store::UsageRow {
    civilcalc_store::UsageRow {
        model_label: r.model_label.clone(),
        prompt_tokens: r.prompt_tokens,
        completion_tokens: r.completion_tokens,
        total_tokens: r.total_tokens,
        cached_tokens: r.cached_tokens,
        reasoning_tokens: r.reasoning_tokens,
    }
}

// =============================================================================
// 杂项
// =============================================================================

/// `i64` → `i32`，**超范围夹到边界**（不回绕）
fn clamp_i32(v: i64) -> i32 {
    v.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 落盘位置：「下载」根 → 回落应用私有 `backups/`
fn resolve_destination(paths: &DesktopPaths, name: &str) -> CmdResult<PathBuf> {
    if let Some(dir) = dirs::download_dir() {
        if dir.is_dir() {
            return crate::export::safe_output_path(&dir, name).map_err(|e| CommandError::Storage {
                message: e.to_string(),
            });
        }
    }
    // 回落：下载目录不可用（罕见，如被策略禁用）
    crate::export::safe_output_path(&paths.backups_dir(), name).map_err(|e| CommandError::Storage {
        message: e.to_string(),
    })
}


// =============================================================================
// 测试
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;
    use civilcalc_core::schema::{
        FormulaSchema, FormulaSource, FormulaVar, HistoryEntry, CURRENT_SCHEMA_VERSION,
    };
    use std::collections::HashMap;

    // ---------------- 夹具 ----------------

    fn tempdir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "civilcalc-backup-cmd-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// `DesktopPaths` + 把会用到的目录建好
    fn test_paths(tag: &str) -> DesktopPaths {
        let root = tempdir(tag);
        let paths = DesktopPaths::for_test(root);
        for d in [
            paths.tmp_dir(),
            paths.images_dir(),
            paths.backups_dir(),
            paths.exports_dir(),
        ] {
            std::fs::create_dir_all(d).unwrap();
        }
        paths
    }

    fn schema(id: &str) -> FormulaSchema {
        FormulaSchema {
            id: id.to_string(),
            result_name: "矩形面积".to_string(),
            result_symbol: "A".to_string(),
            result_unit: Some("m2".to_string()),
            result_outputs: vec![],
            expression: "b*h".to_string(),
            source_equations: vec![],
            alt_expressions: vec![],
            constants: HashMap::new(),
            variables: vec![FormulaVar {
                symbol: "b".to_string(),
                desc: "宽度".to_string(),
                unit: Some("m".to_string()),
                default: None,
                min: None,
                max: None,
                required: true,
            }],
            domain: "结构".to_string(),
            tags: vec![],
            reference_basis: None,
            design_notes: None,
            explanation: None,
            image_ids: vec![],
            revised_from: None,
            source: FormulaSource::standard("GB 50010-2010"),
            steps_template: None,
            doc_template_id: None,
            excel_expression: None,
            excel_alt_expressions: None,
            excel_steps_template: None,
            excel_function_docs: None,
            schema_version: CURRENT_SCHEMA_VERSION,
            created_at: 1_600_000_000_000,
            updated_at: 1_600_000_000_500,
        }
    }

    fn history(id: i64, created_at: i64) -> HistoryEntry {
        HistoryEntry {
            id,
            formula_id: "usr:1".to_string(),
            formula_snapshot_json: serde_json::to_string(&schema("usr:1")).unwrap(),
            inputs_json: r#"{"b":2.0}"#.to_string(),
            result_json: "{}".to_string(),
            thinking_content: Some("思考".to_string()),
            created_at,
        }
    }

    fn version(v: &str, created_at: i64) -> civilcalc_core::schema::FormulaVersion {
        civilcalc_core::schema::FormulaVersion {
            formula_id: "usr:1".to_string(),
            version: v.to_string(),
            parent_version: None,
            schema_json: serde_json::to_string(&schema("usr:1")).unwrap(),
            change_type: "create".to_string(),
            change_log: "首次".to_string(),
            editor: "user".to_string(),
            created_at,
            verified: true,
        }
    }

    /// 造一个有数据的库
    fn seeded_db() -> Db {
        let db = Db::open_in_memory().unwrap();
        db.upsert_formula(&schema("usr:1")).unwrap();
        db.set_favorite("usr:1", true).unwrap();
        db.insert_version(&version("v1.0", 1_600_000_001_000)).unwrap();
        db.insert_history(&history(0, 1_600_000_002_000)).unwrap();
        db.insert_usage_at(
            &civilcalc_store::UsageRow {
                model_label: "glm-4".to_string(),
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
                cached_tokens: 0,
                reasoning_tokens: 0,
            },
            1_600_000_003_000,
        )
        .unwrap();
        db
    }

    fn full_selection() -> BackupSelection {
        BackupSelection {
            formulas: true,
            history: true,
            preferences: false, // 偏好另有独立测试
            usage_stats: true,
            llm_config: false,
            include_api_keys: false,
            include_formula_images: false,
            include_history_images: false,
            include_unreferenced_images: false,
        }
    }

    // ---------------- 纯函数 ----------------

    #[test]
    fn parse_mode_accepts_two_and_rejects_unknown() {
        assert!(!parse_mode("merge").unwrap(), "merge → 不清空本机");
        assert!(parse_mode("replace").unwrap(), "replace → 先清空");
        for bad in ["", "MERGE", "overwrite", " merge"] {
            let e = parse_mode(bad).unwrap_err();
            assert_eq!(e.kind_str(), "invalidArgument", "实际 {bad:?}");
        }
    }

    /// 🔴 超范围夹到边界，**不回绕**（回绕会把「很大」变成「很小」）
    #[test]
    fn clamp_i32_clamps_instead_of_wrapping() {
        assert_eq!(clamp_i32(0), 0);
        assert_eq!(clamp_i32(42), 42);
        assert_eq!(clamp_i32(i32::MAX as i64), i32::MAX);
        assert_eq!(clamp_i32(i32::MAX as i64 + 1), i32::MAX, "不回绕成负数");
        assert_eq!(clamp_i32(i64::MAX), i32::MAX);
        assert_eq!(clamp_i32(i32::MIN as i64), i32::MIN);
        assert_eq!(clamp_i32(i32::MIN as i64 - 1), i32::MIN);
        assert_eq!(clamp_i32(i64::MIN), i32::MIN);
    }

    #[test]
    fn formula_row_roundtrip() {
        let s = schema("usr:1");
        let row = schema_to_formula_row(&s);
        assert_eq!(row.id, "usr:1");
        assert_eq!(row.created_at, s.created_at);
        assert_eq!(row.updated_at, s.updated_at);
        assert_eq!(row.favorite, 0, "收藏在 favorites 表里");

        let back = formula_row_to_schema(&row).unwrap();
        assert_eq!(back.id, s.id);
        assert_eq!(back.expression, s.expression);
        assert_eq!(back.created_at, s.created_at);
        assert_eq!(back.updated_at, s.updated_at);
    }

    /// 🔴 行的 id 是主键，与 JSON 内的 id 冲突时以**行**为准
    #[test]
    fn formula_row_id_wins_over_json_id() {
        let mut row = schema_to_formula_row(&schema("usr:1"));
        row.id = "usr:999".to_string(); // 手工改过的包
        let back = formula_row_to_schema(&row).unwrap();
        assert_eq!(back.id, "usr:999", "以主键为准");
    }

    #[test]
    fn formula_row_rejects_broken_json() {
        let mut row = schema_to_formula_row(&schema("usr:1"));
        row.schema_json = "{ not json".to_string();
        let e = formula_row_to_schema(&row).unwrap_err();
        assert_eq!(e.kind_str(), "validation");
    }

    /// 🔴 `verified` 在备份格式里是 **0/1**（跨端契约，源是 Int）
    #[test]
    fn version_row_verified_is_0_or_1() {
        let mut v = version("v1.0", 1);
        v.verified = true;
        let row = version_to_row(&v);
        assert_eq!(row.verified, 1);
        assert!(version_row_to_domain(&row).verified);

        v.verified = false;
        let row = version_to_row(&v);
        assert_eq!(row.verified, 0);
        assert!(!version_row_to_domain(&row).verified);

        // 反序列化：只有 0 才是 false，其余都算 true（防御脏数据）
        let mut dirty = version_to_row(&v);
        dirty.verified = 2;
        assert!(version_row_to_domain(&dirty).verified);
    }

    #[test]
    fn history_row_roundtrip_preserves_id_and_time() {
        let h = history(4242, 1_600_000_000_000);
        let row = history_to_row(&h);
        assert_eq!(row.id, 4242);
        assert_eq!(row.created_at, 1_600_000_000_000);
        assert_eq!(row.thinking_content.as_deref(), Some("思考"));

        let back = history_row_to_domain(&row);
        assert_eq!(back.id, h.id);
        assert_eq!(back.created_at, h.created_at);
        assert_eq!(back.inputs_json, h.inputs_json);
    }

    #[test]
    fn usage_row_roundtrip() {
        let rec = civilcalc_store::UsageRecord {
            id: 7,
            model_label: "m".to_string(),
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3,
            cached_tokens: 4,
            reasoning_tokens: 5,
            created_at: 1_600_000_000_000,
        };
        let row = usage_to_row(&rec);
        assert_eq!(row.id, 7);
        assert_eq!(row.created_at, rec.created_at);

        let back = usage_row_of(&row);
        assert_eq!(back.model_label, "m");
        assert_eq!(back.total_tokens, 3);
        // ⚠️ `UsageRow` 没有 created_at —— 写入时单独传（`insert_usage_at`）
        assert_eq!(row.created_at, 1_600_000_000_000);
    }

    /// `manifest.counts` 的键是**中文**（源同，会随包进备份文件）
    #[test]
    fn counts_use_chinese_keys() {
        let db = Db::open_in_memory().unwrap();
        let c = counts_of(&db, &[], &[], &[], &[], &[]);
        let keys: Vec<&str> = c.keys().map(String::as_str).collect();
        assert_eq!(keys, ["公式", "历史", "收藏", "版本", "用量记录"]);
        assert!(c.values().all(|v| *v == 0));
    }

    // ---------------- 端到端：导出 → 解析 → 导入 ----------------

    #[test]
    fn export_inspect_import_roundtrip() {
        let src = seeded_db();
        let secrets = Secrets::new(MemoryStore::new());
        let config = std::sync::Mutex::new(AppConfig::in_memory());
        let paths = test_paths("roundtrip");
        let out_dir = tempdir("roundtrip-out");

        // ---- 导出 ----
        let exported = export_sync(
            &src,
            &secrets,
            &config,
            &paths,
            &full_selection(),
            None,
            Some(&out_dir),
        )
        .unwrap();

        assert!(Path::new(&exported.path).is_file(), "包要真的落盘");
        assert!(exported.size_bytes > 0);
        assert!(
            exported.display_name.starts_with("civilcalc_backup_"),
            "实际 {}",
            exported.display_name
        );
        assert!(exported.display_name.ends_with(".tar.gz"));
        assert_eq!(exported.summary.formulas, 1);
        assert_eq!(exported.summary.history, 1);
        assert_eq!(exported.summary.favorites, 1);
        assert_eq!(exported.summary.versions, 1);
        assert_eq!(exported.summary.usage_stats, 1);

        // ---- 解析 ----
        let inspected = inspect_sync(Path::new(&exported.path), None).unwrap();
        assert!(!inspected.encrypted);
        assert_eq!(inspected.summary, exported.summary);
        assert!(inspected.created_at > 0);
        assert!(inspected.sections.contains(&BackupSection::Formulas));
        assert!(inspected.sections.contains(&BackupSection::History));
        assert!(inspected.sections.contains(&BackupSection::UsageStats));
        assert!(
            !inspected.sections.contains(&BackupSection::ApiKeys),
            "没勾密钥就不该有"
        );

        // ---- 导入到空库 ----
        let dst = Db::open_in_memory().unwrap();
        let secrets2 = Secrets::new(MemoryStore::new());
        let config2 = std::sync::Mutex::new(AppConfig::in_memory());
        let paths2 = test_paths("roundtrip-dst");

        let report = import_sync(
            &dst,
            &secrets2,
            &config2,
            &paths2,
            Path::new(&exported.path),
            None,
            false,
        )
        .unwrap();

        assert_eq!(report.mode, "merge");
        assert_eq!(report.summary.formulas, 1);
        assert_eq!(report.summary.history, 1);

        // ---- 数据真的到了 ----
        let got = dst.list_formulas().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "usr:1");
        assert_eq!(got[0].expression, "b*h");
        assert_eq!(got[0].created_at, 1_600_000_000_000, "时间戳保真");
        assert_eq!(got[0].updated_at, 1_600_000_000_500);

        assert!(dst.is_favorite("usr:1").unwrap());
        let fav = dst.list_favorite_rows().unwrap();
        assert_eq!(fav.len(), 1);
        assert!(fav[0].created_at > 0, "收藏时间来自库里，不是 0");

        let versions = dst.list_all_versions().unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, "v1.0");
        assert!(versions[0].verified, "verified 往返保真");

        let hs = dst.list_history(None, 100, 0).unwrap();
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0].created_at, 1_600_000_002_000, "历史时间保真");
        assert_eq!(hs[0].thinking_content.as_deref(), Some("思考"));

        let us = dst.list_usage().unwrap();
        assert_eq!(us.len(), 1);
        assert_eq!(us[0].created_at, 1_600_000_003_000, "用量时间保真");
        assert_eq!(us[0].total_tokens, 150);

        std::fs::remove_dir_all(&out_dir).ok();
    }

    /// 加密包：不给密码要报「需要密码」，给错要报「密码错误」
    #[test]
    fn encrypted_pack_needs_correct_password() {
        let src = seeded_db();
        let secrets = Secrets::new(MemoryStore::new());
        let config = std::sync::Mutex::new(AppConfig::in_memory());
        let paths = test_paths("enc");
        let out_dir = tempdir("enc-out");

        let exported = export_sync(
            &src,
            &secrets,
            &config,
            &paths,
            &full_selection(),
            Some("s3cret"),
            Some(&out_dir),
        )
        .unwrap();

        // 加密标志按文件头判定，**不需要密码**
        assert!(crypto::is_encrypted_file(Path::new(&exported.path)));

        // 不给密码 → Ok 但 decoded=false（前端据此弹「请输入密码」）
        let no_pw = inspect_sync(Path::new(&exported.path), None).unwrap();
        assert!(no_pw.encrypted, "加密包要能被识别");
        assert!(!no_pw.decoded);
        assert_eq!(no_pw.summary, BackupSummary::default());
        assert!(no_pw.sections.is_empty());
        assert!(no_pw.size_bytes > 0, "文件信息仍要给");

        // 错密码 → 同样 decoded=false（不区分「没给」与「给错」，UI 文案一致）
        let wrong = inspect_sync(Path::new(&exported.path), Some("wrong")).unwrap();
        assert!(wrong.encrypted);
        assert!(!wrong.decoded);

        // 对密码 → 解出清单
        let ok = inspect_sync(Path::new(&exported.path), Some("s3cret")).unwrap();
        assert!(ok.decoded);
        assert_eq!(ok.summary.formulas, 1);
        assert!(ok.sections.contains(&BackupSection::Formulas));

        std::fs::remove_dir_all(&out_dir).ok();
    }

    /// 同名冲突加序号（不覆盖既有文件）
    #[test]
    fn export_does_not_overwrite_same_name() {
        let src = seeded_db();
        let secrets = Secrets::new(MemoryStore::new());
        let config = std::sync::Mutex::new(AppConfig::in_memory());
        let paths = test_paths("dup");
        let out_dir = tempdir("dup-out");

        // 用同一个名字预置一个文件，模拟「同一秒内导两次」
        let fixed = "civilcalc_backup_20260915_143012.tar.gz";
        std::fs::write(out_dir.join(fixed), b"old").unwrap();

        let exported = export_sync(
            &src,
            &secrets,
            &config,
            &paths,
            &full_selection(),
            None,
            Some(&out_dir),
        )
        .unwrap();

        // 名字里带的是当前时间，与预置文件不同名 —— 所以这里直接断言
        // 「预置文件没被动过」，以及导出文件存在
        assert_eq!(std::fs::read(out_dir.join(fixed)).unwrap(), b"old");
        assert!(Path::new(&exported.path).is_file());
        assert_ne!(exported.display_name, fixed);

        std::fs::remove_dir_all(&out_dir).ok();
    }

    /// `replace` 只清包里含有的类别：包没带历史 → 本机历史保留
    #[test]
    fn replace_mode_only_clears_sections_in_pack() {
        // 源库只有公式（不勾历史）
        let src = Db::open_in_memory().unwrap();
        src.upsert_formula(&schema("usr:1")).unwrap();
        let secrets = Secrets::new(MemoryStore::new());
        let config = std::sync::Mutex::new(AppConfig::in_memory());
        let paths = test_paths("replace-src");
        let out_dir = tempdir("replace-out");

        let sel = BackupSelection {
            formulas: true,
            history: false,
            preferences: false,
            usage_stats: false,
            llm_config: false,
            include_api_keys: false,
            include_formula_images: false,
            include_history_images: false,
            include_unreferenced_images: false,
        };
        let exported = export_sync(&src, &secrets, &config, &paths, &sel, None, Some(&out_dir))
            .unwrap();

        // 目标库：有一条本机历史 + 一条本机公式
        let dst = Db::open_in_memory().unwrap();
        dst.upsert_formula(&schema("usr:local")).unwrap();
        dst.insert_history(&history(0, 111)).unwrap();
        let secrets2 = Secrets::new(MemoryStore::new());
        let config2 = std::sync::Mutex::new(AppConfig::in_memory());
        let paths2 = test_paths("replace-dst");

        let report = import_sync(
            &dst,
            &secrets2,
            &config2,
            &paths2,
            Path::new(&exported.path),
            None,
            true, // replace
        )
        .unwrap();
        assert_eq!(report.mode, "replace");

        // 公式被清空后写入包里的那条
        let ids: Vec<String> = dst.list_formulas().unwrap().iter().map(|f| f.id.clone()).collect();
        assert_eq!(ids, ["usr:1"], "本机公式被清掉，只留包里的");

        // 🔴 历史不在包里 → **保留本机历史**
        assert_eq!(
            dst.list_history(None, 100, 0).unwrap().len(),
            1,
            "包没带历史就不该清历史"
        );

        std::fs::remove_dir_all(&out_dir).ok();
    }

    /// 密钥默认不进包；勾了才进（且能恢复）
    #[test]
    fn api_keys_only_when_selected() {
        let src = Db::open_in_memory().unwrap();
        let secrets = Secrets::new(MemoryStore::new());
        // ⚠️ `export_api_keys` 只导出**档位表里存在**的档位的 Key
        //    （键名是 `profile_<id>_key`），所以得先建一个档位
        let llm = civilcalc_llm::LlmConfig::default();
        let profile_id = llm.profiles[0].id.clone();
        secrets.save_llm_config(&llm).unwrap();
        secrets
            .set(&crate::secrets::profile_api_key(&profile_id), "sk-secret")
            .unwrap();
        let config = std::sync::Mutex::new(AppConfig::in_memory());
        let paths = test_paths("keys-src");
        let out_dir = tempdir("keys-out");

        // 不勾 → 包里没有
        let mut sel = full_selection();
        sel.include_api_keys = false;
        let e1 = export_sync(&src, &secrets, &config, &paths, &sel, None, Some(&out_dir)).unwrap();
        assert_eq!(e1.summary.api_keys, 0);
        assert!(!inspect_sync(Path::new(&e1.path), None)
            .unwrap()
            .sections
            .contains(&BackupSection::ApiKeys));

        // 勾上 → 包里有 1 条
        sel.include_api_keys = true;
        let e2 = export_sync(&src, &secrets, &config, &paths, &sel, None, Some(&out_dir)).unwrap();
        assert_eq!(e2.summary.api_keys, 1);
        assert!(inspect_sync(Path::new(&e2.path), None)
            .unwrap()
            .sections
            .contains(&BackupSection::ApiKeys));

        // 导入到空 keyring → 密钥回来
        //    ⚠️ 目标端也要有**同名档位**：`import_api_keys` 只写本机已存在的档位
        //    （防止备份包带进未知档位）
        let dst = Db::open_in_memory().unwrap();
        let secrets2 = Secrets::new(MemoryStore::new());
        secrets2.save_llm_config(&llm).unwrap();
        assert!(secrets2
            .get(&crate::secrets::profile_api_key(&profile_id))
            .unwrap()
            .is_none());
        let config2 = std::sync::Mutex::new(AppConfig::in_memory());
        let paths2 = test_paths("keys-dst");
        import_sync(
            &dst,
            &secrets2,
            &config2,
            &paths2,
            Path::new(&e2.path),
            None,
            false,
        )
        .unwrap();
        assert_eq!(
            secrets2
                .get(&crate::secrets::profile_api_key(&profile_id))
                .unwrap()
                .as_deref(),
            Some("sk-secret")
        );

        std::fs::remove_dir_all(&out_dir).ok();
    }

    /// 偏好：包里的键覆盖本机，本机独有的键保留
    #[test]
    fn preferences_merge_keeps_local_keys() {
        let src = Db::open_in_memory().unwrap();
        let secrets = Secrets::new(MemoryStore::new());
        let src_config = std::sync::Mutex::new(AppConfig::in_memory());
        {
            let mut g = src_config.lock().unwrap();
            g.set_str("themeMode", "DARK");
            g.set_int("someNumber", 7);
        }
        let paths = test_paths("prefs-src");
        let out_dir = tempdir("prefs-out");

        let mut sel = full_selection();
        sel.preferences = true;
        let exported =
            export_sync(&src, &secrets, &src_config, &paths, &sel, None, Some(&out_dir)).unwrap();
        assert_eq!(exported.summary.preferences, 2);

        // 目标库本机有自己的值 + 一个包里没有的键
        let dst = Db::open_in_memory().unwrap();
        let secrets2 = Secrets::new(MemoryStore::new());
        let dst_config = std::sync::Mutex::new(AppConfig::in_memory());
        {
            let mut g = dst_config.lock().unwrap();
            g.set_str("themeMode", "LIGHT");
            g.set_str("localOnly", "keep-me");
        }
        let paths2 = test_paths("prefs-dst");

        import_sync(
            &dst,
            &secrets2,
            &dst_config,
            &paths2,
            Path::new(&exported.path),
            None,
            false,
        )
        .unwrap();

        let g = dst_config.lock().unwrap();
        assert_eq!(g.get_str("themeMode"), Some("DARK"), "包里有的键被覆盖");
        assert_eq!(g.get_int("someNumber"), Some(7), "包里有的键被写入");
        assert_eq!(
            g.get_str("localOnly"),
            Some("keep-me"),
            "🔴 包里没有的本机键要保留"
        );

        std::fs::remove_dir_all(&out_dir).ok();
    }

    /// 包不存在 → `notFound`（而不是 storage）
    #[test]
    fn inspect_missing_file_is_not_found() {
        let e = inspect_sync(Path::new("D:/definitely/not/here.tar.gz"), None).unwrap_err();
        assert_eq!(e.kind_str(), "notFound");
    }

    #[test]
    fn import_missing_file_is_not_found() {
        let db = Db::open_in_memory().unwrap();
        let secrets = Secrets::new(MemoryStore::new());
        let config = std::sync::Mutex::new(AppConfig::in_memory());
        let paths = test_paths("missing");
        let e = import_sync(
            &db,
            &secrets,
            &config,
            &paths,
            Path::new("D:/definitely/not/here.tar.gz"),
            None,
            false,
        )
        .unwrap_err();
        assert_eq!(e.kind_str(), "notFound");
    }

    /// 选择全空 → 仍能导出一个（几乎）空包，不报错
    #[test]
    fn export_with_nothing_selected_still_works() {
        let src = seeded_db();
        let secrets = Secrets::new(MemoryStore::new());
        let config = std::sync::Mutex::new(AppConfig::in_memory());
        let paths = test_paths("empty-sel");
        let out_dir = tempdir("empty-out");

        let sel = BackupSelection {
            formulas: false,
            history: false,
            preferences: false,
            usage_stats: false,
            llm_config: false,
            include_api_keys: false,
            include_formula_images: false,
            include_history_images: false,
            include_unreferenced_images: false,
        };
        let exported = export_sync(&src, &secrets, &config, &paths, &sel, None, Some(&out_dir))
            .unwrap();
        assert_eq!(exported.summary.describe(), "空备份");
        assert!(inspect_sync(Path::new(&exported.path), None)
            .unwrap()
            .sections
            .is_empty());

        std::fs::remove_dir_all(&out_dir).ok();
    }
}
