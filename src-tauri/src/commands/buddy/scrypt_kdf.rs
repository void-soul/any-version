//! scrypt（RFC 7914）最小实现，供 WorkDaddy 加密导出包派生密钥。
//!
//! **为什么不用 `scrypt` crate**：本机到 crates.io 及其镜像的 TLS 握手被 Windows schannel 的
//! 吊销检查阻断（`CRYPT_E_REVOCATION_OFFLINE`），新增依赖会破坏「拉下代码即可构建」。
//! 这里只实现 WorkDaddy 需要的形态（PBKDF2-HMAC-SHA256 + ROMix + Salsa20/8），
//! 复用工程里已有的 `pbkdf2` / `sha2`。
//!
//! 正确性由 `tests` 中的 Node 原生 `crypto.scryptSync` 已知答案向量保证
//! （向量与 RFC 7914 §12 官方测试向量一致）。该模块只做**解密**，不用于生成新凭证。

use pbkdf2::pbkdf2_hmac;
use sha2::Sha256;

/// scrypt 派生密钥。`log_n` 为 log2(N)（Node `crypto.scryptSync` 默认 N=16384 → 14）。
pub fn derive(
    password: &[u8],
    salt: &[u8],
    log_n: u8,
    r: u32,
    p: u32,
    out: &mut [u8],
) -> Result<(), String> {
    if out.is_empty() {
        return Err("scrypt 输出长度不能为 0".to_string());
    }
    if !(1..=20).contains(&log_n) {
        return Err(format!("scrypt 参数 N 超出支持范围 (log2(N)={})", log_n));
    }
    if r == 0 || p == 0 {
        return Err("scrypt 参数 r/p 必须大于 0".to_string());
    }
    let n: usize = 1usize << log_n;
    let r = r as usize;
    let p = p as usize;
    let block_len = 128 * r;

    // B = PBKDF2-HMAC-SHA256(P, S, 1, p * 128 * r)
    let mut b = vec![0u8; p * block_len];
    pbkdf2_hmac::<Sha256>(password, salt, 1, &mut b);
    for chunk in b.chunks_mut(block_len) {
        romix(chunk, n, r);
    }
    // DK = PBKDF2-HMAC-SHA256(P, B, 1, dkLen)
    pbkdf2_hmac::<Sha256>(password, &b, 1, out);
    Ok(())
}

/// ROMix（RFC 7914 §5）：顺序填充 V 表，再随机索引回写。
fn romix(block: &mut [u8], n: usize, r: usize) {
    let len = block.len();
    let mut x = block.to_vec();
    let mut v = vec![0u8; n * len];
    let mut tmp = vec![0u8; len];

    for i in 0..n {
        v[i * len..(i + 1) * len].copy_from_slice(&x);
        block_mix(&mut x, &mut tmp, r);
    }
    for _ in 0..n {
        let j = integerify(&x, r) % (n as u64);
        let vj = &v[j as usize * len..(j as usize + 1) * len];
        for k in 0..len {
            x[k] ^= vj[k];
        }
        block_mix(&mut x, &mut tmp, r);
    }
    block.copy_from_slice(&x);
}

/// `Integerify(X)` = 最后一个 64 字节块的前 8 字节按小端解释。
fn integerify(x: &[u8], r: usize) -> u64 {
    let offset = (2 * r - 1) * 64;
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&x[offset..offset + 8]);
    u64::from_le_bytes(buf)
}

/// `BlockMix`（RFC 7914 §4）：2r 个 64 字节块经 Salsa20/8 串联后奇偶交错重排。
fn block_mix(block: &mut [u8], tmp: &mut [u8], r: usize) {
    let mut x = [0u8; 64];
    x.copy_from_slice(&block[(2 * r - 1) * 64..2 * r * 64]);

    for i in 0..2 * r {
        let b_i = &block[i * 64..(i + 1) * 64];
        for k in 0..64 {
            x[k] ^= b_i[k];
        }
        salsa20_8(&mut x);
        tmp[i * 64..(i + 1) * 64].copy_from_slice(&x);
    }
    // B' = (Y0, Y2, …, Y_{2r-2}, Y1, Y3, …, Y_{2r-1})
    for i in 0..r {
        block[i * 64..(i + 1) * 64].copy_from_slice(&tmp[2 * i * 64..(2 * i + 1) * 64]);
    }
    for i in 0..r {
        block[(r + i) * 64..(r + i + 1) * 64]
            .copy_from_slice(&tmp[(2 * i + 1) * 64..(2 * i + 2) * 64]);
    }
}

