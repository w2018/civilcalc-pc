//! WebDAV 路径处理。
//!
//! 源：`civilcalc-android-v2/core/backup/WebDavPath.kt`
//!
//! ## 状态
//!
//! | 函数 | 状态 |
//! |---|---|
//! | `normalize_base_url` | ✅ |
//! | `encode_segment` | ✅ |
//! | `remote_dir_url` | ✅ |
//! | `file_url` | ✅ |
//! | `ancestor_urls` | ✅ P4-13 |
//! | `decode` | ✅ P4-13 |
//! | `name_of_href` | ✅ P4-13 |
//! | `base_url_error` | ✅ P4-13 |
//!
//! ## 为什么 `encode_segment` 要自己写
//!
//! 段编码规则是 **RFC 3986 的 unreserved 集合**（`A-Z a-z 0-9 - _ . ~`），
//! 其余按 UTF-8 百分号编码。这与"表单编码"（空格 → `+`）和
//! "整条 URL 编码"（会连 `/` 一起编码）都不同，用通用库反而容易错。
//! 源项目也是手写的，这里逐行对齐。

/// 校验 base URL；返回中文错误说明，`None` 表示通过。
///
/// ## 🔴 只放行 https（源的硬约束）
///
/// 源注释原话：
///
/// > 只支持 https：App 的 `network_security_config` 全局禁止明文流量，
/// > 内网 http 服务器会被系统直接拦掉，所以在这里提前给出中文提示，
/// > 而不是等请求失败后报一个看不懂的错误。
///
/// PC 端**保留这条约束** —— 理由与平台无关：备份包里含 API Key 与 WebDAV 密码，
/// 明文 HTTP 会把凭据暴露在链路上。提前拦下来比事后排查好用。
///
/// ⚠️ 校验只针对**用户填写的配置**；[`crate::webdav_client`] 本身不校验协议，
/// 这样本地 mock server（http）可以在测试里直接对接。
///
/// ```
/// # use civilcalc_backup::webdav_path::base_url_error;
/// assert_eq!(base_url_error("").as_deref(), Some("请填写服务器地址"));
/// assert_eq!(
///     base_url_error("http://192.168.2.10:5005/dav").as_deref(),
///     Some("只支持 https 地址（本机安全策略禁止明文流量）")
/// );
/// assert_eq!(base_url_error("dav.jianguoyun.com/dav").as_deref(), Some("地址需以 https:// 开头"));
/// assert_eq!(base_url_error("https:///dav").as_deref(), Some("地址缺少主机名"));
/// assert!(base_url_error("https://dav.jianguoyun.com/dav").is_none());
/// ```
pub fn base_url_error(raw: &str) -> Option<String> {
    let url = raw.trim();
    if url.is_empty() {
        return Some("请填写服务器地址".to_string());
    }
    // 大小写不敏感（源用 `ignoreCase = true`）
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") {
        return Some("只支持 https 地址（本机安全策略禁止明文流量）".to_string());
    }
    if !lower.starts_with("https://") {
        return Some("地址需以 https:// 开头".to_string());
    }
    // scheme 已确认是纯 ASCII，按**字节**切安全
    let rest = &url[HTTPS_SCHEME.len()..];
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.trim().is_empty() {
        return Some("地址缺少主机名".to_string());
    }
    None
}

/// `https://` 的长度（8）。源用 `HTTPS.length` 取，这里保留同名常量便于对照。
const HTTPS_SCHEME: &str = "https://";

/// 去掉首尾空白与**末尾所有** `/`。
///
/// `" https://dav.jianguoyun.com/dav/ "` → `"https://dav.jianguoyun.com/dav"`
pub fn normalize_base_url(raw: &str) -> String {
    raw.trim().trim_end_matches('/').to_string()
}

/// 段编码：保留 unreserved 字符（`A-Z a-z 0-9 - _ . ~`），其余按 UTF-8 百分号编码。
///
/// ```
/// # use civilcalc_backup::webdav_path::encode_segment;
/// assert_eq!(encode_segment("civilcalc"), "civilcalc");
/// assert_eq!(encode_segment("a b"), "a%20b");        // 空格 → %20（不是 +）
/// assert_eq!(encode_segment("中文"), "%E4%B8%AD%E6%96%87");
/// assert_eq!(encode_segment("a/b"), "a%2Fb");        // 斜杠必须编码，否则会变成新的一级目录
/// ```
pub fn encode_segment(segment: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(segment.len());
    for b in segment.as_bytes() {
        let c = *b as char;
        let unreserved = c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~';
        if unreserved {
            out.push(c);
        } else {
            out.push('%');
            out.push(HEX[((b >> 4) & 0x0F) as usize] as char);
            out.push(HEX[(b & 0x0F) as usize] as char);
        }
    }
    out
}

