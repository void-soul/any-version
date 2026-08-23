//! collab.json 数据模型持久化：路径、读写、ID/时间戳生成。
//! 与 collab.rs 为父子模块关系：类型定义在父模块，本模块只负责存储层。

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use parking_lot::Mutex;
use crate::commands::config::get_data_dir;

use super::CollabStore;

fn collab_path() -> PathBuf {
    get_data_dir().join("collab.json")
}

/// collab.json 的 schema 版本；读取即打上当前版本，便于将来迁移
const CURRENT_STORE_VERSION: u32 = 1;

pub(crate) fn load_store() -> CollabStore {
    let path = collab_path();
    if path.exists() {
        match fs::read_to_string(&path) {
            Ok(data) => match serde_json::from_str::<CollabStore>(&data) {
                Ok(mut store) => {
                    store.version = CURRENT_STORE_VERSION;
                    return store;
                }
                Err(e) => {
                    // 解析失败：保留损坏文件并备份，避免静默丢弃数据（运维可据此恢复）
                    eprintln!("[collab] ⚠ collab.json 解析失败，已备份原文件为 .corrupt: {}", e);
                    let _ = fs::copy(&path, path.with_extension("json.corrupt"));
                }
            },
            Err(e) => eprintln!("[collab] ⚠ 读取 collab.json 失败: {}", e),
        }
    }
    let mut s = CollabStore::default();
    s.version = CURRENT_STORE_VERSION;
    s
}

pub(crate) fn save_store(store: &CollabStore) -> Result<(), String> {
    let path = collab_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let data = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    // 原子写：先写临时文件再 rename，避免写入中途崩溃导致 collab.json 损坏/数据丢失
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &data).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/// 保护 collab.json 的读-改-写临界区：所有 load_store + 修改 + save_store 必须在该锁内完成，
/// 避免并发派发（不同房间）互相覆盖导致丢失更新。
pub(crate) static STORE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);
pub(crate) fn new_id() -> String {
    let n = ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    let ts = chrono::Local::now().timestamp_millis();
    format!("m{}_{}", ts, n)
}

pub(crate) fn now_str() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// 兼容的 updated_at 比较：约定由 now_str() 生成（"%Y-%m-%dT%H:%M:%S"，字典序即时间序），
/// 同时兼容 RFC3339（含时区偏移）解析比较，任一解析失败时回退字典序。返回降序（新→旧）。
pub(crate) fn cmp_updated_at(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| chrono::DateTime::parse_from_rfc3339(s).map(|d| d.timestamp()).ok();
    match (parse(a), parse(b)) {
        (Some(x), Some(y)) => y.cmp(&x),
        _ => b.cmp(a),
    }
}
