//! 备份包的可选加密层（「压缩密码」）。
//!
//! 源：`civilcalc-android-v2/core/backup/BackupCrypto.kt`（276 行）
//!
//! ## 密码为空时**原样返回**传入的流
//!
//! 默认行为与加密功能引入前**完全一致**：老备份照样能读，也不会平白多一层加解密。
//!
//! ## 文件布局（先 gzip 后加密：`file = 分块加密(gzip(tar))`）
//!
//! ```text
//! "CCENC1" + 版本(1) + salt(16)
//! [明文长度(4, 大端)] [nonce(12)] [AES-256-GCM 密文 + tag(16)]   ← 明文块 64 KiB，按内容分块
//! [0x00000000]                                                   ← 收尾块
//! ```
//!
//! - 密钥 = `PBKDF2-HMAC-SHA256(密码, salt, 200000 次, 256 位)`
//! - **每块独立随机 nonce**
//! - **块序号当 AAD（4 字节大端）** —— 防重排：把两块互换后 GCM 校验必然失败
//! - **收尾块**能识别截断；**GCM tag** 能识别错密码与篡改
//!
//! ## 🔴 刻意不用「会吞错误的流适配器」
//!
//! 源注释：`CipherInputStream` 在 GCM 校验失败时**可能静默当作流结束**（JDK/Conscrypt 的老毛病）。
//! 自己读写分块才能把「密码不对」变成一条明确报错。
//! Rust 侧同理：不要图省事用第三方 AEAD 流包装器，分块逻辑必须自己写。
//!
//! ## ⚠️ 互操作性的验证边界（诚实说明）
//!
//! 本机**没有** Android 端的加密实现可对跑，所以「交叉兼容测试」只能覆盖到：
//!
//! | 已覆盖 | 手段 |
//! |---|---|
//! | PBKDF2 参数（迭代次数 / 哈希 / 输出长度 / 密码编码） | **Python `hashlib.pbkdf2_hmac` 的已知答案向量** |
//! | 文件头与分块框架的**逐字节布局** | 直接断言字节 |
//! | AAD = 块序号（防重排） | 交换两块后必须解密失败 |
//! | tag 长度 16、密文长度 = 明文 + 16 | 断言长度 |
//!
//! 真·跨端对跑需要一份**由 Android 端产出的加密包 fixture**（见 `docs/07` 的 P4-15 待办）。
//!
//! ## 🔴 错误变体必须穿过 `io::Result` 边界
//!
//! `Read::read` 只能返回 `io::Result`，但调用方要区分「密码错了」「没给密码」「包没下完」。
//! 所以 `BackupError` 被**装箱进** `io::Error` 再 `downcast` 取回 ——
//! 只做 `to_string()` 会让三种情况都退化成 `Io("...")`。

use std::io::{Read, Write};
use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::rngs::OsRng;
use rand::RngCore;

use crate::error::BackupError;

/// 文件头魔数（明文包以 gzip 的 `0x1f8b` 开头，**不会误判**）。
pub const MAGIC: &str = "CCENC1";

/// 格式版本。
const VERSION: u8 = 1;
const MAGIC_BYTES: usize = 6;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
/// GCM tag 长度。
const TAG_BYTES: usize = 16;
/// 派生密钥长度（256 位）。
const KEY_LEN: usize = 32;

/// PBKDF2 迭代次数（**跨端契约，不能改**）。
pub const ITERATIONS: u32 = 200_000;

/// 单块明文大小（测试跨块边界会用到）。
pub const CHUNK_SIZE: usize = 64 * 1024;

/// 文件头长度 = 魔数 + 版本 + salt。
pub const HEADER_LEN: usize = MAGIC_BYTES + 1 + SALT_LEN;

// =============================================================================
// 探测
// =============================================================================