/// 远端文件夹 URL（**末尾带斜杠** —— PROPFIND / MKCOL 都按集合处理）。
///
/// `remote_dir` 为空时退化为 base URL + `/`。
pub fn remote_dir_url(base_url: &str, remote_dir: &str) -> String {
    let dir = remote_dir.trim().trim_matches('/');
    let base = normalize_base_url(base_url);
    if dir.is_empty() {
        format!("{base}/")
    } else {
        format!("{base}/{}/", encode_segment(dir))
    }
}

/// 远端某个文件的 URL（文件名按段编码）
pub fn file_url(base_url: &str, remote_dir: &str, file_name: &str) -> String {
    format!(
        "{}{}",
        remote_dir_url(base_url, remote_dir),
        encode_segment(file_name)
    )
}

/// 逐级父目录 URL。
///
/// `https://host/dav` + `a/b/c` → `["https://host/dav/a", "https://host/dav/a/b", "https://host/dav/a/b/c"]`
/// （**不含**末尾斜杠那一级，自身由调用方拼）。
///
/// MKCOL 必须**逐级创建**（父目录不存在时直接建深层目录会 409）。
pub fn ancestor_urls(base_url: &str, remote_dir: &str) -> Vec<String> {
    let base = normalize_base_url(base_url);
    let dir = remote_dir.trim().trim_matches('/');
    if dir.is_empty() {
        return Vec::new();
    }
    let mut acc = base;
    dir.split('/')
        .filter(|s| !s.trim().is_empty())
        .map(|segment| {
            acc = format!("{acc}/{}", encode_segment(segment));
            acc.clone()
        })
        .collect()
}

/// 取 `href` 的最后一段作为名字（去掉 query/fragment、做百分号解码）。
///
/// 目录 href 末尾的斜杠已归一。
pub fn name_of_href(href: &str) -> String {
    let path = href
        .split('?')
        .next()
        .unwrap_or(href)
        .split('#')
        .next()
        .unwrap_or(href);
    let path = path.trim_end_matches('/');
    let last = path.rsplit('/').next().unwrap_or("");
    decode(last)
}

/// 百分号解码：**非法转义原样保留**。
///
/// 服务端返回的 href 不一定规范，**不能因为一个字符丢掉整条记录**。
pub fn decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        // 取一个**完整 UTF-8 字符**的字节（与源 `c.toString().toByteArray(UTF_8)` 等价）
        let len = utf8_char_len(bytes[i]);
        let end = (i + len).min(bytes.len());
        out.extend_from_slice(&bytes[i..end]);
        i = end;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

