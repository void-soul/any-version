//! FinalShell 连接密码的加解密（方案 B）。
//!
//! 算法由本机 `finalshell.jar` 反汇编 + 运行时 oracle 实证（2026-11-11），不依赖 Java 运行时：
//! - `xk.b(String)` 加密 / `xk.a(String)` 解密；常量 `static int a = 8`（salt 长度）、
//!   `String[] b = [DES, utf-8, utf-8, DES]`、`yN.H = 0x33157bf662ec9de1`。
//! - 密钥：`xk.a(byte[])` 用 Java `Random` 从 salt 生成 8 个 long（大端 64 字节）后取 MD5，
//!   `DESKeySpec` 取其前 8 字节 → **DES/ECB/PKCS5Padding**（`Cipher.getInstance("DES")`）。
//! - 密文 = Base64 标准编码（**带 `=` 填充**）的 `salt(8B) || ciphertext`。
//!   本机 14 个样本恰为 24 字节才没有 `=`，不是「无填充」。
//!
//! 明文一律 UTF-8。绝不在此模块读取或落盘任何真实连接文件。
use base64::{engine::general_purpose, Engine as _};
use des::Des;
use ecb::cipher::block_padding::Pkcs7;
use ecb::cipher::generic_array::GenericArray;
use ecb::cipher::{BlockDecryptMut, BlockEncryptMut, KeyInit};

#[cfg(test)]
#[path = "crypto_tests.rs"]
mod tests;

const SALT_LEN: usize = 8;
/// FinalShell `yN.H`：派生时参与的固定常量（反汇编 + 反射读取得出）。
const DERIVE_SEED: i64 = 0x3315_7bf6_62ec_9de1;

type DesEcbEnc = ecb::Encryptor<Des>;
type DesEcbDec = ecb::Decryptor<Des>;

/// Java `java.util.Random` 的精确复刻（48 位 LCG）。密钥派生依赖它的字节级行为。
struct JavaRandom {
    seed: i64,
}

impl JavaRandom {
    fn new(seed: i64) -> Self {
        Self {
            seed: (seed ^ 0x5DEE_CE66_D) & ((1_i64 << 48) - 1),
        }
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.seed = (self.seed
            .wrapping_mul(0x5DEE_CE66_D)
            .wrapping_add(0xB))
            & ((1_i64 << 48) - 1);
        (self.seed >> (48 - bits)) as i32
    }

