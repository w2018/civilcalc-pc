//! WebDAV 多状态（207 Multi-Status）响应解析。
//!
//! 源：`civilcalc-android-v2/core/backup/WebDavXml.kt`（114 行）
//!
//! ## 🔴 为什么用宽松正则而不是 XML 解析器
//!
//! 源注释：*「各家服务端命名空间前缀不一（`D:` / `d:` / 无前缀），属性还可能同时出现在
//! 404 propstat 里（只有标签没有值）—— 所以取「第一个**非空**值」，而不是第一个匹配值。」*
//!
//! 这三点决定了不能上严格的 XML 解析器：
//! 1. 前缀不固定（`D:` / `d:` / 空 / 其它）
//! 2. 同一属性会出现**多次**（先 404 propstat 的空标签，再 200 propstat 的真值）
//! 3. 属性可能自闭合（`<D:getcontentlength/>`）
//!
//! ## 正则里的 `[^>/]*` 很关键
//!
//! 属性名后面若只写 `[^>]*`，自闭合标签 `<D:getcontentlength/>` 会匹配上，
//! 非贪婪的 `(.*?)` 就会一路吃到**下一条 propstat 的闭合标签**，把值读成垃圾。
//! 加上 `[^>/]*` 就要求「标签名后不能有 `/`」，自闭合直接被排除。

use std::sync::OnceLock;

use regex::Regex;

use crate::webdav_path;

/// 远端目录里的一条记录（PROPFIND 响应解析结果）。
///
/// [`RemoteEntry::href`] 保留服务端返回的原始路径（可能是绝对 URL，也可能是 `/dav/xxx`
/// 相对路径，且已做百分号解码）；[`RemoteEntry::display_name`] 优先取 `displayname` 属性，
/// 缺失时用 `href` 最后一段替代。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEntry {
    pub href: String,
    pub display_name: String,
    pub size_bytes: i64,
    pub last_modified_ms: i64,
    pub is_collection: bool,
}

/// PROPFIND 请求体：只取展示需要的最小属性集。
pub const PROP_FIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:">
  <D:prop>
    <D:displayname/>
    <D:getcontentlength/>
    <D:getlastmodified/>
    <D:resourcetype/>
  </D:prop>
</D:propfind>"#;

/// 命名空间前缀（可有可无）。
const TAG: &str = r"(?:[A-Za-z0-9_]+:)?";
/// 属性名之后到 `>` 之间的部分；**排除 `/`** 以挡掉自闭合标签（见模块文档）。
/// 🔴 五个提取正则**必须带 `(?s)`**（源用 `RegexOption.DOT_MATCHES_ALL`）。
///
/// 真实服务端返回的 207 是 **pretty-printed 多行 XML**：
///
/// ```xml
/// <D:response>
///   <D:href>/dav/x</D:href>
///   ...
/// </D:response>
/// ```
///
/// 没有 `(?s)` 时 `.` 不匹配换行，`<D:response>.*?</D:response>` 直接**匹配不到**，
/// 结果是「目录明明有备份，列表却是空的」—— 不报错、不提示，最难查的一类。
/// PC 端首版漏了这个标志，被 mock server 的多行响应用例抓出来。
///
/// （`COLLECTION` 不用 `.`，所以不需要。）
const OPEN: &str = r"\b[^>/]*>";

fn re_response() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(&format!("(?s)<{TAG}response\\b.*?</{TAG}response>"))
            .expect("response 正则合法")
    })
}

fn re_href() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(&format!("(?s)<{TAG}href{OPEN}(.*?)</{TAG}href>")).expect("href 正则合法")
    })
}

fn re_display_name() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(&format!("(?s)<{TAG}displayname{OPEN}(.*?)</{TAG}displayname>"))
            .expect("displayname 正则合法")
    })
}

fn re_content_length() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(&format!(
            "(?s)<{TAG}getcontentlength{OPEN}(.*?)</{TAG}getcontentlength>"
        ))
        .expect("getcontentlength 正则合法")
    })
}

fn re_last_modified() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(&format!(
            "(?s)<{TAG}getlastmodified{OPEN}(.*?)</{TAG}getlastmodified>"
        ))
        .expect("getlastmodified 正则合法")
    })
}

fn re_collection() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(&format!("<{TAG}collection\\b[^>]*/?>")).expect("collection 正则合法")
    })
}

