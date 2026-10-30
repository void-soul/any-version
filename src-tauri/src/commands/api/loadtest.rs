//! 并发压测引擎：tokio 多 worker 并发请求，实时进度 + 完整统计报告。
//!
//! 启动后立即返回 run_id，调用方通过 `load_run_status(run_id)` 轮询进度；
//! 完成时报告落库 api_load_runs 供历史回看。

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use super::models::*;

const MAX_SAMPLES: usize = 500_000;
pub const MAX_CONCURRENCY: u32 = 128;
pub const MAX_RUNNING_TESTS: usize = 4;
const MAX_DURATION_SECS: u32 = 3600;
const MAX_COMPLETED_REPORTS: usize = 16;

pub fn validate_config(config: &LoadTestConfig) -> Result<(), String> {
    if !(1..=MAX_CONCURRENCY).contains(&config.concurrency) {
        return Err(format!("压测并发数须在 1 到 {MAX_CONCURRENCY} 之间"));
    }
    if !(1..=MAX_DURATION_SECS).contains(&config.duration_secs) {
        return Err(format!("压测时长须在 1 到 {MAX_DURATION_SECS} 秒之间"));
    }
    if config.ramp_up_secs > config.duration_secs {
        return Err("升压时间不能超过压测时长".into());
    }
    Ok(())
}

struct Sample {
    sec: u64,
    time_ms: f64,
    ok: bool,
}

struct LoadRunState {
    pub id: String,
    pub running: AtomicBool,
    finished: AtomicBool,
    pub start: Instant,
    pub duration_secs: u64,
    pub total: AtomicU64,
    pub success: AtomicU64,
    pub failed: AtomicU64,
    pub samples: Mutex<Vec<Sample>>,
    pub status_codes: Mutex<HashMap<u16, u64>>,
    cancel: tokio::sync::watch::Sender<bool>,
}

static RUNS: std::sync::OnceLock<Mutex<HashMap<String, std::sync::Arc<LoadRunState>>>> = std::sync::OnceLock::new();

fn runs() -> &'static Mutex<HashMap<String, std::sync::Arc<LoadRunState>>> {
    RUNS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn completed_reports() -> &'static Mutex<VecDeque<(String, LoadRunStatus)>> {
    static COMPLETED: std::sync::OnceLock<Mutex<VecDeque<(String, LoadRunStatus)>>> = std::sync::OnceLock::new();
    COMPLETED.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// 启动压测。立即返回 run_id；后台 tokio 任务执行请求并生成报告。
