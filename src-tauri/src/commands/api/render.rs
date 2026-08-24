//! 变量渲染：项目变量 + 随机变量。
//!
//! 语法（Postman 风格）：
//! - `{{变量名}}`：从环境变量表取值，未命中保留原样
//! - `{{$guid}}` / `{{$uuid}}`：UUID v4
//! - `{{$timestamp}}`：Unix 秒
//! - `{{$isoTimestamp}}`：ISO8601 UTC
//! - `{{$randomInt}}`：0–999999
//! - `{{random:int:1:100}}`：区间随机整数（含两端）
//! - `{{random:string:8}}`：随机字母数字串（指定长度）
//! - `{{$randomEmail}}`：随机邮箱

use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

/// 随机数种子（进程内递增，避免并发下重复）。
static SEED: AtomicU64 = AtomicU64::new(0x9e3779b97f4a7c15);

fn next_rand() -> u64 {
    // xorshift64star
    let mut x = SEED.fetch_add(0x9e3779b97f4a7c15, Ordering::Relaxed);
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    x.wrapping_mul(0x2545F4914F6CDD1D)
}

fn rand_u64() -> u64 {
    next_rand()
}

fn rand_range(min: i64, max: i64) -> i64 {
    if max <= min {
        return min;
    }
    min + (rand_u64() % ((max - min + 1) as u64)) as i64
}

fn rand_string(len: usize) -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut out = String::with_capacity(len);
    for _ in 0..len {
        out.push(CHARS[(rand_u64() % CHARS.len() as u64) as usize] as char);
    }
    out
}

fn rand_email() -> String {
    let name = rand_string(8);
    let domains = ["example.com", "test.com", "mail.com"];
    let domain = domains[(rand_u64() % domains.len() as u64) as usize];
    format!("{}@{}", name, domain)
}

fn rand_uuid() -> String {
    let mut b = [0u8; 16];
    for chunk in b.chunks_mut(4) {
        let v = rand_u64();
        chunk.copy_from_slice(&v.to_le_bytes()[..chunk.len()]);
    }
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

fn unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn iso_timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// 渲染单个模板中的 `{{...}}`。
pub fn render_template(template: &str, variables: &serde_json::Map<String, Value>) -> String {
    if !template.contains("{{") {
        return template.to_string();
    }
    let mut out = String::with_capacity(template.len() + 32);
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(close) = template[i + 2..].find("}}") {
                let name = &template[i + 2..i + 2 + close];
                let replaced = resolve_variable(name, variables);
                out.push_str(&replaced);
                i += 2 + close + 2;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// 解析单个变量名（可能带随机函数），返回渲染结果。未命中返回原文 `{{name}}`。
fn resolve_variable(name: &str, variables: &serde_json::Map<String, Value>) -> String {
    match name {
        "$guid" | "$uuid" | "random:uuid" => return rand_uuid(),
        "$timestamp" => return unix_secs().to_string(),
        "$isoTimestamp" | "$isoTimestampUtc" => return iso_timestamp(),
        "$randomInt" => return rand_range(0, 999999).to_string(),
        "$randomEmail" => return rand_email(),
        _ => {}
    }
    if let Some(rest) = name.strip_prefix("random:int:") {
        let parts: Vec<&str> = rest.split(':').collect();
        let min: i64 = parts.first().and_then(|p| p.parse().ok()).unwrap_or(0);
        let max: i64 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(100);
        return rand_range(min, max).to_string();
    }
    if let Some(rest) = name.strip_prefix("random:string:") {
        let len: usize = rest.parse().unwrap_or(8);
        return rand_string(len.clamp(1, 64));
    }
    if let Some(v) = variables.get(name) {
        return match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
    }
    format!("{{{{{}}}}}", name)
}

/// 渲染路径参数（把 `:id` 换成值）与查询参数等键值对。
pub fn render_kv(items: &[super::models::KeyValueItem], variables: &serde_json::Map<String, Value>) -> Vec<super::models::KeyValueItem> {
    items
        .iter()
        .map(|kv| super::models::KeyValueItem {
            key: render_template(&kv.key, variables),
            value: render_template(&kv.value, variables),
            enabled: kv.enabled,
            description: kv.description.clone(),
        })
        .collect()
}

/// 将路径参数 `:name`/`{name}` 替换进 URL。
pub fn apply_path_params(url: &str, path_params: &[super::models::KeyValueItem]) -> String {
    let mut out = url.to_string();
    for kv in path_params {
        if !kv.enabled || kv.key.is_empty() {
            continue;
        }
        out = out.replace(&format!(":{}", kv.key), &kv.value);
        out = out.replace(&format!("{{{}}}", kv.key), &kv.value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> serde_json::Map<String, Value> {
        let mut m = serde_json::Map::new();
        m.insert("baseUrl".into(), Value::String("https://api.example.com".into()));
        m.insert("token".into(), Value::String("abc123".into()));
        m.insert("userId".into(), Value::Number(42.into()));
        m
    }

    #[test]
    fn renders_env_variables() {
        let v = vars();
        assert_eq!(render_template("{{baseUrl}}/users/{{userId}}", &v), "https://api.example.com/users/42");
        assert_eq!(render_template("Bearer {{token}}", &v), "Bearer abc123");
    }

    #[test]
    fn unknown_variable_left_untouched() {
        assert_eq!(render_template("{{missing}}", &vars()), "{{missing}}");
    }

    #[test]
    fn random_variables_produce_values() {
        let v = vars();
        assert!(render_template("{{$guid}}", &v).len() == 36);
        assert!(render_template("{{random:uuid}}", &v).len() == 36);
        let ts: i64 = render_template("{{$timestamp}}", &v).parse().unwrap();
        assert!(ts > 1_500_000_000);
        let n: i64 = render_template("{{random:int:5:5}}", &v).parse().unwrap();
        assert_eq!(n, 5);
        let s = render_template("{{random:string:10}}", &v);
        assert_eq!(s.len(), 10);
        assert!(render_template("{{$randomEmail}}", &v).contains('@'));
        assert!(render_template("{{$isoTimestamp}}", &v).contains('T'));
    }

    #[test]
    fn renders_multiple_in_one_template() {
        let v = vars();
        let out = render_template("{{baseUrl}}/u/{{$timestamp}}?t={{token}}", &v);
        assert!(out.starts_with("https://api.example.com/u/"));
        assert!(out.ends_with("?t=abc123"));
    }

    #[test]
    fn apply_path_params_replaces_colon_and_brace() {
        let items = vec![super::super::models::KeyValueItem { key: "id".into(), value: "7".into(), enabled: true, description: String::new() }];
        assert_eq!(apply_path_params("/users/:id", &items), "/users/7");
        assert_eq!(apply_path_params("/users/{id}", &items), "/users/7");
    }
}
