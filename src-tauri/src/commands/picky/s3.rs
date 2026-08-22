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
fn default_timeout() -> u64 { 120 }
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

    /// 下载对象并自动解压（gzip magic 1f 8b 检测），再解析 JSON。
    /// 兼容旧版未压缩的明文 JSON 对象。
    async fn download_json(&self, key: &str) -> Result<Option<serde_json::Value>, String> {
        let raw = self.raw();
        let Some(bytes) = raw.download_bytes(key).await? else {
            return Ok(None);
        };
        let text = if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
            let mut gz = flate2::read::GzDecoder::new(bytes.as_slice());
            let mut out = Vec::new();
            std::io::Read::read_to_end(&mut gz, &mut out)
                .map_err(|e| format!("解压 {} 失败: {}", key, e))?;
            String::from_utf8(out).map_err(|e| format!("解压 {} 编码错误: {}", key, e))?
        } else {
            String::from_utf8(bytes).map_err(|e| format!("{} 编码错误: {}", key, e))?
        };
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| format!("解析 {} 失败: {}", key, e))
    }

    /// 序列化 JSON 并用 gzip 压缩后上传（Content-Type: application/gzip）。
    async fn upload_json_gzip(&self, key: &str, data: &serde_json::Value) -> Result<(), String> {
        let raw = self.raw();
        let body = serde_json::to_vec(data).map_err(|e| format!("序列化失败: {}", e))?;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut gz, &body).map_err(|e| format!("压缩失败: {}", e))?;
        let compressed = gz.finish().map_err(|e| format!("压缩失败: {}", e))?;
        crate::exit_log!(
            "[picky-sync] {} 压缩: {} 字节 → {} 字节",
            key,
            body.len(),
            compressed.len()
        );
        raw.upload_bytes(key, &compressed, "application/gzip").await?;
        Ok(())
    }

    /// 下载全量状态：优先单文件 state.json，不存在则按 manifest 并发下载各分片合并。
    /// 与 Flutter 端 `downloadFullState` 逻辑完全一致：
    /// 1) 先试 state.json（兼容旧版 / concurrent<=1）；
    /// 2) 不存在则读 state.manifest.json，按 partCount 并发下载 state.part<i>.json；
    /// 3) bookmarks 汇总所有分片；共享数据（comments/tags/bookmarkTags/syncedAt）
    ///    取 partIndex==0 的分片，缺失时取第一个非空分片（不重复下载）。
    pub async fn download_full_state(&self) -> Result<Option<serde_json::Value>, String> {
        if let Some(single) = self.download_json("state.json").await? {
            return Ok(Some(single));
        }
        let manifest = match self.download_json("state.manifest.json").await? {
            Some(m) => m,
            None => return Ok(None),
        };
        let part_count = manifest.get("partCount").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        if part_count == 0 {
            return Ok(None);
        }

        // 并发下载所有分片（与 Flutter Future.wait 一致）
        let part_keys: Vec<String> =
            (0..part_count).map(|i| format!("state.part{}.json", i)).collect();
        let parts: Vec<Result<Option<serde_json::Value>, String>> =
            futures_util::future::join_all(part_keys.iter().map(|k| self.download_json(k))).await;

        let mut bookmarks: Vec<serde_json::Value> = Vec::new();
        let mut meta0: Option<serde_json::Value> = None;
        for (i, p) in parts.into_iter().enumerate() {
            match p? {
                Some(part) => {
                    if let Some(arr) = part.get("bookmarks").and_then(|v| v.as_array()) {
                        bookmarks.extend(arr.iter().cloned());
                    }
                    if part.get("partIndex").and_then(|v| v.as_u64()) == Some(0) {
                        meta0 = Some(part);
                    } else if meta0.is_none() {
                        // 兼容：partIndex==0 缺失时取第一个非空分片（复用已下载结果，不重复下载）
                        meta0 = Some(part);
                    }
                }
                None => {
                    crate::exit_log::exit_log(&format!(
                        "[picky-sync] 分片缺失 state.part{}.json，同步结果可能不完整",
                        i
                    ));
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

    /// 上传全量状态。与 Flutter 端 `syncFullState` 逻辑完全一致：
    /// `concurrent_reqs <= 1`（或书签数不足以分片）时写单文件 `state.json`；
    /// 否则按 `concurrent_reqs` 把书签切成多片，写 `state.manifest.json` +
    /// `state.part<i>.json`（共享数据 comments/tags/bookmarkTags 只随第 0 片）。
    pub async fn upload_full_state(
        &self,
        bookmarks: &[serde_json::Value],
        comments: &[serde_json::Value],
        tags: &[serde_json::Value],
        bookmark_tags: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<String, String> {
        let synced_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        // 分片规则与 Flutter _splitBookmarks 一致：
        // n = clamp(concurrent, 1, max(1, bookmarks.len()))；size = ceil(len / n)
        let n = self
            .config
            .concurrent_reqs
            .max(1)
            .min(if bookmarks.is_empty() { 1 } else { bookmarks.len() as u32 }) as usize;
        let size = if bookmarks.is_empty() {
            0
        } else {
            (bookmarks.len() + n - 1) / n
        };
        let parts: Vec<&[serde_json::Value]> = if size == 0 {
            vec![]
        } else {
            bookmarks.chunks(size).collect()
        };

        if parts.len() <= 1 {
            // 单文件全量状态（与旧版本 / concurrent<=1 兼容）
            let state = serde_json::json!({
                "version": 1,
                "bookmarks": bookmarks,
                "comments": comments,
                "tags": tags,
                "bookmarkTags": bookmark_tags,
                "syncedAt": synced_at,
            });
            self.upload_json_gzip("state.json", &state).await?;
            return Ok(synced_at);
        }

        // 分片上传：manifest + 各 part（共享数据只随第 0 片）
        let manifest = serde_json::json!({
            "version": 1,
            "partCount": parts.len(),
            "total": bookmarks.len(),
            "syncedAt": synced_at,
        });
        self.upload_json_gzip("state.manifest.json", &manifest).await?;
        let empty_tags: Vec<serde_json::Value> = Vec::new();
        let empty_map: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
        for (i, part) in parts.iter().enumerate() {
            let part_json = serde_json::json!({
                "version": 1,
                "partIndex": i,
                "partCount": parts.len(),
                "bookmarks": part,
                "comments": if i == 0 { comments } else { &empty_tags },
                "tags": if i == 0 { tags } else { &empty_tags },
                "bookmarkTags": if i == 0 { bookmark_tags } else { &empty_map },
                "syncedAt": synced_at,
            });
            self.upload_json_gzip(&format!("state.part{}.json", i), &part_json).await?;
        }
        Ok(synced_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// gzip 往返：序列化 → gzip → 解压 → 解析，与 Flutter 端 gzip.encode/decode 兼容。
    #[test]
    fn gzip_roundtrip_preserves_json() {
        let data = serde_json::json!({
            "version": 1,
            "bookmarks": [
                {"id": "a", "title": "测试", "refined": 0, "metaFetched": 1},
                {"id": "b", "title": "标题", "refined": 1, "metaFetched": 0},
            ],
            "comments": [],
            "tags": [],
            "bookmarkTags": {},
            "syncedAt": "2026-08-22T16:05:34.747Z",
        });
        let body = serde_json::to_vec(&data).unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut gz, &body).unwrap();
        let compressed = gz.finish().unwrap();

        // gzip magic 校验（Flutter 端 gzip.encode 输出同样以 1f 8b 开头）
        assert_eq!(compressed[0], 0x1f);
        assert_eq!(compressed[1], 0x8b);
        // 压缩确实更小
        assert!(compressed.len() < body.len());

        // 解压解析还原
        let mut dec = flate2::read::GzDecoder::new(compressed.as_slice());
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut dec, &mut out).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed, data);
        assert_eq!(
            parsed["bookmarks"].as_array().unwrap().len(),
            2
        );
    }
}
