//! 测试专用：极简 HTTP/1.1 mock server + 共享测试夹具。
//!
//! 只在 `cfg(test)` 下编译（见 `lib.rs` 的声明）。
//!
//! ## 为什么手写而不是引 `wiremock` / `mockito`
//!
//! - 本机离线，加新依赖要过缓存可用性这一关（P4-11 已踩过 `sha1` crate 的坑）
//! - 需求很窄：按脚本顺序回答、记录请求。手写 ~200 行足够，且**行为完全可见**
//!
//! ## ⚠️ 测试必须 `no_proxy`
//!
//! `reqwest` 默认读 `HTTP_PROXY` / `HTTPS_PROXY` 环境变量。本机实测环境里就有
//! `HTTP_PROXY=http://127.0.0.1:6798`，于是 `http://127.0.0.1:PORT` 的请求会被
//! 送进那个代理：
//!
//! - mock server 的用例仍能过（代理会转发）
//! - 但「连不上」用例会拿到代理返回的 **502**，而不是本机的网络错误
//!
//! 所以 [`test_client`] 显式 `.no_proxy()`。**生产客户端保留代理支持**
//! （企业网络需要），只有测试关掉。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::webdav_client::{WebDavClient, CONNECT_TIMEOUT, READ_TIMEOUT};
use crate::webdav_config::{WebDavConfig, WebDavConnection, WebDavPreset};

/// 一条被记录下来的请求
#[derive(Debug, Clone)]
pub struct Req {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Req {
    /// 按名字取头（大小写不敏感）
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// 去掉 query 的路径
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }
}

/// 一条预设响应
#[derive(Debug, Clone)]
pub struct Resp {
    pub status: u16,
    pub body: Vec<u8>,
    pub content_type: &'static str,
}

impl Resp {
    /// XML 响应（PROPFIND 用）
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            body: body.into(),
            content_type: "application/xml; charset=utf-8",
        }
    }

    /// 二进制响应（GET 用）
    pub fn octet(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            body: body.into(),
            content_type: "application/octet-stream",
        }
    }
}

/// 极简 mock server：按脚本顺序回答，记录每个请求
pub struct Server {
    pub addr: SocketAddr,
    pub requests: Arc<Mutex<Vec<Req>>>,
    handle: tokio::task::JoinHandle<()>,
}

impl Server {
    /// `http://<addr><path>`
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    /// 已收到的请求数
    pub fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    /// 第 n 个请求（越界会 panic，测试里应当先断言 `count()`）
    pub fn req(&self, n: usize) -> Req {
        self.requests.lock().unwrap()[n].clone()
    }

    /// 所有请求的 (method, path)
    pub fn method_paths(&self) -> Vec<(String, String)> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|r| (r.method.clone(), r.path().to_string()))
            .collect()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// 启动 mock server；`responses` 按请求顺序消费，用尽后一律 500
pub async fn start(responses: Vec<Resp>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let sink = requests.clone();
    let handle = tokio::spawn(async move {
        let mut queue: std::collections::VecDeque<Resp> = responses.into();
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let resp = queue
                .pop_front()
                .unwrap_or_else(|| Resp::new(500, "mock: no scripted response"));
            let sink = sink.clone();
            tokio::spawn(async move {
                let _ = serve(stream, resp, sink).await;
            });
        }
    });
    Server {
        addr,
        requests,
        handle,
    }
}

/// 测试用客户端：**禁用代理**（见模块文档）
pub fn test_client() -> WebDavClient {
    WebDavClient::with_http(
        reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .build()
            .unwrap(),
    )
}

/// 指向 mock server 的连接（默认目录 `civilcalc`）
pub fn conn_to(server: &Server, remote_dir: &str) -> WebDavConnection {
    let cfg = WebDavConfig {
        base_url: server.url("/dav"),
        username: "me@x.com".to_string(),
        remote_dir: remote_dir.to_string(),
        preset: WebDavPreset::Custom,
    };
    WebDavConnection::new(cfg, "app-pass")
}

/// 指向 mock server 的连接（默认目录）
pub fn conn(server: &Server) -> WebDavConnection {
    conn_to(server, "civilcalc")
}

/// 临时目录（按 tag + 线程区分，避免并发测试互踩）
pub fn tempdir(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "civilcalc-backup-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// 一个典型的多行 207 响应：目录项 + 一个备份文件
pub fn multistatus_with(href_tail: &str, display_name: &str, size: u64) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/dav/civilcalc/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/></D:resourcetype>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/civilcalc/{href_tail}</D:href>
    <D:propstat>
      <D:prop>
        <D:displayname>{display_name}</D:displayname>
        <D:getcontentlength>{size}</D:getcontentlength>
        <D:getlastmodified>Mon, 15 Sep 2026 14:30:12 GMT</D:getlastmodified>
        <D:resourcetype/>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#
    )
}

/// 默认的多行 207（`civilcalc_backup_20260915_143012.tar.gz`，2048 字节）
pub fn multistatus() -> String {
    multistatus_with(
        "civilcalc_backup_20260915_143012.tar.gz",
        "civilcalc_backup_20260915_143012.tar.gz",
        2048,
    )
}

// =============================================================================
// HTTP/1.1 服务端实现
// =============================================================================

async fn serve(
    mut stream: TcpStream,
    resp: Resp,
    sink: Arc<Mutex<Vec<Req>>>,
) -> std::io::Result<()> {
    // ---- 读请求头 ----
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let head_end = loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let headers: Vec<(String, String)> = lines
        .filter(|l| !l.is_empty())
        .filter_map(|l| {
            l.split_once(':')
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        })
        .collect();

    // ---- 读请求体 ----
    let mut body = buf[head_end..].to_vec();
    let content_len = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok());
    let chunked = headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked")
    });
    if let Some(len) = content_len {
        while body.len() < len {
            let n = stream.read(&mut tmp).await?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
        body.truncate(len);
    } else if chunked {
        body = read_chunked(&mut stream, body).await?;
    }

    sink.lock().unwrap().push(Req {
        method,
        target,
        headers,
        body,
    });

    // ---- 写响应（Connection: close → 一请求一连接，简单且不会串包）----
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nContent-Type: {}\r\nConnection: close\r\n\r\n",
        resp.status,
        reason(resp.status),
        resp.body.len(),
        resp.content_type
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&resp.body).await?;
    stream.flush().await?;
    Ok(())
}

/// 解 chunked 编码（reqwest 在长度未知时会用；本客户端显式设了 Content-Length，
/// 但保留这条分支以免将来改动后测试莫名失败）
async fn read_chunked(stream: &mut TcpStream, mut buf: Vec<u8>) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        let line_end = loop {
            if let Some(pos) = find(&buf, b"\r\n") {
                break pos;
            }
            let n = stream.read(&mut tmp).await?;
            if n == 0 {
                return Ok(out);
            }
            buf.extend_from_slice(&tmp[..n]);
        };
        let line = String::from_utf8_lossy(&buf[..line_end]).to_string();
        buf.drain(..line_end + 2);
        let size = usize::from_str_radix(line.split(';').next().unwrap_or("0").trim(), 16)
            .unwrap_or(0);
        if size == 0 {
            return Ok(out);
        }
        while buf.len() < size + 2 {
            let n = stream.read(&mut tmp).await?;
            if n == 0 {
                return Ok(out);
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        out.extend_from_slice(&buf[..size]);
        buf.drain(..size + 2);
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        207 => "Multi-Status",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        418 => "I'm a teapot",
        423 => "Locked",
        500 => "Internal Server Error",
        507 => "Insufficient Storage",
        _ => "Unknown",
    }
}