/// UTF-8 首字节 → 该字符的总字节数（非法首字节按 1 处理，交给 lossy 兜底）。
fn utf8_char_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else if b >> 3 == 0b11110 {
        4
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_trims_whitespace_and_trailing_slashes() {
        assert_eq!(
            normalize_base_url("  https://dav.jianguoyun.com/dav/  "),
            "https://dav.jianguoyun.com/dav"
        );
        assert_eq!(
            normalize_base_url("https://dav.jianguoyun.com/dav///"),
            "https://dav.jianguoyun.com/dav"
        );
        // 只有根路径时不把 host 的斜杠也去掉
        assert_eq!(normalize_base_url("https://x.com/"), "https://x.com");
        assert_eq!(normalize_base_url(""), "");
    }

    #[test]
    fn encode_keeps_unreserved() {
        assert_eq!(encode_segment("abcXYZ019"), "abcXYZ019");
        assert_eq!(encode_segment("-_.~"), "-_.~");
    }

    #[test]
    fn encode_escapes_specials() {
        assert_eq!(encode_segment("a b"), "a%20b");
        assert_eq!(encode_segment("a/b"), "a%2Fb");
        assert_eq!(encode_segment("a?b"), "a%3Fb");
        assert_eq!(encode_segment("a#b"), "a%23b");
        assert_eq!(encode_segment("a%b"), "a%25b");
        assert_eq!(encode_segment("a&b"), "a%26b");
    }

    #[test]
    fn encode_utf8_multibyte() {
        // "中" = E4 B8 AD，"文" = E6 96 87
        assert_eq!(encode_segment("中文"), "%E4%B8%AD%E6%96%87");
        assert_eq!(encode_segment("备份.tar.gz"), "%E5%A4%87%E4%BB%BD.tar.gz");
    }

    #[test]
    fn remote_dir_url_always_ends_with_slash() {
        assert_eq!(
            remote_dir_url("https://dav.jianguoyun.com/dav", "civilcalc"),
            "https://dav.jianguoyun.com/dav/civilcalc/"
        );
        // base 末尾多斜杠 / dir 首尾带斜杠 → 归一化
        assert_eq!(
            remote_dir_url("https://dav.jianguoyun.com/dav/", "/civilcalc/"),
            "https://dav.jianguoyun.com/dav/civilcalc/"
        );
    }

    #[test]
    fn remote_dir_url_empty_dir_is_base_plus_slash() {
        assert_eq!(
            remote_dir_url("https://dav.jianguoyun.com/dav", ""),
            "https://dav.jianguoyun.com/dav/"
        );
        assert_eq!(
            remote_dir_url("https://dav.jianguoyun.com/dav", "   "),
            "https://dav.jianguoyun.com/dav/"
        );
    }

    #[test]
    fn remote_dir_url_encodes_dir_name() {
        assert_eq!(
            remote_dir_url("https://x.com/dav", "我的备份"),
            "https://x.com/dav/%E6%88%91%E7%9A%84%E5%A4%87%E4%BB%BD/"
        );
    }

    #[test]
    fn file_url_appends_encoded_name() {
        assert_eq!(
            file_url("https://x.com/dav", "civilcalc", "backup_2026.tar.gz"),
            "https://x.com/dav/civilcalc/backup_2026.tar.gz"
        );
        assert_eq!(
            file_url("https://x.com/dav", "civilcalc", "备份 1.tar.gz"),
            "https://x.com/dav/civilcalc/%E5%A4%87%E4%BB%BD%201.tar.gz"
        );
    }

    // ---------------------------------------------------------------------
    // base_url_error（源 `地址校验拦截明文与非法地址`）
    // ---------------------------------------------------------------------

    /// 🔴 文案逐字对齐源项目（`WebDavPathTest.kt` 有同样的断言）
    #[test]
    fn base_url_error_blocks_plaintext_and_bad_urls() {
        assert_eq!(base_url_error("").as_deref(), Some("请填写服务器地址"));
        assert_eq!(base_url_error("   ").as_deref(), Some("请填写服务器地址"));
        assert_eq!(
            base_url_error("http://192.168.2.10:5005/dav").as_deref(),
            Some("只支持 https 地址（本机安全策略禁止明文流量）")
        );
        assert_eq!(
            base_url_error("dav.jianguoyun.com/dav").as_deref(),
            Some("地址需以 https:// 开头")
        );
        assert_eq!(base_url_error("https:///dav").as_deref(), Some("地址缺少主机名"));
        assert_eq!(base_url_error("https://dav.jianguoyun.com/dav"), None);
    }

    /// 大小写不敏感（源断言 `HTTPS://Dav.JianguoYun.com/dav` 通过）
    #[test]
    fn base_url_error_is_case_insensitive() {
        assert_eq!(base_url_error("HTTPS://Dav.JianguoYun.com/dav"), None);
        // 大写的明文地址同样要被拦（先判 http:// 再判 https://）
        assert_eq!(
            base_url_error("HTTP://192.168.1.1/dav").as_deref(),
            Some("只支持 https 地址（本机安全策略禁止明文流量）")
        );
    }

    /// 主机名判定：`?` / `#` 也要截断，空白主机名要拦
    #[test]
    fn base_url_error_checks_host() {
        assert_eq!(base_url_error("https://?x=1"), Some("地址缺少主机名".to_string()));
        assert_eq!(base_url_error("https://#frag"), Some("地址缺少主机名".to_string()));
        assert_eq!(base_url_error("https:// /dav"), Some("地址缺少主机名".to_string()));
        // 只有主机名也合法
        assert_eq!(base_url_error("https://x.com"), None);
        assert_eq!(base_url_error("https://x.com:8443/dav"), None);
    }

    /// 首尾空白先 trim（源 `url = raw.trim()`）
    #[test]
    fn base_url_error_trims_first() {
        assert_eq!(base_url_error("  https://x.com/dav  "), None);
        assert_eq!(base_url_error("  \t ").as_deref(), Some("请填写服务器地址"));
    }

    /// 非法协议（ftp / file / 相对路径）都按「需以 https:// 开头」拦
    #[test]
    fn base_url_error_rejects_other_schemes() {
        for bad in ["ftp://x.com/dav", "file:///dav", "/dav", "//x.com/dav", "x.com"] {
            assert_eq!(
                base_url_error(bad).as_deref(),
                Some("地址需以 https:// 开头"),
                "应拦住 {bad}"
            );
        }
    }

    /// 文件名里的 `/` 必须被编码，否则会变成新的一级目录
    #[test]
    fn file_url_never_creates_extra_path_levels() {
        let u = file_url("https://x.com/dav", "civilcalc", "a/b.tar.gz");
        assert_eq!(u, "https://x.com/dav/civilcalc/a%2Fb.tar.gz");
        assert_eq!(u.matches('/').count(), 5, "斜杠数应与正常情况一致");
    }

    // ---------------------------------------------------------------------
    // ancestor_urls（MKCOL 必须逐级创建）
    // ---------------------------------------------------------------------

    #[test]
    fn ancestors_are_progressive() {
        assert_eq!(
            ancestor_urls("https://x.com/dav", "a/b/c"),
            vec![
                "https://x.com/dav/a",
                "https://x.com/dav/a/b",
                "https://x.com/dav/a/b/c",
            ]
        );
    }

    #[test]
    fn ancestors_empty_for_blank_dir() {
        assert!(ancestor_urls("https://x.com/dav", "").is_empty());
        assert!(ancestor_urls("https://x.com/dav", "   ").is_empty());
        assert!(ancestor_urls("https://x.com/dav", "///").is_empty());
    }

    /// 单级目录 → 只有一个祖先（就是它自己那级，不含末尾斜杠）
    #[test]
    fn ancestors_single_level() {
        assert_eq!(
            ancestor_urls("https://x.com/dav/", "/civilcalc/"),
            vec!["https://x.com/dav/civilcalc"]
        );
    }

    /// 多级目录名要**逐级编码**
    #[test]
    fn ancestors_encode_each_segment() {
        assert_eq!(
            ancestor_urls("https://x.com/dav", "我的/备份"),
            vec![
                "https://x.com/dav/%E6%88%91%E7%9A%84",
                "https://x.com/dav/%E6%88%91%E7%9A%84/%E5%A4%87%E4%BB%BD",
            ]
        );
    }

    /// 空段被跳过（`a//b` 与 `a/b` 等价）
    #[test]
    fn ancestors_skip_blank_segments() {
        assert_eq!(
            ancestor_urls("https://x.com/dav", "a//b"),
            vec!["https://x.com/dav/a", "https://x.com/dav/a/b"]
        );
    }

    // ---------------------------------------------------------------------
    // decode（非法转义原样保留）
    // ---------------------------------------------------------------------

    #[test]
    fn decode_plain_text_untouched() {
        assert_eq!(decode("civilcalc"), "civilcalc");
        assert_eq!(decode(""), "");
    }

    #[test]
    fn decode_percent_escapes() {
        assert_eq!(decode("a%20b"), "a b");
        assert_eq!(decode("%E4%B8%AD%E6%96%87"), "中文");
        assert_eq!(decode("a%2Fb"), "a/b");
        assert_eq!(decode("%25"), "%");
    }

    /// 🔴 非法转义**原样保留**（不能丢字符）
    #[test]
    fn decode_keeps_invalid_escapes() {
        assert_eq!(decode("100%"), "100%", "末尾孤立的 %");
        assert_eq!(decode("a%zz"), "a%zz", "非十六进制");
        assert_eq!(decode("a%4"), "a%4", "不足两位");
        assert_eq!(decode("%"), "%");
    }

    /// 混合：合法转义 + 非法 + 多字节
    #[test]
    fn decode_mixed() {
        assert_eq!(decode("中文%20ok%zz"), "中文 ok%zz");
    }

    /// 大小写十六进制都认
    #[test]
    fn decode_hex_case_insensitive() {
        assert_eq!(decode("%e4%b8%ad"), "中");
        assert_eq!(decode("%E4%B8%AD"), "中");
    }

    // ---------------------------------------------------------------------
    // name_of_href
    // ---------------------------------------------------------------------

    #[test]
    fn name_of_href_takes_last_segment() {
        assert_eq!(name_of_href("/dav/civilcalc/a.tar.gz"), "a.tar.gz");
        assert_eq!(name_of_href("https://x.com/dav/a.tar.gz"), "a.tar.gz");
    }

    /// 目录 href 的末尾斜杠被归一
    #[test]
    fn name_of_href_trims_trailing_slash() {
        assert_eq!(name_of_href("/dav/civilcalc/"), "civilcalc");
        assert_eq!(name_of_href("/dav/civilcalc///"), "civilcalc");
    }

    /// query / fragment 被剥掉
    #[test]
    fn name_of_href_strips_query_and_fragment() {
        assert_eq!(name_of_href("/dav/a.tar.gz?token=1"), "a.tar.gz");
        assert_eq!(name_of_href("/dav/a.tar.gz#frag"), "a.tar.gz");
    }

    /// 百分号编码的名字被解码
    #[test]
    fn name_of_href_decodes() {
        assert_eq!(name_of_href("/dav/%E5%A4%87%E4%BB%BD.tar.gz"), "备份.tar.gz");
    }

    /// 空 href / 根路径 → 空串（不 panic）
    #[test]
    fn name_of_href_edge_cases() {
        assert_eq!(name_of_href(""), "");
        assert_eq!(name_of_href("/"), "");
        assert_eq!(name_of_href("///"), "");
    }
}
