//! WebDAV 备份仓库（编排层）。
//!
//! 源：`data/repository/WebDavRepositoryImpl.kt`（399 行）+ `core/backup/WebDavRepository.kt`（接口）
//!
//! ## 本模块的边界
//!
//! 只做**与平台无关的编排**：建目录 → 列 / 传 / 下 / 删，以及纯函数式的
//! 「条目筛选排序」「提示文案」「文件名校验」。
//!
//! **不做**（留给 `src-tauri` 的命令层）：
//! - 读写 `WebDavConfig` 与密码（要走 keyring）
//! - 临时目录的创建与清理（要走 `DesktopPaths`）
//! - 打包（[`crate::archive`]）与导入（需要 db）
//! - 事件发射（`backup://progress` / `backup://cancelled`）
//!
//! 这样切分的收益：本模块的每条路径都能用 mock server 离线测，
//! 而命令层只剩「接线」——那部分靠手工验收。
//!
//! ## 两段式流程（源的设计）
//!
//! 先打包出**真实体积**再让用户确认上传；先下载出清单再让用户确认导入。
//! 确认弹窗里给出的都是实际数字，不是估算。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::BackupError;
use crate::file_name;
use crate::webdav_client::{CancelFlag, WebDavClient};
use crate::webdav_config::{WebDavConnection, LIST_LIMIT};
use crate::webdav_path::file_url;
use crate::webdav_xml::RemoteEntry;

/// 远端一条备份记录。
///
/// 序列化用 camelCase（跨 IPC 给前端）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBackup {
    pub name: String,
    pub size_bytes: i64,
    /// 服务端给的修改时间（可能为 0 —— 有的网盘不返回 `getlastmodified`）
    pub last_modified_ms: i64,
    /// **展示与排序**用时间：文件名里的时间优先。
    ///
    /// 服务端时间不可靠（不返回、或返回 UTC 却标成本地）；文件名里的时间戳
    /// 是本机生成备份时写进去的，更可信。构造时算好，避免前端重复实现文件名解析。
    pub timestamp_ms: i64,
}

impl RemoteBackup {
    /// 按源的 `RemoteBackup.timestampMs` 语义构造（文件名时间 → 服务端时间兜底）
    pub fn new(name: impl Into<String>, size_bytes: i64, last_modified_ms: i64) -> Self {
        let name = name.into();
        let timestamp_ms = file_name::timestamp_of(&name).unwrap_or(last_modified_ms);
        Self {
            name,
            size_bytes,
            last_modified_ms,
            timestamp_ms,
        }
    }
}

/// 远端列表结果。
///
/// `backups` 已截断到 [`LIST_LIMIT`]，`total` 是**服务器上的总数** ——
/// 界面据此提示「还有 N 条更早的」。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteListResult {
    pub backups: Vec<RemoteBackup>,
    pub total: usize,
}

/// 上传成功的结果
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadResult {
    pub name: String,
    pub size_bytes: u64,
    /// 最终上传到的 URL（可用于展示与排错）
    pub remote_url: String,
}

// =============================================================================
// 纯函数
// =============================================================================

/// 从 PROPFIND 条目里筛出备份记录：**过滤目录与非本应用的包 → 按时间倒序 → 截断**。
///
/// 截断后仍保留 `total`（截断前条数），所以「还有 N 条更早的」是准的。
pub fn pick_backups(entries: &[RemoteEntry]) -> RemoteListResult {
    let mut backups: Vec<RemoteBackup> = entries
        .iter()
        .filter(|e| !e.is_collection && file_name::is_backup(&e.display_name))
        .map(|e| RemoteBackup::new(e.display_name.clone(), e.size_bytes, e.last_modified_ms))
        .collect();
    // 稳定排序（`sort_by_key` 是稳定的）：同一时间戳（同一秒内导出多个）
    // 保持服务端返回的顺序
    backups.sort_by_key(|b| std::cmp::Reverse(b.timestamp_ms));
    let total = backups.len();
    backups.truncate(LIST_LIMIT);
    RemoteListResult { backups, total }
}

/// 测试连接成功的提示文案（源 `WebDavRepositoryImpl.testConnection` 逐字对齐）
pub fn connection_ok_message(dir_url: &str, file_count: usize) -> String {
    format!("连接成功：远端目录 {dir_url}（已有 {file_count} 个文件）")
}

/// 远端某个备份包的 URL
pub fn remote_file_url(conn: &WebDavConnection, name: &str) -> String {
    file_url(&conn.config.base_url, &conn.config.remote_dir, name)
}

