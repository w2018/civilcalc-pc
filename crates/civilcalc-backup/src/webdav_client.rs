//! WebDAV 客户端。
//!
//! 源：`civilcalc-android-v2/core/backup/WebDavClient.kt`（239 行，OkHttp）
//!
//! ## 只用 5 个方法
//!
//! `MKCOL` / `PROPFIND` / `PUT` / `GET` / `DELETE` —— 够备份用，
//! 不实现 `COPY` / `MOVE` / `LOCK` 等（多数个人网盘对这些支持参差）。
//!
//! ## 两处「容错即契约」的成功码
//!
//! | 方法 | 成功码 | 为什么这么宽 |
//! |---|---|---|
//! | MKCOL | `200/201/204/301/302/**405**` | **坚果云对已存在的目录返回 405**；301/302 是某些服务端把创建重定向到带斜杠的集合 |
//! | DELETE | `200/202/204/**404**` | 文件已经不在，目标状态已达成 —— 报错只会让「刷新列表」失败 |
//!
//! ## 🔴 取消要立即断流
//!
//! [`CancelFlag`] 一路传进来，**上传在分块之间、下载在分块之间**各检查一次，
//! 命中即返回 [`BackupError::Cancelled`]（上传体直接中断，连接随之关闭）。
//! 不是「传完再丢弃结果」。
//!
//! ## 网络层与决策层分离（便于离线测试）
//!
//! 成功码判定、状态码→中文文案、网络异常→中文文案、Basic 认证串
//! 全是**纯函数**；HTTP 交互本身由 [`WebDavClient`] 负责，
//! 测试用本模块自带的极简 mock server（`127.0.0.1` + 手写 HTTP/1.1）驱动。
//!
//! ## 与源的三处有意偏差
//!
//! 1. **`User-Agent` = `CivilCalc-PC`**（源为 `CivilCalc-Android`）——
//!    `docs/04` 明确建议改名。服务端不会据此判权限，只影响日志可读性。
//! 2. **Basic 认证用 UTF-8 编码**（OkHttp 的 `Credentials.basic` 默认 ISO-8859-1）——
//!    邮箱与应用密码都是 ASCII，两者字节完全相同；非 ASCII 场景 UTF-8 才是对的
//!    （ISO-8859-1 对 >U+00FF 的字符会直接产出 `?`）。
//! 3. **没有 write 超时** —— `reqwest 0.12` 无 `write_timeout`（源 OkHttp 有 60s）。
//!    详见 [`WebDavClient::new`]。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{status_error, BackupError};
use crate::webdav_config::WebDavConnection;
use crate::webdav_path::ancestor_urls;
use crate::webdav_xml::{self, RemoteEntry};

/// `User-Agent`。⚠️ 源是 `CivilCalc-Android`，PC 端按 `docs/04` 改名。
pub const USER_AGENT: &str = "CivilCalc-PC";

/// `Content-Type`：XML 请求体（PROPFIND）
pub const XML_CONTENT_TYPE: &str = "application/xml; charset=utf-8";

/// `Content-Type`：上传体
pub const OCTET_STREAM: &str = "application/octet-stream";

/// MKCOL 的成功码（含**已存在**的 405）
pub const OK_MKCOL: &[u16] = &[200, 201, 204, 301, 302, 405];

/// DELETE 的成功码（含**已不存在**的 404）
pub const OK_DELETE: &[u16] = &[200, 202, 204, 404];

/// 上传/下载的分块大小（源用 16 KiB）
pub const CHUNK: usize = 16 * 1024;

/// 连接超时（源 OkHttp 15s）
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// 读超时（源 OkHttp 120s）。
///
/// ⚠️ 在 `reqwest 0.12` 里它是**单次读操作**的超时，不是整个请求的超时 ——
/// 上传/下载只要持续有数据流动就不会触发，正合备份场景。
pub const READ_TIMEOUT: Duration = Duration::from_secs(120);

/// MKCOL 是否算成功（见模块文档的成功码表）
pub fn is_mkcol_ok(status: u16) -> bool {
    OK_MKCOL.contains(&status)
}

/// DELETE 是否算成功（见模块文档的成功码表）
pub fn is_delete_ok(status: u16) -> bool {
    OK_DELETE.contains(&status)
}