/// 解析 207 响应；**无有效条目时返回空列表**（不抛异常，交给上层判断「目录为空」）。
#[must_use]
pub fn parse_multi_status(xml: &str) -> Vec<RemoteEntry> {
    if xml.trim().is_empty() {
        return Vec::new();
    }
    re_response()
        .find_iter(xml)
        .filter_map(|m| {
            let block = m.as_str();
            let href = first_non_blank(re_href(), block)?;
            let raw_href = unescape(&href).trim().to_string();
            if raw_href.is_empty() {
                return None;
            }
            let name = match first_non_blank(re_display_name(), block) {
                Some(d) => {
                    let t = unescape(&d).trim().to_string();
                    if t.is_empty() {
                        webdav_path::name_of_href(&raw_href)
                    } else {
                        t
                    }
                }
                None => webdav_path::name_of_href(&raw_href),
            };
            Some(RemoteEntry {
                href: webdav_path::decode(&raw_href),
                display_name: name,
                size_bytes: first_non_blank(re_content_length(), block)
                    .map(|s| s.trim().parse::<i64>().unwrap_or(0))
                    .unwrap_or(0),
                last_modified_ms: first_non_blank(re_last_modified(), block)
                    .map(|s| parse_http_date(unescape(&s).trim()))
                    .unwrap_or(0),
                is_collection: re_collection().is_match(block),
            })
        })
        .collect()
}

/// 取第一个**非空**捕获组的反转义文本；标签自闭合（`<x/>`）时捕获组为空 → 跳过。
fn first_non_blank(re: &Regex, block: &str) -> Option<String> {
    re.captures_iter(block)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .find(|s| !s.trim().is_empty())
}

/// XML 实体反转义（属性值里的 `&amp;` 等）。
#[must_use]
pub fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// HTTP 日期 → 毫秒时间戳；无法识别返回 `0`。
///
/// 时间**仅用于排序展示**，不影响下载 —— 所以解析失败不报错。
///
/// ## 🔴 严格 + 宽松两遍（与源 `isLenient` 的严格/宽松两遍对应）
///
/// 源先 `isLenient = false` 严格解析（Java 会校验**星期与日期是否自洽**），失败再放开。
///
/// ⚠️ **chrono 同样会校验星期**（这一点我先入为主地以为它不校验，被测试直接抓到）：
/// 写 `Mon, 15 Sep 2026 …`（该日实为周二）严格解析会失败。
/// 所以「宽松」这一遍必须**显式去掉星期再解析** —— 效果等价于忽略星期，
/// 时间只由年月日时分秒决定。这正是源想要的兜底：
/// *「个别服务端的星期写错，不该因此丢掉这条记录的修改时间」*。
///
/// ## ⚠️ 用 `NaiveDateTime` 而不是 `DateTime::parse_from_str`
///
/// 后者要求格式里出现时区**占位符**（`%z` / `%Z`）；这里的 `GMT` 是字面量，
/// 会直接报「缺时区」而解析失败。服务端时间本就是 GMT，按 UTC 解释即可。
#[must_use]
pub fn parse_http_date(text: &str) -> i64 {
    let t = text.trim();
    if t.is_empty() {
        return 0;
    }

    /// `(严格格式, 去掉星期后的宽松格式)`
    const PATTERNS: [(&str, Option<&str>); 5] = [
        // RFC 1123（WebDAV 标准）
        ("%a, %d %b %Y %H:%M:%S GMT", Some("%d %b %Y %H:%M:%S GMT")),
        // 少数服务端不带时区名
        ("%a, %d %b %Y %H:%M:%S", Some("%d %b %Y %H:%M:%S")),
        // RFC 850（两位年份）
        ("%A, %d-%b-%y %H:%M:%S GMT", Some("%d-%b-%y %H:%M:%S GMT")),
        // asctime（日期空格填充，用 %e）
        ("%a %b %e %H:%M:%S %Y", Some("%b %e %H:%M:%S %Y")),
        // ISO8601（少数服务端）—— 无星期，无需宽松分支
        ("%Y-%m-%dT%H:%M:%SZ", None),
    ];

    for (strict, lenient) in PATTERNS {
        if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(t, strict) {
            return ndt.and_utc().timestamp_millis();
        }
        if let Some(fmt) = lenient {
            if let Some(rest) = strip_weekday_prefix(t) {
                if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(rest, fmt) {
                    return ndt.and_utc().timestamp_millis();
                }
            }
        }
    }
    0
}