/// 校验文件名是本应用产生的备份包。
///
/// 🔴 **下载与删除都必须先过这一关**（源同）——
/// `name` 来自远端目录列表，而目录里可能混着用户自己放的文件。
/// 不校验就会「删除用户的文件」，或把无关文件当备份包下载并尝试解码。
///
/// ## ⚠️ 这是**弱校验**：只看「前缀 + 后缀 + 不是空壳」
///
/// 判据就是 [`crate::file_name::is_backup`]，**不校验中间的时间戳格式**。
/// 所以 `civilcalc_backup_坏名字.tar.gz` 会**通过**校验 —— 源项目也是如此。
///
/// 这不是疏漏：它的职责是「别把用户自己的文件当备份包」，
/// 而不是「证明这是本应用生成的」（那要靠包内 manifest 校验，见
/// [`crate::archive::read_meta`]）。真正的把关在解码阶段。
pub fn ensure_backup_name(name: &str) -> Result<(), BackupError> {
    if file_name::is_backup(name) {
        return Ok(());
    }
    Err(BackupError::WebDav {
        status: None,
        user_message: format!("这不是本应用产生的备份包：{name}"),
    })
}

// =============================================================================
// 编排
// =============================================================================

/// 远端备份列表。
///
/// 先 [`WebDavClient::ensure_directory`]（首次使用时远端目录还不存在，
/// 已存在时 MKCOL 返回 405 被当成功），再 PROPFIND。
pub async fn list_remote(
    client: &WebDavClient,
    conn: &WebDavConnection,
) -> Result<RemoteListResult, BackupError> {
    client.ensure_directory(conn, None).await?;
    let entries = client.list(conn, &conn.dir_url()).await?;
    Ok(pick_backups(&entries))
}

/// 测试连接：建目录 + 列一次，返回给用户看的说明。
pub async fn test_connection(
    client: &WebDavClient,
    conn: &WebDavConnection,
) -> Result<String, BackupError> {
    client.ensure_directory(conn, None).await?;
    let entries = client.list(conn, &conn.dir_url()).await?;
    // 只数文件（不含目录项本身）
    let files = entries.iter().filter(|e| !e.is_collection).count();
    Ok(connection_ok_message(&conn.dir_url(), files))
}

/// 上传备份包（自动建目录）。
///
/// ⚠️ **不校验文件名**（源同）：名字由 [`crate::file_name::create`] 生成，不可能不合法；
/// 而下载/删除的 `name` 来自远端列表，必须校验。
pub async fn upload_backup<F>(
    client: &WebDavClient,
    conn: &WebDavConnection,
    name: &str,
    path: &Path,
    on_progress: F,
    cancel: Option<CancelFlag>,
) -> Result<UploadResult, BackupError>
where
    F: Fn(u64, u64) + Send + Sync + 'static,
{
    client.ensure_directory(conn, cancel.as_ref()).await?;
    let url = remote_file_url(conn, name);
    let size_bytes = client.upload(conn, &url, path, on_progress, cancel).await?;
    Ok(UploadResult {
        name: name.to_string(),
        size_bytes,
        remote_url: url,
    })
}

/// 下载备份包到 `target`。
///
/// ⚠️ 失败/取消时**半包会留在 `target`** —— 调用方负责清理（命令层用临时文件，
/// 流程结束一律删）。
pub async fn download_backup<F>(
    client: &WebDavClient,
    conn: &WebDavConnection,
    name: &str,
    target: &Path,
    on_progress: F,
    cancel: Option<&CancelFlag>,
) -> Result<u64, BackupError>
where
    F: Fn(u64, i64),
{
    ensure_backup_name(name)?;
    let url = remote_file_url(conn, name);
    client.download(conn, &url, target, on_progress, cancel).await
}

/// 删除远端备份（已不存在也算成功）。
pub async fn delete_remote(
    client: &WebDavClient,
    conn: &WebDavConnection,
    name: &str,
) -> Result<(), BackupError> {
    ensure_backup_name(name)?;
    client.delete(conn, &remote_file_url(conn, name)).await
}