pub fn start_load_run(
    run_id: String,
    _endpoint_id: String,
    _name: String,
    config: LoadTestConfig,
    _created_at: String,
    input: SendRequestInput,
) -> Result<String, String> {
    validate_config(&config)?;
    let duration_secs = config.duration_secs.max(1) as u64;
    let state = std::sync::Arc::new(LoadRunState {
        id: run_id.clone(),
        running: AtomicBool::new(true),
        finished: AtomicBool::new(false),
        start: Instant::now(),
        duration_secs,
        total: AtomicU64::new(0),
        success: AtomicU64::new(0),
        failed: AtomicU64::new(0),
        samples: Mutex::new(Vec::new()),
        status_codes: Mutex::new(HashMap::new()),
        cancel: tokio::sync::watch::channel(false).0,
    });

    // 共享 Client（连接池复用，提升 QPS）
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(input.timeout_ms.clamp(100, 120_000) as u64))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => return Err(format!("无法创建压测客户端: {e}")),
    };

    {
        // A stopped run still owns its slot until all workers have drained.
        let mut active = runs().lock().unwrap_or_else(|e| e.into_inner());
        if active.len() >= MAX_RUNNING_TESTS { return Err(format!("最多同时运行 {MAX_RUNNING_TESTS} 个压测任务")); }
        if active.contains_key(&run_id) { return Err("压测任务已存在".into()); }
        active.insert(run_id.clone(), state.clone());
    }

    let concurrency = config.concurrency.max(1) as usize;
    let ramp_up = config.ramp_up_secs as f64;
    // RPS 上限（0 = 不限）：全局发送配额调度，每个请求按序号分配精确的发送时刻
    let rps_limit = config.rps_limit;
    let send_counter = std::sync::Arc::new(AtomicU64::new(0));

    let mut workers = Vec::with_capacity(concurrency);
    for worker_index in 0..concurrency {
        let state = state.clone();
        let client = client.clone();
        let input = input.clone();
        let send_counter = send_counter.clone();
        let mut cancelled = state.cancel.subscribe();
        // 用 tauri 的运行时 handle：命令可能从主线程（无 ambient runtime）调用，
        // 直接 tokio::spawn 会 panic；tauri::async_runtime::spawn 自动适配。
        workers.push(tauri::async_runtime::spawn(async move {
            if ramp_up > 0.0 {
                let delay = ramp_up * (worker_index as f64) / (concurrency as f64);
                if delay > 0.0 {
                    tokio::select! {
                        _ = cancelled.changed() => return,
                        _ = tokio::time::sleep(std::time::Duration::from_secs_f64(delay)) => {}
                    }
                }
            }
            loop {
                if !state.running.load(Ordering::SeqCst) || *cancelled.borrow() {
                    break;
                }
                let sec = state.start.elapsed().as_secs();
                if sec >= state.duration_secs {
                    break;
                }
                if rps_limit > 0 {
                    // 取全局发送配额：第 n 个请求应发生在 start + n/rps 时刻
                    let n = send_counter.fetch_add(1, Ordering::Relaxed);
                    let scheduled = state.start + std::time::Duration::from_secs_f64((n as f64 + 1.0) / rps_limit as f64);
                    let now = Instant::now();
                    if scheduled > now {
                        tokio::select! {
                            _ = cancelled.changed() => break,
                            _ = tokio::time::sleep(scheduled - now) => {}
                        }
                    }
                    if !state.running.load(Ordering::SeqCst)
                        || state.start.elapsed().as_secs() >= state.duration_secs
                    {
                        break;
                    }
                }
                let started = Instant::now();
                let request = match super::exec::build_request(&client, &input) {
                    Ok(r) => r,
                    Err(_) => {
                        record_failure(&state, sec, started);
                        continue;
                    }
                };
                let response = tokio::select! {
                    _ = cancelled.changed() => break,
                    response = client.execute(request) => response,
                };
                match response {
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
        }));
    }

    // 监督任务：时长结束或被手动停止后标记完成并落库报告（每 200ms 检查，停止能立即收尾）
    let state_for_report = state.clone();
    tauri::async_runtime::spawn(async move {
        let deadline = state_for_report.duration_secs;
        loop {
            if !state_for_report.running.load(Ordering::SeqCst)
                || state_for_report.start.elapsed().as_secs() >= deadline
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        state_for_report.running.store(false, Ordering::SeqCst);
        state_for_report.cancel.send_replace(true);
        // Cancellation wakes requests and ramp/rate-limit waits. Join every worker
        // before freezing the report; a fixed delay cannot guarantee final counts.
        for worker in workers { let _ = worker.await; }
        state_for_report.finished.store(true, Ordering::SeqCst);
        let report = build_report(&state_for_report);
        let json = serde_json::to_string(&report).unwrap_or_default();
        let id = state_for_report.id.clone();
        let persisted = super::db::with_db(|conn| {
            conn.execute(
                "UPDATE api_load_runs SET report = ?1 WHERE id = ?2",
                rusqlite::params![json, id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        });
        if let Err(error) = persisted { tracing::error!(run_id = %id, "保存压测报告失败: {error}"); }
        {
            let mut completed = completed_reports().lock().unwrap_or_else(|e| e.into_inner());
            completed.push_back((id.clone(), status_from_state(&state_for_report)));
            while completed.len() > MAX_COMPLETED_REPORTS { completed.pop_front(); }
        }
        // No completed run retains its samples; history is read from SQLite.
        runs().lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
    });

    Ok(run_id)
}

/// 手动停止压测：worker 在下一轮循环退出，监督任务随即生成报告。
pub fn stop_load_run(run_id: &str) -> Result<(), String> {
    let guard = runs().lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get(run_id).ok_or("压测任务不存在")?;
    state.running.store(false, Ordering::SeqCst);
    state.cancel.send_replace(true);
    Ok(())
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
    let active = runs().lock().unwrap_or_else(|e| e.into_inner()).get(run_id).cloned();
    if let Some(state) = active { return Ok(status_from_state(&state)); }
    if let Some((_, status)) = completed_reports().lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(id, _)| id == run_id) {
        return Ok(status.clone());
    }
    super::db::with_db(|conn| {
        let report: String = conn.query_row("SELECT report FROM api_load_runs WHERE id = ?1", [run_id], |row| row.get(0))
            .map_err(|_| "压测任务不存在".to_string())?;
        let report: LoadTestReport = serde_json::from_str(&report).map_err(|_| "压测报告尚未生成".to_string())?;
        Ok(LoadRunStatus {
            running: false, elapsed_secs: report.timeline.len().saturating_sub(1) as u32,
            total: report.total, success: report.success, failed: report.failed,
            qps: report.qps_avg, latency_avg_ms: report.latency_avg_ms, latency_p95_ms: report.latency_p95_ms,
            report: Some(report),
        })
    })
}

pub fn ensure_not_active(run_id: &str) -> Result<(), String> {
    if runs().lock().unwrap_or_else(|e| e.into_inner()).contains_key(run_id) {
        return Err("请先停止压测并等待报告完成后删除".into());
    }
    Ok(())
}

pub fn forget_completed(run_id: &str) {
    completed_reports().lock().unwrap_or_else(|e| e.into_inner()).retain(|(id, _)| id != run_id);
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
    let running = !state.finished.load(Ordering::SeqCst);
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
    // 实际耗时：手动停止时报告按真实时长统计，而非原计划时长
    let elapsed = state.start.elapsed().as_secs().max(1);
    let total = state.total.load(Ordering::Relaxed);
    let success = state.success.load(Ordering::Relaxed);
    let failed = state.failed.load(Ordering::Relaxed);
    let samples = state.samples.lock().unwrap_or_else(|e| e.into_inner());
    let latencies: Vec<f64> = samples.iter().map(|s| s.time_ms).collect();

    // 每秒时间线（按 Sample.sec 精确分桶；桶数跟随实际运行时长）
    let sec_count = (elapsed as u32) + 1;
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
        qps_avg: total as f64 / elapsed as f64,
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

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn invalid_resource_limits_are_rejected_before_reserving_a_run() {
        for config in [
            LoadTestConfig { concurrency: 0, duration_secs: 1, ..Default::default() },
            LoadTestConfig { concurrency: MAX_CONCURRENCY + 1, duration_secs: 1, ..Default::default() },
            LoadTestConfig { concurrency: 1, duration_secs: MAX_DURATION_SECS + 1, ..Default::default() },
            LoadTestConfig { concurrency: 1, duration_secs: 1, ramp_up_secs: 2, rps_limit: 1 },
        ] {
            assert!(start_load_run("invalid-resource-limit".into(), "".into(), "".into(), config, "".into(), SendRequestInput::default()).is_err());
        }
        assert!(!runs().lock().unwrap().contains_key("invalid-resource-limit"));
    }

    #[test]
    fn stopping_drains_workers_and_releases_sample_state() {
        let id = format!("lifecycle-{}", std::process::id());
        start_load_run(id.clone(), "".into(), "".into(), LoadTestConfig {
            concurrency: 2, duration_secs: 5, ramp_up_secs: 5, rps_limit: 1,
        }, "".into(), SendRequestInput { url: "http://127.0.0.1:9/".into(), ..Default::default() }).unwrap();
        stop_load_run(&id).unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while runs().lock().unwrap().contains_key(&id) {
            assert!(Instant::now() < deadline, "stopped run retained its sample state");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!load_run_status(&id).unwrap().running);
        assert!(completed_reports().lock().unwrap().len() <= MAX_COMPLETED_REPORTS);
        forget_completed(&id);
    }
}

#[cfg(test)]
mod rps_limit_tests {
    //! rps_limit 端到端验证：内置临时慢速接口（本机随机端口，每请求延迟 20ms），
    //! 分别以「限速 5 RPS」和「不限速」压测同一接口，对比实际吞吐。

    use super::*;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::Arc;

    /// 临时慢速接口：每连接读一次请求、延迟 20ms、返回 200 + "ok"（Connection: close）。
    fn spawn_slow_server() -> (String, Arc<AtomicU64>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定测试端口失败");
        let addr = listener.local_addr().unwrap().to_string();
        let hits = Arc::new(AtomicU64::new(0));
        let hits2 = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let hits = hits2.clone();
                std::thread::spawn(move || {
                    let mut buf = [0u8; 2048];
                    let _ = stream.read(&mut buf); // 不解析内容，读到即算收到请求
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    hits.fetch_add(1, Ordering::Relaxed);
                    let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
                    let _ = stream.write_all(resp.as_bytes());
                });
            }
        });
        (addr, hits)
    }

    fn run_load(addr: &str, duration_secs: u32, concurrency: u32, rps_limit: u32) -> LoadRunStatus {
        let input = SendRequestInput {
            method: "GET".into(),
            url: format!("http://{}/slow", addr),
            timeout_ms: 5000,
            ..Default::default()
        };
        let config = LoadTestConfig {
            concurrency,
            duration_secs,
            ramp_up_secs: 0,
            rps_limit,
        };
        let run_id = format!("test-{}-{}", std::process::id(), rps_limit);
        start_load_run(run_id.clone(), "ep".into(), "验证".into(), config, "".into(), input).unwrap();
        // 轮询直到结束
        let deadline = Instant::now() + std::time::Duration::from_secs((duration_secs as u64) + 10);
        loop {
            let st = load_run_status(&run_id).expect("查询压测状态失败");
            if !st.running {
                return st;
            }
            assert!(Instant::now() < deadline, "压测超时未结束");
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }

    #[test]
    fn limited_run_stays_near_rps_cap() {
        let (addr, _hits) = spawn_slow_server();
        let (dur, cap) = (6u32, 5u32);
        let st = run_load(&addr, dur, 32, cap);
        println!("[限速 {} RPS × {}s] total={} success={} failed={} qps_avg={:.2}", cap, dur, st.total, st.success, st.failed, st.qps);
        // 上限：调度器按 n/rps 排发送时刻，最多 cap×(dur+1) 个左右；给少量调度抖动余量
        assert!(
            st.total <= (cap * (dur + 1) + 3) as u64,
            "限速失效：total={} 超过上限 {}", st.total, cap * (dur + 1)
        );
        // 下限：不应显著低于 cap×dur
        assert!(
            st.total >= (cap * (dur - 1)) as u64,
            "吞吐异常偏低：total={}", st.total
        );
        assert_eq!(st.failed, 0, "不应有失败请求");
    }

    #[test]
    fn unlimited_run_far_exceeds_cap() {
        let (addr, _hits) = spawn_slow_server();
        let st = run_load(&addr, 3, 16, 0);
        println!("[不限速 × 3s] total={} success={} failed={} qps_avg={:.2}", st.total, st.success, st.failed, st.qps);
        // 同样 16 并发、20ms 延迟的服务端，不限速时吞吐应远超 5 RPS
        assert!(st.total > 100, "不限速吞吐异常偏低：total={}", st.total);
    }
}
