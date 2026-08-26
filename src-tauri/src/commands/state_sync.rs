//! any-version 统一数据配置备份与同步。
//!
//! 各模块平时仍使用各自的存储（config.json / *.db 等），但备份层面统一为
//! **一个快照文件**：把 data_dir 下所有模块的配置与数据打包进单个 JSON
//! （文件内容 base64），本地导出时再整体 gzip 压缩，可导入/导出，也可同步到
//! S3（独立于 Picky 的同步，使用自己的 S3 配置与对象 key，互不影响）。
//!
//! 全量打包（无勾选）：含 picky 数据库（含其云同步版本号 lastSyncAt）、剪贴板图片、
//! mihomo 配置、自定义字体、环境备份目录等所有数据。
//! 刻意排除：`sdk/`（体积大，另行管理）、`version_cache/`（可重建的缓存）、
//! mihomo 的 geo/日志/内核（可重新下载）。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};


use crate::commands::config::{get_base_dir, get_data_dir};
use crate::commands::secrets::{decrypt_secret, encrypt_secret};

/// 快照打包的固定文件（相对 data_dir；config.json 例外，始终位于 base_dir）。
const MANAGED_FILES: &[&str] = &[
    "config.json",
    "backups.json",
    "ai_config.json",
    "ai_sessions.json",
    "last_launch_configs.json",
    "collab.json",
    "skills.json",
    "mcp.json",
    "translate_config.json",
    "sync_state_config.json",
    "tasks.db",
    "launcher.db",
    "ai_usage.db",
    "api.db",
    "mindmap.db",
    "otp/otp.db",
    "clipboard/clipboard.db",
    "picky/picky.db",
];

/// 快照打包的整目录（相对 data_dir，递归收集其下所有文件）。
const MANAGED_DIRS: &[&str] = &[
    "certs",
    "clipboard/images",
    "backup",
    "fonts",
    "mihomo/profiles",
    "mihomo/override",
];

/// 快照打包的 mihomo 配置文件（用户订阅/代理配置，不包含可重新下载的 geo/日志/内核）。
const MIHOMO_FILES: &[&str] = &[
    "mihomo/app.json",
    "mihomo/controled.json",
    "mihomo/profile.json",
    "mihomo/override.json",
];


// ─── S3 同步配置（独立于 Picky） ───

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StateSyncConfig {
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
    #[serde(default = "default_style")]
    pub addressing_style: String,
    #[serde(default = "default_true")]
    pub tls_verify: bool,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub last_sync_at: Option<String>,
    // —— 打包范围（可选目录开关，默认全部包含） ——
    #[serde(default = "default_true")]
    pub include_clipboard_images: bool,
    #[serde(default = "default_true")]
    pub include_mihomo: bool,
    #[serde(default = "default_true")]
    pub include_fonts: bool,
    #[serde(default = "default_true")]
    pub include_backup: bool,
}

fn default_region() -> String { "us-east-1".to_string() }
fn default_style() -> String { "auto".to_string() }
fn default_true() -> bool { true }
fn default_timeout() -> u64 { 30 }

/// 落盘结构：secret 加密存储。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct StoredConfig {
    endpoint: Option<String>,
    #[serde(default)]
    region: String,
    #[serde(default)]
    access_key_id: String,
    #[serde(default)]
    secret_access_key_enc: String,
    #[serde(default)]
    bucket_name: String,
    #[serde(default)]
    prefix: Option<String>,
    #[serde(default = "default_style")]
    addressing_style: String,
    #[serde(default = "default_true")]
    tls_verify: bool,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    last_sync_at: Option<String>,
    // —— 打包范围（可选目录开关，默认全部包含） ——
    #[serde(default = "default_true")]
    include_clipboard_images: bool,
    #[serde(default = "default_true")]
    include_mihomo: bool,
    #[serde(default = "default_true")]
    include_fonts: bool,
    #[serde(default = "default_true")]
    include_backup: bool,
}

fn sync_config_path() -> PathBuf {
    get_data_dir().join("sync_state_config.json")
}

fn load_stored_config() -> StoredConfig {
    let path = sync_config_path();
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str::<StoredConfig>(&data) {
                return cfg;
            }
        }
    }
    StoredConfig::default()
}

fn save_stored_config(cfg: &StoredConfig) -> Result<(), String> {
    let path = sync_config_path();
    let data = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, data).map_err(|e| format!("写入同步配置失败: {}", e))
}

impl StateSyncConfig {
    fn to_stored(&self) -> Result<StoredConfig, String> {
        Ok(StoredConfig {
            endpoint: self.endpoint.clone(),
            region: self.region.clone(),
            access_key_id: self.access_key_id.clone(),
            secret_access_key_enc: encrypt_secret(&self.secret_access_key)?,
            bucket_name: self.bucket_name.clone(),
            prefix: self.prefix.clone(),
            addressing_style: self.addressing_style.clone(),
            tls_verify: self.tls_verify,
            timeout_seconds: self.timeout_seconds,
            enabled: self.enabled,
            last_sync_at: self.last_sync_at.clone(),
            include_clipboard_images: self.include_clipboard_images,
            include_mihomo: self.include_mihomo,
            include_fonts: self.include_fonts,
            include_backup: self.include_backup,
        })
    }
}