/// 去掉前导的星期名，返回其余部分（宽松重试用）。
///
/// 同时认两种写法：`"Mon, rest"`（RFC 1123/850）与 `"Mon rest"`（asctime）。
fn strip_weekday_prefix(s: &str) -> Option<&str> {
    fn is_weekday(name: &str) -> bool {
        matches!(
            name,
            "Mon" | "Tue" | "Wed" | "Thu" | "Fri" | "Sat" | "Sun"
                | "Monday"
                | "Tuesday"
                | "Wednesday"
                | "Thursday"
                | "Friday"
                | "Saturday"
                | "Sunday"
        )
    }
    if let Some((head, rest)) = s.split_once(", ") {
        if is_weekday(head) {
            return Some(rest);
        }
    }
    if let Some((head, rest)) = s.split_once(' ') {
        if is_weekday(head) {
            return Some(rest);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------
    // 典型响应
    // ---------------------------------------------------------------------

    fn wrap(body: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">{body}</D:multistatus>"#
        )
    }

    fn response(href: &str, extra: &str) -> String {
        format!(
            r#"<D:response><D:href>{href}</D:href><D:propstat><D:prop>{extra}</D:prop></D:propstat></D:response>"#
        )
    }

    /// 标准 `D:` 前缀 + 完整属性
    #[test]
    fn parses_standard_response() {
        let xml = wrap(&response(
            "/dav/civilcalc/civilcalc_backup_20260915_143005.tar.gz",
            "<D:displayname>civilcalc_backup_20260915_143005.tar.gz</D:displayname>\
             <D:getcontentlength>12345</D:getcontentlength>\
             <D:getlastmodified>Tue, 15 Sep 2026 14:30:05 GMT</D:getlastmodified>",
        ));
        let list = parse_multi_status(&xml);
        assert_eq!(list.len(), 1);
        let e = &list[0];
        assert_eq!(e.href, "/dav/civilcalc/civilcalc_backup_20260915_143005.tar.gz");
        assert_eq!(e.display_name, "civilcalc_backup_20260915_143005.tar.gz");
        assert_eq!(e.size_bytes, 12345);
        assert!(!e.is_collection);
        assert!(e.last_modified_ms > 0);
    }

    /// 小写前缀 / 无前缀都能认
    #[test]
    fn parses_any_namespace_prefix() {
        let lower = wrap(
            "<d:response><d:href>/a/b.tar.gz</d:href><d:propstat><d:prop>\
             <d:displayname>b.tar.gz</d:displayname></d:prop></d:propstat></d:response>",
        );
        assert_eq!(parse_multi_status(&lower)[0].display_name, "b.tar.gz");

        let bare = wrap(
            "<response><href>/a/c.tar.gz</href><propstat><prop>\
             <displayname>c.tar.gz</displayname></prop></propstat></response>",
        );
        assert_eq!(parse_multi_status(&bare)[0].display_name, "c.tar.gz");
    }

    /// 多个 response 都解析出来，顺序保持
    #[test]
    fn parses_multiple_responses_in_order() {
        let xml = wrap(&format!(
            "{}{}",
            response("/dav/a.tar.gz", "<D:displayname>a.tar.gz</D:displayname>"),
            response("/dav/b.tar.gz", "<D:displayname>b.tar.gz</D:displayname>"),
        ));
        let list = parse_multi_status(&xml);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].display_name, "a.tar.gz");
        assert_eq!(list[1].display_name, "b.tar.gz");
    }

    // ---------------------------------------------------------------------
    // 目录 / 大小
    // ---------------------------------------------------------------------

    /// `resourcetype` 里有 `collection` → 目录
    #[test]
    fn detects_collection() {
        let xml = wrap(&response(
            "/dav/civilcalc/",
            "<D:displayname>civilcalc</D:displayname>\
             <D:resourcetype><D:collection/></D:resourcetype>",
        ));
        let e = &parse_multi_status(&xml)[0];
        assert!(e.is_collection);
        assert_eq!(e.display_name, "civilcalc", "目录 href 末尾斜杠被归一");
    }

    /// 非目录（`resourcetype` 为空标签）
    #[test]
    fn not_collection_for_plain_file() {
        let xml = wrap(&response(
            "/dav/a.tar.gz",
            "<D:resourcetype/>",
        ));
        assert!(!parse_multi_status(&xml)[0].is_collection);
    }

    /// 🔴 自闭合的长度标签 → 0（**不能**把后面 propstat 的闭合标签吃进来）
    #[test]
    fn self_closing_length_is_zero_not_garbage() {
        let xml = wrap(
            "<D:response><D:href>/dav/a.tar.gz</D:href>\
             <D:propstat><D:prop><D:getcontentlength/></D:prop></D:propstat>\
             <D:propstat><D:prop><D:displayname>a.tar.gz</D:displayname></D:prop></D:propstat>\
             </D:response>",
        );
        let e = &parse_multi_status(&xml)[0];
        assert_eq!(e.size_bytes, 0, "自闭合应视为无值");
        assert_eq!(e.display_name, "a.tar.gz");
    }

    /// 🔴 404 propstat 的空标签在前，200 的真值在后 → 取**第一个非空**
    #[test]
    fn takes_first_non_blank_across_propstats() {
        let xml = wrap(
            "<D:response><D:href>/dav/a.tar.gz</D:href>\
             <D:propstat><D:prop><D:displayname/><D:getcontentlength/></D:prop>\
             <D:status>HTTP/1.1 404 Not Found</D:status></D:propstat>\
             <D:propstat><D:prop><D:displayname>a.tar.gz</D:displayname>\
             <D:getcontentlength>999</D:getcontentlength></D:prop>\
             <D:status>HTTP/1.1 200 OK</D:status></D:propstat>\
             </D:response>",
        );
        let e = &parse_multi_status(&xml)[0];
        assert_eq!(e.display_name, "a.tar.gz");
        assert_eq!(e.size_bytes, 999);
    }

    /// 长度非数字 → 0（不 panic）
    #[test]
    fn non_numeric_length_is_zero() {
        let xml = wrap(&response(
            "/dav/a",
            "<D:getcontentlength>not-a-number</D:getcontentlength>",
        ));
        assert_eq!(parse_multi_status(&xml)[0].size_bytes, 0);
    }

    // ---------------------------------------------------------------------
    // displayname 缺失 / 转义 / 解码
    // ---------------------------------------------------------------------

    /// 缺 `displayname` → 用 href 最后一段
    #[test]
    fn falls_back_to_href_last_segment() {
        let xml = wrap(&response("/dav/civilcalc/a.tar.gz", ""));
        assert_eq!(parse_multi_status(&xml)[0].display_name, "a.tar.gz");
    }

    /// `displayname` 是空白 → 也用 href 兜底
    #[test]
    fn blank_displayname_falls_back() {
        let xml = wrap(&response(
            "/dav/civilcalc/a.tar.gz",
            "<D:displayname>   </D:displayname>",
        ));
        assert_eq!(parse_multi_status(&xml)[0].display_name, "a.tar.gz");
    }

    /// XML 实体被反转义
    #[test]
    fn unescapes_entities() {
        let xml = wrap(&response(
            "/dav/a%20%26%20b.tar.gz",
            "<D:displayname>a &amp; b.tar.gz</D:displayname>",
        ));
        let e = &parse_multi_status(&xml)[0];
        assert_eq!(e.display_name, "a & b.tar.gz");
        assert_eq!(e.href, "/dav/a & b.tar.gz", "href 做百分号解码");
    }

    /// href 里的百分号编码被解码（含中文）
    #[test]
    fn decodes_percent_encoded_href() {
        let xml = wrap(&response(
            "/dav/%E5%A4%87%E4%BB%BD.tar.gz",
            "<D:displayname>备份.tar.gz</D:displayname>",
        ));
        let e = &parse_multi_status(&xml)[0];
        assert_eq!(e.href, "/dav/备份.tar.gz");
        assert_eq!(e.display_name, "备份.tar.gz");
    }

    /// 无 `href` 的 response 被跳过
    #[test]
    fn response_without_href_is_skipped() {
        let xml = wrap(
            "<D:response><D:propstat><D:prop><D:displayname>x</D:displayname></D:prop></D:propstat></D:response>\
             <D:response><D:href>/dav/a.tar.gz</D:href></D:response>",
        );
        let list = parse_multi_status(&xml);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].href, "/dav/a.tar.gz");
    }

    /// 空白 href 被跳过
    #[test]
    fn blank_href_is_skipped() {
        let xml = wrap("<D:response><D:href>   </D:href></D:response>");
        assert!(parse_multi_status(&xml).is_empty());
    }

    // ---------------------------------------------------------------------
    // 空 / 异常输入
    // ---------------------------------------------------------------------

    /// 🔴 空响应 → 空列表（不抛异常，交给上层判断「目录为空」）
    #[test]
    fn empty_input_returns_empty_list() {
        assert!(parse_multi_status("").is_empty());
        assert!(parse_multi_status("   \n  ").is_empty());
    }

    /// 没有 response 元素的 XML → 空列表
    #[test]
    fn xml_without_response_elements() {
        assert!(parse_multi_status(&wrap("")).is_empty());
        assert!(parse_multi_status("<html><body>403 Forbidden</body></html>").is_empty());
    }

    /// 畸形 XML（未闭合）不 panic
    #[test]
    fn malformed_xml_does_not_panic() {
        let _ = parse_multi_status("<D:multistatus><D:response><D:href>/a");
        let _ = parse_multi_status("<<<>>>");
        let _ = parse_multi_status("&amp;");
    }

    // ---------------------------------------------------------------------
    // HTTP 日期
    // ---------------------------------------------------------------------

    /// RFC 1123（WebDAV 标准）
    #[test]
    fn parse_rfc1123() {
        let ms = parse_http_date("Tue, 15 Sep 2026 14:30:05 GMT");
        assert!(ms > 0);
        // 反算回 UTC 校验
        let dt = chrono::DateTime::from_timestamp_millis(ms).unwrap();
        assert_eq!(dt.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-09-15 14:30:05");
    }

    /// 不带时区名也认
    #[test]
    fn parse_without_timezone_name() {
        let ms = parse_http_date("Tue, 15 Sep 2026 14:30:05");
        assert!(ms > 0);
    }

    /// RFC 850（两位年份）
    #[test]
    fn parse_rfc850() {
        let ms = parse_http_date("Tuesday, 15-Sep-26 14:30:05 GMT");
        assert!(ms > 0);
        let dt = chrono::DateTime::from_timestamp_millis(ms).unwrap();
        assert_eq!(dt.format("%Y-%m-%d").to_string(), "2026-09-15");
    }

    /// asctime（日期空格填充）
    #[test]
    fn parse_asctime() {
        let ms = parse_http_date("Tue Sep 15 14:30:05 2026");
        assert!(ms > 0);
        let dt = chrono::DateTime::from_timestamp_millis(ms).unwrap();
        assert_eq!(dt.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-09-15 14:30:05");
    }

    /// ISO8601
    #[test]
    fn parse_iso8601() {
        let ms = parse_http_date("2026-09-15T14:30:05Z");
        assert!(ms > 0);
        let dt = chrono::DateTime::from_timestamp_millis(ms).unwrap();
        assert_eq!(dt.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-09-15 14:30:05");
    }

    /// 🔴 无法识别 → 0（时间只用于排序展示，不影响下载）
    #[test]
    fn unparseable_date_is_zero() {
        assert_eq!(parse_http_date(""), 0);
        assert_eq!(parse_http_date("   "), 0);
        assert_eq!(parse_http_date("not a date"), 0);
        assert_eq!(parse_http_date("2026-13-45T99:99:99Z"), 0);
    }

    /// 🔴 星期写错也能解析（走「忽略星期」的宽松分支）
    ///
    /// 2026-09-15 实为**周二**，这里故意写 Mon。严格解析会失败，
    /// 宽松分支去掉星期后成功 —— 与源 `isLenient = true` 的兜底一致。
    #[test]
    fn wrong_weekday_still_parses() {
        let ms = parse_http_date("Mon, 15 Sep 2026 14:30:05 GMT");
        assert!(ms > 0, "星期不自洽不该让整条记录丢掉修改时间");
        let dt = chrono::DateTime::from_timestamp_millis(ms).unwrap();
        assert_eq!(
            dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-09-15 14:30:05",
            "时间只由年月日时分秒决定"
        );
    }

    /// asctime 的星期写错也能解析（宽松分支要认「无逗号」写法）
    #[test]
    fn wrong_weekday_asctime_still_parses() {
        // 2026-09-15 是周二，故意写 Mon
        let ms = parse_http_date("Mon Sep 15 14:30:05 2026");
        assert!(ms > 0);
        let dt = chrono::DateTime::from_timestamp_millis(ms).unwrap();
        assert_eq!(dt.format("%Y-%m-%d").to_string(), "2026-09-15");
    }

    /// RFC 850 的星期写错也能解析
    #[test]
    fn wrong_weekday_rfc850_still_parses() {
        let ms = parse_http_date("Monday, 15-Sep-26 14:30:05 GMT");
        assert!(ms > 0);
        let dt = chrono::DateTime::from_timestamp_millis(ms).unwrap();
        assert_eq!(dt.format("%Y-%m-%d").to_string(), "2026-09-15");
    }

    /// `strip_weekday_prefix` 只认真正的星期名
    #[test]
    fn strip_weekday_only_for_real_weekday_names() {
        assert_eq!(strip_weekday_prefix("Mon, rest"), Some("rest"));
        assert_eq!(strip_weekday_prefix("Monday, rest"), Some("rest"));
        assert_eq!(strip_weekday_prefix("Tue Sep 15"), Some("Sep 15"));
        assert_eq!(strip_weekday_prefix("Notaday, rest"), None);
        assert_eq!(strip_weekday_prefix("15 Sep 2026"), None);
        assert_eq!(strip_weekday_prefix(""), None);
    }

    // ---------------------------------------------------------------------
    // 常量与实体
    // ---------------------------------------------------------------------

    /// PROPFIND 请求体含 4 个属性且是合法 XML 片段
    #[test]
    fn propfind_body_has_four_props() {
        for prop in ["displayname", "getcontentlength", "getlastmodified", "resourcetype"] {
            assert!(PROP_FIND_BODY.contains(prop), "缺属性 {prop}");
        }
        assert!(PROP_FIND_BODY.starts_with("<?xml"));
        assert!(PROP_FIND_BODY.contains(r#"xmlns:D="DAV:""#));
    }

    #[test]
    fn unescape_all_five_entities() {
        assert_eq!(unescape("&lt;a&gt;"), "<a>");
        assert_eq!(unescape("&quot;q&quot;"), "\"q\"");
        assert_eq!(unescape("&apos;a&apos;"), "'a'");
        assert_eq!(unescape("&amp;"), "&");
        assert_eq!(unescape("plain"), "plain");
    }

    /// `&amp;lt;` 不该被二次反转义（替换顺序：先 `&lt;` 后 `&amp;`）
    #[test]
    fn unescape_is_single_pass() {
        assert_eq!(unescape("&amp;lt;"), "&lt;", "只反转义一层");
    }

    /// 🔴 回归：**多行**（pretty-printed）207 必须能解析
    ///
    /// 首版漏了 `(?s)`，`.` 不匹配换行 → 一条都解析不出来（列表恒为空）。
    /// 真实服务端返回的都是这种格式，所以这条测试比单行用例更贴近现实。
    #[test]
    fn parses_pretty_printed_multiline_response() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
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
    <D:href>/dav/civilcalc/civilcalc_backup_20260915_143012.tar.gz</D:href>
    <D:propstat>
      <D:prop>
        <D:displayname>civilcalc_backup_20260915_143012.tar.gz</D:displayname>
        <D:getcontentlength>2048</D:getcontentlength>
        <D:getlastmodified>Mon, 15 Sep 2026 14:30:12 GMT</D:getlastmodified>
        <D:resourcetype/>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

        let entries = parse_multi_status(xml);
        assert_eq!(entries.len(), 2, "多行 XML 必须解析出 2 条");
        assert!(entries[0].is_collection);
        assert_eq!(entries[1].display_name, "civilcalc_backup_20260915_143012.tar.gz");
        assert_eq!(entries[1].size_bytes, 2048);
        assert!(!entries[1].is_collection);
        assert!(entries[1].last_modified_ms > 0);
    }

    /// 属性值跨行也要能取到（`OPEN` 里的 `[^>/]*` 允许换行以外的任何字符）
    #[test]
    fn parses_attribute_on_own_line() {
        let xml = "<D:multistatus><D:response>\n<D:href\n>/a/b.tar.gz</D:href>\n</D:response></D:multistatus>";
        let entries = parse_multi_status(xml);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].display_name, "b.tar.gz");
    }
}
