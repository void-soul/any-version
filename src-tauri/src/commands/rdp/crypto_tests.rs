//! FinalShell 密码兼容层的回归测试。
//!
//! 固定向量来自本机 `finalshell.jar` 的 `xk` 类（ZKM 混淆）做的运行时 oracle：
//! 用**独立 DES 实现**造出密文后，交给 FinalShell 自己的 `xk.a(String)` 解密验收。
//! 这些向量把「Java Random 派生 + DES/ECB/PKCS5 + Base64」整条链路钉住，
//! 将来换 DES 实现、改字节序或改填充都必须保持结果一致。
use super::{decrypt, derive_key, encrypt, encrypt_with_salt, is_finalshell_cipher};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn salt(s: &str) -> [u8; 8] {
    let raw: Vec<u8> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect();
    raw.try_into().unwrap()
}

/// salt -> 派生密钥（MD5 结果），3 组全部来自 oracle 的 `xk.a(byte[])`
const DERIVE_VECTORS: &[(&str, &str)] = &[
    ("0102030405060708", "9892b7ef325f2b9d48bebd4f1cb461ea"),
    ("6a252f67066b0a5f", "4b10de924924ecadbdbfecf065bd9967"),
    ("ff8000010203fe63", "96cce39363fdd1d85814897068abdc66"),
];

/// (明文, salt, 密文) —— 密文由独立 DES 生成并经 FinalShell 自身解密函数验收
const CIPHER_VECTORS: &[(&str, &str, &str)] = &[
    ("TestPass123!", "0102030405060708", "AQIDBAUGBwilR6eyge2ytjlkFhf60b0q"),
    ("a", "6a252f67066b0a5f", "aiUvZwZrCl+prWRh/2FM/w=="),
    ("", "ff8000010203fe63", "/4AAAQID/mOUwP0OZaabCw=="),
    ("中文密码Abc!", "0102030405060708", "AQIDBAUGBwiaLKZ7HCtE4jpbFaqQ7LwhTKkcgmOGhO4="),
    ("0123456789abcdefXYZ", "6a252f67066b0a5f", "aiUvZwZrCl93uVO9rz8onikLBwpRjmHoq/nDJnlSt3c="),
    ("!@#$%^&*()", "ff8000010203fe63", "/4AAAQID/mOY5FPmoLiMPX8RLUMtiRP1"),
];

#[test]
fn derive_key_matches_finalshell_oracle() {
    for (s, want) in DERIVE_VECTORS {
        assert_eq!(hex(&derive_key(&salt(s)).unwrap()), *want, "salt={s}");
    }
}

#[test]
fn decrypt_matches_known_vectors() {
    for (plain, _, cipher) in CIPHER_VECTORS {
        assert_eq!(decrypt(cipher).unwrap(), *plain, "cipher={cipher}");
    }
}

#[test]
fn encrypt_with_salt_matches_known_vectors() {
    for (plain, s, want) in CIPHER_VECTORS {
        assert_eq!(encrypt_with_salt(plain, &salt(s)).unwrap(), *want, "plain={plain}");
    }
}

#[test]
/// 真机全量跑出的缺陷：`JavaRandom::next_int(127)` 可能返回 0，直接用作除数会 panic。
/// Java 侧同样会抛 ArithmeticException，因此这种 salt 应被判为「不可用」而不是崩溃。
#[test]
fn unusable_salt_is_reported_instead_of_panicking() {
    let mut bad = None;
    for b in 0u8..=255u8 {
        let candidate = [0u8, 0, 0, 0, 0, b, 0, 0];
        if derive_key(&candidate).is_err() {
            bad = Some(candidate);
            break;
        }
    }
    let bad = bad.expect("遍历 256 个 s[5] 必然存在使 nextInt(127)==0 的 salt");
    assert!(
        encrypt_with_salt("x", &bad).is_err(),
        "不可用 salt 必须返回错误，不能 panic"
    );
}

/// 加密路径要自己跳过不可用 salt，用户无感。
#[test]
fn encrypt_never_fails_because_of_salt() {
    for _ in 0..200 {
        let cipher = encrypt("retry-salt").expect("加密不应因 salt 失败");
        assert_eq!(decrypt(&cipher).unwrap(), "retry-salt");
    }
}

fn random_salt_round_trips_for_many_lengths() {
    for plain in ["x", "12345678", "123456789", "a".repeat(200).as_str(), "中文🔐 mixed"] {
        let cipher = encrypt(plain).unwrap();
        assert!(is_finalshell_cipher(&cipher), "密文形态应被接受: {cipher}");
        assert_eq!(decrypt(&cipher).unwrap(), plain);
    }
}

#[test]
fn cipher_shape_allows_padding_and_longer_plaintexts() {
    // 明文长度不同 -> blob 16/24/32 字节 -> Base64 24/32/44 字符，且可能带 `=`
    for (_, _, cipher) in CIPHER_VECTORS {
        assert!(is_finalshell_cipher(cipher), "应接受 {cipher}");
    }
    assert!(!is_finalshell_cipher("short"));
    assert!(!is_finalshell_cipher("not@base64#"));
}

#[test]
fn malformed_cipher_is_rejected_not_panicking() {
    for bad in ["", "AAAA", "!!!!", "AQIDBAUGBw==", "AQIDBAUGBwilR6eyge2ytjlkFhf60b0!"] {
        assert!(decrypt(bad).is_err(), "应拒绝 {bad:?}");
    }
}
