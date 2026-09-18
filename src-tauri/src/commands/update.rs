//! 更新检查：向 GitHub Releases 取最新版本并比对。
//!
//! 源：`civilcalc-android-v2/app/.../update/UpdateChecker.kt`（约 220 行）。
//!
//! ## 设计要点
//!
//! - 版本比对逐段比较（"1.2.4" vs "1.10.0" → 1.10 更大）。
//! - `update_open_download` 只负责用系统浏览器打开下载页 —— 不在此处下载安装
//!   （PC 端安装走 NSIS，由用户手动完成；源项目的 APK 自下载不适用于桌面）。
//!
//! ## 🔴 「没更新」必须能与「没查成」区分开
//!
//! 原先沿用源项目做法：网络失败 / 接口异常 / 无 releases **一律**返回
//! `hasUpdate = false`，不抛错也不留痕。结果是界面上「已是最新」与
//! 「根本没连上」长得一模一样 —— 用户会以为更新功能坏了，我们也没法自证。
//!
//! 实测就撞上了：GitHub 的**匿名** API 是 **60 次/小时/IP**，多查几次
//! （或与别人共用出口 IP）就会拿到 `HTTP 403 API rate limit exceeded`，
//! 而界面只会说「没有可用的更新」。
//!
//! 现在分三档表达：
//! - `checkOk = false` + `failReason` —— 没查成，附原因码；
//! - `checkOk = true` + `degraded = true` —— 主路径失败，**兜底路径**拿到了
//!   版本号（但拿不到安装包大小 / 摘要 / 直链）；
//! - `checkOk = true` + `degraded = false` —— 真查到了，`hasUpdate` 可信。

use crate::error::{CmdResult, CommandError};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

/// GitHub 仓库（`owner/repo`）。
const GITHUB_REPO: &str = "w2018/civilcalc-pc";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// GitHub 要求带 UA，否则部分接口直接 403
const USER_AGENT: &str = "CivilCalc/PC";

/// 检查失败的原因码。
///
/// 与前端 `src/api/update.ts` 的 `UpdateFailReason` 联合类型逐字对应，
/// 由契约测试 `enum_values_match_ts` 钉住。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateFailReason {
    /// 请求根本没发出去 / 连不上（断网、DNS、超时）
    Offline,
    /// GitHub 匿名接口限流（HTTP 403 / 429）
    RateLimited,
    /// 仓库或发布不存在（HTTP 404）
    NotFound,
    /// 其它非 2xx
    HttpError,
    /// 返回体解析失败（接口改版等）
    ParseError,
}

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

    /// 这次检查**到底查成没查成**。
    ///
    /// 🔴 `false` 时 `has_update = false` **只代表「没查成」**，
    /// 不代表「已是最新」—— 前端必须把这两件事分开说。
    pub check_ok: bool,
    /// **主路径**（GitHub API）的失败原因。
    ///
    /// - `check_ok = false` → 这就是彻底失败的原因；
    /// - `check_ok = true && degraded = true` → 说明**为什么**降级走了兜底；
    /// - 主路径成功 → `None`。
    pub fail_reason: Option<UpdateFailReason>,
    /// 结果来自**兜底路径**：只有版本号是准的，安装包大小 / 摘要 / 直链都没有。
    pub degraded: bool,
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
/// 契约：这个命令**不抛错** —— 「检查更新」失败不该弹错误框打断用户。
/// 失败一律通过 `check_ok = false` + `fail_reason` 表达，由界面如实呈现。
#[tauri::command]
pub async fn update_check() -> CmdResult<UpdateInfo> {
    Ok(check().await)
}

/// 主路径 → 兜底路径 → 彻底失败，三级降级。
async fn check() -> UpdateInfo {
    let current = env!("CARGO_PKG_VERSION").to_string();

    let client = match reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
    {
        Ok(c) => c,
        // 建 client 失败基本只可能是 TLS 后端初始化失败，按「连不上」处理
        Err(_) => return uncheckable(&current, UpdateFailReason::Offline),
    };

    match fetch_from_api(&client, &current).await {
        Ok(info) => info,
        Err(reason) => {
            // 🔴 兜底：`/releases/latest` 会 302 到 `/releases/tag/vX.Y.Z`。
            //    网页端点**没有** API 那套匿名限流，所以限流时这条路照样通 ——
            //    至少让用户知道有没有新版，而不是把限流说成「已是最新」。
            match fetch_latest_tag_via_web(&client).await {
                Ok(tag) => degraded_info(&current, &tag, reason),
                Err(_) => uncheckable(&current, reason),
            }
        }
    }
}

