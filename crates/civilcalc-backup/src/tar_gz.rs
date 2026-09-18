//! 极简 tar(USTAR) + gzip 读写：备份包 = 单个 `.tar.gz`。
//!
//! 源：`civilcalc-android-v2/core/backup/TarGz.kt`（232 行）
//!
//! ## 🔴 为什么**自研 tar**（ADR-009）
//!
//! 不引 `tar` crate —— 备份包要与 Android 端**互相可读**，
//! 必须精确控制：512 字节块、USTAR `prefix` 切分、GNU LongLink 的形状、两个零块收尾。
//! 第三方 crate 的默认行为（PAX 扩展头、长名策略、mtime 处理）与源不一致，
//! 会让 Android 端读到不同的条目名。
//!
//! ## 只实现自用子集
//!
//! | typeflag | 含义 | 处理 |
//! |---|---|---|
//! | `'0'` / `'\0'` / `'7'` | 常规文件 | 读出来回调 |
//! | `'L'` | GNU 长名（`././@LongLink`） | 内容即下一个条目的名字 |
//! | `'5'` / `'x'` / `'g'` / 其它 | 目录 / 扩展头 / 未知 | **跳过字节** |
//!
//! ## 名称编码策略
//!
//! 1. UTF-8 字节数 ≤ 100 → 直接放 `name`
//! 2. 否则按 `/` **从右往左**切：`head` ≤ 155 且 `tail` ≤ 100 → 放 `prefix` + `name`
//! 3. 还不行（单段过长）→ 先写一个 `././@LongLink` 条目（内容 = 完整名 + NUL），
//!    真条目名**按字节截断**，读时用长名覆盖
//!
//! ## ⚠️ gzip 容器字节可能与 Java 不完全一致（可接受）
//!
//! Java `GZIPOutputStream` 与 `flate2` 在 gzip **头部**（mtime / OS 字节）上可能不同。
//! 这**不影响互读** —— 解压只依赖 DEFLATE 流。ADR-009 要求的是「能被对方读出来」，
//! 不是「gzip 头逐字节相同」。tar 载荷本身是逐字节对齐的。

use std::io::{Read, Write};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;

/// tar 块大小。
const BLOCK: usize = 512;
/// `name` 字段长度。
const NAME_LEN: usize = 100;
/// `prefix` 字段长度。
const PREFIX_LEN: usize = 155;
/// 单条目上限（512 MB）—— 超过视为包损坏/恶意。
pub const MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024;

const TYPE_FILE: u8 = b'0';
const TYPE_LONG_NAME: u8 = b'L';
const LONG_LINK_NAME: &str = "././@LongLink";

/// 一个 tar 条目：名字 + 内容字节。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TarEntry {
    pub name: String,
    pub bytes: Vec<u8>,
}

impl TarEntry {
    #[must_use]
    pub fn new(name: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            name: name.into(),
            bytes,
        }
    }
}

// =============================================================================
// 写
// =============================================================================

/// 按顺序写入条目（gzip 包装），最后补两个零块收尾。
///
/// 接受 `IntoIterator` 是刻意的：每个条目的字节在被遍历时才需要存在，
/// 图片可以「读一张写一张」，**不必先把整包拼进内存**。
///
/// 返回**内层写出器**（gzip 层已 `finish`）—— 调用方可能还要在上面收尾
/// （例如加密层要写收尾块），靠 `Drop` 兜底既不明确也拿不到错误。
pub fn write_to<W, I>(out: W, entries: I) -> std::io::Result<W>
where
    W: Write,
    I: IntoIterator<Item = TarEntry>,
{
    let mut gz = GzEncoder::new(out, Compression::default());
    for entry in entries {
        write_entry(&mut gz, &entry.name, &entry.bytes)?;
    }
    // 两个零块收尾（tar 规范）
    gz.write_all(&[0u8; BLOCK])?;
    gz.write_all(&[0u8; BLOCK])?;
    // 返回内层写出器：调用方还要在上面收尾（如加密层写收尾块），
    // 靠 `Drop` 兜底不够明确，也拿不到错误。
    gz.finish()
}

