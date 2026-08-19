//! sqleet / SQLite3MultipleCiphers (chacha20 非 legacy 模式) 加密数据库解密。
//!
//! Dawn Launcher 的 `Data.db` 使用 `better-sqlite3-multiple-ciphers` 以
//! chacha20 加密（SQLite3MultipleCiphers 的默认 cipher，非 legacy 模式），
//! 密钥为其内置 addon 中固定的 UUID。普通 rusqlite 无法直接打开，
//! 这里按官方实现逐页解密为明文 SQLite 文件后再读取。
//!
//! 页面格式（来自 SQLite3MultipleCiphers cipher_chacha20.c）：
//! - 页大小 4096，每页保留 32 字节（16 随机 nonce + 16 Poly1305 tag）
//! - 密钥 = PBKDF2-HMAC-SHA256(passphrase, salt=文件头16字节, 64007 轮, 32 字节)
//! - 每页：counter = LE32(nonce[12..16]) ^ 页码(1 起)
//!   otk[64] = ChaCha20(master_key, nonce[0..12], counter) 首 64 字节
//!   poly1305 key = otk[0..32]，数据加密 key = otk[32..64]
//!   数据 = ChaCha20(otk[32..64], nonce[0..12], counter+1) 流加密 data[offset..4064)
//!   tag = Poly1305(otk[0..32], data[0..4080))
//! - 第 1 页 offset = 24（前 24 字节为明文 salt + SQLite 头字段），其余页 offset = 0

use chacha20::cipher::{KeyIvInit, StreamCipher, StreamCipherSeek};
use chacha20::ChaCha20;
use pbkdf2::pbkdf2_hmac;
use poly1305::universal_hash::KeyInit;
use poly1305::Poly1305;
use sha2::Sha256;

/// Dawn Launcher 固定数据库密钥（其内置 addon `getKey()` 返回值，sqleet chacha20 格式）
pub const DAWNLAUNCHER_DB_KEY: &str = "d62a8560-362c-5a6a-b397-b36960d23a44";

const PAGE_SIZE: usize = 4096;
const NONCE_LEN: usize = 16;
const RESERVED: usize = 32; // nonce(16) + tag(16)
const DATA_LEN: usize = PAGE_SIZE - RESERVED; // 4064
const KDF_ITER: u32 = 64007; // 非 legacy 模式默认迭代次数
const PAGE1_OFFSET: usize = 24; // 第 1 页明文头长度（salt 16 + SQLite 头字段 8）
const SQLITE_HEADER: [u8; 16] = *b"SQLite format 3\0";

/// 判断文件是否为 sqleet/chacha20 加密的 SQLite 数据库（非标准 SQLite 头）
pub fn is_sqleet_db(head: &[u8]) -> bool {
    head.len() >= 16 && !head.starts_with(b"SQLite format 3\0")
}

