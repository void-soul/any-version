//! 流量统计落盘（对齐 clash-party：主进程 worker 线程采样 + 落库）。
//!
//! 之前的统计跑在**渲染进程**的 IndexedDB 里：切到别的页面、或关掉窗口就不再采样，
//! 历史曲线一段一段地缺。这里下沉到 Rust 侧常驻任务，每 2 秒从内核 `/traffic`
//! 取一次累计值写 SQLite，页面只负责查。
//!
//! 只存**累计值**（up/down 单调增），查询时再算差值：
//! - 差值天然对齐「这一段时间用了多少」，不受采样点是否在区间边界上影响；
//! - 内核重启会让累计值归零 → 差值变负，直接丢弃这一段（不能记成负流量）。

use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// 采样保留时长（30 天）
const RETENTION_SECS: u64 = 30 * 24 * 3600;
/// 采样间隔（秒）
pub const SAMPLE_INTERVAL_SECS: u64 = 2;

static RECORDER_RUNNING: AtomicBool = AtomicBool::new(false);
static RECORDER_GUARD: Mutex<()> = Mutex::new(());

pub fn recorder_running() -> bool {
    RECORDER_RUNNING.load(Ordering::SeqCst)
}

fn db_file(data_dir: &Path) -> PathBuf {
    data_dir.join("traffic.db")
}

fn open(data_dir: &Path) -> Result<Connection, String> {
    let conn = Connection::open(db_file(data_dir))
        .map_err(|e| format!("打开流量数据库失败: {e}"))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS traffic_sample (
            ts INTEGER PRIMARY KEY,
            up INTEGER NOT NULL,
            down INTEGER NOT NULL
         );",
    )
    .map_err(|e| format!("建表失败: {e}"))?;
    Ok(conn)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// 写入一个采样点（累计值）。
pub fn record_sample(data_dir: &Path, up: i64, down: i64) -> Result<(), String> {
    let conn = open(data_dir)?;
    let now = now_secs();
    conn.execute(
        "INSERT OR REPLACE INTO traffic_sample (ts, up, down) VALUES (?1, ?2, ?3)",
        params![now as i64, up, down],
    )
    .map_err(|e| format!("写入采样失败: {e}"))?;
    // 顺手裁剪（每秒最多一次 DELETE，成本可忽略）
    let cutoff = now.saturating_sub(RETENTION_SECS) as i64;
    let _ = conn.execute("DELETE FROM traffic_sample WHERE ts < ?1", params![cutoff]);
    Ok(())
}

/// 聚合后的一个时间桶（毫秒时间戳 + 该桶内的增量字节数）
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrafficPoint {
    pub ts_ms: u64,
    pub upload: u64,
    pub download: u64,
}