/// 主路径：GitHub Releases API（能拿到安装包直链 / 大小 / 摘要）。
async fn fetch_from_api(
    client: &reqwest::Client,
    current: &str,
) -> Result<UpdateInfo, UpdateFailReason> {
    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await
        .map_err(|_| UpdateFailReason::Offline)?;

    match resp.status().as_u16() {
        200 => {}
        // GitHub 对匿名请求用 403 表达限流（配额耗尽），429 是显式限流
        403 | 429 => return Err(UpdateFailReason::RateLimited),
        404 => return Err(UpdateFailReason::NotFound),
        _ => return Err(UpdateFailReason::HttpError),
    }

    let release: GithubRelease = resp.json().await.map_err(|_| UpdateFailReason::ParseError)?;
    Ok(interpret(&release, current))
}

/// 兜底路径：靠网页跳转拿标签，**不吃 API 限流**。
async fn fetch_latest_tag_via_web(client: &reqwest::Client) -> Result<String, UpdateFailReason> {
    let url = format!("https://github.com/{GITHUB_REPO}/releases/latest");
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|_| UpdateFailReason::Offline)?;
    if !resp.status().is_success() {
        return Err(UpdateFailReason::HttpError);
    }
    // reqwest 默认跟随跳转，`url()` 给的是**最终**地址
    tag_from_release_url(resp.url().as_str()).ok_or(UpdateFailReason::ParseError)
}

/// 从 `https://github.com/<owner>/<repo>/releases/tag/<tag>` 里取出 `<tag>`。
///
/// 单独成函数是为了能单测 —— 这段错了兜底路径会**静默**失效，
/// 又回到「限流被说成已是最新」。
fn tag_from_release_url(url: &str) -> Option<String> {
    const MARK: &str = "/releases/tag/";
    let idx = url.find(MARK)?;
    let tag = url[idx + MARK.len()..]
        .split(['?', '#', '/'])
        .next()?
        .trim();
    if tag.is_empty() {
        None
    } else {
        Some(tag.to_string())
    }
}

/// 主路径的解析（纯函数，便于拿真实返回体做单测）。
fn interpret(release: &GithubRelease, current: &str) -> UpdateInfo {
    let latest = release.tag_name.trim_start_matches('v').trim().to_string();
    let has_update = compare_versions(&latest, current) > 0;

    // PC 端安装包优先 `.exe` / `.msi`，没有则取第一个资产
    let asset = release
        .assets
        .iter()
        .find(|a| a.name.ends_with(".exe") || a.name.ends_with(".msi"))
        .or_else(|| release.assets.first());

    UpdateInfo {
        has_update,
        latest_version: latest,
        current_version: current.to_string(),
        release_notes: release.body.clone().unwrap_or_default(),
        download_url: asset.map(|a| a.browser_download_url.clone()),
        asset_size: asset.map(|a| a.size).unwrap_or(0),
        asset_digest: asset.and_then(|a| a.digest.clone()),
        check_ok: true,
        fail_reason: None,
        degraded: false,
    }
}

/// 兜底路径的结果：只有版本号可信，安装包信息一律留空。
fn degraded_info(current: &str, tag: &str, reason: UpdateFailReason) -> UpdateInfo {
    let latest = tag.trim_start_matches('v').trim().to_string();
    UpdateInfo {
        has_update: compare_versions(&latest, current) > 0,
        latest_version: latest,
        current_version: current.to_string(),
        release_notes: String::new(),
        // 直链给**发布页** —— 兜底路径拿不到资产名，不要去猜文件名
        download_url: Some(format!(
            "https://github.com/{GITHUB_REPO}/releases/tag/{tag}"
        )),
        asset_size: 0,
        asset_digest: None,
        // 版本号是准的，所以算「查成了」；但结果降级，界面要说清楚
        check_ok: true,
        fail_reason: Some(reason),
        degraded: true,
    }
}