fn write_entry<W: Write>(out: &mut W, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    if let Some((n, prefix)) = split_name(name) {
        write_header(out, &n, &prefix, bytes.len() as u64, TYPE_FILE)?;
        out.write_all(bytes)?;
        write_pad(out, bytes.len() as u64)?;
        return Ok(());
    }
    // USTAR 装不下（单段名过长）：先写长名条目，真条目名截断，读时用长名覆盖
    let mut name_bytes = name.as_bytes().to_vec();
    name_bytes.push(0);
    write_header(out, LONG_LINK_NAME, "", name_bytes.len() as u64, TYPE_LONG_NAME)?;
    out.write_all(&name_bytes)?;
    write_pad(out, name_bytes.len() as u64)?;

    let stub = truncate_to_bytes(name, NAME_LEN);
    write_header(out, &stub, "", bytes.len() as u64, TYPE_FILE)?;
    out.write_all(bytes)?;
    write_pad(out, bytes.len() as u64)?;
    Ok(())
}

fn write_header<W: Write>(
    out: &mut W,
    name: &str,
    prefix: &str,
    size: u64,
    typeflag: u8,
) -> std::io::Result<()> {
    let mut header = [0u8; BLOCK];
    put_string(&mut header, 0, NAME_LEN, name);
    put_octal(&mut header, 100, 8, 0o644); // mode
    put_octal(&mut header, 108, 8, 0); // uid
    put_octal(&mut header, 116, 8, 0); // gid
    put_octal(&mut header, 124, 12, size);
    put_octal(&mut header, 136, 12, now_secs());
    // 校验和先按 8 个空格算
    for b in &mut header[148..156] {
        *b = b' ';
    }
    header[156] = typeflag;
    put_string(&mut header, 257, 6, "ustar"); // magic "ustar\0"
    header[262] = 0;
    header[263] = b'0';
    header[264] = b'0';
    put_string(&mut header, 265, 32, "civilcalc"); // uname
    put_string(&mut header, 297, 32, "civilcalc"); // gname
    if !prefix.is_empty() {
        put_string(&mut header, 345, PREFIX_LEN, prefix);
    }
    let checksum: u64 = header.iter().map(|b| u64::from(*b)).sum();
    // `%06o` + NUL + 空格（共 8 字节）
    let text = format!("{checksum:06o}\0 ");
    put_string(&mut header, 148, 8, &text);
    out.write_all(&header)
}

/// 返回 `(name, prefix)`；无法用 USTAR 表达时返回 `None`。
fn split_name(raw_name: &str) -> Option<(String, String)> {
    if raw_name.len() <= NAME_LEN {
        return Some((raw_name.to_string(), String::new()));
    }
    // 从右往左找 '/'，让 tail 尽量短（tail 必须 ≤100）
    let mut search_end = raw_name.len();
    while let Some(idx) = raw_name[..search_end].rfind('/') {
        if idx == 0 {
            break;
        }
        let head = &raw_name[..idx];
        let tail = &raw_name[idx + 1..];
        if head.len() <= PREFIX_LEN && tail.len() <= NAME_LEN {
            return Some((tail.to_string(), head.to_string()));
        }
        search_end = idx;
    }
    None
}