/// `Authorization: Basic <base64(user:pass)>`。
///
/// 手写而不是用 `RequestBuilder::basic_auth`：这样认证串本身是**可单测的纯函数**，
/// 也便于断言「用户名里的冒号不会被当成分隔符」这类边界。
///
/// ⚠️ 用 UTF-8 编码（见模块文档的偏差说明）。
pub fn basic_auth_value(username: &str, password: &str) -> String {
    use base64::Engine as _;
    let raw = format!("{username}:{password}");
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
    )
}

/// 网络异常 → 中文文案。
///
/// `haystack` 是**整条错误链**拼起来的文本（`reqwest` 的 `Display` 只有
/// `error sending request for url (...)`，真正的成因在 `source()` 链里），
/// `is_timeout` 由 `reqwest::Error::is_timeout()` 提供。
///
/// ## 判定顺序（与源**不同**，有意）
///
/// 源是 timeout → resolve host → failed to connect → SSL。
/// 这里把 **SSL 提到 connect 之前** —— `reqwest` 的证书错误文案是
/// `error trying to connect: invalid peer certificate`，**同时含 "connect"**，
/// 按源的顺序会被误判成「无法连接服务器」。有测试钉住。
pub fn network_message(haystack: &str, is_timeout: bool) -> String {
    let lower = haystack.to_ascii_lowercase();
    if is_timeout || lower.contains("timeout") || lower.contains("timed out") {
        return "连接超时，请检查网络或服务器地址".to_string();
    }
    if lower.contains("dns")
        || lower.contains("resolve")
        || lower.contains("lookup")
        || lower.contains("unable to resolve host")
    {
        return "无法解析服务器地址，请检查网络与地址拼写".to_string();
    }
    if lower.contains("certificate")
        || lower.contains("ssl")
        || lower.contains("tls")
        || lower.contains("invalid peer")
    {
        return "安全连接失败（证书或协议不受支持）".to_string();
    }
    if lower.contains("refused")
        || lower.contains("failed to connect")
        || lower.contains("connect error")
        || lower.contains("network is unreachable")
        || lower.contains("no route to host")
        || lower.contains("connection reset")
        || lower.contains("broken pipe")
    {
        return "无法连接服务器，请检查地址与端口".to_string();
    }
    format!("网络错误：{}", haystack.trim())
}