/// 彻底没查成：主路径与兜底都失败。
fn uncheckable(current: &str, reason: UpdateFailReason) -> UpdateInfo {
    UpdateInfo {
        has_update: false,
        latest_version: current.to_string(),
        current_version: current.to_string(),
        release_notes: String::new(),
        download_url: None,
        asset_size: 0,
        asset_digest: None,
        check_ok: false,
        fail_reason: Some(reason),
        degraded: false,
    }
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
            check_ok: true,
            fail_reason: Some(UpdateFailReason::RateLimited),
            degraded: false,
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
            "checkOk",
            "failReason",
            "degraded",
        ] {
            assert!(obj.contains_key(key), "缺少 camelCase 键 {key}");
        }
        assert!(
            !obj.keys().any(|k| k.contains('_')),
            "不应出现 snake_case 键：{:?}",
            obj.keys().collect::<Vec<_>>()
        );
        // 原因码本身也必须是 camelCase 字符串（前端按字面量比对）
        assert_eq!(obj["failReason"], serde_json::json!("rateLimited"));
    }

    /// 用**真实**的 GitHub 返回体验证解析链路。
    ///
    /// 这条用例的价值：网络/限流问题与**解析逻辑**无关，这样即使
    /// 接口被限流（本地实测撞过 403），也能证明「拿到返回体之后」是对的。
    #[test]
    fn interpret_real_release_payload() {
        // 取自 https://api.github.com/repos/w2018/civilcalc-pc/releases/latest
        // （只删掉与解析无关的字段）
        let json = r#"{
            "tag_name": "v1.0.11",
            "body": "**Full Changelog**: https://github.com/w2018/civilcalc-pc/commits/v1.0.11",
            "assets": [
                {
                    "name": "AI-Calculator_1.0.11_x64-setup.exe",
                    "browser_download_url": "https://github.com/w2018/civilcalc-pc/releases/download/v1.0.11/AI-Calculator_1.0.11_x64-setup.exe",
                    "size": 6430675,
                    "digest": "sha256:453c61e61e9818c0c48b193cf385d5af09b24e79769d04d97204070ff7b2b7bc"
                },
                {
                    "name": "Source code (zip)",
                    "browser_download_url": "https://github.com/w2018/civilcalc-pc/archive/refs/tags/v1.0.11.zip",
                    "size": 0
                }
            ]
        }"#;
        let release: GithubRelease = serde_json::from_str(json).expect("真实返回体必须能解析");

        // 当前就是 1.0.11 → 不该报「有更新」
        let same = interpret(&release, "1.0.11");
        assert!(!same.has_update, "版本相同不该报有更新");
        assert_eq!(same.latest_version, "1.0.11", "要去掉前缀 v");
        assert!(same.check_ok);
        assert!(same.fail_reason.is_none());
        assert!(!same.degraded);
        // 资产要挑中 .exe，而不是排在后面的「Source code (zip)」
        assert_eq!(same.asset_size, 6_430_675);
        assert_eq!(
            same.download_url.as_deref(),
            Some("https://github.com/w2018/civilcalc-pc/releases/download/v1.0.11/AI-Calculator_1.0.11_x64-setup.exe")
        );
        assert!(same.asset_digest.as_deref().unwrap().starts_with("sha256:"));

        // 低一个版本 → 必须报有更新（否则用户永远收不到升级提示）
        let older = interpret(&release, "1.0.10");
        assert!(older.has_update);
        assert_eq!(older.latest_version, "1.0.11");
    }

    /// 兜底路径的标签提取
    #[test]
    fn tag_from_release_url_works() {
        assert_eq!(
            tag_from_release_url("https://github.com/w2018/civilcalc-pc/releases/tag/v1.0.11"),
            Some("v1.0.11".to_string())
        );
        assert_eq!(
            tag_from_release_url("https://github.com/o/r/releases/tag/v2.0?x=1"),
            Some("v2.0".to_string()),
            "带查询串也要能取"
        );
        assert_eq!(
            tag_from_release_url("https://github.com/o/r/releases"),
            None,
            "没有发布时 GitHub 跳到发布列表 —— 必须返回 None，不能乱猜"
        );
        assert_eq!(tag_from_release_url("https://github.com/o/r"), None);
    }

    /// 降级结果：版本号准、安装包信息为空、`degraded` 置位
    #[test]
    fn degraded_result_is_marked() {
        let d = degraded_info("1.0.10", "v1.0.11", UpdateFailReason::RateLimited);
        assert!(d.has_update);
        assert!(d.check_ok, "兜底拿到了版本号，算查成了");
        assert!(d.degraded, "但必须标出是降级结果");
        assert_eq!(d.fail_reason, Some(UpdateFailReason::RateLimited));
        assert_eq!(d.asset_size, 0, "兜底路径没有安装包信息");
        assert!(d.asset_digest.is_none());
        assert!(d
            .download_url
            .as_deref()
            .unwrap()
            .ends_with("/releases/tag/v1.0.11"));
    }

    /// 彻底没查成：`check_ok = false`，界面据此区分「已是最新」
    #[test]
    fn uncheckable_is_marked() {
        let u = uncheckable("1.0.11", UpdateFailReason::Offline);
        assert!(!u.check_ok);
        assert!(!u.has_update, "查不成时 has_update 只能是 false");
        assert_eq!(u.fail_reason, Some(UpdateFailReason::Offline));
        assert!(u.download_url.is_none());
        assert!(!u.degraded);
    }
}