#[tauri::command]
pub fn state_sync_get_config() -> Result<StateSyncConfig, String> {
    let stored = load_stored_config();
    Ok(StateSyncConfig {
        endpoint: stored.endpoint,
        region: stored.region,
        access_key_id: stored.access_key_id,
        secret_access_key: decrypt_secret(&stored.secret_access_key_enc)?,
        bucket_name: stored.bucket_name,
        prefix: stored.prefix,
        addressing_style: stored.addressing_style,
        tls_verify: stored.tls_verify,
        timeout_seconds: stored.timeout_seconds,
        enabled: stored.enabled,
        last_sync_at: stored.last_sync_at,
        include_clipboard_images: stored.include_clipboard_images,
        include_mihomo: stored.include_mihomo,
        include_fonts: stored.include_fonts,
        include_backup: stored.include_backup,
    })
}

#[tauri::command]
pub fn state_sync_save_config(config: StateSyncConfig) -> Result<(), String> {
    save_stored_config(&config.to_stored()?)
}

// ─── 快照构建 / 恢复 ───

fn collect_dir_files(data_dir: &Path, rel_dir: &str) -> Vec<String> {
    let root = data_dir.join(rel_dir);
    let mut out = Vec::new();
    if !root.is_dir() {
        return out;
    }
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    if let Ok(rel) = path.strip_prefix(data_dir) {
                        out.push(rel.to_string_lossy().to_string());
                    }
                }
            }
        }
    }
    out
}

/// config.json 是数据入口，固定留在 base_dir（即使 data_dir 被改到其它盘）。
/// 其余文件/目录都在 data_dir 下。
fn resolve_snapshot_path(rel: &str) -> PathBuf {
    if rel == "config.json" {
        get_base_dir().join(rel)
    } else {
        get_data_dir().join(rel)
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// 构建统一快照（JSON）：{ version, type, createdAt, files: { relPath: base64 } }。
/// 全量打包：固定文件 + mihomo 配置 + 全部管理目录（剪贴板图片/mihomo/字体/环境备份/certs），
/// 以及 picky 数据库（含其云同步版本号 lastSyncAt）——不做任何勾选过滤。
fn build_snapshot() -> Result<JsonValue, String> {
    build_snapshot_with_progress(|_, _| {})
}

/// 打包快照；`on_progress` 在读取每个文件后回调（参数为 (当前已处理文件数, 文件总数)），
/// 供前端展示进度。
fn build_snapshot_with_progress(mut on_progress: impl FnMut(usize, usize)) -> Result<JsonValue, String> {
    let data_dir = get_data_dir();
    let mut files = JsonMap::new();
    let mut count = 0usize;
    let mut total = 0usize;

    let mut rels: Vec<String> = MANAGED_FILES.iter().map(|s| s.to_string()).collect();
    rels.extend(MIHOMO_FILES.iter().map(|s| s.to_string()));
    for dir in MANAGED_DIRS {
        rels.extend(collect_dir_files(&data_dir, dir));
    }
    // WAL 兼容：SQLite 主库存在同名 `-wal` 时一并打包（WAL 模式下最新数据在 wal 文件里，
    // 只拷主库会丢失最近写入；恢复时一并写回，SQLite 会自动合并）。
    let mut rels_with_wal: Vec<String> = Vec::with_capacity(rels.len());
    for rel in rels {
        rels_with_wal.push(rel.clone());
        if rel.ends_with(".db") {
            let wal_rel = format!("{}-wal", rel);
            if resolve_snapshot_path(&wal_rel).is_file() {
                rels_with_wal.push(wal_rel);
            }
        }
    }
    let rels = rels_with_wal;

    let total_files = rels.len().max(1);
    for rel in rels {
        let path = resolve_snapshot_path(&rel);
        if !path.is_file() {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("读取 {} 失败: {}", rel, e))?;
        total += bytes.len();
        files.insert(rel, JsonValue::String(general_purpose::STANDARD.encode(&bytes)));
        count += 1;
        on_progress(count.min(total_files), total_files);
    }

    Ok(serde_json::json!({
        "version": 1,
        "type": "any-version-state",
        "createdAt": now_iso(),
        "fileCount": count,
        "sizeBytes": total,
        "files": files,
    }))
}

/// 校验相对路径安全（不允许绝对路径 / 越界 ..）。
fn is_safe_rel(rel: &str) -> bool {
    !rel.is_empty()
        && !Path::new(rel).is_absolute()
        && !rel.split(['/', '\\']).any(|c| c == "..")
}

/// 从快照 JSON 恢复文件，返回恢复的文件数。
/// `selected` 为 Some 时只恢复其中列出的文件（部分恢复），None 表示恢复全部。
fn restore_snapshot(
    state: &JsonValue,
    selected: Option<&std::collections::HashSet<String>>,
) -> Result<usize, String> {
    if state.get("type").and_then(|v| v.as_str()) != Some("any-version-state") {
        return Err("不是有效的 any-version 数据快照（type 不匹配）".to_string());
    }
    let files = state
        .get("files")
        .and_then(|v| v.as_object())
        .ok_or_else(|| "快照缺少 files 字段".to_string())?;

    let mut restored = 0usize;
    for (rel, val) in files {
        let rel_str = rel.as_str();
        if let Some(sel) = selected {
            if !sel.contains(rel_str) {
                continue;
            }
        }
        if !is_safe_rel(rel_str) {
            return Err(format!("快照包含非法路径，已中止: {}", rel_str));
        }
        let b64 = val.as_str().ok_or_else(|| format!("快照文件 {} 内容非法", rel_str))?;
        let bytes = general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| format!("快照文件 {} 解码失败: {}", rel_str, e))?;
        let path = resolve_snapshot_path(rel_str);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败 {}: {}", parent.display(), e))?;
        }
        std::fs::write(&path, &bytes).map_err(|e| format!("写入 {} 失败: {}", rel_str, e))?;
        restored += 1;
    }
    Ok(restored)
}