    /// `nextInt(bound)`；派生里只用 bound=127（非 2 的幂，走取模重试分支）。
    fn next_int(&mut self, bound: i32) -> i32 {
        debug_assert!(bound > 0);
        if (bound & -bound) == bound {
            return ((i64::from(bound) * i64::from(self.next(31))) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let val = bits % bound;
            if bits - val + (bound - 1) >= 0 {
                return val;
            }
        }
    }

    /// `nextLong()` = `(next(32) << 32) + next(32)`，低位 int 按 Java 语义符号扩展。
    fn next_long(&mut self) -> i64 {
        let high = i64::from(self.next(32)) << 32;
        let low = i64::from(self.next(32) as i32);
        high + low
    }
}

/// 复刻 `xk.a(byte[])`：由 8 字节 salt 确定性地派生 16 字节 DES 密钥材料。
///
/// `uR.f()` 在运行时返回非 null，因此循环按 `salt[0]` 次消费 `rnd1`（负数时 0 次）。
pub fn derive_key(s: &[u8; SALT_LEN]) -> Result<[u8; 16], String> {
    let mut rnd0 = JavaRandom::new(i64::from(s[5] as i8));
    let divisor = rnd0.next_int(127);
    // Java 侧这里同样会抛 ArithmeticException：该 salt 无法派生密钥，必须判为不可用。
    // 加密路径会换一个 salt 重试；解密路径返回错误，绝不 panic。
    let divisor = i64::from(divisor);
    // Java 侧这里同样会抛 ArithmeticException：该 salt 无法派生密钥，必须判为不可用。
    // 加密路径会换一个 salt 重试；解密路径返回错误，绝不 panic。
    if divisor == 0 {
        return Err("该 salt 无法派生密钥（Java Random 返回 0）".to_string());
    }
    let quotient = DERIVE_SEED / divisor;
    let mut rnd1 = JavaRandom::new(quotient);
    for _ in 0..i32::from(s[0] as i8).max(0) {
        let _ = rnd1.next_long();
    }
    let mut rnd2 = JavaRandom::new(rnd1.next_long());
    let longs: [i64; 8] = [
        i64::from(s[4] as i8),
        rnd2.next_long(),
        i64::from(s[7] as i8),
        i64::from(s[3] as i8),
        rnd2.next_long(),
        i64::from(s[1] as i8),
        rnd1.next_long(),
        i64::from(s[2] as i8),
    ];
    let mut buf = [0u8; 64];
    for (chunk, value) in buf.chunks_exact_mut(8).zip(longs) {
        chunk.copy_from_slice(&value.to_be_bytes());
    }
    Ok(md5::compute(buf).0)
}

fn des_key(
    salt: &[u8; SALT_LEN],
) -> Result<GenericArray<u8, <Des as ecb::cipher::KeySizeUser>::KeySize>, String> {
    let derived = derive_key(salt)?;
    Ok(GenericArray::clone_from_slice(&derived[..8]))
}

/// 判断是否为可解密的 FinalShell 密文：标准 Base64，解码后 ≥ 16 字节且 `(len-8)` 是 8 的倍数。
pub fn is_finalshell_cipher(s: &str) -> bool {
    if s.is_empty() || s.len() % 4 != 0 {
        return false;
    }
    let Ok(blob) = general_purpose::STANDARD.decode(s) else {
        return false;
    };
    blob.len() > SALT_LEN && (blob.len() - SALT_LEN) % 8 == 0
}

fn decode_blob(cipher: &str) -> Result<Vec<u8>, String> {
    if !is_finalshell_cipher(cipher) {
        return Err("FinalShell 密码密文形态不符（应为标准 Base64，长度 ≥16 且密文为 8 的倍数）".into());
    }
    general_purpose::STANDARD
        .decode(cipher)
        .map_err(|e| format!("FinalShell 密码 Base64 解码失败: {e}"))
}

/// 解密 FinalShell `password` 字段为明文。
pub fn decrypt(cipher: &str) -> Result<String, String> {
    let blob = decode_blob(cipher)?;
    let salt: [u8; SALT_LEN] = blob[..SALT_LEN].try_into().expect("长度已校验");
    let key = des_key(&salt)?;
    let plain = DesEcbDec::new(&key)
        .decrypt_padded_vec_mut::<Pkcs7>(&blob[SALT_LEN..])
        .map_err(|_| "FinalShell 密码解密失败（填充或密钥不匹配）".to_string())?;
    String::from_utf8(plain).map_err(|e| format!("FinalShell 密码不是有效 UTF-8: {e}"))
}

/// 用给定 salt 加密明文（测试与确定性复现用）。
pub fn encrypt_with_salt(plain: &str, salt: &[u8; SALT_LEN]) -> Result<String, String> {
    let key = des_key(salt)?;
    let mut ct = DesEcbEnc::new(&key).encrypt_padded_vec_mut::<Pkcs7>(plain.as_bytes());
    let mut blob = Vec::with_capacity(SALT_LEN + ct.len());
    blob.extend_from_slice(salt);
    blob.append(&mut ct);
    Ok(general_purpose::STANDARD.encode(blob))
}

/// 用随机 salt 加密明文，输出可被 FinalShell 直接读取的 `password` 字段。
///
/// 少量 salt 会让 Java `Random.nextInt(127)` 返回 0（Java 侧会抛异常、无法生成密文），
/// 这里换一个 salt 重试，用户无感。
pub fn encrypt(plain: &str) -> Result<String, String> {
    for _ in 0..16 {
        let mut salt = [0u8; SALT_LEN];
        getrandom::getrandom(&mut salt).map_err(|e| format!("生成 salt 失败: {e}"))?;
        match encrypt_with_salt(plain, &salt) {
            Ok(cipher) => return Ok(cipher),
            Err(_) => continue,
        }
    }
    Err("连续多次生成的 salt 都不可用，请重试".to_string())
}