// =============================================================================
// 测试
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_mock::{conn, multistatus_with, tempdir, test_client, Resp};

    fn entry(name: &str, size: i64, modified: i64, collection: bool) -> RemoteEntry {
        RemoteEntry {
            href: format!("/dav/civilcalc/{name}"),
            display_name: name.to_string(),
            size_bytes: size,
            last_modified_ms: modified,
            is_collection: collection,
        }
    }

    // ---------------- 纯函数 ----------------

    #[test]
    fn pick_backups_filters_dirs_and_foreign_files() {
        let entries = vec![
            entry("civilcalc_backup_20260915_143012.tar.gz", 2048, 0, false),
            entry("civilcalc_backup_20260914_090000.tar.gz", 1024, 0, false),
            entry("civilcalc_backup_20260913_080000.tar.gz", 512, 0, true), // 目录 → 排除
            entry("我的笔记.txt", 10, 0, false),                            // 用户自己的文件 → 排除
            entry("other_app_backup_20260101_000000.tar.gz", 10, 0, false), // 别家的包 → 排除
            entry("civilcalc_backup_.tar.gz", 10, 0, false),                // 空壳（只有前后缀）→ 排除
        ];
        let r = pick_backups(&entries);
        assert_eq!(r.backups.len(), 2);
        assert_eq!(r.total, 2);
        assert_eq!(r.backups[0].name, "civilcalc_backup_20260915_143012.tar.gz");
        assert_eq!(r.backups[1].name, "civilcalc_backup_20260914_090000.tar.gz");
    }

    /// ⚠️ `is_backup` 是**弱校验**：`civilcalc_backup_坏名字.tar.gz` 会**通过**
    /// （只看前后缀）。源项目同样如此 —— 它的职责是「别把用户自己的文件当备份包」。
    /// 这类文件的时间戳取不到，`timestampMs` 回落到服务端时间。
    #[test]
    fn pick_backups_keeps_prefix_suffix_files_with_bad_timestamp() {
        let entries = vec![entry("civilcalc_backup_坏名字.tar.gz", 10, 1_700_000_000_000, false)];
        let r = pick_backups(&entries);
        assert_eq!(r.backups.len(), 1, "弱校验会放行");
        assert_eq!(
            r.backups[0].timestamp_ms, 1_700_000_000_000,
            "时间戳取不到 → 回落服务端时间"
        );
    }

    #[test]
    fn pick_backups_sorts_desc_by_filename_timestamp() {
        // 故意打乱顺序，且服务端时间与文件名时间**矛盾**（服务端不可靠）
        let entries = vec![
            entry("civilcalc_backup_20260101_000000.tar.gz", 1, 9_999_999_999_999, false),
            entry("civilcalc_backup_20260915_143012.tar.gz", 1, 0, false),
            entry("civilcalc_backup_20260301_120000.tar.gz", 1, 0, false),
        ];
        let r = pick_backups(&entries);
        let names: Vec<&str> = r.backups.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "civilcalc_backup_20260915_143012.tar.gz",
                "civilcalc_backup_20260301_120000.tar.gz",
                "civilcalc_backup_20260101_000000.tar.gz",
            ],
            "按文件名时间倒序，服务端时间不参与"
        );
    }

    #[test]
    fn remote_backup_falls_back_to_server_time() {
        // 文件名不合规（拿不到时间）→ 用服务端时间
        let b = RemoteBackup::new("随便一个名字.tar.gz", 10, 1_700_000_000_000);
        assert_eq!(b.timestamp_ms, 1_700_000_000_000);

        // 文件名合规 → 用文件名时间
        let b = RemoteBackup::new("civilcalc_backup_20260915_143012.tar.gz", 10, 1);
        assert!(b.timestamp_ms > 1_700_000_000_000, "实际 {}", b.timestamp_ms);
    }

    #[test]
    fn pick_backups_truncates_but_keeps_total() {
        let entries: Vec<RemoteEntry> = (0..25)
            .map(|i| {
                entry(
                    &format!("civilcalc_backup_202601{:02}_000000.tar.gz", i + 1),
                    1,
                    0,
                    false,
                )
            })
            .collect();
        let r = pick_backups(&entries);
        assert_eq!(r.backups.len(), LIST_LIMIT, "截断到 {LIST_LIMIT}");
        assert_eq!(r.total, 25, "total 是截断前的总数");
    }

    #[test]
    fn pick_backups_empty_directory() {
        let r = pick_backups(&[]);
        assert!(r.backups.is_empty());
        assert_eq!(r.total, 0);
    }

    /// 同一时间戳（同一秒内导两次）保持稳定顺序
    #[test]
    fn pick_backups_is_stable_for_equal_timestamps() {
        let entries = vec![
            entry("civilcalc_backup_20260915_143012.tar.gz", 1, 0, false),
            entry("civilcalc_backup_20260915_143012(2).tar.gz", 2, 0, false),
        ];
        let r = pick_backups(&entries);
        assert_eq!(r.backups.len(), 2);
        assert_eq!(r.backups[0].name, "civilcalc_backup_20260915_143012.tar.gz");
        assert_eq!(r.backups[1].name, "civilcalc_backup_20260915_143012(2).tar.gz");
    }

    #[test]
    fn connection_message_matches_source() {
        assert_eq!(
            connection_ok_message("https://dav.jianguoyun.com/dav/civilcalc/", 3),
            "连接成功：远端目录 https://dav.jianguoyun.com/dav/civilcalc/（已有 3 个文件）"
        );
        assert_eq!(
            connection_ok_message("https://x.com/dav/c/", 0),
            "连接成功：远端目录 https://x.com/dav/c/（已有 0 个文件）"
        );
    }

    #[test]
    fn ensure_backup_name_rejects_foreign_files() {
        assert!(ensure_backup_name("civilcalc_backup_20260915_143012.tar.gz").is_ok());

        // ⚠️ 注意「弱校验」：`civilcalc_backup_20260915.tar.gz`（缺时分秒）**会通过** ——
        //    所以这里只用真正会被拒的形态。
        for bad in [
            "我的笔记.txt",
            "other_app_backup_20260101_000000.tar.gz", // 别家的前缀
            "civilcalc_backup_.tar.gz",                // 空壳
            "civilcalc_backup_20260915_143012.zip",    // 后缀不对
            "civilcalc_backup_20260915_143012.tar.gz.bak",
        ] {
            let e = ensure_backup_name(bad).unwrap_err();
            assert_eq!(e.user_message(), format!("这不是本应用产生的备份包：{bad}"));
            assert_eq!(e.http_status(), None, "本地校验错误没有状态码");
        }
    }

    /// 弱校验的边界：缺时分秒也放行（源同），这里钉住以免后人「顺手加强」
    #[test]
    fn ensure_backup_name_is_intentionally_weak() {
        assert!(ensure_backup_name("civilcalc_backup_20260915.tar.gz").is_ok());
        assert!(ensure_backup_name("civilcalc_backup_坏名字.tar.gz").is_ok());
        // 空壳是唯一的「长度」门槛
        assert!(ensure_backup_name("civilcalc_backup_.tar.gz").is_err());
    }

    #[test]
    fn remote_file_url_encodes_name() {
        // 备份名是纯 ASCII（`civilcalc_backup_yyyyMMdd_HHmmss.tar.gz`），
        // 不需要百分号编码；这条只验证目录与文件名的拼接口径。
        let conn = WebDavConnection::new(
            crate::webdav_config::WebDavConfig {
                base_url: "https://dav.jianguoyun.com/dav".to_string(),
                username: "u".to_string(),
                remote_dir: "civilcalc".to_string(),
                preset: crate::webdav_config::WebDavPreset::Nutstore,
            },
            "p",
        );
        assert_eq!(
            remote_file_url(&conn, "civilcalc_backup_20260915_143012.tar.gz"),
            "https://dav.jianguoyun.com/dav/civilcalc/civilcalc_backup_20260915_143012.tar.gz"
        );
    }

    // ---------------- 编排（mock server） ----------------

    #[tokio::test]
    async fn list_remote_ensures_dir_then_propfind() {
        let srv = crate::test_mock::start(vec![
            Resp::new(405, ""), // MKCOL：目录已存在
            Resp::new(207, multistatus_with(
                "civilcalc_backup_20260915_143012.tar.gz",
                "civilcalc_backup_20260915_143012.tar.gz",
                2048,
            )),
        ])
        .await;
        let c = conn(&srv);

        let r = list_remote(&test_client(), &c).await.unwrap();

        assert_eq!(
            srv.method_paths(),
            vec![
                ("MKCOL".to_string(), "/dav/civilcalc".to_string()),
                ("PROPFIND".to_string(), "/dav/civilcalc/".to_string()),
            ],
            "先建目录再列"
        );
        assert_eq!(r.total, 1);
        assert_eq!(r.backups.len(), 1);
        assert_eq!(r.backups[0].size_bytes, 2048);
    }

    #[tokio::test]
    async fn test_connection_counts_files_only() {
        let srv = crate::test_mock::start(vec![
            Resp::new(405, ""),
            Resp::new(207, multistatus_with(
                "civilcalc_backup_20260915_143012.tar.gz",
                "civilcalc_backup_20260915_143012.tar.gz",
                2048,
            )),
        ])
        .await;
        let c = conn(&srv);

        let msg = test_connection(&test_client(), &c).await.unwrap();
        // 响应里有 1 个目录项 + 1 个文件 → 只数 1
        assert_eq!(msg, "连接成功：远端目录 http://127.0.0.1:".to_string() + &srv.addr.port().to_string() + "/dav/civilcalc/（已有 1 个文件）");
    }

    #[tokio::test]
    async fn test_connection_propagates_auth_failure() {
        let srv = crate::test_mock::start(vec![Resp::new(401, "bad")]).await;
        let c = conn(&srv);
        let e = test_connection(&test_client(), &c).await.unwrap_err();
        assert_eq!(e.http_status(), Some(401));
    }

    #[tokio::test]
    async fn upload_backup_puts_into_dir_url() {
        let dir = tempdir("repo-upload");
        let path = dir.join("pack.tar.gz");
        std::fs::write(&path, vec![3u8; 100]).unwrap();

        let srv = crate::test_mock::start(vec![Resp::new(405, ""), Resp::new(201, "")]).await;
        let c = conn(&srv);

        let r = upload_backup(
            &test_client(),
            &c,
            "civilcalc_backup_20260915_143012.tar.gz",
            &path,
            |_, _| {},
            None,
        )
        .await
        .unwrap();

        assert_eq!(r.size_bytes, 100);
        assert_eq!(
            r.remote_url,
            format!("http://127.0.0.1:{}/dav/civilcalc/civilcalc_backup_20260915_143012.tar.gz", srv.addr.port())
        );
        assert_eq!(
            srv.method_paths(),
            vec![
                ("MKCOL".to_string(), "/dav/civilcalc".to_string()),
                (
                    "PUT".to_string(),
                    "/dav/civilcalc/civilcalc_backup_20260915_143012.tar.gz".to_string()
                ),
            ]
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 🔴 下载前必须校验文件名 —— **在发任何网络请求之前**就拒绝
    #[tokio::test]
    async fn download_backup_rejects_foreign_name_without_network() {
        let dir = tempdir("repo-download-bad");
        let target = dir.join("out.bin");
        let srv = crate::test_mock::start(vec![]).await;
        let c = conn(&srv);

        let e = download_backup(&test_client(), &c, "我的笔记.txt", &target, |_, _| {}, None)
            .await
            .unwrap_err();
        assert_eq!(e.user_message(), "这不是本应用产生的备份包：我的笔记.txt");
        assert_eq!(srv.count(), 0, "不应发出任何请求");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn download_backup_writes_file() {
        let dir = tempdir("repo-download");
        let target = dir.join("out.bin");
        let payload = vec![5u8; 1234];
        let srv = crate::test_mock::start(vec![Resp::octet(200, payload.clone())]).await;
        let c = conn(&srv);

        let n = download_backup(
            &test_client(),
            &c,
            "civilcalc_backup_20260915_143012.tar.gz",
            &target,
            |_, _| {},
            None,
        )
        .await
        .unwrap();

        assert_eq!(n, payload.len() as u64);
        assert_eq!(std::fs::read(&target).unwrap(), payload);
        assert_eq!(srv.req(0).path(), "/dav/civilcalc/civilcalc_backup_20260915_143012.tar.gz");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 🔴 删除前同样校验（不校验会删掉用户自己放进目录的文件）
    #[tokio::test]
    async fn delete_remote_rejects_foreign_name_without_network() {
        let srv = crate::test_mock::start(vec![]).await;
        let c = conn(&srv);

        let e = delete_remote(&test_client(), &c, "我的笔记.txt").await.unwrap_err();
        assert_eq!(e.user_message(), "这不是本应用产生的备份包：我的笔记.txt");
        assert_eq!(srv.count(), 0);
    }

    #[tokio::test]
    async fn delete_remote_issues_delete() {
        let srv = crate::test_mock::start(vec![Resp::new(204, "")]).await;
        let c = conn(&srv);

        delete_remote(&test_client(), &c, "civilcalc_backup_20260915_143012.tar.gz")
            .await
            .unwrap();

        assert_eq!(srv.req(0).method, "DELETE");
        assert_eq!(
            srv.req(0).path(),
            "/dav/civilcalc/civilcalc_backup_20260915_143012.tar.gz"
        );
    }

    /// 远端已不存在（404）也算删除成功
    #[tokio::test]
    async fn delete_remote_tolerates_404() {
        let srv = crate::test_mock::start(vec![Resp::new(404, "")]).await;
        let c = conn(&srv);
        delete_remote(&test_client(), &c, "civilcalc_backup_20260915_143012.tar.gz")
            .await
            .unwrap();
    }
}
