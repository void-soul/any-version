//! S3 兼容存储通用客户端（AWS Signature V4）。
//!
//! 供两个场景共用：
//! - Picky 模块的云同步（书签/评论/标签，state.json 与 Flutter 端互通）
//! - any-version 统一数据快照同步（见 commands/state_sync.rs）
//!
//! 签名算法与 Flutter picky 端一致（AWS 官方向量验证通过），
//! 支持 auto / path / virtual-host 寻址、本地 http 保留、云商 https 升级、TLS 开关。

use std::collections::BTreeMap;

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// S3 连接配置（通用字段；各模块在此之上扩展自己的配置）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct S3Connection {
    pub endpoint: Option<String>,
    #[serde(default = "default_region")]
    pub region: String,
    #[serde(default)]
    pub access_key_id: String,
    #[serde(default)]
    pub secret_access_key: String,
    #[serde(default)]
    pub bucket_name: String,
    #[serde(default)]
    pub prefix: Option<String>,
    /// auto / path / virtual-host
    #[serde(default = "default_style")]
    pub addressing_style: String,
    #[serde(default = "default_true")]
    pub tls_verify: bool,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

fn default_region() -> String { "us-east-1".to_string() }
fn default_style() -> String { "auto".to_string() }
fn default_true() -> bool { true }
fn default_timeout() -> u64 { 120 }

impl S3Connection {
    pub fn is_configured(&self) -> bool {
        self.endpoint.as_deref().is_some_and(|e| !e.trim().is_empty())
            && !self.access_key_id.is_empty()
            && !self.secret_access_key.is_empty()
            && !self.bucket_name.is_empty()
    }
}

// ─── 加密原语（公开，供测试与各模块复用） ───

pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex_encode(&Sha256::digest(data))
}

pub fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC key 长度合法");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

// ─── endpoint 规范化 ───

fn is_localhost_host(host: &str) -> bool {
    host == "localhost"
        || host == "127.0.0.1"
        || host.starts_with("192.168.")
        || host.starts_with("10.")
        || host.starts_with("172.16.")
        || host.starts_with("172.17.")
        || host.starts_with("172.18.")
        || host.starts_with("172.19.")
        || host.starts_with("172.2")
        || host.starts_with("172.3")
}

fn normalize_endpoint(uri: &reqwest::Url) -> reqwest::Url {
    if uri.scheme() == "http" && !is_localhost_host(uri.host_str().unwrap_or("")) {
        let mut u = uri.clone();
        u.set_scheme("https").ok();
        u
    } else {
        uri.clone()
    }
}

fn clean_key(key: &str) -> String {
    key.trim_start_matches('/').to_string()
}

// ─── S3 客户端 ───

pub struct S3Client {
    pub config: S3Connection,
}

impl S3Client {
    pub fn new(config: S3Connection) -> S3Client {
        S3Client { config }
    }

