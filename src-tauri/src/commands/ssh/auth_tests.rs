//! 阶段 3：主机密钥提示编解码与私钥认证参数校验。
use super::host_keys::{decode_host_key_prompt, encode_host_key_prompt, HostKeyDecision};
use super::keys::{is_private_key, safe_key_name};
use super::target::{validate_key_auth, KeyAuth};

fn key_auth() -> KeyAuth {
    KeyAuth { path: "/keys/id_ed25519".into(), passphrase: None }
}

#[test]
fn host_key_prompt_roundtrips() {
    let encoded = encode_host_key_prompt("203.0.113.9", 2222, "SHA256:abc+/=", HostKeyDecision::Unknown);
    let p = decode_host_key_prompt(&encoded).expect("应能解析");
    assert_eq!(p.host, "203.0.113.9");
    assert_eq!(p.port, 2222);
    assert_eq!(p.fingerprint, "SHA256:abc+/=");
    assert_eq!(p.decision, HostKeyDecision::Unknown);
}

#[test]
fn normal_errors_are_not_mistaken_for_host_key_prompts() {
    assert!(decode_host_key_prompt("连接超时").is_none());
    assert!(decode_host_key_prompt("认证失败：用户名或密码不正确").is_none());
    assert!(decode_host_key_prompt("__HOST_KEY_PROMPT__:garbage").is_none());
}

#[test]
fn changed_key_is_distinct_from_unknown() {
    let encoded = encode_host_key_prompt("h", 22, "SHA256:NEW", HostKeyDecision::Changed);
    assert_eq!(decode_host_key_prompt(&encoded).unwrap().decision, HostKeyDecision::Changed);
}

#[test]
fn key_auth_requires_a_path_but_not_a_passphrase() {
    assert!(validate_key_auth(&key_auth()).is_ok());
    let mut with_pwd = key_auth();
    with_pwd.passphrase = Some("secret".into());
    assert!(validate_key_auth(&with_pwd).is_ok());
    let mut empty = key_auth();
    empty.path = "  ".into();
    assert!(validate_key_auth(&empty).is_err());
}

#[test]
fn key_auth_debug_never_prints_passphrase() {
    let mut auth = key_auth();
    auth.passphrase = Some("synth-secret".into());
    assert!(!format!("{auth:?}").contains("synth-secret"));
}

/// 密钥库文件名：只留最后一段并替换不安全字符，防止写出目标目录。
#[test]
fn key_names_cannot_escape_the_store_directory() {
    assert_eq!(safe_key_name("../../etc/passwd"), "passwd");
    assert_eq!(safe_key_name(r"C:\keys\id_rsa"), "id_rsa");
    assert_eq!(safe_key_name("id_ed25519"), "id_ed25519");
    assert_eq!(safe_key_name(""), "key");
    assert_eq!(safe_key_name(".."), "key");
    assert_eq!(safe_key_name("a b:c"), "a_b_c");
}

/// 只接受看起来像私钥的内容，避免把任意文件复制进密钥库。
#[test]
fn only_private_key_pem_headers_are_accepted() {
    assert!(is_private_key("-----BEGIN OPENSSH PRIVATE KEY-----\nabc"));
    assert!(is_private_key("-----BEGIN RSA PRIVATE KEY-----\nabc"));
    assert!(is_private_key("-----BEGIN PRIVATE KEY-----\nabc"));
    assert!(!is_private_key("ssh-ed25519 AAAAC3Nza public"));
    assert!(!is_private_key("hello world"));
    assert!(!is_private_key(""));
}
