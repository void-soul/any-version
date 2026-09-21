//! 增量 + 容错的 JSONL 会话扫描。
//!
//! 参考 cc-switch 的两个 Codex rollout 修复（我们的对应物是 Codex / Reasonix 的 JSONL 会话扫描）：
//! - `fix(codex): skip unchanged incomplete rollout tails`
//!   → [`read_lines_tolerant`]：会话文件是边生成边追加的，扫描时最后一行很可能是半截 JSON，
//!     而且用 `fs::read_to_string` 遇到非法 UTF-8 会**整个文件失败**（会话直接从列表里消失）。
//! - `fix(codex): detect growing rollouts with a persisted byte cursor`
//!   → [`ScanCache`]：文件指纹（大小 + mtime）没变就复用上一轮解析结果，省掉整份重读。
//!
//! 与参考实现的差别：参考把游标落盘跨重启复用；这里只做**进程内**缓存，
//! 避免多一份需要迁移 / 失效判断的文件格式（重启后第一次扫描仍是全量，属可接受代价）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use super::models::ToolSession;

/// 文件指纹：大小 + 修改时间（毫秒）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStamp {
    pub size: u64,
    pub mtime_ms: i64,
}

/// 读取文件指纹；文件不存在 / 无权限时返回 None（调用方跳过该文件）。
pub fn stamp_of(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Some(FileStamp {
        size: meta.len(),
        mtime_ms,
    })
}

/// 逐行读取 JSONL，并**丢掉正在写的那一行**。
///
/// - 按字节读：`fs::read_to_string` 遇到半截 UTF-8 会整个失败；
/// - `from_utf8_lossy`：坏字节替换成 U+FFFD，保住其余内容；
/// - 只切到最后一个换行符：末尾没有换行的那一行视为「写了一半」，不参与解析。
///
/// 返回 None 表示文件读不到（不存在 / 无权限）；文件为空返回空 Vec。
pub fn read_lines_tolerant(path: &Path) -> Option<Vec<String>> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() {
        return Some(Vec::new());
    }
    let complete_end = if bytes.last() == Some(&b'\n') {
        bytes.len()
    } else {
        // 末尾没有换行 → 最后一行是半截，切到它之前
        bytes
            .iter()
            .rposition(|b| *b == b'\n')
            .map(|pos| pos + 1)
            .unwrap_or(0)
    };
    let text = String::from_utf8_lossy(&bytes[..complete_end]);
    Some(text.lines().map(|line| line.to_string()).collect())
}

/// 进程内扫描缓存：按 (路径, 指纹) 复用上次解析结果。
///
/// 会话文件只会在追加时变化，指纹不变即内容未变，可直接复用上一轮解析出的
/// `ToolSession`，避免整份重读（大历史目录下这是扫描的主要开销）。
#[derive(Default)]
pub struct ScanCache {
    entries: HashMap<PathBuf, (FileStamp, Vec<ToolSession>)>,
}

impl ScanCache {
    /// 命中（同一路径且指纹一致）时返回上次的解析结果。
    pub fn get(&self, path: &Path, stamp: &FileStamp) -> Option<Vec<ToolSession>> {
        match self.entries.get(path) {
            Some((cached, sessions)) if cached == stamp => Some(sessions.clone()),
            _ => None,
        }
    }

    pub fn put(&mut self, path: &Path, stamp: FileStamp, sessions: Vec<ToolSession>) {
        self.entries.insert(path.to_path_buf(), (stamp, sessions));
    }

    /// 清理指向已删除文件的条目（长期运行 + 会话被删时避免只增不减）。
    pub fn prune_missing(&mut self) -> usize {
        let before = self.entries.len();
        self.entries.retain(|path, _| path.exists());
        before - self.entries.len()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

static CACHE: OnceLock<Mutex<ScanCache>> = OnceLock::new();

/// 在全局扫描缓存上执行一段扫描逻辑（进程内单例）。
///
/// 锁中毒（持锁线程 panic）时退回临时缓存：扫描功能本身不能因此挂掉，
/// 代价只是这一轮失去缓存加速。
pub fn with_cache<T>(f: impl FnOnce(&mut ScanCache) -> T) -> T {
    let mutex = CACHE.get_or_init(|| Mutex::new(ScanCache::default()));
    match mutex.lock() {
        Ok(mut guard) => f(&mut guard),
        Err(_) => f(&mut ScanCache::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::{read_lines_tolerant, stamp_of, FileStamp, ScanCache};
    use crate::commands::ai::models::ToolSession;
    use std::path::PathBuf;

    fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("anyver-jsonl-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn session(id: &str) -> ToolSession {
        ToolSession {
            session_id: id.to_string(),
            project_path: "/tmp/p".to_string(),
            last_used: "2026-09-21 10:00:00".to_string(),
            summary: None,
            resume_cmd: None,
        }
    }

    /// 正在追加写的最后一行必须被丢掉，前面的完整行要保留。
    #[test]
    fn drops_torn_last_line() {
        let path = temp_file("torn", b"{\"a\":1}\n{\"b\":2}\n{\"c\":");
        let lines = read_lines_tolerant(&path).expect("readable");
        assert_eq!(lines.len(), 2, "半截的最后一行不应参与解析: {:?}", lines);
        assert_eq!(lines[1], "{\"b\":2}");

        // 收尾换行写完后，最后一行才出现
        std::fs::write(&path, b"{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n").unwrap();
        assert_eq!(read_lines_tolerant(&path).unwrap().len(), 3);
    }

    /// 半截 UTF-8（多字节字符被切断）不能让整份文件失败。
    #[test]
    fn tolerates_invalid_utf8_instead_of_losing_the_file() {
        let path = temp_file("badutf8", b"{\"a\":\"ok\"}\n{\"b\":\"\xe4\xb8\"}\n");
        let lines = read_lines_tolerant(&path).expect("读不到文件会丢掉整个会话列表");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "{\"a\":\"ok\"}");
    }

    /// 只有半行 → 没有任何可解析内容（但不能是 None，否则调用方会以为是 I/O 失败）。
    #[test]
    fn file_without_newline_yields_nothing() {
        let path = temp_file("noline", b"{\"partial\":");
        assert_eq!(read_lines_tolerant(&path).unwrap(), Vec::<String>::new());
    }

    /// 指纹不变复用缓存；文件变化后必须重新解析。
    #[test]
    fn cache_reuses_result_until_the_file_changes() {
        let path = temp_file("cache", b"{}\n");
        let stamp = stamp_of(&path).expect("stamp");
        let mut cache = ScanCache::default();
        assert!(cache.get(&path, &stamp).is_none(), "初次未缓存");
        cache.put(&path, stamp.clone(), vec![session("s1")]);
        assert_eq!(cache.get(&path, &stamp).unwrap()[0].session_id, "s1");

        // 指纹不同（例如追加了新事件）→ 不命中，必须重读
        let changed = FileStamp {
            size: stamp.size + 1,
            mtime_ms: stamp.mtime_ms + 1,
        };
        assert!(cache.get(&path, &changed).is_none());

        assert_eq!(cache.len(), 1);
        assert!(!cache.is_empty());
    }

    /// 会话文件被删除后，缓存条目应被清掉。
    #[test]
    fn prune_drops_entries_for_removed_files() {
        let path = temp_file("prune", b"{}\n");
        let stamp = stamp_of(&path).unwrap();
        let mut cache = ScanCache::default();
        cache.put(&path, stamp, vec![session("s1")]);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(cache.prune_missing(), 1);
        assert!(cache.is_empty());
    }
}