/// 按 **UTF-8 字节数**截断（不能按字符数，中文一个字 3 字节），且落在字符边界。
fn truncate_to_bytes(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

fn write_pad<W: Write>(out: &mut W, size: u64) -> std::io::Result<()> {
    let pad = pad_len(size);
    if pad > 0 {
        out.write_all(&vec![0u8; pad])?;
    }
    Ok(())
}

fn put_string(buf: &mut [u8], offset: usize, length: usize, value: &str) {
    let bytes = value.as_bytes();
    let n = bytes.len().min(length);
    buf[offset..offset + n].copy_from_slice(&bytes[..n]);
}

/// 写 `len-1` 位八进制数字 + NUL（超长时**取低位**）。
fn put_octal(buf: &mut [u8], offset: usize, len: usize, value: u64) {
    let digits = format!("{value:o}");
    let width = len - 1;
    let padded: String = if digits.len() >= width {
        // 超长取末 width 位
        digits[digits.len() - width..].to_string()
    } else {
        format!("{digits:0>width$}")
    };
    put_string(buf, offset, width, &padded);
    buf[offset + len - 1] = 0;
}

// =============================================================================
// 读
// =============================================================================

/// 逐条读出。目录/未知类型条目**直接跳过**，只回调常规文件。
///
/// 回调式而非返回 `Vec`：导入时要按条落到图片库与数据库，
/// 一次只让一张图进内存，不必先把整包图片读进列表。
pub fn read_entries<R, F>(input: R, on_entry: &mut F) -> std::io::Result<()>
where
    R: Read,
    F: FnMut(TarEntry),
{
    let mut gz = GzDecoder::new(input);
    let mut header = [0u8; BLOCK];
    let mut long_name: Option<String> = None;

    loop {
        if !read_fully(&mut gz, &mut header)? {
            break;
        }
        if header.iter().all(|b| *b == 0) {
            break;
        }
        let name = match long_name.take() {
            Some(n) => n,
            None => name_of(&header),
        };
        let size = size_of(&header);
        if size > MAX_ENTRY_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("备份包条目异常（size={size}）"),
            ));
        }
        let padded = pad_len(size) as u64;

        match header[156] {
            // GNU 长名：内容就是下一个条目的名字
            TYPE_LONG_NAME => {
                let bytes = read_n(&mut gz, size)?;
                skip_fully(&mut gz, padded)?;
                long_name = Some(
                    String::from_utf8_lossy(&bytes)
                        .trim_end_matches('\0')
                        .trim_end_matches('\n')
                        .to_string(),
                );
            }
            TYPE_FILE | 0 | b'7' => {
                let bytes = read_n(&mut gz, size)?;
                skip_fully(&mut gz, padded)?;
                on_entry(TarEntry { name, bytes });
            }
            // 目录（'5'）、扩展头（'x'/'g'）等：只跳字节
            _ => skip_fully(&mut gz, size + padded)?,
        }
    }
    Ok(())
}

/// 一次性读全部条目（测试与元信息解析用；图片走 [`read_entries`] 逐条处理）。
pub fn read_all<R: Read>(input: R) -> std::io::Result<Vec<TarEntry>> {
    let mut list = Vec::new();
    read_entries(input, &mut |e| list.push(e))?;
    Ok(list)
}

fn name_of(header: &[u8; BLOCK]) -> String {
    let name = read_string(header, 0, NAME_LEN);
    let magic = read_string(header, 257, 6);
    let prefix = if magic == "ustar" {
        read_string(header, 345, PREFIX_LEN)
    } else {
        String::new()
    };
    if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    }
}

fn size_of(header: &[u8; BLOCK]) -> u64 {
    // base-256 大数编码（GNU/star 用于 >8GB 或含高位字节）
    if header[124] & 0x80 != 0 {
        let mut value = u64::from(header[124] & 0x7F);
        for b in &header[125..136] {
            value = (value << 8) | u64::from(*b);
        }
        return value;
    }
    let text = read_string(header, 124, 12);
    let text = text.trim();
    if text.is_empty() {
        return 0;
    }
    u64::from_str_radix(text, 8).unwrap_or(0)
}

fn read_string(buf: &[u8], offset: usize, length: usize) -> String {
    let limit = (offset + length).min(buf.len());
    let mut end = offset;
    while end < limit && buf[end] != 0 {
        end += 1;
    }
    String::from_utf8_lossy(&buf[offset..end]).to_string()
}

/// `size` 之后需要补齐的字节数。
fn pad_len(size: u64) -> usize {
    ((BLOCK as u64 - (size % BLOCK as u64)) % BLOCK as u64) as usize
}

/// 读满一个块返回 `true`；流已结束（含**只读到半个块 = 包被截断**）返回 `false`。
fn read_fully<R: Read>(input: &mut R, buf: &mut [u8]) -> std::io::Result<bool> {
    let mut read = 0usize;
    while read < buf.len() {
        let n = input.read(&mut buf[read..])?;
        if n == 0 {
            break;
        }
        read += n;
    }
    Ok(read == buf.len())
}