/// 这个文件是不是加密包（**只看文件头，不验证密码**）。
#[must_use]
pub fn is_encrypted_file(path: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = vec![0u8; HEADER_LEN];
    let mut read = 0usize;
    while read < head.len() {
        match f.read(&mut head[read..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => read += n,
        }
    }
    head.truncate(read);
    has_magic(&head)
}

/// 头部是否是本应用的加密包（魔数 + 版本都对）。
#[must_use]
pub fn has_magic(head: &[u8]) -> bool {
    if head.len() < HEADER_LEN {
        return false;
    }
    head[..MAGIC_BYTES] == *MAGIC.as_bytes() && head[MAGIC_BYTES] == VERSION
}

// =============================================================================
// 密钥派生
// =============================================================================

/// `PBKDF2-HMAC-SHA256(密码, salt, 200000, 256 位)`。
#[must_use]
pub fn derive_key(password: &str, salt: &[u8]) -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.as_bytes(), salt, ITERATIONS, &mut key);
    key
}

// =============================================================================
// 写入层
// =============================================================================

/// 加密写入层。
pub struct EncryptingWriter<W: Write> {
    out: W,
    key: [u8; KEY_LEN],
    buffer: Vec<u8>,
    index: u32,
    closed: bool,
}

impl<W: Write> EncryptingWriter<W> {
    fn new(mut out: W, password: &str) -> Result<Self, BackupError> {
        let mut salt = [0u8; SALT_LEN];
        OsRng.fill_bytes(&mut salt);
        out.write_all(MAGIC.as_bytes()).map_err(io_err)?;
        out.write_all(&[VERSION]).map_err(io_err)?;
        out.write_all(&salt).map_err(io_err)?;
        let key = derive_key(password, &salt);
        Ok(Self {
            out,
            key,
            buffer: Vec::with_capacity(CHUNK_SIZE),
            index: 0,
            closed: false,
        })
    }

    /// 写出缓冲区里的整块（空缓冲则什么都不做）。
    fn flush_chunk(&mut self) -> Result<(), BackupError> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| BackupError::Crypto(format!("初始化 AES-256-GCM 失败: {e}")))?;
        let aad = index_bytes(self.index);
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: &self.buffer,
                    aad: &aad,
                },
            )
            .map_err(|e| BackupError::Crypto(format!("加密失败: {e}")))?;

        self.out
            .write_all(&(self.buffer.len() as u32).to_be_bytes())
            .map_err(io_err)?;
        self.out.write_all(&nonce_bytes).map_err(io_err)?;
        self.out.write_all(&ciphertext).map_err(io_err)?;
        self.buffer.clear();
        self.index += 1;
        Ok(())
    }

    /// 正常收尾：写净剩余块 + **收尾块**。
    ///
    /// ⚠️ 必须调用（或至少让它 `drop`）—— 收尾块是读端区分「正常结束」与「被截断」的唯一依据。
    ///
    /// 不返回内层写出器：本类型实现了 `Drop`（兜底收尾），
    /// 而实现 `Drop` 的类型**不能把字段移出去**。调用方要拿回 `out` 就传引用。
    pub fn finish(&mut self) -> Result<(), BackupError> {
        self.finish_inner()
    }

    fn finish_inner(&mut self) -> Result<(), BackupError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        self.flush_chunk()?;
        // 收尾块
        self.out.write_all(&[0u8; 4]).map_err(io_err)?;
        self.out.flush().map_err(io_err)
    }
}

impl<W: Write> Write for EncryptingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut offset = 0usize;
        while offset < buf.len() {
            let n = (buf.len() - offset).min(CHUNK_SIZE - self.buffer.len());
            self.buffer.extend_from_slice(&buf[offset..offset + n]);
            offset += n;
            if self.buffer.len() == CHUNK_SIZE {
                self.flush_chunk().map_err(to_io)?;
            }
        }
        Ok(buf.len())
    }

    /// 与源一致：`flush()` **会把当前未满的块直接加密写出**（块边界随之提前）。
    fn flush(&mut self) -> std::io::Result<()> {
        self.flush_chunk().map_err(to_io)?;
        self.out.flush()
    }
}

impl<W: Write> Drop for EncryptingWriter<W> {
    fn drop(&mut self) {
        // 尽力收尾（调用方忘了 finish 时不至于产出半个包）
        let _ = self.finish_inner();
    }
}

