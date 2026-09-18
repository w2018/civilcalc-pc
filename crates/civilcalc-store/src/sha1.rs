//! SHA-1（**自实现**）—— 仅用于图片内容寻址。
//!
//! ## 为什么不用 `sha1` crate
//!
//! 本机离线：`sha1` 的**索引条目不在本地 registry 缓存**里（只有 `.crate` 文件），
//! `cargo` 解析依赖时直接报 `no matching package named 'sha1' found`。
//! 换 `sha2` 虽然可用，但会让 PC 端算出的文件名与 Android 端**不一致** ——
//! 图片文件名是 `"<sha1>.<ext>"`，两端算法不同会导致同一张图在两端各存一份
//! （跨端去重失效），且备份包里的文件名语义漂移。
//!
//! 因此自实现：算法短且完全确定（RFC 3174），有标准测试向量可钉。
//!
//! ## ⚠️ 这不是密码学用途
//!
//! SHA-1 早已**不适合**签名/口令等安全场景。这里只做「同样字节 → 同样文件名」的
//! 内容寻址，且输入是用户自己上传的图片，**不构成安全边界**。
//! 需要安全哈希时用 `sha2`（工作区已依赖）。

/// 计算 SHA-1 摘要（20 字节）。
#[must_use]
pub fn sha1(data: &[u8]) -> [u8; 20] {
    // 初始链接变量（FIPS 180-1 / RFC 3174）
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];

    // ---- 填充：0x80 + 若干个 0，直到长度 ≡ 56 (mod 64)，再补 8 字节大端比特长度 ----
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = Vec::with_capacity(data.len() + 72);
    msg.extend_from_slice(data);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    // ---- 逐 512 位块处理 ----
    //
    // 用 `as_chunks` 而不是 `chunks_exact`（clippy `chunks_exact_to_as_chunks`）：
    // 填充后长度**必是 64 的整数倍**，两者语义等价（余数都为空），
    // 但 `as_chunks` 给出定长数组引用，按 4 字节分组时可直接 `from_be_bytes(*chunk)`，
    // 省掉手工索引与越界顾虑。
    for block in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for (i, chunk) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*chunk);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    let mut out = [0u8; 20];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

/// 小写十六进制摘要（40 字符）—— 与 Kotlin `"%02x".format(it)` 输出一致。
#[must_use]
pub fn sha1_hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(40);
    for b in sha1(data) {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 标准测试向量（FIPS 180-1 / RFC 3174 附录）
    #[test]
    fn rfc3174_vectors() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            sha1_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        assert_eq!(
            sha1_hex(b"The quick brown fox jumps over the lazy dog"),
            "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12"
        );
    }

    /// 100 万个 `a`（RFC 3174 的经典向量，覆盖多块循环）
    #[test]
    fn one_million_a() {
        let data = vec![b'a'; 1_000_000];
        assert_eq!(sha1_hex(&data), "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
    }

    /// 🔴 填充边界逐条对照真实值（55/56/63/64/65/119/120 字节）
    ///
    /// 55→56 要额外补**一整块**（长度跨过 `≡56 mod 64` 的临界），64→65 同理。
    /// 这些是最容易写错填充的地方，所以用显式向量而不是「长度不同则摘要不同」这种弱断言。
    ///
    /// 期望值由 `python -c "hashlib.sha1(b'x'*N).hexdigest()"` 生成。
    #[test]
    fn padding_boundaries_match_known_values() {
        let cases: [(usize, &str); 7] = [
            (55, "cef734ba81a024479e09eb5a75b6ddae62e6abf1"),
            (56, "901305367c259952f4e7af8323f480d59f81335b"),
            (63, "0ddc4e0cccd9a12850deb5abb0853a4425559fec"),
            (64, "bb2fa3ee7afb9f54c6dfb5d021f14b1ffe40c163"),
            (65, "78c741ddc482e4cdf8c474a0876347a0905b6233"),
            (119, "4300320394f7ee239bcdce7d3b8bcee173a0cd5c"),
            (120, "ceb2821639c4b6dcb10bce0e522ca2e608ce056d"),
        ];
        for (n, expect) in cases {
            assert_eq!(sha1_hex(&vec![b'x'; n]), expect, "{n} 字节填充错误");
        }
    }

    /// 恰好 64 字节（单块）的已知值
    #[test]
    fn known_64_byte_vector() {
        // python -c "import hashlib;print(hashlib.sha1(b'0123456789abcdef'*4).hexdigest())"
        assert_eq!(
            sha1_hex(b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
            "ce4303f6b22257d9c9cf314ef1dee4707c6e1c13"
        );
    }

    /// 摘要长度与十六进制长度
    #[test]
    fn digest_shape() {
        assert_eq!(sha1(b"abc").len(), 20);
        assert_eq!(sha1_hex(b"abc").len(), 40);
        assert!(sha1_hex(b"abc").bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(sha1_hex(b"abc")
            .bytes()
            .all(|b| !b.is_ascii_uppercase()), "必须小写（与 Kotlin 一致）");
    }

    /// 二进制（非 UTF-8）输入不 panic
    #[test]
    fn binary_input_is_fine() {
        let data: Vec<u8> = (0u8..=255).collect();
        assert_eq!(sha1_hex(&data).len(), 40);
        assert_ne!(sha1_hex(&data), sha1_hex(&[]));
    }
}