    fn http_client(&self) -> Result<reqwest::Client, String> {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(self.config.timeout_seconds.max(1)))
            .danger_accept_invalid_certs(!self.config.tls_verify)
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))
    }

    /// 组装对象完整 URL（寻址风格 auto / path / virtual-host）。
    pub fn object_url(&self, uri: &reqwest::Url, key: &str) -> String {
        let base_path = if uri.path().is_empty() || uri.path() == "/" {
            String::new()
        } else {
            uri.path().trim_end_matches('/').to_string()
        };

        // host 是否已携带 bucket 作为子域（虚拟主机式 endpoint，如
        // `https://<bucket>.s3...`）。只要 host 已含 bucket，路径就绝不再追加
        // bucket 段，避免生成 `<bucket>.s3.../<bucket>/state.json` 的重复 bucket。
        // 与 Flutter 端 picky 的寻址规则一致，保证双端读写同一对象。
        let host_has_bucket = uri
            .host_str()
            .unwrap_or("")
            .starts_with(&format!("{}.", self.config.bucket_name));

        let use_virtual_host = match self.config.addressing_style.as_str() {
            // 用户显式选路径式：但若 host 已含 bucket（虚拟主机式 endpoint），
            // 仍视为虚拟主机式，避免把 bucket 写进路径段。
            "path" => host_has_bucket,
            "virtual-host" | "virtualHost" => true,
            _ => host_has_bucket,
        };

        let mut effective = uri.clone();
        if use_virtual_host && !host_has_bucket {
            if let Some(host) = uri.host_str() {
                // 用 set_host 注入 bucket（保留端口），避免手工重建 URL 丢失 port
                let new_host = format!("{}.{}", self.config.bucket_name, host);
                let _ = effective.set_host(Some(&new_host));
            }
        }

        let authority = match effective.port() {
            Some(p) => format!("{}:{}", effective.host_str().unwrap_or(""), p),
            None => effective.host_str().unwrap_or("").to_string(),
        };
        let bucket_segment = if use_virtual_host {
            ""
        } else {
            &format!("/{}", self.config.bucket_name)
        };
        format!(
            "{}://{}{}{}/{}",
            effective.scheme(),
            authority,
            base_path,
            bucket_segment,
            key
        )
    }

    /// 计算 SigV4 Authorization 头。
    pub fn sign_request(
        &self,
        method: &str,
        url: &str,
        headers: &BTreeMap<String, String>,
        body: &[u8],
        date: &chrono::DateTime<chrono::Utc>,
        service: &str,
    ) -> String {
        let region = self.config.region.as_str();
        let date_str = date.format("%Y%m%dT%H%M%SZ").to_string();
        let credential_scope = format!("{}/{}/{}/aws4_request", &date_str[..8], region, service);

        let uri = reqwest::Url::parse(url).expect("object url 可解析");
        let canonical_uri = if uri.path().is_empty() {
            "/".to_string()
        } else {
            uri.path().to_string()
        };
        let canonical_query = "";

        // 参与签名的头：host + 所有 x-amz-*（按字母序）。
        let mut signable: Vec<(&String, &String)> = headers
            .iter()
            .filter(|(k, _)| k.to_lowercase() == "host" || k.to_lowercase().starts_with("x-amz-"))
            .collect();
        signable.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

        let canonical_headers: String = signable
            .iter()
            .map(|(k, v)| format!("{}:{}\n", k.to_lowercase(), v.trim()))
            .collect();
        let signed_headers: Vec<String> = signable.iter().map(|(k, _)| k.to_lowercase()).collect();
        let signed_headers_str = signed_headers.join(";");

        let payload_hash = headers
            .get("x-amz-content-sha256")
            .cloned()
            .unwrap_or_else(|| sha256_hex(body));

        let canonical_request = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            method, canonical_uri, canonical_query, canonical_headers, signed_headers_str, payload_hash
        );

        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            date_str,
            credential_scope,
            sha256_hex(canonical_request.as_bytes())
        );

        let k_secret = format!("AWS4{}", self.config.secret_access_key);
        let k_date = hmac_sha256(k_secret.as_bytes(), date_str[..8].as_bytes());
        let k_region = hmac_sha256(&k_date, region.as_bytes());
        let k_service = hmac_sha256(&k_region, service.as_bytes());
        let k_signing = hmac_sha256(&k_service, b"aws4_request");
        let signature = hex_encode(&hmac_sha256(&k_signing, string_to_sign.as_bytes()));

        format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.config.access_key_id, credential_scope, signed_headers_str, signature
        )
    }

    /// 上传任意字节到 S3，返回对象 key。
    pub async fn upload_bytes(
        &self,
        object_key: &str,
        body: &[u8],
        content_type: &str,
    ) -> Result<String, String> {
        if !self.config.is_configured() {
            return Err("S3 未配置（需填写 endpoint / AccessKey / SecretKey / Bucket）".to_string());
        }
        let parsed = reqwest::Url::parse(self.config.endpoint.as_deref().unwrap_or(""))
            .map_err(|_| format!("无效的 endpoint URL: {:?}", self.config.endpoint))?;
        let uri = normalize_endpoint(&parsed);
        let key = clean_key(&format!("{}{}", self.config.prefix.as_deref().unwrap_or(""), object_key));
        let full_url = self.object_url(&uri, &key);
        crate::exit_log!(
            "[s3] 上传 bucket={} key={} url={} 字节={}",
            self.config.bucket_name, key, full_url, body.len()
        );

        let date = chrono::Utc::now();
        let date_str = date.format("%Y%m%dT%H%M%SZ").to_string();

        let url_uri = reqwest::Url::parse(&full_url).map_err(|e| format!("URL 解析失败: {}", e))?;
        let host = match url_uri.port() {
            Some(p) => format!("{}:{}", url_uri.host_str().unwrap_or(""), p),
            None => url_uri.host_str().unwrap_or("").to_string(),
        };

        let mut headers = BTreeMap::new();
        headers.insert("Host".to_string(), host);
        headers.insert("Content-Type".to_string(), content_type.to_string());
        headers.insert("x-amz-date".to_string(), date_str.clone());
        headers.insert("x-amz-content-sha256".to_string(), sha256_hex(body));

        let auth = self.sign_request("PUT", &full_url, &headers, body, &date, "s3");
        headers.insert("Authorization".to_string(), auth);

        let client = self.http_client()?;
        let mut req = client.put(url_uri).body(body.to_vec());
        for (k, v) in &headers {
            if k == "Host" {
                continue;
            }
            req = req.header(k, v);
        }
        let started = std::time::Instant::now();
        let resp = req
            .send()
            .await
            .map_err(|e| format!("S3 上传请求失败 ({} 字节, 超时 {}s): {}", body.len(), self.config.timeout_seconds, e))?;
        let elapsed_ms = started.elapsed().as_millis();
        let status = resp.status();
        if status.is_success() {
            crate::exit_log!(
                "[s3] 上传成功: key={} {} 字节 耗时 {}ms",
                key,
                body.len(),
                elapsed_ms
            );
            Ok(key)
        } else {
            let text = resp.text().await.unwrap_or_default();
            let msg = format!(
                "S3 上传失败 ({}): {} ({} 字节, 耗时 {}ms)",
                status.as_u16(),
                text.chars().take(300).collect::<String>(),
                body.len(),
                elapsed_ms
            );
            crate::exit_log::exit_log(&msg);
            Err(msg)
        }
    }

    /// 下载任意字节；不存在（404）返回 None。
    pub async fn download_bytes(&self, object_key: &str) -> Result<Option<Vec<u8>>, String> {
        if !self.config.is_configured() {
            return Ok(None);
        }
        let parsed = reqwest::Url::parse(self.config.endpoint.as_deref().unwrap_or(""))
            .map_err(|_| format!("无效的 endpoint URL: {:?}", self.config.endpoint))?;
        let uri = normalize_endpoint(&parsed);
        let key = clean_key(&format!("{}{}", self.config.prefix.as_deref().unwrap_or(""), object_key));
        let full_url = self.object_url(&uri, &key);
        crate::exit_log!(
            "[s3] 下载 bucket={} key={} url={}",
            self.config.bucket_name, key, full_url
        );

        let date = chrono::Utc::now();
        let date_str = date.format("%Y%m%dT%H%M%SZ").to_string();

        let url_uri = reqwest::Url::parse(&full_url).map_err(|e| format!("URL 解析失败: {}", e))?;
        let host = match url_uri.port() {
            Some(p) => format!("{}:{}", url_uri.host_str().unwrap_or(""), p),
            None => url_uri.host_str().unwrap_or("").to_string(),
        };

        let mut headers = BTreeMap::new();
        headers.insert("Host".to_string(), host);
        headers.insert("x-amz-date".to_string(), date_str);
        headers.insert("x-amz-content-sha256".to_string(), "UNSIGNED-PAYLOAD".to_string());

        let auth = self.sign_request("GET", &full_url, &headers, &[], &date, "s3");
        headers.insert("Authorization".to_string(), auth);

        let client = self.http_client()?;
        let mut req = client.get(url_uri);
        for (k, v) in &headers {
            if k == "Host" {
                continue;
            }
            req = req.header(k, v);
        }
        let resp = req.send().await.map_err(|e| format!("S3 下载请求失败: {}", e))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if status.is_success() {
            let bytes = resp.bytes().await.map_err(|e| format!("读取 S3 响应失败: {}", e))?;
            Ok(Some(bytes.to_vec()))
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(format!(
                "S3 下载失败 ({}): {}",
                status.as_u16(),
                text.chars().take(300).collect::<String>()
            ))
        }
    }

    /// 上传 JSON 对象到 S3，返回对象 key。
    pub async fn upload(&self, object_key: &str, data: &serde_json::Value) -> Result<String, String> {
        if !self.config.is_configured() {
            return Err("S3 未配置（需填写 endpoint / AccessKey / SecretKey / Bucket）".to_string());
        }
        let parsed = reqwest::Url::parse(self.config.endpoint.as_deref().unwrap_or(""))
            .map_err(|_| format!("无效的 endpoint URL: {:?}", self.config.endpoint))?;
        let uri = normalize_endpoint(&parsed);
        let key = clean_key(&format!("{}{}", self.config.prefix.as_deref().unwrap_or(""), object_key));
        let full_url = self.object_url(&uri, &key);
        crate::exit_log!(
            "[s3] 上传 bucket={} key={} url={}",
            self.config.bucket_name, key, full_url
        );

        let body = serde_json::to_vec(data).map_err(|e| format!("序列化失败: {}", e))?;
        let date = chrono::Utc::now();
        let date_str = date.format("%Y%m%dT%H%M%SZ").to_string();

        let url_uri = reqwest::Url::parse(&full_url).map_err(|e| format!("URL 解析失败: {}", e))?;
        let host = match url_uri.port() {
            Some(p) => format!("{}:{}", url_uri.host_str().unwrap_or(""), p),
            None => url_uri.host_str().unwrap_or("").to_string(),
        };

        let mut headers = BTreeMap::new();
        headers.insert("Host".to_string(), host);
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        headers.insert("x-amz-date".to_string(), date_str.clone());
        headers.insert("x-amz-content-sha256".to_string(), sha256_hex(&body));

        let auth = self.sign_request("PUT", &full_url, &headers, &body, &date, "s3");
        headers.insert("Authorization".to_string(), auth);

        let client = self.http_client()?;
        let mut req = client.put(url_uri).body(body);
        for (k, v) in &headers {
            if k == "Host" {
                continue; // reqwest 自动设置 Host
            }
            req = req.header(k, v);
        }
        let resp = req.send().await.map_err(|e| format!("S3 上传请求失败: {}", e))?;
        let status = resp.status();
        if status.is_success() {
            Ok(key)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(format!(
                "S3 上传失败 ({}): {}",
                status.as_u16(),
                text.chars().take(300).collect::<String>()
            ))
        }
    }

    /// 下载 JSON 对象；不存在（404）返回 None。
    pub async fn download(&self, object_key: &str) -> Result<Option<serde_json::Value>, String> {
        if !self.config.is_configured() {
            return Ok(None);
        }
        let parsed = reqwest::Url::parse(self.config.endpoint.as_deref().unwrap_or(""))
            .map_err(|_| format!("无效的 endpoint URL: {:?}", self.config.endpoint))?;
        let uri = normalize_endpoint(&parsed);
        let key = clean_key(&format!("{}{}", self.config.prefix.as_deref().unwrap_or(""), object_key));
        let full_url = self.object_url(&uri, &key);
        crate::exit_log!(
            "[s3] 下载 bucket={} key={} url={}",
            self.config.bucket_name, key, full_url
        );

        let date = chrono::Utc::now();
        let date_str = date.format("%Y%m%dT%H%M%SZ").to_string();

        let url_uri = reqwest::Url::parse(&full_url).map_err(|e| format!("URL 解析失败: {}", e))?;
        let host = match url_uri.port() {
            Some(p) => format!("{}:{}", url_uri.host_str().unwrap_or(""), p),
            None => url_uri.host_str().unwrap_or("").to_string(),
        };

        let mut headers = BTreeMap::new();
        headers.insert("Host".to_string(), host);
        headers.insert("x-amz-date".to_string(), date_str);
        headers.insert("x-amz-content-sha256".to_string(), "UNSIGNED-PAYLOAD".to_string());

        let auth = self.sign_request("GET", &full_url, &headers, &[], &date, "s3");
        headers.insert("Authorization".to_string(), auth);

        let client = self.http_client()?;
        let mut req = client.get(url_uri);
        for (k, v) in &headers {
            if k == "Host" {
                continue;
            }
            req = req.header(k, v);
        }
        let resp = req.send().await.map_err(|e| format!("S3 下载请求失败: {}", e))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if status.is_success() {
            let text = resp.text().await.map_err(|e| format!("读取 S3 响应失败: {}", e))?;
            serde_json::from_str(&text)
                .map(Some)
                .map_err(|e| format!("解析云端数据失败: {}", e))
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(format!(
                "S3 下载失败 ({}): {}",
                status.as_u16(),
                text.chars().take(300).collect::<String>()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// AWS Signature V4 官方测试向量（aws-sig-v4-test-suite get-vanilla）。
    #[test]
    fn sigv4_get_vanilla_vector() {
        let config = S3Connection {
            region: "us-east-1".to_string(),
            access_key_id: "AKIDEXAMPLE".to_string(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_string(),
            ..Default::default()
        };
        let client = S3Client::new(config);
        let date = chrono::DateTime::parse_from_rfc3339("2015-08-30T12:36:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let mut headers = BTreeMap::new();
        headers.insert("Host".to_string(), "example.amazonaws.com".to_string());
        headers.insert("x-amz-date".to_string(), "20150830T123600Z".to_string());

        let auth = client.sign_request("GET", "https://example.amazonaws.com/", &headers, &[], &date, "service");
        assert!(
            auth.contains("Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"),
            "签名不匹配: {}",
            auth
        );
    }

    /// 校验对象 URL 寻址风格（path / virtual-host / auto）。
    #[test]
    fn object_url_styles() {
        let base = S3Connection {
            bucket_name: "picky-9527".to_string(),
            ..Default::default()
        };
        let mut cfg = base.clone();
        cfg.addressing_style = "path".to_string();
        let client = S3Client::new(cfg);
        let uri = reqwest::Url::parse("https://s3.cn-north-1.qiniucs.com").unwrap();
        assert_eq!(
            client.object_url(&uri, "state.json"),
            "https://s3.cn-north-1.qiniucs.com/picky-9527/state.json"
        );

        let mut cfg = base.clone();
        cfg.addressing_style = "virtual-host".to_string();
        let client = S3Client::new(cfg);
        let uri = reqwest::Url::parse("http://localhost:9000").unwrap();
        assert_eq!(
            client.object_url(&uri, "state.json"),
            "http://picky-9527.localhost:9000/state.json"
        );

        let client = S3Client::new(base.clone());
        let uri = reqwest::Url::parse("https://picky-9527.s3.example.com").unwrap();
        assert_eq!(
            client.object_url(&uri, "state.json"),
            "https://picky-9527.s3.example.com/state.json"
        );

        let client = S3Client::new(base);
        let uri = reqwest::Url::parse("https://example.com/base/").unwrap();
        assert_eq!(
            client.object_url(&uri, "state.json"),
            "https://example.com/base/picky-9527/state.json"
        );
    }

    /// 回归：path 风格 + endpoint host 已含 bucket 子域（七牛虚拟主机式 endpoint，
    /// 如 `https://picky-9527.s3...`）时，路径里不再重复拼 bucket 段，
    /// 避免双端读到不同对象（与 Flutter 端 picky 寻址规则一致）。
    #[test]
    fn object_url_path_style_no_duplicate_bucket_when_host_has_bucket() {
        let mut cfg = S3Connection {
            bucket_name: "picky-9527".to_string(),
            ..Default::default()
        };
        cfg.addressing_style = "path".to_string();
        let client = S3Client::new(cfg);
        let uri = reqwest::Url::parse("https://picky-9527.s3.cn-north-1.qiniucs.com").unwrap();
        assert_eq!(
            client.object_url(&uri, "state.json"),
            "https://picky-9527.s3.cn-north-1.qiniucs.com/state.json"
        );

        // auto 风格同样去重
        let cfg = S3Connection {
            bucket_name: "picky-9527".to_string(),
            ..Default::default()
        };
        let client = S3Client::new(cfg);
        let uri = reqwest::Url::parse("https://picky-9527.s3.cn-north-1.qiniucs.com").unwrap();
        assert_eq!(
            client.object_url(&uri, "state.json"),
            "https://picky-9527.s3.cn-north-1.qiniucs.com/state.json"
        );
    }
}