/// 读 `size` 字节；提前结束则返回已读到的部分（不报错）。
fn read_n<R: Read>(input: &mut R, size: u64) -> std::io::Result<Vec<u8>> {
    let mut out = vec![0u8; size as usize];
    let mut read = 0usize;
    while read < out.len() {
        let n = input.read(&mut out[read..])?;
        if n == 0 {
            break;
        }
        read += n;
    }
    out.truncate(read);
    Ok(out)
}

fn skip_fully<R: Read>(input: &mut R, count: u64) -> std::io::Result<()> {
    let mut remaining = count;
    let mut buf = [0u8; 8 * 1024];
    while remaining > 0 {
        let want = buf.len().min(remaining as usize);
        let n = input.read(&mut buf[..want])?;
        if n == 0 {
            return Ok(());
        }
        remaining -= n as u64;
    }
    Ok(())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 解压成原始 tar 字节（测试用；`.bytes()` 会被 clippy 判为低效）
    fn gunzip(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(data)
            .read_to_end(&mut out)
            .unwrap();
        out
    }

    fn pack(entries: Vec<TarEntry>) -> Vec<u8> {
        let mut buf = Vec::new();
        write_to(&mut buf, entries).unwrap();
        buf
    }

    fn roundtrip(entries: Vec<TarEntry>) -> Vec<TarEntry> {
        read_all(&pack(entries)[..]).unwrap()
    }

    // ---------------------------------------------------------------------
    // 基本往返
    // ---------------------------------------------------------------------

    #[test]
    fn single_entry_roundtrip() {
        let out = roundtrip(vec![TarEntry::new("manifest.json", b"{}".to_vec())]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "manifest.json");
        assert_eq!(out[0].bytes, b"{}");
    }

    #[test]
    fn multiple_entries_keep_order() {
        let out = roundtrip(vec![
            TarEntry::new("manifest.json", b"a".to_vec()),
            TarEntry::new("tables.json", b"b".to_vec()),
            TarEntry::new("prefs.json", b"c".to_vec()),
        ]);
        let names: Vec<&str> = out.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["manifest.json", "tables.json", "prefs.json"]);
    }

    /// 空条目列表 → 只有两个零块 → 读回空
    #[test]
    fn empty_archive() {
        assert!(roundtrip(vec![]).is_empty());
    }

    /// 空内容文件（size=0）也要能往返
    #[test]
    fn empty_file_entry() {
        let out = roundtrip(vec![TarEntry::new("empty.txt", Vec::new())]);
        assert_eq!(out.len(), 1);
        assert!(out[0].bytes.is_empty());
    }

    /// 恰好一个块的内容（512 字节）→ 不额外补 pad
    #[test]
    fn exact_block_size_no_padding() {
        let data = vec![7u8; BLOCK];
        let out = roundtrip(vec![TarEntry::new("blk.bin", data.clone())]);
        assert_eq!(out[0].bytes, data);
    }

    #[test]
    fn padding_boundaries_roundtrip() {
        for size in [1usize, 511, 512, 513, 1023, 1024, 1025] {
            let data = vec![9u8; size];
            let out = roundtrip(vec![TarEntry::new(format!("f{size}.bin"), data.clone())]);
            assert_eq!(out[0].bytes.len(), size, "size={size}");
            assert_eq!(out[0].bytes, data);
        }
    }

    /// 二进制内容（非 UTF-8）逐字节保真
    #[test]
    fn binary_content_is_faithful() {
        let data: Vec<u8> = (0u8..=255).collect();
        let out = roundtrip(vec![TarEntry::new("img.bin", data.clone())]);
        assert_eq!(out[0].bytes, data);
    }

    // ---------------------------------------------------------------------
    // 长名：prefix 切分
    // ---------------------------------------------------------------------

    /// `images/<64位哈希>.png` 这类：总长 >100 但能按 '/' 切开
    #[test]
    fn long_name_uses_prefix_field() {
        // tail 恰好 100 字节（96 + ".png"），head 是 "images"
        let hash = "a".repeat(96);
        let name = format!("images/{hash}.png");
        assert_eq!(name.len(), 107, "构造用例必须超 100 字节");
        let out = roundtrip(vec![TarEntry::new(&name, b"x".to_vec())]);
        assert_eq!(out[0].name, name, "prefix + name 拼回原名");
    }

    /// 单段过长（无 '/' 可切）→ 走 GNU LongLink
    #[test]
    fn overlong_single_segment_uses_longlink() {
        let name = "x".repeat(200);
        let out = roundtrip(vec![TarEntry::new(&name, b"y".to_vec())]);
        assert_eq!(out[0].name, name, "长名条目必须还原完整名");
        assert_eq!(out[0].bytes, b"y");
    }

    /// 中文长名（多字节）也不能被切碎
    #[test]
    fn multibyte_long_name_roundtrips() {
        let name = format!("images/{}.png", "中".repeat(50));
        assert!(name.len() > NAME_LEN);
        let out = roundtrip(vec![TarEntry::new(&name, b"z".to_vec())]);
        assert_eq!(out[0].name, name);
    }

    /// 长名条目**不产生多余条目**（读时被吞掉）
    #[test]
    fn longlink_is_consumed_not_emitted() {
        let out = roundtrip(vec![
            TarEntry::new("x".repeat(200), b"1".to_vec()),
            TarEntry::new("short.txt", b"2".to_vec()),
        ]);
        assert_eq!(out.len(), 2, "LongLink 自身不能出现在结果里");
        assert_eq!(out[1].name, "short.txt");
    }

    #[test]
    fn split_name_returns_none_when_unsplittable() {
        assert!(split_name(&"a".repeat(200)).is_none());
    }

    #[test]
    fn split_name_short_passthrough() {
        assert_eq!(
            split_name("a.json"),
            Some(("a.json".to_string(), String::new()))
        );
    }

    /// tail 超 100 时继续往左找（用更长的 prefix 换更短的 tail）
    #[test]
    fn split_name_searches_leftwards() {
        let tail = "b".repeat(120); // 超 100，必须再切
        let name = format!("a/{tail}");
        assert!(name.len() > NAME_LEN);
        // 无更左的 '/' → 切不开
        assert!(split_name(&name).is_none());
    }

    // ---------------------------------------------------------------------
    // 截断
    // ---------------------------------------------------------------------

    #[test]
    fn truncate_to_bytes_ascii() {
        assert_eq!(truncate_to_bytes(&"a".repeat(200), 100).len(), 100);
    }

    /// 🔴 中文按**字节**截断且不切碎字符
    #[test]
    fn truncate_to_bytes_is_char_safe() {
        let s = "中".repeat(50); // 150 字节
        let t = truncate_to_bytes(&s, 100);
        assert!(t.len() <= 100);
        assert_eq!(t.len() % 3, 0, "必须落在 3 字节字符边界");
        assert_eq!(t.chars().count(), 33, "100/3 = 33 个完整汉字");
        assert!(s.starts_with(&t));
    }

    #[test]
    fn truncate_to_bytes_short_passthrough() {
        assert_eq!(truncate_to_bytes("abc", 100), "abc");
    }

    // ---------------------------------------------------------------------
    // 头部字段（逐字节对齐）
    // ---------------------------------------------------------------------

    /// 🔴 校验和必须等于「校验和字段按 8 空格算」的字节和
    #[test]
    fn header_checksum_is_correct() {
        let data = pack(vec![TarEntry::new("a.txt", b"hi".to_vec())]);
        let raw = gunzip(&data);
        let header = &raw[..BLOCK];
        // 复算：把 148..156 换成空格再求和
        let mut copy = header.to_vec();
        for b in &mut copy[148..156] {
            *b = b' ';
        }
        let sum: u64 = copy.iter().map(|b| u64::from(*b)).sum();
        let stored = String::from_utf8_lossy(&header[148..154]).to_string();
        assert_eq!(u64::from_str_radix(&stored, 8).unwrap(), sum);
        assert_eq!(header[154], 0, "校验和以 NUL 结尾");
        assert_eq!(header[155], b' ', "第 8 字节是空格");
    }

    #[test]
    fn header_magic_and_version() {
        let data = pack(vec![TarEntry::new("a.txt", b"x".to_vec())]);
        let raw = gunzip(&data);
        let header = &raw[..BLOCK];
        assert_eq!(&header[257..263], b"ustar\0");
        assert_eq!(header[262], 0);
        assert_eq!(header[263], b'0');
        assert_eq!(header[264], b'0');
        assert_eq!(&header[265..274], b"civilcalc");
        assert_eq!(&header[297..306], b"civilcalc");
    }

    #[test]
    fn header_mode_is_0644() {
        let data = pack(vec![TarEntry::new("a.txt", b"x".to_vec())]);
        let raw = gunzip(&data);
        let mode = String::from_utf8_lossy(&raw[100..107]).to_string();
        assert_eq!(mode, "0000644");
        assert_eq!(raw[107], 0);
    }

    #[test]
    fn header_typeflag_is_file() {
        let data = pack(vec![TarEntry::new("a.txt", b"x".to_vec())]);
        let raw = gunzip(&data);
        assert_eq!(raw[156], b'0');
    }

    /// 两个零块收尾：tar 流总长是 512 的整数倍且末尾 ≥1024 字节全零
    #[test]
    fn archive_ends_with_two_zero_blocks() {
        let data = pack(vec![TarEntry::new("a.txt", b"x".to_vec())]);
        let raw = gunzip(&data);
        assert_eq!(raw.len() % BLOCK, 0);
        let tail = &raw[raw.len() - 1024..];
        assert!(tail.iter().all(|b| *b == 0), "末尾两个块必须全零");
    }

    #[test]
    fn put_octal_pads_and_truncates() {
        let mut buf = [0u8; 8];
        put_octal(&mut buf, 0, 8, 0o644);
        assert_eq!(&buf[..7], b"0000644");
        assert_eq!(buf[7], 0);

        // 超长取低位
        let mut buf2 = [0u8; 4];
        put_octal(&mut buf2, 0, 4, 0o7777777);
        assert_eq!(&buf2[..3], b"777");
        assert_eq!(buf2[3], 0);
    }

    // ---------------------------------------------------------------------
    // 读取健壮性
    // ---------------------------------------------------------------------

    /// 目录条目（'5'）被跳过
    #[test]
    fn directory_entry_is_skipped() {
        // 手工构造：目录头 + 文件头
        let mut raw = Vec::new();
        let mut dir_header = [0u8; BLOCK];
        put_string(&mut dir_header, 0, NAME_LEN, "images/");
        put_octal(&mut dir_header, 124, 12, 0);
        for b in &mut dir_header[148..156] {
            *b = b' ';
        }
        dir_header[156] = b'5';
        put_string(&mut dir_header, 257, 6, "ustar");
        let sum: u64 = dir_header.iter().map(|b| u64::from(*b)).sum();
        put_string(&mut dir_header, 148, 8, &format!("{sum:06o}\0 "));
        raw.extend_from_slice(&dir_header);

        let mut file_header = [0u8; BLOCK];
        put_string(&mut file_header, 0, NAME_LEN, "a.txt");
        put_octal(&mut file_header, 124, 12, 1);
        for b in &mut file_header[148..156] {
            *b = b' ';
        }
        file_header[156] = b'0';
        put_string(&mut file_header, 257, 6, "ustar");
        let sum2: u64 = file_header.iter().map(|b| u64::from(*b)).sum();
        put_string(&mut file_header, 148, 8, &format!("{sum2:06o}\0 "));
        raw.extend_from_slice(&file_header);
        raw.push(b'Z');
        raw.extend_from_slice(&[0u8; BLOCK - 1]);
        raw.extend_from_slice(&[0u8; BLOCK]);
        raw.extend_from_slice(&[0u8; BLOCK]);

        let mut gz = Vec::new();
        {
            let mut enc = GzEncoder::new(&mut gz, Compression::default());
            enc.write_all(&raw).unwrap();
            enc.finish().unwrap();
        }
        let out = read_all(&gz[..]).unwrap();
        assert_eq!(out.len(), 1, "目录条目应被跳过");
        assert_eq!(out[0].name, "a.txt");
        assert_eq!(out[0].bytes, b"Z");
    }

    /// size 超上限 → 报错（不是静默截断）
    #[test]
    fn oversize_entry_is_rejected() {
        let mut raw = Vec::new();
        let mut header = [0u8; BLOCK];
        put_string(&mut header, 0, NAME_LEN, "big.bin");
        put_octal(&mut header, 124, 12, 0); // 先占位
        for b in &mut header[148..156] {
            *b = b' ';
        }
        header[156] = b'0';
        put_string(&mut header, 257, 6, "ustar");
        // 用 base-256 写一个超过上限的值
        header[124] = 0x80;
        let huge: u64 = MAX_ENTRY_BYTES + 1;
        let be = huge.to_be_bytes();
        header[125..133].copy_from_slice(&be[..8]);
        let sum: u64 = header.iter().map(|b| u64::from(*b)).sum();
        put_string(&mut header, 148, 8, &format!("{sum:06o}\0 "));
        raw.extend_from_slice(&header);
        raw.extend_from_slice(&[0u8; BLOCK]);
        raw.extend_from_slice(&[0u8; BLOCK]);

        let mut gz = Vec::new();
        {
            let mut enc = GzEncoder::new(&mut gz, Compression::default());
            enc.write_all(&raw).unwrap();
            enc.finish().unwrap();
        }
        let err = read_all(&gz[..]).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    /// 🔴 base-256 大数 size 能被正确读出
    #[test]
    fn base256_size_is_decoded() {
        let mut raw = Vec::new();
        let mut header = [0u8; BLOCK];
        put_string(&mut header, 0, NAME_LEN, "n.bin");
        header[124] = 0x80; // base-256 标记
        header[135] = 5; // 低字节 = 5
        for b in &mut header[148..156] {
            *b = b' ';
        }
        header[156] = b'0';
        put_string(&mut header, 257, 6, "ustar");
        let sum: u64 = header.iter().map(|b| u64::from(*b)).sum();
        put_string(&mut header, 148, 8, &format!("{sum:06o}\0 "));
        raw.extend_from_slice(&header);
        raw.extend_from_slice(b"hello");
        raw.extend_from_slice(&[0u8; BLOCK - 5]);
        raw.extend_from_slice(&[0u8; BLOCK]);
        raw.extend_from_slice(&[0u8; BLOCK]);

        let mut gz = Vec::new();
        {
            let mut enc = GzEncoder::new(&mut gz, Compression::default());
            enc.write_all(&raw).unwrap();
            enc.finish().unwrap();
        }
        let out = read_all(&gz[..]).unwrap();
        assert_eq!(out[0].bytes, b"hello");
    }

    /// 截断的包（只有半个块）→ 优雅结束，不 panic
    #[test]
    fn truncated_archive_stops_gracefully() {
        let data = pack(vec![TarEntry::new("a.txt", b"x".to_vec())]);
        // 去掉尾部若干字节（解压后短于一个块）
        let raw = gunzip(&data);
        let cut = &raw[..BLOCK]; // 只留头部，无内容块

        let mut gz = Vec::new();
        {
            let mut enc = GzEncoder::new(&mut gz, Compression::default());
            enc.write_all(cut).unwrap();
            enc.finish().unwrap();
        }
        // 不应 panic；内容为空（因为 size=1 但流已结束）
        let out = read_all(&gz[..]).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out[0].bytes.is_empty(), "提前结束 → 读到的字节为空");
    }

    #[test]
    fn pad_len_cases() {
        assert_eq!(pad_len(0), 0);
        assert_eq!(pad_len(1), 511);
        assert_eq!(pad_len(511), 1);
        assert_eq!(pad_len(512), 0);
        assert_eq!(pad_len(513), 511);
    }

    /// 条目名里的 `prefix` 只在 magic 为 ustar 时才拼（老 tar 无 prefix 字段）
    #[test]
    fn name_of_ignores_prefix_without_ustar_magic() {
        let mut header = [0u8; BLOCK];
        put_string(&mut header, 0, NAME_LEN, "a.txt");
        put_string(&mut header, 345, PREFIX_LEN, "images");
        assert_eq!(name_of(&header), "a.txt", "magic 不是 ustar → 不拼 prefix");
    }
}