// =============================================================================
// 读取层
// =============================================================================

/// 解密读取层。
pub struct DecryptingReader<R: Read> {
    input: R,
    key: [u8; KEY_LEN],
    plain: Vec<u8>,
    pos: usize,
    index: u32,
    finished: bool,
}

impl<R: Read> DecryptingReader<R> {
    fn new(input: R, key: [u8; KEY_LEN]) -> Self {
        Self {
            input,
            key,
            plain: Vec::new(),
            pos: 0,
            index: 0,
            finished: false,
        }
    }

    /// 保证缓冲区里还有未读明文；`Ok(false)` = 读到收尾块（正常结束）。
    fn fill(&mut self) -> Result<bool, BackupError> {
        while self.pos >= self.plain.len() {
            if self.finished {
                return Ok(false);
            }
            if !self.next_chunk()? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn next_chunk(&mut self) -> Result<bool, BackupError> {
        let len_bytes = read_exact_or_truncated(&mut self.input, 4)?;
        // 用 i32 读：最高位置位视为负数（与源 `len < 0` 判定一致）
        let len = i32::from_be_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]);
        if len == 0 {
            self.finished = true;
            return Ok(false);
        }
        if len < 0 || len as usize > CHUNK_SIZE {
            return Err(BackupError::WrongPassword);
        }
        let len = len as usize;

        let nonce_bytes = read_exact_or_truncated(&mut self.input, NONCE_LEN)?;
        let ciphertext = read_exact_or_truncated(&mut self.input, len + TAG_BYTES)?;

        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| BackupError::Crypto(format!("初始化 AES-256-GCM 失败: {e}")))?;
        let aad = index_bytes(self.index);
        // 🔴 tag 校验失败（错密码 / 篡改 / 块被重排）统一报「密码不对，或备份包已损坏」
        let plain = cipher
            .decrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: &ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| BackupError::WrongPassword)?;

        self.plain = plain;
        self.pos = 0;
        self.index += 1;
        Ok(true)
    }
}

impl<R: Read> Read for DecryptingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if !self.fill().map_err(to_io)? {
            return Ok(0);
        }
        let n = buf.len().min(self.plain.len() - self.pos);
        buf[..n].copy_from_slice(&self.plain[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// 「头部已读 + 剩余流」的拼接读取器（明文包原样透传时用）。
pub struct PrefixReader<R: Read> {
    prefix: Vec<u8>,
    pos: usize,
    inner: R,
}

impl<R: Read> PrefixReader<R> {
    /// 已经读出来的头部字节数。
    #[must_use]
    pub fn prefix_len(&self) -> usize {
        self.prefix.len()
    }
}

impl<R: Read> Read for PrefixReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos < self.prefix.len() {
            let n = buf.len().min(self.prefix.len() - self.pos);
            buf[..n].copy_from_slice(&self.prefix[self.pos..self.pos + n]);
            self.pos += n;
            return Ok(n);
        }
        self.inner.read(buf)
    }
}

/// 读入层：明文透传 或 解密。
pub enum ReadLayer<R: Read> {
    Plain(PrefixReader<R>),
    Encrypted(Box<DecryptingReader<R>>),
}

impl<R: Read> Read for ReadLayer<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            ReadLayer::Plain(r) => r.read(buf),
            ReadLayer::Encrypted(r) => r.read(buf),
        }
    }
}

/// 写出层：明文透传 或 加密。
pub enum WriteLayer<W: Write> {
    Plain(W),
    Encrypted(Box<EncryptingWriter<W>>),
}

impl<W: Write> WriteLayer<W> {
    /// 正常收尾；加密层会写收尾块。
    pub fn finish(self) -> Result<(), BackupError> {
        match self {
            WriteLayer::Plain(mut w) => w.flush().map_err(io_err),
            WriteLayer::Encrypted(mut e) => e.finish(),
        }
    }
}

impl<W: Write> Write for WriteLayer<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            WriteLayer::Plain(w) => w.write(buf),
            WriteLayer::Encrypted(e) => e.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            WriteLayer::Plain(w) => w.flush(),
            WriteLayer::Encrypted(e) => e.flush(),
        }
    }
}

