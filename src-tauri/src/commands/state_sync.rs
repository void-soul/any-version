//! any-version 统一数据配置备份与同步。
//!
//! 各模块平时仍使用各自的存储（config.json / *.db 等），但备份层面统一为
//! **一个快照文件**：按 storage_assets 的模块资产声明打包配置与数据进单个 JSON
//! （文件内容 base64），本地导出时再整体 gzip 压缩，可导入/导出，也可同步到
//! S3（独立于 Picky 的同步，使用自己的 S3 配置与对象 key，互不影响）。
//!
//! 完整打包已声明模块的可备份数据，包括 Buddy、Suno、设计器会话、任务图片、
//! picky 数据库、剪贴板图片、mihomo 配置、自定义字体和环境备份。
//! 刻意排除：`sdk/`（体积大，另行管理）、`version_cache/`（可重建的缓存）、
//! mihomo 的 geo/日志/内核（可重新下载）。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};


use crate::commands::config::{get_base_dir, get_data_dir};
use crate::commands::secrets::{decrypt_secret, encrypt_secret};

use super::storage_assets;
use super::storage_maintenance;


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

fn collect_dir_files(data_dir: &Path, rel_dir: &str) -> Result<Vec<String>, String> {
    let root = data_dir.join(rel_dir);
    let mut out = Vec::new();
    if !root.is_dir() {
        return Ok(out);
    }
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        storage_maintenance::checked_target(data_dir, &dir.strip_prefix(data_dir).map_err(|e| e.to_string())?.to_string_lossy())?;
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("读取备份目录失败 {}: {e}", dir.display()))?;
            for entry in entries {
                let entry = entry.map_err(|e| e.to_string())?;
                let path = entry.path();
                let rel = path.strip_prefix(data_dir).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
                storage_maintenance::checked_target(data_dir, &rel)?;
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    if let Ok(rel) = path.strip_prefix(data_dir) {
                        let rel = rel.to_string_lossy().replace('\\', "/");
                        if !rel.ends_with(".db-wal") && !rel.ends_with(".db-shm") && !rel.ends_with(".vscdb-wal") && !rel.ends_with(".vscdb-shm") { out.push(rel); }
                    }
                }
            }
    }
    Ok(out)
}

/// config.json 是数据入口，固定留在 base_dir（即使 data_dir 被改到其它盘）。
/// 其余文件/目录都在 data_dir 下。
fn resolve_snapshot_path(rel: &str) -> PathBuf {
    // 只有 config.json 是数据入口，固定留在 base_dir；
    // settings.json（业务选项）与其它数据一样在 data_dir 下。
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

    let mut rels: Vec<String> = std::iter::once("config.json").chain(storage_assets::snapshot_files()).map(str::to_string).collect();
    for dir in storage_assets::snapshot_directories() {
        rels.extend(collect_dir_files(&data_dir, dir)?);
    }
    rels.sort(); rels.dedup();

    let total_files = rels.len().max(1);
    for rel in rels {
        let path = resolve_snapshot_path(&rel);
        if !path.is_file() {
            continue;
        }
        let root = if rel == "config.json" { get_base_dir() } else { data_dir.clone() };
        storage_maintenance::checked_target(&root, &rel)?;
        if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > storage_maintenance::MAX_SNAPSHOT_BYTES as u64 {
            return Err(format!("备份文件过大，请先单独备份: {rel}"));
        }
        let bytes = storage_maintenance::snapshot_file_bytes(&path)?;
        if total.checked_add(bytes.len()).is_none_or(|size| size > storage_maintenance::MAX_SNAPSHOT_BYTES) {
            return Err("备份数据超过1 GiB，请分批备份".into());
        }
        total += bytes.len();
        files.insert(rel, JsonValue::String(general_purpose::STANDARD.encode(&bytes)));
        count += 1;
        on_progress(count.min(total_files), total_files);
    }

    Ok(serde_json::json!({
        "version": 2,
        "type": "any-version-state",
        "createdAt": now_iso(),
        "fileCount": count,
        "sizeBytes": total,
        "files": files,
        "modules": storage_assets::MODULE_ASSETS.iter().map(|m| m.module).collect::<Vec<_>>(),
        "excluded": ["SDK installations", "Node project working directories", "rebuildable caches and runtime binaries", "music plugin dependencies and bundled bridge"],
        "containsCredentials": true,
    }))
}

