//! # civilcalc-backup —— 备份体系
//!
//! 源：`civilcalc-android-v2/core/backup/`（13 文件 1,683 行）
//!
//! ## 硬约束
//!
//! - **不得依赖 `tauri`**（ADR-002）
//! - **ADR-009：与 Android 端二进制兼容** —— 用户可能已有云端备份
//!
//! ## 模块对应
//!
//! | 本 crate 模块 | 源文件 | 任务 |
//! |---|---|---|
//! | `model.rs` | `BackupModel.kt` | P4-9 |
//! | `file_name.rs` | `BackupFileName.kt` | P4-9 |
//! | `stats.rs` | `BackupStats.kt` | P4-9 |
//! | `temp_cache.rs` | `TempCache.kt` | P4-9 |
//! | `tar_gz.rs` | `TarGz.kt` | P4-10 |
//! | `crypto.rs` | `BackupCrypto.kt` | P4-11 |
//! | `archive.rs` | `BackupArchive.kt` | P4-12 |
//! | `webdav_path.rs` | `WebDavPath.kt` | P4-13 |
//! | `webdav_xml.rs` | `WebDavXml.kt` | P4-13 |
//! | `webdav_client.rs` | `WebDavClient.kt` | P4-13 |
//! | `webdav_repo.rs` | `WebDavRepository.kt` | P4-13 |
//! | `webdav_config.rs` | `WebDavConfig.kt` | P4-9 |
//!
//! ## ⚠️ 二进制兼容的 8 个兼容点（ADR-009，逐字节对齐）
//!
//! | 项 | 要求 |
//! |---|---|
//! | 文件名 | `civilcalc_backup_yyyyMMdd_HHmmss.tar.gz` |
//! | tar 格式 | 512 字节块；长文件名用 GNU LongLink（`././@LongLink`，typeflag `'L'`）；单条目上限 512 MB |
//! | 条目名 | `manifest.json` / `tables.json` / `prefs.json` / `llm_config.json` / `secrets.json` / `images.json` / `background.jpg` / `images/<hash>` |
//! | 加密头 | `MAGIC("CCENC1", 6B)` + `VERSION(1B)` + `salt(16B)` |
//! | 加密参数 | PBKDF2-HMAC-SHA256，**200,000 次迭代**，256 位 |
//! | 加密算法 | AES-256-GCM；**明文块 64 KiB**；nonce 12B；tag 16B |
//! | 块格式 | `[明文长度(4, 大端)] [nonce(12)] [密文+tag(16)]`；**块序号作 AAD** |
//! | 加密识别 | 按文件头自动识别 → 明文包误填密码仍可读；加密包必须填对密码 |
//!
//! ## ⚠️ 两条实现纪律
//!
//! 1. **保留自研 tar**（不引入 `tar` crate）—— 才能精确控制 LongLink 与块填充
//! 2. **不要用会吞掉错误的流适配器** —— 源项目刻意不用 `CipherInputStream`，
//!    因为它在 GCM 校验失败时可能静默当作流结束

pub mod error;
pub mod webdav_config;
pub mod webdav_path;

pub mod file_name;
pub mod image_group;
pub mod model;
pub mod stats;
pub mod temp_cache;
pub mod tar_gz;
pub mod crypto;
pub mod archive;
pub mod webdav_xml;
pub mod webdav_client;

/// 测试专用：mock server 与夹具（只在 `cfg(test)` 下编译）
#[cfg(test)]
mod test_mock;
pub mod webdav_repo;

pub use image_group::{group_of, mime_of, ImageRefGroup};
pub use model::BackupSummary;
pub use error::{webdav_error, webdav_status_message, BackupError};
pub use webdav_client::{is_delete_ok, is_mkcol_ok, CancelFlag, WebDavClient};
pub use webdav_repo::{RemoteBackup, RemoteListResult, UploadResult};
pub use webdav_config::{
    default_base_url_of, WebDavConfig, WebDavConnection, WebDavPreset, DEFAULT_BASE_URL,
    DEFAULT_REMOTE_DIR, LIST_LIMIT,
};