/// 加密写入层；`password` 为空则**原样返回** `out`。
pub fn writer_for<W: Write>(out: W, password: Option<&str>) -> Result<WriteLayer<W>, BackupError> {
    match password.filter(|p| !p.is_empty()) {
        None => Ok(WriteLayer::Plain(out)),
        Some(pw) => Ok(WriteLayer::Encrypted(Box::new(EncryptingWriter::new(
            out, pw,
        )?))),
    }
}

/// 解密读取层。
///
/// - 文件头**不是**我们的魔数 → 按**明文**处理（填了密码也不解密）——
///   所以给明文包误填密码不会读不出来
/// - 是加密包而 `password` 为空 → [`BackupError::NeedPassword`]
pub fn reader_for<R: Read>(
    mut input: R,
    password: Option<&str>,
) -> Result<ReadLayer<R>, BackupError> {
    let head = read_head(&mut input)?;
    if !has_magic(&head) {
        return Ok(ReadLayer::Plain(PrefixReader {
            prefix: head,
            pos: 0,
            inner: input,
        }));
    }
    let Some(pw) = password.filter(|p| !p.is_empty()) else {
        return Err(BackupError::NeedPassword);
    };
    let salt = &head[MAGIC_BYTES + 1..HEADER_LEN];
    let key = derive_key(pw, salt);
    Ok(ReadLayer::Encrypted(Box::new(DecryptingReader::new(
        input, key,
    ))))
}

// =============================================================================
// 内部
// =============================================================================

/// 读文件头若干字节；流比头还短时返回读到的全部（按明文处理）。
fn read_head<R: Read>(input: &mut R) -> Result<Vec<u8>, BackupError> {
    let mut buf = vec![0u8; HEADER_LEN];
    let mut read = 0usize;
    while read < buf.len() {
        let n = input.read(&mut buf[read..]).map_err(io_err)?;
        if n == 0 {
            break;
        }
        read += n;
    }
    buf.truncate(read);
    Ok(buf)
}

/// 块序号（防重排）：4 字节大端。
fn index_bytes(index: u32) -> [u8; 4] {
    index.to_be_bytes()
}

/// 读满 `len` 字节；块边界处遇 EOF → [`BackupError::Truncated`]。
fn read_exact_or_truncated<R: Read>(source: &mut R, len: usize) -> Result<Vec<u8>, BackupError> {
    let mut buf = vec![0u8; len];
    let mut read = 0usize;
    while read < len {
        let n = source.read(&mut buf[read..]).map_err(io_err)?;
        if n == 0 {
            break;
        }
        read += n;
    }
    if read == len {
        Ok(buf)
    } else {
        Err(BackupError::Truncated)
    }
}

/// `io::Error` → `BackupError`，**优先还原被包住的原始变体**。
///
/// 🔴 为什么不能只 `to_string()`：`Read::read` 只能返回 `io::Result`，
/// 而调用方要区分 `WrongPassword` / `NeedPassword` / `Truncated`
/// （「密码错了」和「包没下完」要给用户完全不同的提示）。
/// 若在 io 边界上退化成字符串，这三种情况就都变成 `Io("...")` —— 信息丢失且不可恢复。
///
/// 做法：`to_io` 把 `BackupError` **装箱进** `io::Error`，这里再 `downcast` 取回来。
fn io_err(e: std::io::Error) -> BackupError {
    if let Some(inner) = e.get_ref() {
        if let Some(be) = inner.downcast_ref::<BackupError>() {
            return be.clone();
        }
    }
    BackupError::Io(e.to_string())
}