/// 将 sqleet 加密的数据库解密为明文 SQLite 文件内容。
/// 解密失败（密钥不匹配 / 格式不符）时返回带说明的错误。
pub fn decrypt_sqleet_db(src: &[u8], passphrase: &str) -> Result<Vec<u8>, String> {
    if src.len() < PAGE_SIZE || src.len() % PAGE_SIZE != 0 {
        return Err(format!(
            "文件大小 {} 不是 sqleet 4096 字节页面的整数倍（可能不是有效的 Dawn Launcher 数据库）",
            src.len()
        ));
    }

    let salt: [u8; 16] = src[0..16]
        .try_into()
        .map_err(|_| "无法读取数据库盐值".to_string())?;

    let mut master_key = [0u8; 32];
    pbkdf2_hmac::<Sha256>(passphrase.as_bytes(), &salt, KDF_ITER, &mut master_key);

    let num_pages = src.len() / PAGE_SIZE;
    let mut out = Vec::with_capacity(src.len());

    for p in 0..num_pages {
        let page_num = (p as u32) + 1;
        let start = p * PAGE_SIZE;
        let page = &src[start..start + PAGE_SIZE];

        let nonce16 = &page[DATA_LEN..DATA_LEN + NONCE_LEN];
        let mut nonce12 = [0u8; 12];
        nonce12.copy_from_slice(&nonce16[0..12]);
        let nonce_last = u32::from_le_bytes([nonce16[12], nonce16[13], nonce16[14], nonce16[15]]);
        let counter = nonce_last ^ page_num;

        // 一次性密钥 otk：ChaCha20(master_key, nonce, counter) 的前 64 字节
        let mut otk = [0u8; 64];
        let mut c = ChaCha20::new(&master_key.into(), &nonce12.into());
        c.seek((counter as u64) * 64);
        c.apply_keystream(&mut otk);

        // 数据加密密钥 = otk[32..64]
        let mut data_key = [0u8; 32];
        data_key.copy_from_slice(&otk[32..64]);

        let offset = if p == 0 { PAGE1_OFFSET } else { 0 };
        let mut plain = page[0..DATA_LEN].to_vec();
        let mut c2 = ChaCha20::new(&data_key.into(), &nonce12.into());
        c2.seek(((counter as u64) + 1) * 64);
        c2.apply_keystream(&mut plain[offset..]);

        // 校验 Poly1305 tag
        let tag = Poly1305::new(otk[0..32].into())
            .compute_unpadded(&page[0..DATA_LEN + NONCE_LEN]);
        if tag.as_slice() != &page[DATA_LEN + NONCE_LEN..PAGE_SIZE] {
            return Err(format!(
                "第 {} 页数据校验失败（可能不是 Dawn Launcher 数据库，或密钥已变更）",
                page_num
            ));
        }

        if p == 0 {
            plain[0..16].copy_from_slice(&SQLITE_HEADER);
        }
        out.extend_from_slice(&plain);
        out.extend_from_slice(&[0u8; RESERVED]);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 按 sqleet/chacha20 非 legacy 格式构造加密数据库（decrypt_sqleet_db 的逆过程）。
    /// plain_pages 为解密后的期望明文（第 1 页前 16 字节为 SQLite 头）。
    fn build_encrypted_db(plain_pages: &[Vec<u8>], passphrase: &str) -> Vec<u8> {
        let salt = [0x11u8; 16];
        let mut master_key = [0u8; 32];
        pbkdf2_hmac::<Sha256>(passphrase.as_bytes(), &salt, KDF_ITER, &mut master_key);

        let mut out = Vec::new();
        for (i, plain) in plain_pages.iter().enumerate() {
            let page_num = (i as u32) + 1;
            let nonce16 = [0x22u8; 16];
            let mut page = [0u8; PAGE_SIZE];
            if page_num == 1 {
                // 第 1 页前 24 字节为明文：salt(16) + SQLite 头字段(8)
                page[0..16].copy_from_slice(&salt);
                page[16..24].copy_from_slice(&plain[16..24]);
                page[24..DATA_LEN].copy_from_slice(&plain[24..DATA_LEN]);
            } else {
                page[0..DATA_LEN].copy_from_slice(plain);
            }
            page[DATA_LEN..DATA_LEN + NONCE_LEN].copy_from_slice(&nonce16);

            let mut nonce12 = [0u8; 12];
            nonce12.copy_from_slice(&nonce16[0..12]);
            let nonce_last = u32::from_le_bytes([nonce16[12], nonce16[13], nonce16[14], nonce16[15]]);
            let counter = nonce_last ^ page_num;

            let mut otk = [0u8; 64];
            let mut c = ChaCha20::new(&master_key.into(), &nonce12.into());
            c.seek((counter as u64) * 64);
            c.apply_keystream(&mut otk);

            let mut data_key = [0u8; 32];
            data_key.copy_from_slice(&otk[32..64]);

            let offset = if page_num == 1 { PAGE1_OFFSET } else { 0 };
            let mut c2 = ChaCha20::new(&data_key.into(), &nonce12.into());
            c2.seek(((counter as u64) + 1) * 64);
            c2.apply_keystream(&mut page[offset..DATA_LEN]);

            let tag = Poly1305::new(otk[0..32].into())
                .compute_unpadded(&page[0..DATA_LEN + NONCE_LEN]);
            page[DATA_LEN + NONCE_LEN..PAGE_SIZE].copy_from_slice(tag.as_slice());
            out.extend_from_slice(&page);
        }
        out
    }

    #[test]
    fn roundtrip_decrypts_encrypted_db() {
        let passphrase = "d62a8560-362c-5a6a-b397-b36960d23a44";

        let mut page1 = vec![0u8; DATA_LEN];
        page1[0..16].copy_from_slice(&SQLITE_HEADER);
        page1[16..24].copy_from_slice(&[0x10, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00]);
        page1[24..40].copy_from_slice(b"anyversion-test!");

        let mut page2 = vec![7u8; DATA_LEN];
        page2[0..8].copy_from_slice(b"payload2");

        let encrypted = build_encrypted_db(&[page1.clone(), page2.clone()], passphrase);
        assert!(is_sqleet_db(&encrypted[0..16]));

        let decrypted = decrypt_sqleet_db(&encrypted, passphrase).expect("解密应成功");

        let mut expected = Vec::new();
        expected.extend_from_slice(&page1);
        expected.extend_from_slice(&[0u8; RESERVED]);
        expected.extend_from_slice(&page2);
        expected.extend_from_slice(&[0u8; RESERVED]);
        assert_eq!(decrypted, expected);

        // 错误密钥必须校验失败
        assert!(decrypt_sqleet_db(&encrypted, "wrong-key").is_err());
    }
}