// ─── 命令：本地导出 / 导入 ───

/// gzip 压缩字节（gzip 头 1f 8b，便于导入时自动识别）。
fn gzip_bytes(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).map_err(|e| format!("压缩快照失败: {}", e))?;
    encoder.finish().map_err(|e| format!("压缩快照收尾失败: {}", e))
}

/// 若为 gzip（magic 1f 8b）则解压，否则原样返回（兼容旧版未压缩的 JSON 快照）。
fn gunzip_if_needed(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b {
        let mut decoder = flate2::read::GzDecoder::new(data);
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).map_err(|e| format!("解压快照失败: {}", e))?;
        Ok(out)
    } else {
        Ok(data.to_vec())
    }
}

/// 读取快照字节并解析为 JSON（自动兼容 gzip 压缩与旧版明文 JSON）。
fn parse_snapshot(data: &[u8]) -> Result<JsonValue, String> {
    let raw = gunzip_if_needed(data)?;
    serde_json::from_slice(&raw).map_err(|e| format!("解析快照失败: {}", e))
}

/// 构建快照并导出到用户选择的路径（gzip 压缩）。
#[tauri::command]
pub fn state_sync_export(target_path: String) -> Result<serde_json::Value, String> {
    let snapshot = build_snapshot()?;
    let raw = serde_json::to_vec(&snapshot).map_err(|e| format!("序列化快照失败: {}", e))?;
    let compressed = gzip_bytes(&raw)?;
    let path = PathBuf::from(&target_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败 {}: {}", parent.display(), e))?;
    }
    std::fs::write(&path, &compressed).map_err(|e| format!("写入快照文件失败: {}", e))?;

    Ok(serde_json::json!({
        "path": path.to_string_lossy().to_string(),
        "fileCount": snapshot.get("fileCount").cloned().unwrap_or_default(),
        "sizeBytes": snapshot.get("sizeBytes").cloned().unwrap_or_default(),
        "compressedBytes": compressed.len(),
        "createdAt": snapshot.get("createdAt").cloned().unwrap_or_default(),
    }))
}

/// 从本地快照文件恢复。`files` 为 Some 时只恢复列出的文件（部分恢复），None 表示恢复全部。
#[tauri::command]
pub fn state_sync_import(path: String, files: Option<Vec<String>>) -> Result<String, String> {
    let p = PathBuf::from(&path);
    if !p.is_file() {
        return Err(format!("快照文件不存在: {}", path));
    }
    let data = std::fs::read(&p).map_err(|e| format!("读取快照失败: {}", e))?;
    let state = parse_snapshot(&data)?;
    let selection = files.map(|v| v.into_iter().collect::<std::collections::HashSet<_>>());
    let restored = restore_snapshot(&state, selection.as_ref())?;
    Ok(format!("恢复完成：已还原 {} 个配置文件/数据库（{}）", restored, path))
}

// ─── 命令：S3 同步（独立于 Picky） ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gzip_roundtrip() {
        let data = br#"{"hello":"world","files":{"a":"AAAA"}}"#;
        let gz = gzip_bytes(data).unwrap();
        // gzip magic
        assert!(gz.len() >= 2 && gz[0] == 0x1f && gz[1] == 0x8b);
        let back = gunzip_if_needed(&gz).unwrap();
        assert_eq!(back, data);
        // 明文快照原样通过（兼容旧版）
        let plain = gunzip_if_needed(data).unwrap();
        assert_eq!(plain, data);
    }

    #[test]
    fn safe_rel_rejects_traversal() {
        assert!(is_safe_rel("config.json"));
        assert!(is_safe_rel("mihomo/profiles/a.yaml"));
        assert!(is_safe_rel("clipboard/images/thumb/1.png"));
        assert!(!is_safe_rel("../config.json"));
        assert!(!is_safe_rel("..\\config.json"));
        assert!(!is_safe_rel("C:\\abs\\config.json"));
        assert!(!is_safe_rel("\\\\server\\share\\x"));
        assert!(!is_safe_rel(""));
    }
}