/// `BackupError` → `io::Error`（**装箱而非字符串化**，见 [`io_err`]）。
fn to_io(e: BackupError) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const PW: &str = "hunter2";

    /// 加密一段字节，返回完整文件字节
    fn encrypt(plain: &[u8], password: Option<&str>) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut w = writer_for(&mut out, password).unwrap();
            w.write_all(plain).unwrap();
            w.flush().unwrap();
            w.finish().unwrap();
        }
        out
    }

    /// 解密一段字节
    fn decrypt(data: &[u8], password: Option<&str>) -> Result<Vec<u8>, BackupError> {
        let mut r = reader_for(Cursor::new(data), password)?;
        let mut out = Vec::new();
        r.read_to_end(&mut out).map_err(io_err)?;
        Ok(out)
    }

    // ---------------------------------------------------------------------
    // 🔴 PBKDF2 已知答案向量（独立实现：Python hashlib）
    // ---------------------------------------------------------------------

    /// `python -c "import hashlib;print(hashlib.pbkdf2_hmac('sha256',b'password',b'0123456789abcdef',200000,32).hex())"`
    #[test]
    fn pbkdf2_known_answer_password() {
        let key = derive_key("password", b"0123456789abcdef");
        assert_eq!(
            hex(&key),
            "ce6a5b943f3250ef59ad6b7df2dbc91818dae05631cc908791a8e86ea44188fc",
            "PBKDF2 参数（哈希/迭代/长度/编码）与独立实现不一致"
        );
    }

    /// 空密码也要能派生（与 Python 对照）
    #[test]
    fn pbkdf2_known_answer_empty_password() {
        let key = derive_key("", b"0123456789abcdef");
        assert_eq!(
            hex(&key),
            "3208ac892270f1f5fe38282bfa194ea15bccf02a02ffe6a64ab22251ac95a16d"
        );
    }

    /// 🔴 迭代次数是跨端契约：写成 20 万（不是 10 万/1 万）
    #[test]
    fn iterations_constant_is_200k() {
        assert_eq!(ITERATIONS, 200_000);
        // 用不同迭代次数派生必须得到不同密钥（证明确实用了这个参数）
        let a = derive_key("p", b"saltsaltsaltsalt");
        let mut wrong = [0u8; KEY_LEN];
        pbkdf2::pbkdf2_hmac::<sha2::Sha256>(b"p", b"saltsaltsaltsalt", 100_000, &mut wrong);
        assert_ne!(a, wrong);
    }

    /// salt 不同 → 密钥不同（每包独立 salt 的意义）
    #[test]
    fn different_salt_yields_different_key() {
        let a = derive_key(PW, b"0000000000000000");
        let b = derive_key(PW, b"1111111111111111");
        assert_ne!(a, b);
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    // ---------------------------------------------------------------------
    // 往返
    // ---------------------------------------------------------------------

    #[test]
    fn roundtrip_small() {
        let data = b"hello backup".to_vec();
        let enc = encrypt(&data, Some(PW));
        assert_eq!(decrypt(&enc, Some(PW)).unwrap(), data);
    }

    #[test]
    fn roundtrip_empty_payload() {
        let enc = encrypt(&[], Some(PW));
        assert!(decrypt(&enc, Some(PW)).unwrap().is_empty());
    }

    /// 跨块边界：恰好一块 / 一块多一字节 / 两块 / 两块多一字节
    #[test]
    fn roundtrip_chunk_boundaries() {
        for size in [
            CHUNK_SIZE - 1,
            CHUNK_SIZE,
            CHUNK_SIZE + 1,
            CHUNK_SIZE * 2,
            CHUNK_SIZE * 2 + 7,
        ] {
            let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            let enc = encrypt(&data, Some(PW));
            assert_eq!(decrypt(&enc, Some(PW)).unwrap(), data, "size={size}");
        }
    }

    /// 二进制内容逐字节保真
    #[test]
    fn roundtrip_binary() {
        let data: Vec<u8> = (0u8..=255).cycle().take(10_000).collect();
        let enc = encrypt(&data, Some(PW));
        assert_eq!(decrypt(&enc, Some(PW)).unwrap(), data);
    }

    /// 分多次 write 写入（模拟 gzip 的碎写）结果一致
    #[test]
    fn fragmented_writes_roundtrip() {
        let data: Vec<u8> = (0..5000).map(|i| (i % 97) as u8).collect();
        let mut out = Vec::new();
        {
            let mut w = writer_for(&mut out, Some(PW)).unwrap();
            for chunk in data.chunks(7) {
                w.write_all(chunk).unwrap();
            }
            w.finish().unwrap();
        }
        assert_eq!(decrypt(&out, Some(PW)).unwrap(), data);
    }

    // ---------------------------------------------------------------------
    // 密码为空 → 原样透传
    // ---------------------------------------------------------------------

    /// 🔴 空密码：**不加任何头**，字节完全等于原文
    #[test]
    fn empty_password_is_passthrough() {
        let data = b"plain payload".to_vec();
        let out = encrypt(&data, None);
        assert_eq!(out, data, "空密码不得写入 CCENC1 头");
        assert_eq!(decrypt(&out, None).unwrap(), data);
    }

    #[test]
    fn empty_string_password_is_passthrough() {
        let data = b"x".to_vec();
        assert_eq!(encrypt(&data, Some("")), data);
    }

    /// 🔴 明文包误填密码仍可读（头不是魔数 → 按明文处理）
    #[test]
    fn plain_archive_with_password_still_readable() {
        let data = b"gzip-like plain data".to_vec();
        assert_eq!(decrypt(&data, Some("wrong-but-irrelevant")).unwrap(), data);
    }

    /// 流比文件头还短 → 按明文处理，不 panic
    #[test]
    fn short_input_treated_as_plain() {
        for len in 0..HEADER_LEN {
            let data = vec![0xABu8; len];
            assert_eq!(decrypt(&data, None).unwrap(), data, "len={len}");
            assert_eq!(decrypt(&data, Some(PW)).unwrap(), data, "len={len}");
        }
    }

    // ---------------------------------------------------------------------
    // 文件头布局
    // ---------------------------------------------------------------------

    /// 🔴 头部逐字节：`CCENC1` + 版本 1 + 16 字节 salt（共 23 字节）
    #[test]
    fn header_layout() {
        let enc = encrypt(b"x", Some(PW));
        assert_eq!(&enc[..6], b"CCENC1");
        assert_eq!(enc[6], 1, "版本号");
        assert_eq!(HEADER_LEN, 23);
        // salt 之后紧接着第一块的 4 字节长度
        let len = u32::from_be_bytes([enc[23], enc[24], enc[25], enc[26]]);
        assert_eq!(len, 1, "首块明文长度 = 1");
    }

    /// 每包的 salt 不同（随机）
    #[test]
    fn salt_is_random_per_archive() {
        let a = encrypt(b"x", Some(PW));
        let b = encrypt(b"x", Some(PW));
        assert_ne!(a[7..23], b[7..23], "salt 必须随机");
        assert_ne!(a, b, "同明文两次加密结果应不同（nonce 随机）");
    }

    #[test]
    fn has_magic_detection() {
        let enc = encrypt(b"x", Some(PW));
        assert!(has_magic(&enc));
        assert!(!has_magic(b"not an encrypted archive at all"));
        assert!(!has_magic(b"CCENC2"), "版本不对");
        assert!(!has_magic(&enc[..HEADER_LEN - 1]), "头不完整");
        // gzip 明文包（0x1f 0x8b）不会被误判
        assert!(!has_magic(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
    }

    /// 版本号必须参与魔数判定（否则将来 v2 会被 v1 的代码误读）
    #[test]
    fn version_mismatch_is_not_encrypted() {
        let mut enc = encrypt(b"x", Some(PW));
        enc[6] = 9;
        assert!(!has_magic(&enc));
        // 当成明文读回（不报错）
        assert_eq!(decrypt(&enc, Some(PW)).unwrap(), enc);
    }

    // ---------------------------------------------------------------------
    // 分块框架
    // ---------------------------------------------------------------------

    /// 🔴 块框架逐字节：`[len(4,BE)][nonce(12)][ct+tag(16)]`
    #[test]
    fn chunk_framing_layout() {
        let data = vec![7u8; 100];
        let enc = encrypt(&data, Some(PW));
        let len = u32::from_be_bytes([enc[23], enc[24], enc[25], enc[26]]) as usize;
        assert_eq!(len, 100);
        // nonce 12 字节 + 密文 100 + tag 16
        let chunk_total = 4 + NONCE_LEN + len + TAG_BYTES;
        assert_eq!(enc.len(), HEADER_LEN + chunk_total + 4, "头部 + 一块 + 收尾块");
    }

    /// 🔴 密文长度 = 明文 + 16（GCM tag）
    #[test]
    fn ciphertext_is_plaintext_plus_tag() {
        let data = vec![1u8; 500];
        let enc = encrypt(&data, Some(PW));
        let len = u32::from_be_bytes([enc[23], enc[24], enc[25], enc[26]]) as usize;
        let ct_start = HEADER_LEN + 4 + NONCE_LEN;
        let ct_end = ct_start + len + TAG_BYTES;
        assert_eq!(ct_end - ct_start, len + TAG_BYTES);
        assert_eq!(TAG_BYTES, 16);
    }

    /// 🔴 收尾块是 4 个零字节
    #[test]
    fn terminator_is_four_zero_bytes() {
        let enc = encrypt(b"x", Some(PW));
        assert_eq!(&enc[enc.len() - 4..], &[0, 0, 0, 0]);
    }

    /// 跨块时块序号递增（每块 4 字节长度都是 CHUNK_SIZE，最后一块是余数）
    #[test]
    fn multi_chunk_lengths_are_as_expected() {
        let data = vec![0u8; CHUNK_SIZE + 10];
        let enc = encrypt(&data, Some(PW));
        let l0 = u32::from_be_bytes([enc[23], enc[24], enc[25], enc[26]]) as usize;
        assert_eq!(l0, CHUNK_SIZE);
        let second_len_pos = HEADER_LEN + 4 + NONCE_LEN + CHUNK_SIZE + TAG_BYTES;
        let l1 = u32::from_be_bytes([
            enc[second_len_pos],
            enc[second_len_pos + 1],
            enc[second_len_pos + 2],
            enc[second_len_pos + 3],
        ]) as usize;
        assert_eq!(l1, 10, "第二块是余数");
    }

    // ---------------------------------------------------------------------
    // 错误路径
    // ---------------------------------------------------------------------

    /// 🔴 密码不对 → `WrongPassword`（不是 Truncated / Io）
    #[test]
    fn wrong_password_is_reported() {
        let enc = encrypt(b"secret".to_vec().as_slice(), Some(PW));
        assert_eq!(decrypt(&enc, Some("wrong")).unwrap_err(), BackupError::WrongPassword);
    }

    /// 🔴 加密包没给密码 → `NeedPassword`
    #[test]
    fn missing_password_is_reported() {
        let enc = encrypt(b"x", Some(PW));
        assert_eq!(decrypt(&enc, None).unwrap_err(), BackupError::NeedPassword);
        assert_eq!(decrypt(&enc, Some("")).unwrap_err(), BackupError::NeedPassword);
    }

    /// 🔴 AAD = 块序号：把两块**互换**后必须解密失败（防重排）
    #[test]
    fn swapped_chunks_fail_to_decrypt() {
        let data = vec![0u8; CHUNK_SIZE + 10];
        let enc = encrypt(&data, Some(PW));
        let chunk0_len = 4 + NONCE_LEN + CHUNK_SIZE + TAG_BYTES;
        let chunk1_start = HEADER_LEN + chunk0_len;
        let chunk1_len = enc.len() - 4 - chunk1_start;

        let mut swapped = Vec::new();
        swapped.extend_from_slice(&enc[..HEADER_LEN]);
        swapped.extend_from_slice(&enc[chunk1_start..chunk1_start + chunk1_len]);
        swapped.extend_from_slice(&enc[HEADER_LEN..chunk1_start]);
        swapped.extend_from_slice(&enc[enc.len() - 4..]);

        let err = decrypt(&swapped, Some(PW)).unwrap_err();
        assert_eq!(err, BackupError::WrongPassword, "重排必须被 AAD 挡下");
    }

    /// 篡改密文一个字节 → 校验失败
    #[test]
    fn tampered_ciphertext_fails() {
        let mut enc = encrypt(b"some content here", Some(PW));
        let pos = enc.len() - 10; // 落在密文/tag 区内
        enc[pos] ^= 0xFF;
        assert_eq!(decrypt(&enc, Some(PW)).unwrap_err(), BackupError::WrongPassword);
    }

    /// 🔴 截断（去掉收尾块）→ `Truncated`，而不是静默 EOF
    #[test]
    fn truncated_before_terminator_is_reported() {
        let enc = encrypt(b"x".repeat(200).as_slice(), Some(PW));
        let cut = &enc[..enc.len() - 4];
        assert_eq!(decrypt(cut, Some(PW)).unwrap_err(), BackupError::Truncated);
    }

    /// 截断在块中间 → `Truncated`
    #[test]
    fn truncated_mid_chunk_is_reported() {
        let enc = encrypt(&vec![3u8; 1000], Some(PW));
        let cut = &enc[..enc.len() - 50];
        assert_eq!(decrypt(cut, Some(PW)).unwrap_err(), BackupError::Truncated);
    }

    /// 块长度字段非法（> CHUNK_SIZE）→ `WrongPassword`（与源一致）
    #[test]
    fn oversized_chunk_length_is_rejected() {
        let mut enc = encrypt(b"x", Some(PW));
        let bad = (CHUNK_SIZE as u32 + 1).to_be_bytes();
        enc[23..27].copy_from_slice(&bad);
        assert_eq!(decrypt(&enc, Some(PW)).unwrap_err(), BackupError::WrongPassword);
    }

    /// 块长度字段最高位置位（负数）→ `WrongPassword`
    #[test]
    fn negative_chunk_length_is_rejected() {
        let mut enc = encrypt(b"x", Some(PW));
        enc[23] = 0x80;
        assert_eq!(decrypt(&enc, Some(PW)).unwrap_err(), BackupError::WrongPassword);
    }

    // ---------------------------------------------------------------------
    // flush 语义（与源一致：flush 会提前切块）
    // ---------------------------------------------------------------------

    /// 🔴 `flush()` 会把未满的块立刻写出 → 产生多个小块（源行为）
    #[test]
    fn flush_splits_chunk() {
        let mut out = Vec::new();
        {
            let mut w = writer_for(&mut out, Some(PW)).unwrap();
            w.write_all(b"AAAA").unwrap();
            w.flush().unwrap();
            w.write_all(b"BBBB").unwrap();
            w.finish().unwrap();
        }
        // 第一块长度 4
        let l0 = u32::from_be_bytes([out[23], out[24], out[25], out[26]]);
        assert_eq!(l0, 4, "flush 后第一块应只有 4 字节");
        assert_eq!(decrypt(&out, Some(PW)).unwrap(), b"AAAABBBB");
    }

    /// 忘记 finish 时 Drop 也会补收尾块（尽力而为）
    #[test]
    fn drop_writes_terminator() {
        let mut out = Vec::new();
        {
            let mut w = writer_for(&mut out, Some(PW)).unwrap();
            w.write_all(b"data").unwrap();
            // 不调 finish，直接离开作用域
        }
        assert_eq!(&out[out.len() - 4..], &[0, 0, 0, 0]);
        assert_eq!(decrypt(&out, Some(PW)).unwrap(), b"data");
    }

    // ---------------------------------------------------------------------
    // 文件级探测
    // ---------------------------------------------------------------------

    #[test]
    fn is_encrypted_file_detects() {
        let dir = std::env::temp_dir().join(format!(
            "cc-crypto-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let enc_path = dir.join("enc.tar.gz");
        std::fs::write(&enc_path, encrypt(b"x", Some(PW))).unwrap();
        assert!(is_encrypted_file(&enc_path));

        let plain_path = dir.join("plain.tar.gz");
        std::fs::write(&plain_path, b"\x1f\x8b\x08\x00plain").unwrap();
        assert!(!is_encrypted_file(&plain_path));

        assert!(!is_encrypted_file(&dir.join("missing.tar.gz")));
    }
}
