//! Picky 云同步：在共享 S3 客户端（commands/s3_common.rs）之上，
//! 提供 Picky 专属配置与 state.json 全量状态的读写（与 Flutter 端格式互通）。

use serde::{Deserialize, Serialize};

use crate::commands::s3_common::{S3Client, S3Connection};

/// Picky 云同步配置（与 Flutter CloudSyncConfig 字段一致，camelCase）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PickySyncConfig {
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
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub last_sync_at: Option<String>,
    /// auto / path / virtual-host
    #[serde(default = "default_style")]
    pub addressing_style: String,
    #[serde(default = "default_true")]
    pub tls_verify: bool,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    #[serde(default = "default_concurrent")]
    pub concurrent_reqs: u32,
}

fn default_region() -> String { "us-east-1".to_string() }
fn default_style() -> String { "auto".to_string() }
fn default_true() -> bool { true }
fn default_timeout() -> u64 { 30 }
fn default_concurrent() -> u32 { 1 }

impl PickySyncConfig {
    /// 是否已配置完整（与 Flutter isConfigured 一致）。
    pub fn is_configured(&self) -> bool {
        self.enabled
            && self.endpoint.as_deref().is_some_and(|e| !e.trim().is_empty())
            && !self.access_key_id.is_empty()
            && !self.secret_access_key.is_empty()
            && !self.bucket_name.is_empty()
    }

    fn connection(&self) -> S3Connection {
        S3Connection {
            endpoint: self.endpoint.clone(),
            region: self.region.clone(),
            access_key_id: self.access_key_id.clone(),
            secret_access_key: self.secret_access_key.clone(),
            bucket_name: self.bucket_name.clone(),
            prefix: self.prefix.clone(),
            addressing_style: self.addressing_style.clone(),
            tls_verify: self.tls_verify,
            timeout_seconds: self.timeout_seconds,
        }
    }
}

/// 对象 key 约定（与 Flutter 端完全一致）：
/// - `{prefix}state.json`：单文件全量状态（concurrent<=1）
/// - `{prefix}state.manifest.json` + `{prefix}state.part<i>.json`：并发分片（concurrent>1）

pub struct PickyS3Client {
    config: PickySyncConfig,
}

impl PickyS3Client {
    pub fn new(config: PickySyncConfig) -> PickyS3Client {
        PickyS3Client { config }
    }

    fn raw(&self) -> S3Client {
        S3Client::new(self.config.connection())
    }

    /// 下载全量状态：优先单文件 state.json，不存在则按 manifest 合并分片。
    pub async fn download_full_state(&self) -> Result<Option<serde_json::Value>, String> {
        let raw = self.raw();
        if let Some(single) = raw.download("state.json").await? {
            return Ok(Some(single));
        }
        let manifest = match raw.download("state.manifest.json").await? {
            Some(m) => m,
            None => return Ok(None),
        };
        let part_count = manifest.get("partCount").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        if part_count == 0 {
            return Ok(None);
        }
        let mut bookmarks: Vec<serde_json::Value> = Vec::new();
        let mut meta0: Option<serde_json::Value> = None;
        for i in 0..part_count {
            if let Some(p) = raw.download(&format!("state.part{}.json", i)).await? {
                if let Some(arr) = p.get("bookmarks").and_then(|v| v.as_array()) {
                    bookmarks.extend(arr.iter().cloned());
                }
                if p.get("partIndex").and_then(|v| v.as_u64()) == Some(i as u64) && i == 0 {
                    meta0 = Some(p);
                }
            }
        }
        if meta0.is_none() {
            // 兼容：从第一个非空分片取共享数据
            for i in 0..part_count {
                if let Some(p) = raw.download(&format!("state.part{}.json", i)).await? {
                    meta0 = Some(p);
                    break;
                }
            }
        }
        let m = meta0.unwrap_or_else(|| serde_json::json!({}));
        Ok(Some(serde_json::json!({
            "version": 1,
            "bookmarks": bookmarks,
            "comments": m.get("comments").cloned().unwrap_or_else(|| serde_json::json!([])),
            "tags": m.get("tags").cloned().unwrap_or_else(|| serde_json::json!([])),
            "bookmarkTags": m.get("bookmarkTags").cloned().unwrap_or_else(|| serde_json::json!({})),
            "syncedAt": m.get("syncedAt").cloned().unwrap_or_default(),
        })))
    }

    /// 上传全量状态（单文件 state.json，与 Flutter concurrent<=1 输出一致，可被其读取）。
    pub async fn upload_full_state(
        &self,
        bookmarks: &[serde_json::Value],
        comments: &[serde_json::Value],
        tags: &[serde_json::Value],
        bookmark_tags: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<String, String> {
        let synced_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let state = serde_json::json!({
            "version": 1,
            "bookmarks": bookmarks,
            "comments": comments,
            "tags": tags,
            "bookmarkTags": bookmark_tags,
            "syncedAt": synced_at,
        });
        self.raw().upload("state.json", &state).await?;
        Ok(synced_at)
    }
}