/// 校验相对路径安全（不允许绝对路径 / 越界 ..）。
fn is_safe_rel(rel: &str) -> bool {
    storage_maintenance::normalize_relative_path(rel).is_ok()
}

/// 从快照 JSON 恢复文件，返回恢复的文件数。
/// `selected` 为 Some 时只恢复其中列出的文件（部分恢复），None 表示恢复全部。
fn restore_snapshot(
    state: &JsonValue,
    selected: Option<&std::collections::HashSet<String>>,
) -> Result<usize, String> {
    storage_maintenance::stage_restore(&get_base_dir(), &get_data_dir(), state, selected)
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
        decoder.take(storage_maintenance::MAX_SNAPSHOT_BYTES as u64 + 1).read_to_end(&mut out).map_err(|e| format!("解压快照失败: {}", e))?;
        if out.len() > storage_maintenance::MAX_SNAPSHOT_BYTES { return Err("快照解压后超过1 GiB，请分批导入".into()); }
        Ok(out)
    } else {
        if data.len() > storage_maintenance::MAX_SNAPSHOT_BYTES { return Err("快照超过1 GiB，请分批导入".into()); }
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
    if raw.len() > storage_maintenance::MAX_SNAPSHOT_BYTES {
        return Err("备份超过大小上限，请先单独归档大型文件后重试".into());
    }
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
    if std::fs::metadata(&p).map_err(|e| e.to_string())?.len() > storage_maintenance::MAX_SNAPSHOT_BYTES as u64 {
        return Err("快照超过1 GiB，请分批导入".into());
    }
    let data = std::fs::read(&p).map_err(|e| format!("读取快照失败: {}", e))?;
    let state = parse_snapshot(&data)?;
    let selection = files.map(|v| v.into_iter().collect::<std::collections::HashSet<_>>());
    let restored = restore_snapshot(&state, selection.as_ref())?;
    Ok(format!("已校验并暂存 {} 个配置文件/数据库，重启后应用；当前数据未覆盖，本机路径设置保留", restored))
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

    /// 只有 config.json 是入口（base_dir）；settings.json 跟着数据目录走。
    #[test]
    fn entry_config_resolves_to_base_dir() {
        let base = get_base_dir();
        assert_eq!(resolve_snapshot_path("config.json"), base.join("config.json"));
        assert_eq!(
            resolve_snapshot_path("settings.json"),
            get_data_dir().join("settings.json")
        );
        assert_eq!(
            resolve_snapshot_path("favorites.db"),
            get_data_dir().join("favorites.db")
        );
    }

    /// 全量导出必须覆盖「除数据路径以外的选项」，否则导入后设置会静默回默认。
    #[test]
    fn managed_files_cover_settings_and_all_databases() {
        for must in [
            "config.json",
            "settings.json",
            "backups.json",
            "favorites.db",
            "mindmap.db",
            "api.db",
            "picky/picky.db",
        ] {
            assert!(storage_assets::allows_snapshot_path(must), "快照清单缺少 {}", must);
        }
    }

    /// 音乐插件要进快照（那是用户的音源配置），但**可重建的依赖**与**随 app 释放的桥**
    /// 不能进：前者让快照白白胖 10 MB，后者会让恢复出来的桥版本与 app 对不上。
    #[test]
    fn plugins_are_archived_but_not_their_dependencies() {
        assert!(storage_assets::snapshot_files().any(|p| p == "music/plugins/registry.json"));
        assert!(storage_assets::snapshot_directories().any(|p| p == "music/plugins/scripts"));

        let all: Vec<&str> = storage_assets::snapshot_files().chain(storage_assets::snapshot_directories()).collect();
        for forbidden in [
            "music/plugins",
            "music/plugins/node_modules",
            "music/plugins/bridge.js",
            "music/plugins/package.json",
        ] {
            assert!(
                !all.contains(&forbidden),
                "快照清单不该含 {forbidden}（会带上可重建的依赖或旧版桥）"
            );
        }
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
        assert!(!is_safe_rel("\\outside\\x"));
        assert!(!is_safe_rel("C:relative"));
        assert!(!is_safe_rel(""));
    }
}

