//! 并发压测引擎：tokio 多 worker 并发请求，实时进度 + 完整统计报告。
//!
//! 启动后立即返回 run_id，调用方通过 `load_run_status(run_id)` 轮询进度；
//! 完成时报告落库 api_load_runs 供历史回看。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use super::models::*;

const MAX_SAMPLES: usize = 500_000;

struct Sample {
    sec: u64,
    time_ms: f64,
    ok: bool,
}

struct LoadRunState {
    pub id: String,
    pub running: AtomicBool,
    pub start: Instant,
    pub duration_secs: u64,
    pub total: AtomicU64,
    pub success: AtomicU64,
    pub failed: AtomicU64,
    pub samples: Mutex<Vec<Sample>>,
    pub status_codes: Mutex<HashMap<u16, u64>>,
}

static RUNS: std::sync::OnceLock<Mutex<HashMap<String, std::sync::Arc<LoadRunState>>>> = std::sync::OnceLock::new();

fn runs() -> &'static Mutex<HashMap<String, std::sync::Arc<LoadRunState>>> {
    RUNS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 启动压测。立即返回 run_id；后台 tokio 任务执行请求并生成报告。
pub fn start_load_run(
    run_id: String,
    _endpoint_id: String,
    _name: String,
    config: LoadTestConfig,
    _created_at: String,
    input: SendRequestInput,
) -> String {
    let duration_secs = config.duration_secs.max(1) as u64;
    let state = std::sync::Arc::new(LoadRunState {
        id: run_id.clone(),
        running: AtomicBool::new(true),
        start: Instant::now(),
        duration_secs,
        total: AtomicU64::new(0),
        success: AtomicU64::new(0),
        failed: AtomicU64::new(0),
        samples: Mutex::new(Vec::new()),
        status_codes: Mutex::new(HashMap::new()),
    });
    runs().lock().unwrap_or_else(|e| e.into_inner()).insert(run_id.clone(), state.clone());

    // 共享 Client（连接池复用，提升 QPS）
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(input.timeout_ms.max(100) as u64))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            state.running.store(false, Ordering::SeqCst);
            return run_id;
        }
    };

    let concurrency = config.concurrency.max(1) as usize;
    let ramp_up = config.ramp_up_secs as f64;

    for worker_index in 0..concurrency {
        let state = state.clone();
        let client = client.clone();
        let input = input.clone();
        // 用 tauri 的运行时 handle：命令可能从主线程（无 ambient runtime）调用，
        // 直接 tokio::spawn 会 panic；tauri::async_runtime::spawn 自动适配。
        tauri::async_runtime::spawn(async move {
            if ramp_up > 0.0 {
                let delay = ramp_up * (worker_index as f64) / (concurrency as f64);
                if delay > 0.0 {
                    tokio::time::sleep(std::time::Duration::from_secs_f64(delay)).await;
                }
            }
            loop {
                if !state.running.load(Ordering::SeqCst) {
                    break;
                }
                let sec = state.start.elapsed().as_secs();
                if sec >= state.duration_secs {
                    break;
                }
                let started = Instant::now();
                let request = match super::exec::build_request(&client, &input) {
                    Ok(r) => r,
                    Err(_) => {
                        record_failure(&state, sec, started);
                        continue;
                    }
                };
                match client.execute(request).await {
                    Ok(resp) => {
                        let status = resp.status().as_u16();
                        let ok = resp.status().is_success();
                        let time_ms = started.elapsed().as_secs_f64() * 1000.0;
                        state.total.fetch_add(1, Ordering::Relaxed);
                state.success.fetch_add(ok as u64, Ordering::Relaxed);
                state.failed.fetch_add((!ok) as u64, Ordering::Relaxed);
                {
                    let mut codes = state.status_codes.lock().unwrap_or_else(|e| e.into_inner());
                    *codes.entry(status).or_insert(0) += 1;
                }
                {
                    let mut samples = state.samples.lock().unwrap_or_else(|e| e.into_inner());
                    if samples.len() < MAX_SAMPLES {
                        samples.push(Sample { sec, time_ms, ok });
                    }
                }
                    }
                    Err(_) => {
                        record_failure(&state, sec, started);
                    }
                }
            }
        });
    }

    // 监督任务：等时长结束后标记完成并落库报告
    let state_for_report = state.clone();
    tauri::async_runtime::spawn(async move {
        let total_wait = state_for_report.duration_secs + ramp_up as u64 + 2;
        tokio::time::sleep(std::time::Duration::from_secs(total_wait)).await;
        state_for_report.running.store(false, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let report = build_report(&state_for_report);
        let json = serde_json::to_string(&report).unwrap_or_default();
        let id = state_for_report.id.clone();
        let _ = super::db::with_db(|conn| {
            conn.execute(
                "UPDATE api_load_runs SET report = ?1 WHERE id = ?2",
                rusqlite::params![json, id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        });
    });

    run_id
}

fn record_failure(state: &std::sync::Arc<LoadRunState>, sec: u64, started: Instant) {
    let time_ms = started.elapsed().as_secs_f64() * 1000.0;
    state.total.fetch_add(1, Ordering::Relaxed);
    state.failed.fetch_add(1, Ordering::Relaxed);
    let mut samples = state.samples.lock().unwrap_or_else(|e| e.into_inner());
    if samples.len() < MAX_SAMPLES {
        samples.push(Sample { sec, time_ms, ok: false });
    }
}

/// 读取运行状态（轮询接口）。
pub fn load_run_status(run_id: &str) -> Result<LoadRunStatus, String> {
    let guard = runs().lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get(run_id).ok_or("压测任务不存在")?;
    Ok(status_from_state(state))
}

fn status_from_state(state: &LoadRunState) -> LoadRunStatus {
    let elapsed = state.start.elapsed().as_secs() as u32;
    let total = state.total.load(Ordering::Relaxed);
    let success = state.success.load(Ordering::Relaxed);
    let failed = state.failed.load(Ordering::Relaxed);
    // 注意：必须在调用 build_report 之前释放 samples 锁（std Mutex 不可重入，
    // 否则 build_report 再次加锁会自死锁，压测结束后的轮询将永久卡住）。
    let (avg, p95) = {
        let samples = state.samples.lock().unwrap_or_else(|e| e.into_inner());
        let latencies: Vec<f64> = samples.iter().map(|s| s.time_ms).collect();
        let avg = if latencies.is_empty() { 0.0 } else { latencies.iter().sum::<f64>() / latencies.len() as f64 };
        let p95 = percentile(&latencies, 0.95);
        (avg, p95)
    };
    let qps = if elapsed > 0 { total as f64 / elapsed as f64 } else { 0.0 };
    let running = state.running.load(Ordering::SeqCst);
    let report = if !running { Some(build_report(state)) } else { None };
    LoadRunStatus {
        running,
        elapsed_secs: elapsed.min(state.duration_secs as u32),
        total,
        success,
        failed,
        qps,
        latency_avg_ms: avg,
        latency_p95_ms: p95,
        report,
    }
}

fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[idx.min(v.len() - 1)]
}

fn build_report(state: &LoadRunState) -> LoadTestReport {
    let duration = state.duration_secs.max(1) as f64;
    let total = state.total.load(Ordering::Relaxed);
    let success = state.success.load(Ordering::Relaxed);
    let failed = state.failed.load(Ordering::Relaxed);
    let samples = state.samples.lock().unwrap_or_else(|e| e.into_inner());
    let latencies: Vec<f64> = samples.iter().map(|s| s.time_ms).collect();

    // 每秒时间线（按 Sample.sec 精确分桶）
    let sec_count = (state.duration_secs as u32) + 1;
    let mut sec_success = vec![0u64; sec_count as usize];
    let mut sec_failed = vec![0u64; sec_count as usize];
    let mut sec_avg: Vec<(f64, u64)> = vec![(0.0, 0); sec_count as usize];
    for s in samples.iter() {
        let i = (s.sec as usize).min(sec_count as usize - 1);
        if s.ok {
            sec_success[i] += 1;
        } else {
            sec_failed[i] += 1;
        }
        let n = sec_avg[i].1 + 1;
        sec_avg[i].0 = (sec_avg[i].0 * sec_avg[i].1 as f64 + s.time_ms) / n as f64;
        sec_avg[i].1 = n;
    }
    let timeline: Vec<TimelineSample> = (0..sec_count as u32)
        .map(|t| TimelineSample {
            t,
            qps: (sec_success[t as usize] + sec_failed[t as usize]) as f64,
            success: sec_success[t as usize],
            failed: sec_failed[t as usize],
            avg_ms: sec_avg[t as usize].0,
        })
        .collect();

    let status_codes: Vec<(u16, u64)> = {
        let mut codes: Vec<(u16, u64)> = state
            .status_codes
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();
        codes.sort();
        codes
    };

    let mut sorted = latencies.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    LoadTestReport {
        total,
        success,
        failed,
        error_rate: if total > 0 { failed as f64 / total as f64 } else { 0.0 },
        qps_avg: total as f64 / duration,
        qps_max: timeline.iter().map(|t| t.qps).fold(0.0, f64::max),
        latency_min_ms: sorted.first().copied().unwrap_or(0.0),
        latency_p50_ms: percentile(&sorted, 0.50),
        latency_p90_ms: percentile(&sorted, 0.90),
        latency_p95_ms: percentile(&sorted, 0.95),
        latency_p99_ms: percentile(&sorted, 0.99),
        latency_max_ms: sorted.last().copied().unwrap_or(0.0),
        latency_avg_ms: if sorted.is_empty() { 0.0 } else { sorted.iter().sum::<f64>() / sorted.len() as f64 },
        status_codes,
        timeline,
    }
}
