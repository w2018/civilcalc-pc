//! 更新检查：向 GitHub Releases 取最新版本并比对，网络失败静默返回「无更新」。
//!
//! 源：`civilcalc-android-v2/app/.../update/UpdateChecker.kt`（约 220 行）。
//!
//! ## 设计要点
//!
//! - 网络失败 / 接口异常 / 无 releases → **静默**返回 `hasUpdate = false`（不抛错、不弹窗）。
//! - 版本比对逐段比较（"1.2.4" vs "1.10.0" → 1.10 更大）。
//! - `update_open_download` 只负责用系统浏览器打开下载页 —— 不在此处下载安装
//!   （PC 端安装走 NSIS/MSI，由用户手动完成；源项目的 APK 自下载不适用于桌面）。
//!
//! ⚠️ `GITHUB_REPO` 是占位仓库地址，需与创建好的 GitHub 仓库名一致（见 docs/07 待办）。

use crate::error::{CmdResult, CommandError};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

/// GitHub 仓库（`owner/repo`）。**上线前需确认为实际仓库名。**
const GITHUB_REPO: &str = "w2018/civilcalc-pc";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// 一次更新检查的结果（返回给前端）。
///
/// 🔴 **必须 `rename_all = "camelCase"`** —— 契约（`docs/08` §3 组 14）与
/// 前端 `UpdateInfo` 都按 `hasUpdate` / `latestVersion` 取字段。
/// 漏掉这行会让前端拿到 `has_update`，表现为「永远显示已是最新」（静默失败）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub has_update: bool,
    pub latest_version: String,
    pub current_version: String,
    pub release_notes: String,
    /// 选中的安装包下载地址（`.exe` / `.msi` 优先），没有则 `None`
    pub download_url: Option<String>,
    pub asset_size: i64,
    /// 形如 `"sha256:..."`，无则 `None`
    pub asset_digest: Option<String>,
}

/// GitHub Releases API 返回的 release 对象（只取我们要的字段）。
#[derive(Debug, Deserialize)]
struct GithubRelease {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    #[serde(default)]
    name: String,
    #[serde(default)]
    browser_download_url: String,
    #[serde(default)]
    size: i64,
    /// GitHub 自 2024 起提供资产摘要，形如 `"sha256:abcdef..."`
    #[serde(default)]
    digest: Option<String>,
}

/// 检查更新。
///
/// 网络失败 / 无 releases / 解析异常 → 静默返回「当前已是最新」（`hasUpdate = false`）。
#[tauri::command]
pub async fn update_check() -> CmdResult<UpdateInfo> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    // 静默兜底：任何异常都返回「无更新」，不让检查更新本身报错
    let info = check().await.unwrap_or_else(|_| UpdateInfo {
        has_update: false,
        latest_version: current.clone(),
        current_version: current.clone(),
        release_notes: String::new(),
        download_url: None,
        asset_size: 0,
        asset_digest: None,
    });
    Ok(info)
}

async fn check() -> Result<UpdateInfo, ()> {
    let url = format!(
        "https://api.github.com/repos/{GITHUB_REPO}/releases/latest"
    );
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent("CivilCalc/PC")
        .build()
        .map_err(|_| ())?;
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await
        .map_err(|_| ())?;
    if !resp.status().is_success() {
        return Err(());
    }
    let release: GithubRelease = resp.json().await.map_err(|_| ())?;
    let latest = release.tag_name.trim_start_matches('v').trim().to_string();
    let has_update = compare_versions(&latest, env!("CARGO_PKG_VERSION")) > 0;

    // PC 端安装包优先 `.exe` / `.msi`，没有则取第一个资产
    let asset = release
        .assets
        .iter()
        .find(|a| {
            a.name.ends_with(".exe") || a.name.ends_with(".msi")
        })
        .or_else(|| release.assets.first());

    Ok(UpdateInfo {
        has_update,
        latest_version: latest,
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        release_notes: release.body.unwrap_or_default(),
        download_url: asset.map(|a| a.browser_download_url.clone()),
        asset_size: asset.map(|a| a.size).unwrap_or(0),
        asset_digest: asset.and_then(|a| a.digest.clone()),
    })
}

/// 用系统默认浏览器打开下载地址（或发布页）。
///
/// ⚠️ 只接受 http/https，避免把任意 scheme 交给系统打开。
#[tauri::command]
pub fn update_open_download(app: AppHandle, url: String) -> CmdResult<()> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(CommandError::InvalidArgument {
            message: "只允许打开 http/https 链接".to_string(),
        });
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| CommandError::Unknown {
            message: format!("打开下载链接失败：{e}"),
        })?;
    Ok(())
}

/// 逐段比较版本号：a > b 返回正数，相等返回 0。
fn compare_versions(a: &str, b: &str) -> i64 {
    let parse = |s: &str| {
        s.split('.')
            .map(|p| p.trim().parse::<i64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let pa = parse(a);
    let pb = parse(b);
    let max_len = pa.len().max(pb.len());
    for i in 0..max_len {
        let va = pa.get(i).copied().unwrap_or(0);
        let vb = pb.get(i).copied().unwrap_or(0);
        if va != vb {
            return va - vb;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare_semantics() {
        assert!(compare_versions("1.10.0", "1.2.4") > 0, "1.10 > 1.2");
        assert!(compare_versions("1.2.4", "1.10.0") < 0);
        assert_eq!(compare_versions("1.2.4", "1.2.4"), 0);
        assert!(compare_versions("2.0", "1.9.9") > 0);
        assert!(compare_versions("1.2", "1.2.0") == 0, "缺段视为 0");
    }

    /// 契约：字段名必须是 camelCase，且**一个 snake_case 键都不能漏出去**。
    ///
    /// 前端按 `hasUpdate` 取值；漏 `rename_all` 会静默变成「永远无更新」。
    #[test]
    fn update_info_serializes_camel_case() {
        let info = UpdateInfo {
            has_update: true,
            latest_version: "1.2.0".to_string(),
            current_version: "1.1.0".to_string(),
            release_notes: "notes".to_string(),
            download_url: Some("https://example.com/a.exe".to_string()),
            asset_size: 123,
            asset_digest: Some("sha256:abc".to_string()),
        };
        let v = serde_json::to_value(&info).unwrap();
        let obj = v.as_object().unwrap();

        for key in [
            "hasUpdate",
            "latestVersion",
            "currentVersion",
            "releaseNotes",
            "downloadUrl",
            "assetSize",
            "assetDigest",
        ] {
            assert!(obj.contains_key(key), "缺少 camelCase 键 {key}");
        }
        assert!(
            !obj.keys().any(|k| k.contains('_')),
            "不应出现 snake_case 键：{:?}",
            obj.keys().collect::<Vec<_>>()
        );
    }
}