/// Salsa20/8 核心（8 轮 = 4 个 double round），原地作用于 64 字节块。
fn salsa20_8(block: &mut [u8]) {
    let mut x = [0u32; 16];
    for (i, word) in x.iter_mut().enumerate() {
        *word = u32::from_le_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    let z = x;

    for _ in 0..4 {
        // column rounds
        x[4] ^= x[0].wrapping_add(x[12]).rotate_left(7);
        x[8] ^= x[4].wrapping_add(x[0]).rotate_left(9);
        x[12] ^= x[8].wrapping_add(x[4]).rotate_left(13);
        x[0] ^= x[12].wrapping_add(x[8]).rotate_left(18);
        x[9] ^= x[5].wrapping_add(x[1]).rotate_left(7);
        x[13] ^= x[9].wrapping_add(x[5]).rotate_left(9);
        x[1] ^= x[13].wrapping_add(x[9]).rotate_left(13);
        x[5] ^= x[1].wrapping_add(x[13]).rotate_left(18);
        x[14] ^= x[10].wrapping_add(x[6]).rotate_left(7);
        x[2] ^= x[14].wrapping_add(x[10]).rotate_left(9);
        x[6] ^= x[2].wrapping_add(x[14]).rotate_left(13);
        x[10] ^= x[6].wrapping_add(x[2]).rotate_left(18);
        x[3] ^= x[15].wrapping_add(x[11]).rotate_left(7);
        x[7] ^= x[3].wrapping_add(x[15]).rotate_left(9);
        x[11] ^= x[7].wrapping_add(x[3]).rotate_left(13);
        x[15] ^= x[11].wrapping_add(x[7]).rotate_left(18);
        // row rounds
        x[1] ^= x[0].wrapping_add(x[3]).rotate_left(7);
        x[2] ^= x[1].wrapping_add(x[0]).rotate_left(9);
        x[3] ^= x[2].wrapping_add(x[1]).rotate_left(13);
        x[0] ^= x[3].wrapping_add(x[2]).rotate_left(18);
        x[6] ^= x[5].wrapping_add(x[4]).rotate_left(7);
        x[7] ^= x[6].wrapping_add(x[5]).rotate_left(9);
        x[4] ^= x[7].wrapping_add(x[6]).rotate_left(13);
        x[5] ^= x[4].wrapping_add(x[7]).rotate_left(18);
        x[11] ^= x[10].wrapping_add(x[9]).rotate_left(7);
        x[8] ^= x[11].wrapping_add(x[10]).rotate_left(9);
        x[9] ^= x[8].wrapping_add(x[11]).rotate_left(13);
        x[10] ^= x[9].wrapping_add(x[8]).rotate_left(18);
        x[12] ^= x[15].wrapping_add(x[14]).rotate_left(7);
        x[13] ^= x[12].wrapping_add(x[15]).rotate_left(9);
        x[14] ^= x[13].wrapping_add(x[12]).rotate_left(13);
        x[15] ^= x[14].wrapping_add(x[13]).rotate_left(18);
    }

    for i in 0..16 {
        let value = x[i].wrapping_add(z[i]);
        block[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// 已知答案向量：由 Node 原生 `crypto.scryptSync` 生成，与 RFC 7914 §12 一致。
    #[test]
    fn matches_node_native_scrypt() {
        let cases: [( &str, &str, u8, u32, u32, &str ); 3] = [
            (
                "",
                "",
                4, // N = 16
                1,
                1,
                "77d6576238657b203b19ca42c18a0497f16b4844e3074ae8dfdffa3fede21442fcd0069ded0948f8326a753a0fc81f17e8d3e0fb2e0d3628cf35e20c38d18906",
            ),
            (
                "password",
                "NaCl",
                10, // N = 1024
                8,
                16,
                "fdbabe1c9d3472007856e7190d01e9fe7c6ad7cbc8237830e77376634b3731622eaf30d92e22a3886ff109279d9830dac727afb94a83ee6d8360cbdfa2cc0640",
            ),
            (
                "pleaseletmein",
                "SodiumChloride",
                14, // N = 16384（Node scryptSync 默认）
                8,
                1,
                "7023bdcb3afd7348461c06cd81fd38ebfda8fbba904f8e3ea9b543f6545da1f2d5432955613f0fcf62d49705242a9af9e61e85dc0d651e40dfcf017b45575887",
            ),
        ];
        for (password, salt, log_n, r, p, expected) in cases {
            let mut out = [0u8; 64];
            derive(password.as_bytes(), salt.as_bytes(), log_n, r, p, &mut out).unwrap();
            assert_eq!(hex(&out), expected, "params: log_n={} r={} p={}", log_n, r, p);
        }
    }

    #[test]
    fn rejects_insane_parameters() {
        let mut out = [0u8; 32];
        assert!(derive(b"pw", b"salt", 0, 8, 1, &mut out).is_err());
        assert!(derive(b"pw", b"salt", 32, 8, 1, &mut out).is_err());
        assert!(derive(b"pw", b"salt", 14, 0, 1, &mut out).is_err());
        assert!(derive(b"pw", b"salt", 14, 8, 0, &mut out).is_err());
    }
}