/// 按桶聚合 `since` 之后的流量；同时返回区间总量。
///
/// `bucket_secs` 会被夹到 [1, 86400]。
pub fn query(
    data_dir: &Path,
    since: u64,
    bucket_secs: u64,
) -> Result<(Vec<TrafficPoint>, u64, u64), String> {
    let bucket = bucket_secs.clamp(1, 86400);
    let conn = open(data_dir)?;
    let mut stmt = conn
        .prepare("SELECT ts, up, down FROM traffic_sample WHERE ts >= ?1 ORDER BY ts ASC")
        .map_err(|e| format!("查询失败: {e}"))?;
    let rows = stmt
        .query_map(params![since as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })
        .map_err(|e| format!("遍历失败: {e}"))?;

    let mut points: Vec<TrafficPoint> = Vec::new();
    let mut total_up: u64 = 0;
    let mut total_down: u64 = 0;
    let mut prev: Option<(i64, i64, i64)> = None;
    for row in rows {
        let (ts, up, down) = match row {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some((_pts, pup, pdown)) = prev {
            let du = up.saturating_sub(pup);
            let dd = down.saturating_sub(pdown);
            // 累计值倒退 = 内核重启（或换核心），这一段不算流量
            if du >= 0 && dd >= 0 && ts > _pts {
                total_up += du as u64;
                total_down += dd as u64;
                let bucket_start = (ts as u64 / bucket) * bucket;
                match points.last_mut() {
                    Some(p) if p.ts_ms / 1000 == bucket_start => {
                        p.upload += du as u64;
                        p.download += dd as u64;
                    }
                    _ => points.push(TrafficPoint {
                        ts_ms: bucket_start * 1000,
                        upload: du as u64,
                        download: dd as u64,
                    }),
                }
            }
        }
        prev = Some((ts, up, down));
    }
    Ok((points, total_up, total_down))
}

/// 清空全部历史采样。
pub fn clear(data_dir: &Path) -> Result<(), String> {
    let conn = open(data_dir)?;
    conn.execute("DELETE FROM traffic_sample", [])
        .map_err(|e| format!("清空失败: {e}"))?;
    Ok(())
}

/// 采样任务：每 `SAMPLE_INTERVAL_SECS` 秒取一次内核累计流量并落盘。
///
/// 只在「核心没停」时采集；核心停了就空转等它起来（避免退出时序上抢锁）。
/// 用 `RECORDER_GUARD` + `RECORDER_RUNNING` 保证全局只有一个采样任务。
pub fn start_recorder(inner: std::sync::Arc<super::MihomoInner>) {
    {
        let _g = match RECORDER_GUARD.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if RECORDER_RUNNING.load(Ordering::SeqCst) {
            return;
        }
        RECORDER_RUNNING.store(true, Ordering::SeqCst);
    }
    tauri::async_runtime::spawn(async move {
        loop {
            if inner.stop_flag.load(Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_secs(SAMPLE_INTERVAL_SECS)).await;
                continue;
            }
            let (cfg, data_dir) = {
                let c = inner.app_config.lock().unwrap().clone();
                (c, inner.data_dir.clone())
            };
            if let Ok(text) = super::api::mihomo_api_raw(
                &cfg,
                reqwest::Method::GET,
                "/traffic",
                None,
            )
            .await
            {
                let v: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
                let up = v.get("up").and_then(|x| x.as_i64()).unwrap_or(0);
                let down = v.get("down").and_then(|x| x.as_i64()).unwrap_or(0);
                let total = up + down;
                if total > 0 {
                    if let Err(e) = record_sample(&data_dir, up, down) {
                        eprintln!("[mihomo] 流量采样落盘失败: {e}");
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(SAMPLE_INTERVAL_SECS)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{clear, query, record_sample, TrafficPoint};
    use std::path::PathBuf;

    fn tmpdir(name: &str) -> PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!("anyver-traffic-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 累计值 → 按桶差值；内核重启（累计值归零）的那一段必须被丢掉，
    /// 否则会算出巨大的「负流量」或把旧值当新增。
    #[test]
    fn query_aggregates_deltas_and_drops_core_restarts() {
        let dir = tmpdir("query");
        let base = 1_700_000_000u64;
        // 手工写入：ts 用「相对 base」的秒，模拟连续采样
        for (i, (up, down)) in [
            (100i64, 200i64),
            (150, 260),
            (400, 900),
            // 内核重启：累计值归零后重新计数
            (10, 20),
            (60, 80),
        ]
        .into_iter()
        .enumerate()
        {
            let ts = base + (i as u64) * 2;
            let conn = rusqlite::Connection::open(dir.join("traffic.db")).unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS traffic_sample (ts INTEGER PRIMARY KEY, up INTEGER NOT NULL, down INTEGER NOT NULL);",
            )
            .unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO traffic_sample (ts, up, down) VALUES (?1, ?2, ?3)",
                rusqlite::params![ts as i64, up, down],
            )
            .unwrap();
        }
        let (points, up_total, down_total) = query(&dir, base, 60).unwrap();
        // 有效增量：(150-100)+(400-150) + (60-10) = 350，重启那一步不计
        assert_eq!(up_total, 350, "上行增量: {up_total}");
        assert_eq!(down_total, (260 - 200) + (900 - 260) + (80 - 20));
        assert!(!points.is_empty());
        assert!(points.iter().all(|p: &TrafficPoint| p.ts_ms % 60_000 == 0));
        clear(&dir).unwrap();
        let (after, u, d) = query(&dir, base, 60).unwrap();
        assert!(after.is_empty() && u == 0 && d == 0, "清空后应无数据");
    }

    /// 采样写入后能立刻查到（写入路径本身没写错表/写错列）。
    #[test]
    fn record_sample_persists_and_is_queryable() {
        let dir = tmpdir("record");
        record_sample(&dir, 123, 456).unwrap();
        // 再写一个更大的累计值，产生一次正增量
        std::thread::sleep(std::time::Duration::from_millis(1100));
        record_sample(&dir, 223, 656).unwrap();
        let (_points, up, down) = query(&dir, 0, 3600).unwrap();
        assert_eq!(up, 100);
        assert_eq!(down, 200);
    }
}