/// 取消标志：跨任务共享。
///
/// `webdav_cancel` 命令置位，客户端在分块之间检查。
/// ⚠️ **每次任务开始必须 [`CancelFlag::reset`]** —— 否则上次的取消会秒杀新任务。
#[derive(Debug, Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }

    /// 置位（可跨线程调用）
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// 清除（任务开始时调用）
    pub fn reset(&self) {
        self.0.store(false, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// WebDAV 客户端（持有连接池，应当复用）
pub struct WebDavClient {
    http: reqwest::Client,
}

impl Default for WebDavClient {
    fn default() -> Self {
        Self::new()
    }
}

impl WebDavClient {
    /// 按源项目的超时构造（connect 15s / read 120s）。
    ///
    /// ⚠️ **源 OkHttp 的 write 300s 没有对应项**：`reqwest 0.12` 没有 `write_timeout`。
    /// 上传体是本地文件流，写阻塞的唯一成因是网络中断，此时 `read_timeout` 会兜住。
    /// **不要**用 `.timeout(300s)` 顶替 —— 那是**整体**超时，会把几十 MB 的上传
    /// 硬砍在 300 秒上。
    ///
    /// ⚠️ 源开了 `retryOnConnectionFailure(true)`；`reqwest`/`hyper` 没有同名开关，
    /// 只对**幂等**请求做有限重试。备份的 5 个方法里 PUT/DELETE/MKCOL 都不保证幂等，
    /// 所以这里**不做额外重试** —— 宁可让用户看到失败，也不要重复上传几十 MB。
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { http }
    }

    /// 注入外部构造的 `reqwest::Client`（测试用：接本地 mock server）
    pub fn with_http(http: reqwest::Client) -> Self {
        Self { http }
    }

    /// 逐级创建远端目录；已存在（405 等）视为成功。
    ///
    /// ⚠️ **必须逐级创建**：直接 MKCOL 深层目录，父目录不存在时服务端返回 409。
    pub async fn ensure_directory(
        &self,
        conn: &WebDavConnection,
        cancel: Option<&CancelFlag>,
    ) -> Result<(), BackupError> {
        for url in ancestor_urls(&conn.base_url(), &conn.config.remote_dir) {
            let resp = self
                .send(self.request(conn, &url, "MKCOL")?, cancel)
                .await?;
            let status = resp.status().as_u16();
            if !is_mkcol_ok(status) {
                let body = resp.text().await.unwrap_or_default();
                return Err(status_error(status, Some(&body), "创建目录"));
            }
        }
        Ok(())
    }

    /// 列目录（`Depth: 1`）。返回原始条目（含目录项），由调用方筛出备份文件。
    pub async fn list(
        &self,
        conn: &WebDavConnection,
        dir_url: &str,
    ) -> Result<Vec<RemoteEntry>, BackupError> {
        let req = self
            .request(conn, dir_url, "PROPFIND")?
            .header("depth", "1")
            .header(reqwest::header::CONTENT_TYPE, XML_CONTENT_TYPE)
            .body(webdav_xml::PROP_FIND_BODY);
        let resp = self.send(req, None).await?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        if status != 207 {
            return Err(status_error(status, Some(&text), "读取目录"));
        }
        Ok(webdav_xml::parse_multi_status(&text))
    }

    /// 上传文件（PUT）。
    ///
    /// `on_progress(done, total)`：每写出一块回调一次，`total` 是文件字节数。
    ///
    /// ⚠️ 回调要求 `Send + Sync + 'static` —— 它会在**请求体的流**里被调用，
    /// 而流必须能跨线程。真实调用方（Tauri 事件）天然满足。
    ///
    /// ⚠️ **显式设置 `Content-Length`**：`reqwest::Body::wrap_stream` 的长度是未知的，
    /// 不设头就会走 `Transfer-Encoding: chunked`。部分 WebDAV 服务端对 chunked PUT
    /// 返回 411。hyper 明确「用户设了的头就照用」，所以显式设头即可。
    pub async fn upload<F>(
        &self,
        conn: &WebDavConnection,
        file_url: &str,
        path: &Path,
        on_progress: F,
        cancel: Option<CancelFlag>,
    ) -> Result<u64, BackupError>
    where
        F: Fn(u64, u64) + Send + Sync + 'static,
    {
        let total = std::fs::metadata(path)
            .map_err(|e| BackupError::Io(e.to_string()))?
            .len();
        let body = upload_body(path.to_path_buf(), total, on_progress, cancel.clone());
        let req = self
            .request(conn, file_url, "PUT")?
            .header(reqwest::header::CONTENT_TYPE, OCTET_STREAM)
            .header(reqwest::header::CONTENT_LENGTH, total.to_string())
            .body(body);
        let resp = self.send(req, cancel.as_ref()).await?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(status_error(status, Some(&text), "上传"));
        }
        Ok(total)
    }

    /// 下载到本地文件。
    ///
    /// `on_progress(done, total)`：`total: i64` 是服务端给的 `Content-Length`，
    /// **服务端没给时为 `-1`**（源同，故用有符号数）——
    /// 前端据此显示「已下载 x MB」而不是百分比。
    ///
    /// ⚠️ 失败/取消时**半包会留在 `target`**，清理由调用方负责（编排层用临时文件）。
    pub async fn download<F>(
        &self,
        conn: &WebDavConnection,
        file_url: &str,
        target: &Path,
        on_progress: F,
        cancel: Option<&CancelFlag>,
    ) -> Result<u64, BackupError>
    where
        F: Fn(u64, i64),
    {
        let mut resp = self.send(self.request(conn, file_url, "GET")?, cancel).await?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(status_error(status, Some(&text), "下载"));
        }
        let total = resp.content_length().map_or(-1, |n| n as i64);

        let mut file = tokio::fs::File::create(target)
            .await
            .map_err(|e| BackupError::Io(e.to_string()))?;
        let mut done: u64 = 0;
        loop {
            if cancel.map(CancelFlag::is_cancelled).unwrap_or(false) {
                return Err(BackupError::Cancelled);
            }
            let chunk = match resp.chunk().await {
                Ok(Some(c)) => c,
                Ok(None) => break,
                Err(e) => return Err(classify(&e, cancel)),
            };
            file.write_all(&chunk)
                .await
                .map_err(|e| BackupError::Io(e.to_string()))?;
            done += chunk.len() as u64;
            on_progress(done, total);
        }
        file.flush().await.map_err(|e| BackupError::Io(e.to_string()))?;
        drop(file);

        Ok(std::fs::metadata(target)
            .map_err(|e| BackupError::Io(e.to_string()))?
            .len())
    }

    /// 删除远端文件；**已不存在（404）也算成功**。
    pub async fn delete(
        &self,
        conn: &WebDavConnection,
        file_url: &str,
    ) -> Result<(), BackupError> {
        let resp = self
            .send(self.request(conn, file_url, "DELETE")?, None)
            .await?;
        let status = resp.status().as_u16();
        if !is_delete_ok(status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(status_error(status, Some(&text), "删除"));
        }
        Ok(())
    }

    /// 远端路径是否存在（`Depth: 0` 的 PROPFIND）。
    pub async fn exists(
        &self,
        conn: &WebDavConnection,
        url: &str,
    ) -> Result<bool, BackupError> {
        let req = self
            .request(conn, url, "PROPFIND")?
            .header("depth", "0")
            .header(reqwest::header::CONTENT_TYPE, XML_CONTENT_TYPE)
            .body(webdav_xml::PROP_FIND_BODY);
        let resp = self.send(req, None).await?;
        match resp.status().as_u16() {
            207 => Ok(true),
            404 => Ok(false),
            s => {
                let text = resp.text().await.unwrap_or_default();
                Err(status_error(s, Some(&text), "检查远端文件"))
            }
        }
    }

    // ------------------------------------------------------------------
    // 内部
    // ------------------------------------------------------------------

    fn request(
        &self,
        conn: &WebDavConnection,
        url: &str,
        method: &str,
    ) -> Result<reqwest::RequestBuilder, BackupError> {
        let m = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|e| BackupError::Format(e.to_string()))?;
        Ok(self
            .http
            .request(m, url)
            .header(
                reqwest::header::AUTHORIZATION,
                basic_auth_value(conn.username(), &conn.password),
            )
            .header(reqwest::header::USER_AGENT, USER_AGENT))
    }

    /// 发送并按需把 `reqwest::Error` 译成 [`BackupError`]
    async fn send(
        &self,
        req: reqwest::RequestBuilder,
        cancel: Option<&CancelFlag>,
    ) -> Result<reqwest::Response, BackupError> {
        req.send().await.map_err(|e| classify(&e, cancel))
    }
}

/// `reqwest::Error` → [`BackupError`]（取消优先）
fn classify(e: &reqwest::Error, cancel: Option<&CancelFlag>) -> BackupError {
    if cancel.map(CancelFlag::is_cancelled).unwrap_or(false) {
        return BackupError::Cancelled;
    }
    BackupError::WebDav {
        status: None,
        user_message: network_message(&error_haystack(e), e.is_timeout()),
    }
}

/// 把整条错误链拼成一段文本（`Display` 只有最外层，成因在 `source()` 里）
fn error_haystack(e: &reqwest::Error) -> String {
    let mut out = e.to_string();
    let mut src: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(e);
    while let Some(inner) = src {
        out.push_str(" | ");
        out.push_str(&inner.to_string());
        src = inner.source();
    }
    out
}

/// 上传体的流状态
struct UploadState<F> {
    path: PathBuf,
    file: Option<tokio::fs::File>,
    done: u64,
    total: u64,
    on_progress: F,
    cancel: Option<CancelFlag>,
    finished: bool,
}

/// 带进度的上传体：逐块读文件，每块回调一次。
///
/// ⚠️ 用 `stream::unfold` 而**不是**先把文件读进内存 —— 备份包可能几十 MB。
/// ⚠️ 出错后置 `finished` 并让后续 poll 返回 `None`，避免流在错误上打转。
fn upload_body<F>(
    path: PathBuf,
    total: u64,
    on_progress: F,
    cancel: Option<CancelFlag>,
) -> reqwest::Body
where
    F: Fn(u64, u64) + Send + Sync + 'static,
{
    let state = UploadState {
        path,
        file: None,
        done: 0,
        total,
        on_progress,
        cancel,
        finished: false,
    };
    reqwest::Body::wrap_stream(stream::unfold(state, |mut st| async move {
        if st.finished {
            return None;
        }
        if st.cancel.as_ref().map(CancelFlag::is_cancelled).unwrap_or(false) {
            st.finished = true;
            return Some((Err(cancelled_io()), st));
        }
        if st.file.is_none() {
            let opened = tokio::fs::File::open(&st.path).await;
            match opened {
                Ok(f) => st.file = Some(f),
                Err(e) => {
                    st.finished = true;
                    return Some((Err(e), st));
                }
            }
        }
        let mut buf = vec![0u8; CHUNK];
        let read = match st.file.as_mut() {
            Some(f) => f.read(&mut buf).await,
            None => return None,
        };
        match read {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                st.done += n as u64;
                (st.on_progress)(st.done, st.total);
                Some((Ok(buf), st))
            }
            Err(e) => {
                st.finished = true;
                Some((Err(e), st))
            }
        }
    }))
}

fn cancelled_io() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled")
}

// =============================================================================
// 测试
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // 纯函数
    // ------------------------------------------------------------------

    #[test]
    fn mkcol_success_set_matches_source() {
        // 源：`setOf(200, 201, 204, 301, 302, 405)`
        for ok in [200u16, 201, 204, 301, 302, 405] {
            assert!(is_mkcol_ok(ok), "{ok} 应算成功");
        }
        // 🔴 405 = 已存在（坚果云就是这样），必须放行
        assert!(is_mkcol_ok(405));
        for bad in [400u16, 401, 403, 404, 409, 423, 500, 507] {
            assert!(!is_mkcol_ok(bad), "{bad} 不应算成功");
        }
        assert_eq!(OK_MKCOL.len(), 6);
    }

    #[test]
    fn delete_success_set_matches_source() {
        // 源：`setOf(200, 202, 204, 404)`
        for ok in [200u16, 202, 204, 404] {
            assert!(is_delete_ok(ok), "{ok} 应算成功");
        }
        // 🔴 404 = 已经不在了，目标状态已达成
        assert!(is_delete_ok(404));
        for bad in [400u16, 401, 403, 405, 409, 423, 500, 507] {
            assert!(!is_delete_ok(bad), "{bad} 不应算成功");
        }
        assert_eq!(OK_DELETE.len(), 4);
    }

    #[test]
    fn basic_auth_value_encodes_user_colon_pass() {
        // `Basic base64("user:pass")`
        assert_eq!(basic_auth_value("user", "pass"), "Basic dXNlcjpwYXNz");
        // 邮箱 + 应用密码（坚果云的实际形态）
        assert_eq!(
            basic_auth_value("me@x.com", "abcd1234"),
            "Basic bWVAeC5jb206YWJjZDEyMzQ="
        );
        // 空密码也要发（服务端会回 401，那才是准确的结果）
        assert_eq!(basic_auth_value("u", ""), "Basic dTo=");
    }

    /// 用户名里的冒号**不**做转义（RFC 7617：第一个冒号才是分隔符）
    #[test]
    fn basic_auth_keeps_colon_in_username() {
        assert_eq!(basic_auth_value("a:b", "c"), "Basic YTpiOmM=");
    }

    #[test]
    fn network_message_branches() {
        assert_eq!(
            network_message("error sending request", true),
            "连接超时，请检查网络或服务器地址"
        );
        assert_eq!(
            network_message("operation timed out", false),
            "连接超时，请检查网络或服务器地址"
        );
        assert_eq!(
            network_message("dns error | failed to lookup address information", false),
            "无法解析服务器地址，请检查网络与地址拼写"
        );
        assert_eq!(
            network_message("unable to resolve host", false),
            "无法解析服务器地址，请检查网络与地址拼写"
        );
        assert_eq!(
            network_message("invalid peer certificate: UnknownIssuer", false),
            "安全连接失败（证书或协议不受支持）"
        );
        assert_eq!(
            network_message("tcp connect error | Connection refused (os error 10061)", false),
            "无法连接服务器，请检查地址与端口"
        );
        assert_eq!(
            network_message("No connection could be made because the target machine actively refused it.", false),
            "无法连接服务器，请检查地址与端口",
            "Windows 的 refused 文案"
        );
        assert_eq!(
            network_message("some weird thing", false),
            "网络错误：some weird thing"
        );
    }

    /// 🔴 判定顺序：证书错误里含 "connect"，不能被「无法连接」抢走
    #[test]
    fn network_message_prefers_tls_over_connect() {
        let reqwest_style =
            "error trying to connect: invalid peer certificate: UnknownIssuer";
        assert_eq!(
            network_message(reqwest_style, false),
            "安全连接失败（证书或协议不受支持）",
            "源顺序会把这条误判成「无法连接服务器」"
        );
        // 超时优先级最高（连 TLS 握手超时也算超时）
        assert_eq!(
            network_message("error trying to connect: invalid peer certificate", true),
            "连接超时，请检查网络或服务器地址"
        );
    }

    // ------------------------------------------------------------------
    // 本地 mock server
    // ------------------------------------------------------------------

    // mock server 与夹具在 `crate::test_mock`（webdav_repo 的测试也用它）
    use crate::test_mock::{conn, multistatus, tempdir, test_client, Resp};
    use crate::test_mock as mock;

    // ---- ensure_directory ----

    /// 🔴 单级目录只发一次 MKCOL，且 **405（已存在）算成功** —— 坚果云就是这样
    #[tokio::test]
    async fn ensure_directory_accepts_405_as_already_exists() {
        let srv = mock::start(vec![Resp::new(405, "")]).await;
        let c = conn(&srv);

        test_client().ensure_directory(&c, None).await.unwrap();

        assert_eq!(srv.count(), 1, "remote_dir 是单级 → 只发一次");
        assert_eq!(srv.req(0).method, "MKCOL");
        assert_eq!(srv.req(0).path(), "/dav/civilcalc");
        assert!(srv.req(0).body.is_empty(), "MKCOL 不带请求体");
    }

    /// 201（新建成功）同样放行
    #[tokio::test]
    async fn ensure_directory_accepts_201() {
        let srv = mock::start(vec![Resp::new(201, "")]).await;
        let c = conn(&srv);
        test_client().ensure_directory(&c, None).await.unwrap();
        assert_eq!(srv.count(), 1);
    }

    /// 多级 remote_dir：逐级 MKCOL（父目录不存在直接建深层目录会 409）
    #[tokio::test]
    async fn ensure_directory_walks_multi_level_dir() {
        let srv = mock::start(vec![Resp::new(201, ""), Resp::new(201, ""), Resp::new(201, "")]).await;
        let mut c = conn(&srv);
        c.config.remote_dir = "a/b/c".to_string();

        test_client().ensure_directory(&c, None).await.unwrap();

        let paths: Vec<String> = (0..3).map(|i| srv.req(i).path().to_string()).collect();
        assert_eq!(paths, ["/dav/a", "/dav/a/b", "/dav/a/b/c"]);
    }

    #[tokio::test]
    async fn ensure_directory_reports_error_with_action_word() {
        let srv = mock::start(vec![Resp::new(401, "nope")]).await;
        let c = conn(&srv);

        let e = test_client().ensure_directory(&c, None).await.unwrap_err();
        assert_eq!(e.http_status(), Some(401));
        assert!(
            e.user_message().contains("应用密码"),
            "实际: {}",
            e.user_message()
        );
    }

    /// 403 与 401 文案不同（403 讲目录权限）
    #[tokio::test]
    async fn ensure_directory_403_mentions_permission() {
        let srv = mock::start(vec![Resp::new(403, "")]).await;
        let c = conn(&srv);
        let e = test_client().ensure_directory(&c, None).await.unwrap_err();
        assert_eq!(e.http_status(), Some(403));
        assert_eq!(
            e.user_message(),
            "没有权限访问该目录（请检查账号权限或目录路径）"
        );
    }

    // ---- list ----

    #[tokio::test]
    async fn list_sends_propfind_with_depth_and_parses_entries() {
        let srv = mock::start(vec![Resp::new(207, multistatus())]).await;
        let c = conn(&srv);

        let entries = test_client().list(&c, &c.dir_url()).await.unwrap();

        let r = srv.req(0);
        assert_eq!(r.method, "PROPFIND");
        assert_eq!(r.path(), "/dav/civilcalc/");
        assert_eq!(r.header("depth"), Some("1"));
        assert!(r.header("content-type").unwrap().starts_with("application/xml"));
        assert!(!r.body.is_empty(), "PROPFIND 必须带请求体");
        assert!(String::from_utf8_lossy(&r.body).contains("getcontentlength"));

        assert_eq!(entries.len(), 2);
        assert!(entries[0].is_collection);
        assert_eq!(entries[1].size_bytes, 2048);
        assert!(!entries[1].is_collection);
    }

    #[tokio::test]
    async fn list_sends_authorization_header() {
        let srv = mock::start(vec![Resp::new(207, multistatus())]).await;
        let c = conn(&srv);
        test_client().list(&c, &c.dir_url()).await.unwrap();

        let r = srv.req(0);
        assert_eq!(r.header("authorization"), Some("Basic bWVAeC5jb206YXBwLXBhc3M="));
        assert_eq!(r.header("user-agent"), Some("CivilCalc-PC"));
    }

    #[tokio::test]
    async fn list_non_207_reports_read_directory() {
        let srv = mock::start(vec![Resp::new(404, "gone")]).await;
        let c = conn(&srv);
        let e = test_client().list(&c, &c.dir_url()).await.unwrap_err();
        assert_eq!(e.http_status(), Some(404));
        assert_eq!(e.user_message(), "远端路径不存在（读取目录）");
    }

    // ---- upload ----

    #[tokio::test]
    async fn upload_sends_content_length_and_reports_progress() {
        let dir = tempdir("upload-ok");
        let path = dir.join("pack.tar.gz");
        let data = vec![7u8; CHUNK * 2 + 100]; // 3 块
        std::fs::write(&path, &data).unwrap();

        let srv = mock::start(vec![Resp::new(201, "")]).await;
        let c = conn(&srv);
        let seen = Arc::new(std::sync::Mutex::new(Vec::<(u64, u64)>::new()));
        let sink = seen.clone();

        let url = format!("{}civilcalc_backup_20260915_143012.tar.gz", c.dir_url());
        let sent = test_client()
            .upload(
                &c,
                &url,
                &path,
                move |done, total| sink.lock().unwrap().push((done, total)),
                None,
            )
            .await
            .unwrap();

        assert_eq!(sent, data.len() as u64);

        let r = srv.req(0);
        assert_eq!(r.method, "PUT");
        assert_eq!(
            r.header("content-length"),
            Some(data.len().to_string().as_str()),
            "🔴 必须显式带 Content-Length（否则会退化成 chunked）"
        );
        assert_eq!(r.header("content-type"), Some("application/octet-stream"));
        assert_eq!(r.body.len(), data.len());
        assert!(r.body.iter().all(|b| *b == 7));

        let progress = seen.lock().unwrap().clone();
        assert_eq!(
            progress,
            vec![
                (CHUNK as u64, data.len() as u64),
                ((CHUNK * 2) as u64, data.len() as u64),
                (data.len() as u64, data.len() as u64),
            ],
            "每块回调一次，最后一块是完整长度"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn upload_non_2xx_reports_upload_action() {
        let dir = tempdir("upload-507");
        let path = dir.join("p.bin");
        std::fs::write(&path, b"x").unwrap();
        let srv = mock::start(vec![Resp::new(507, "")]).await;
        let c = conn(&srv);

        let e = test_client()
            .upload(&c, &srv.url("/dav/civilcalc/p.bin"), &path, |_, _| {}, None)
            .await
            .unwrap_err();
        assert_eq!(e.http_status(), Some(507));
        assert_eq!(e.user_message(), "云端空间不足，无法上传");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 预置取消标志 → 上传体立刻中断，返回 `Cancelled`（不是网络错误）
    #[tokio::test]
    async fn upload_with_preset_cancel_returns_cancelled() {
        let dir = tempdir("upload-cancel");
        let path = dir.join("p.bin");
        std::fs::write(&path, vec![0u8; CHUNK * 2]).unwrap();
        let srv = mock::start(vec![Resp::new(201, "")]).await;
        let c = conn(&srv);

        let flag = CancelFlag::new();
        flag.cancel();

        let e = test_client()
            .upload(&c, &srv.url("/dav/civilcalc/p.bin"), &path, |_, _| {}, Some(flag))
            .await
            .unwrap_err();
        assert_eq!(e, BackupError::Cancelled);

        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- download ----

    #[tokio::test]
    async fn download_streams_to_file_with_progress() {
        let dir = tempdir("download-ok");
        let target = dir.join("out.bin");
        let payload = vec![9u8; 5000];
        let srv = mock::start(vec![Resp::octet(200, payload.clone())]).await;
        let c = conn(&srv);
        let seen = Arc::new(std::sync::Mutex::new(Vec::<(u64, i64)>::new()));
        let sink = seen.clone();

        let len = test_client()
            .download(
                &c,
                &srv.url("/dav/civilcalc/x.tar.gz"),
                &target,
                move |done, total| sink.lock().unwrap().push((done, total)),
                None,
            )
            .await
            .unwrap();

        assert_eq!(len, payload.len() as u64);
        assert_eq!(std::fs::read(&target).unwrap(), payload);

        let r = srv.req(0);
        assert_eq!(r.method, "GET");

        let progress = seen.lock().unwrap().clone();
        assert_eq!(
            progress.last().copied(),
            Some((payload.len() as u64, payload.len() as i64)),
            "最后一块要报完整长度与总长度"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn download_non_2xx_reports_download_action() {
        let dir = tempdir("download-404");
        let target = dir.join("out.bin");
        let srv = mock::start(vec![Resp::new(404, "")]).await;
        let c = conn(&srv);

        let e = test_client()
            .download(&c, &srv.url("/dav/civilcalc/x.tar.gz"), &target, |_, _| {}, None)
            .await
            .unwrap_err();
        assert_eq!(e.user_message(), "远端路径不存在（下载）");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 下载途中取消 → `Cancelled`（半包留给编排层清理）
    #[tokio::test]
    async fn download_with_preset_cancel_returns_cancelled() {
        let dir = tempdir("download-cancel");
        let target = dir.join("out.bin");
        let srv = mock::start(vec![Resp::octet(200, vec![1u8; 1000])]).await;
        let c = conn(&srv);
        let flag = CancelFlag::new();
        flag.cancel();

        let e = test_client()
            .download(&c, &srv.url("/dav/civilcalc/x.tar.gz"), &target, |_, _| {}, Some(&flag))
            .await
            .unwrap_err();
        assert_eq!(e, BackupError::Cancelled);

        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- delete / exists ----

    #[tokio::test]
    async fn delete_accepts_404_as_success() {
        let srv = mock::start(vec![Resp::new(404, "")]).await;
        let c = conn(&srv);

        test_client()
            .delete(&c, &srv.url("/dav/civilcalc/x.tar.gz"))
            .await
            .unwrap();
        assert_eq!(srv.req(0).method, "DELETE");
    }

    #[tokio::test]
    async fn delete_reports_other_errors() {
        let srv = mock::start(vec![Resp::new(423, "")]).await;
        let c = conn(&srv);
        let e = test_client()
            .delete(&c, &srv.url("/dav/civilcalc/x.tar.gz"))
            .await
            .unwrap_err();
        assert_eq!(e.http_status(), Some(423));
        assert_eq!(e.user_message(), "远端文件被占用或锁定");
    }

    #[tokio::test]
    async fn exists_distinguishes_207_and_404() {
        let srv = mock::start(vec![Resp::new(207, multistatus()), Resp::new(404, "")]).await;
        let c = conn(&srv);

        assert!(test_client()
            .exists(&c, &srv.url("/dav/civilcalc/x.tar.gz"))
            .await
            .unwrap());
        assert!(!test_client()
            .exists(&c, &srv.url("/dav/civilcalc/y.tar.gz"))
            .await
            .unwrap());

        let r = srv.req(0);
        assert_eq!(r.method, "PROPFIND");
        assert_eq!(r.header("depth"), Some("0"), "exists 用 Depth:0");
    }

    #[tokio::test]
    async fn exists_reports_other_errors() {
        let srv = mock::start(vec![Resp::new(500, "")]).await;
        let c = conn(&srv);
        let e = test_client()
            .exists(&c, &srv.url("/dav/civilcalc/x.tar.gz"))
            .await
            .unwrap_err();
        assert_eq!(e.http_status(), Some(500));
        assert_eq!(e.user_message(), "远端服务器错误（HTTP 500）");
    }

    // ---- 网络层 ----

    /// 连不上的地址 → 中文「无法连接服务器」而不是英文原文
    #[tokio::test]
    async fn connection_refused_maps_to_chinese_message() {
        // 拿一个确定没人监听的端口：先 bind 再立刻 drop
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let cfg = crate::webdav_config::WebDavConfig {
            base_url: format!("http://{addr}/dav"),
            username: "u".to_string(),
            remote_dir: "civilcalc".to_string(),
            preset: crate::webdav_config::WebDavPreset::Custom,
        };
        let c = WebDavConnection::new(cfg, "p");

        let e = test_client()
            .delete(&c, &format!("http://{addr}/dav/x"))
            .await
            .unwrap_err();
        assert_eq!(e.http_status(), None, "网络层错误没有 HTTP 状态码");
        assert_eq!(
            e.user_message(),
            "无法连接服务器，请检查地址与端口",
            "实际: {}",
            e.user_message()
        );
    }
}
