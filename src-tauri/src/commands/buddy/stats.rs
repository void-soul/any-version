//! Token / 积分 / 调用量统计（WorkDaddy §5、§6 的移植版，仅 WorkBuddy）。
//!
//! 两个数据源，语义与参考一致：
//! 1. **Token 统计**（§6）：直接扫本地会话 `~/.workbuddy/projects/**/*.jsonl` 的 usage 记录，
//!    按 `日 / 账号 / 模型` 聚合成桶；同一份会话被导入成副本时靠"整行 sha256 + 同文件出现序号"
//!    去重（参考 `digest:occurrence`），避免重复计费。
//! 2. **积分 / 调用量统计**（§5）：调官方计费接口 `get-user-request-usage`，
//!    按 `日 / 账号 / 模型` 聚合积分与请求数；今日数据 60 秒 TTL，历史日转"不可变"缓存。
//!
//! 与参考的差异（适配说明）：
//! - 参考把结果写进 SQLite（`credit_usage_records` 等 4 张表）并做 anchor 增量同步；
//!   我们改为**JSON 缓存 + 按缺失日补拉**（同样的"缺口合并区间"思路，少一层 schema 依赖），
//!   并把 `final` 标记放在日粒度上。
//! - 账号归属优先取记录里的 `accountUid/accountId/uid/userId`，其次用会话目录/文件名匹配账号 uid；
//!   参考还会查 `sessions` 表的 `user_id`，我们没有把该查询带进来（避免与 `sessions.rs` 的读库逻辑耦合）。
//! - **平台范围：仅 WorkBuddy**（数据源与接口都是 WorkBuddy 的）。

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::models::{BuddyAccount, BuddyPlatform};

/// 本地会话 JSONL 扫描上限（与参考一致，防止异常目录拖死扫描）
const MAX_FILES: usize = 5000;
/// 单次统计最多回溯天数（参考 `MAX_CACHE_DAYS`）
const MAX_DAYS: i64 = 90;
/// 今日积分的缓存 TTL（参考 60 秒）
const TODAY_TTL_MS: i64 = 60_000;
/// 无扫描结果的缓存 TTL（30 秒）
const EMPTY_TTL_MS: i64 = 30_000;

fn workbuddy_root() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".workbuddy"))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn cache_path(name: &str) -> PathBuf {
    crate::commands::config::get_data_dir().join("buddy").join(name)
}

fn read_cache(name: &str) -> Option<Value> {
    let content = std::fs::read_to_string(cache_path(name)).ok()?;
    if content.len() > 64 * 1024 * 1024 {
        return None;
    }
    serde_json::from_str(&content).ok()
}

fn write_cache(name: &str, value: &Value) {
    if let Ok(serialized) = serde_json::to_string(value) {
        let _ = super::store::write_atomic(&cache_path(name), &serialized);
    }
}

fn clamp_days(days: Option<i64>) -> i64 {
    days.unwrap_or(7).clamp(1, MAX_DAYS)
}

fn local_day(offset_days: i64) -> String {
    (chrono::Local::now() + chrono::Duration::days(offset_days))
        .format("%Y-%m-%d")
        .to_string()
}

// ─── Token 统计（§6） ───

/// 从一条记录里递归找 usage（参考 `findUsage`：字段可能藏在任意层级）。
fn find_usage(value: &Value) -> Option<(i64, i64, i64, i64)> {
    if let Value::Object(map) = value {
        let pick = |keys: &[&str]| -> Option<i64> {
            keys.iter()
                .find_map(|key| map.get(*key).and_then(Value::as_i64))
        };
        let input = pick(&["input_tokens", "prompt_tokens", "inputTokens", "promptTokens"]);
        let output = pick(&["output_tokens", "completion_tokens", "outputTokens", "completionTokens"]);
        if input.is_some() || output.is_some() {
            let cache_read = pick(&[
                "cache_read_input_tokens",
                "cache_read_tokens",
                "cached_tokens",
                "cacheReadInputTokens",
            ])
            .unwrap_or(0);
            let cache_write = pick(&[
                "cache_creation_input_tokens",
                "cache_write_tokens",
                "cacheCreationInputTokens",
            ])
            .unwrap_or(0);
            return Some((input.unwrap_or(0), output.unwrap_or(0), cache_read, cache_write));
        }
        for nested in map.values() {
            if let Some(found) = find_usage(nested) {
                return Some(found);
            }
        }
    } else if let Value::Array(items) = value {
        for nested in items {
            if let Some(found) = find_usage(nested) {
                return Some(found);
            }
        }
    }
    None
}

fn record_account(record: &Value) -> Option<String> {
    for key in ["accountUid", "accountId", "uid", "userId", "account_uid", "user_id"] {
        if let Some(text) = record.get(key).and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn record_model(record: &Value) -> Option<String> {
    for key in ["model", "modelName", "model_name"] {
        if let Some(text) = record.get(key).and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// 会话 id：优先记录字段，其次文件名（参考的降级顺序）。
fn record_session_id(record: &Value, file: &Path) -> String {
    for key in ["sessionId", "conversationId", "session_id", "conversation_id"] {
        if let Some(text) = record.get(key).and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    file.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("")
        .to_string()
}

/// 把某个账号 uid 关联到会话：记录里没有时，用"会话目录/文件名里含 uid"做兜底匹配。
fn resolve_account(record: &Value, file: &Path, known_uids: &[String]) -> String {
    if let Some(uid) = record_account(record) {
        return uid;
    }
    let haystack = file.to_string_lossy().to_lowercase();
    for uid in known_uids {
        if !uid.is_empty() && haystack.contains(&uid.to_lowercase()) {
            return uid.clone();
        }
    }
    "unknown".to_string()
}

fn collect_jsonl_files(root: &Path, out: &mut Vec<PathBuf>) {
    if out.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let Ok(metadata) = std::fs::symlink_metadata(&path) else { continue };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            // 参考跳过 subagents 目录（不是用户会话）
            if name == "subagents" {
                continue;
            }
            collect_jsonl_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
}

struct TokenBucket {
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    calls: i64,
}

impl TokenBucket {
    fn new() -> Self {
        Self { input: 0, output: 0, cache_read: 0, cache_write: 0, calls: 0 }
    }
    fn add(&mut self, other: (i64, i64, i64, i64)) {
        self.input += other.0;
        self.output += other.1;
        self.cache_read += other.2;
        self.cache_write += other.3;
        self.calls += 1;
    }
    fn to_json(&self, extra: Value) -> Value {
        let mut object = json!({
            "input": self.input,
            "output": self.output,
            "cacheRead": self.cache_read,
            "cacheWrite": self.cache_write,
            "calls": self.calls,
            "total": self.input + self.output,
        });
        if let (Some(target), Some(source)) = (object.as_object_mut(), extra.as_object()) {
            for (key, value) in source {
                target.insert(key.clone(), value.clone());
            }
        }
        object
    }
}

/// 扫本地会话统计 Token（结果按 `日 / 账号 / 模型` 分桶）。
pub(crate) fn scan_token_stats(days: i64) -> Value {
    let since = local_day(-(days - 1));
    let root = workbuddy_root();
    let mut files = Vec::new();
    if let Some(root) = &root {
        collect_jsonl_files(&root.join("projects"), &mut files);
    }
    let known_uids: Vec<String> = super::store::list_accounts(BuddyPlatform::Workbuddy)
        .into_iter()
        .filter_map(|account| account.uid)
        .collect();

    let mut buckets: BTreeMap<(String, String, String), TokenBucket> = BTreeMap::new();
    // 去重指纹：整行 sha256 + 同文件出现序号（参考 digest:occurrence）
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut scanned_files = 0usize;

    for file in &files {
        let Ok(content) = std::fs::read_to_string(file) else { continue };
        scanned_files += 1;
        let mut occurrence: HashMap<String, usize> = HashMap::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(record) = serde_json::from_str::<Value>(line) else { continue };
            // 快照更新记录不是真实消耗
            if record.get("type").and_then(Value::as_str) == Some("snapshot_update") {
                continue;
            }
            let Some(usage) = find_usage(&record) else { continue };
            let digest = short_hash(line);
            let index = occurrence.entry(digest.clone()).or_insert(0);
            let fingerprint = format!("{}:{}", digest, index);
            *index += 1;
            if !seen.insert(fingerprint) {
                continue;
            }
            // 时间：优先记录里的时间戳，退化为文件修改时间
            let stamp = record
                .get("timestamp")
                .and_then(Value::as_i64)
                .or_else(|| record.get("time").and_then(Value::as_i64))
                .or_else(|| record.get("createdAt").and_then(Value::as_i64))
                .unwrap_or_else(|| file_millis(file));
            let day = millis_to_day(stamp);
            if day.is_empty() || day.as_str() < since.as_str() {
                continue;
            }
            let account = resolve_account(&record, file, &known_uids);
            let model = record_model(&record).unwrap_or_else(|| "unknown".to_string());
            buckets
                .entry((day, account, model))
                .or_insert_with(TokenBucket::new)
                .add(usage);
        }
    }

    aggregate_tokens(&buckets, &since, scanned_files)
}

fn file_millis(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn millis_to_day(millis: i64) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .map(|utc| utc.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn short_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    digest[..24].to_string()
}

/// 把桶聚合成前端要的四段结构 + 扁平的 dailyBreakdown（参考 1.2.76 新增）。
fn aggregate_tokens(
    buckets: &BTreeMap<(String, String, String), TokenBucket>,
    since: &str,
    scanned_files: usize,
) -> Value {
    let mut totals = TokenBucket::new();
    let mut by_day: BTreeMap<String, TokenBucket> = BTreeMap::new();
    let mut by_model: BTreeMap<String, TokenBucket> = BTreeMap::new();
    let mut by_account: BTreeMap<String, TokenBucket> = BTreeMap::new();
    let mut breakdown: Vec<Value> = Vec::new();

    for ((day, account, model), bucket) in buckets {
        totals.input += bucket.input;
        totals.output += bucket.output;
        totals.cache_read += bucket.cache_read;
        totals.cache_write += bucket.cache_write;
        totals.calls += bucket.calls;
        for (map, key) in [
            (&mut by_day, day.clone()),
            (&mut by_model, model.clone()),
            (&mut by_account, account.clone()),
        ] {
            let entry = map.entry(key).or_insert_with(TokenBucket::new);
            entry.input += bucket.input;
            entry.output += bucket.output;
            entry.cache_read += bucket.cache_read;
            entry.cache_write += bucket.cache_write;
            entry.calls += bucket.calls;
        }
        breakdown.push(bucket.to_json(json!({
            "day": day,
            "account": account,
            "model": model,
        })));
    }

    let mut daily: Vec<Value> = by_day
        .iter()
        .map(|(day, bucket)| bucket.to_json(json!({"day": day})))
        .collect();
    daily.sort_by(|a, b| {
        a.get("day").and_then(Value::as_str).unwrap_or("")
            .cmp(b.get("day").and_then(Value::as_str).unwrap_or(""))
    });
    let mut models: Vec<Value> = by_model
        .iter()
        .map(|(model, bucket)| bucket.to_json(json!({"model": model})))
        .collect();
    models.sort_by_key(|item| -item.get("total").and_then(Value::as_i64).unwrap_or(0));
    let mut accounts: Vec<Value> = by_account
        .iter()
        .map(|(account, bucket)| bucket.to_json(json!({"account": account})))
        .collect();
    accounts.sort_by_key(|item| -item.get("total").and_then(Value::as_i64).unwrap_or(0));
    breakdown.sort_by(|a, b| {
        let key = |item: &Value| {
            format!(
                "{}|{}|{}",
                item.get("day").and_then(Value::as_str).unwrap_or(""),
                item.get("account").and_then(Value::as_str).unwrap_or(""),
                item.get("model").and_then(Value::as_str).unwrap_or("")
            )
        };
        key(a).cmp(&key(b))
    });

    json!({
        "source": "local-workbuddy-jsonl",
        "since": since,
        "until": local_day(0),
        "scannedFiles": scanned_files,
        "totals": totals.to_json(json!({})),
        "daily": daily,
        "models": models,
        "accounts": accounts,
        "dailyBreakdown": breakdown,
    })
}

/// 按账号 / 模型 / 天数取 Token 统计（带 60 秒缓存，`force` 可绕过）。
#[tauri::command]
pub async fn buddy_token_stats(
    days: Option<i64>,
    account: Option<String>,
    model: Option<String>,
    force: Option<bool>,
) -> Result<Value, String> {
    let days = clamp_days(days);
    let cache_name = "token-stats-cache.json";
    let cache_key = format!("v1:{}", days);
    let mut cache = read_cache(cache_name).unwrap_or(json!({"entries": {}}));
    let cached = cache
        .get("entries")
        .and_then(|entries| entries.get(&cache_key))
        .cloned();
    let fresh = cached
        .as_ref()
        .and_then(|entry| entry.get("at").and_then(Value::as_i64))
        .map(|at| now_ms() - at < TODAY_TTL_MS)
        .unwrap_or(false);
    let view = if fresh && force != Some(true) {
        cached.and_then(|entry| entry.get("value").cloned()).unwrap_or(Value::Null)
    } else {
        let value = scan_token_stats(days);
        if let Some(entries) = cache.get_mut("entries").and_then(Value::as_object_mut) {
            entries.insert(cache_key, json!({"at": now_ms(), "value": value.clone()}));
        }
        write_cache(cache_name, &cache);
        value
    };

    let mut result = view;
    // 账号/模型筛选在内存里做（参考：聚合一次算全维度，筛选不打回数据源）
    if let Some(account) = account.filter(|value| !value.trim().is_empty()) {
        result = filter_token_view(&result, "account", &account);
    }
    if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
        result = filter_token_view(&result, "model", &model);
    }
    Ok(result)
}

/// 用扁平明细重算筛选后的视图（避免为每种筛选组合重扫文件）。
fn filter_token_view(view: &Value, field: &str, value: &str) -> Value {
    let Some(rows) = view.get("dailyBreakdown").and_then(Value::as_array) else {
        return view.clone();
    };
    let mut totals = TokenBucket::new();
    let mut by_day: BTreeMap<String, TokenBucket> = BTreeMap::new();
    let mut by_model: BTreeMap<String, TokenBucket> = BTreeMap::new();
    let mut by_account: BTreeMap<String, TokenBucket> = BTreeMap::new();
    let mut kept: Vec<Value> = Vec::new();
    for row in rows {
        if row.get(field).and_then(Value::as_str) != Some(value) {
            continue;
        }
        let bucket = (
            row.get("input").and_then(Value::as_i64).unwrap_or(0),
            row.get("output").and_then(Value::as_i64).unwrap_or(0),
            row.get("cacheRead").and_then(Value::as_i64).unwrap_or(0),
            row.get("cacheWrite").and_then(Value::as_i64).unwrap_or(0),
        );
        let calls = row.get("calls").and_then(Value::as_i64).unwrap_or(0);
        totals.input += bucket.0;
        totals.output += bucket.1;
        totals.cache_read += bucket.2;
        totals.cache_write += bucket.3;
        totals.calls += calls;
        let day = row.get("day").and_then(Value::as_str).unwrap_or("").to_string();
        let model = row.get("model").and_then(Value::as_str).unwrap_or("").to_string();
        let account = row.get("account").and_then(Value::as_str).unwrap_or("").to_string();
        for (map, key) in [
            (&mut by_day, day),
            (&mut by_model, model),
            (&mut by_account, account),
        ] {
            let entry = map.entry(key).or_insert_with(TokenBucket::new);
            entry.input += bucket.0;
            entry.output += bucket.1;
            entry.cache_read += bucket.2;
            entry.cache_write += bucket.3;
            entry.calls += calls;
        }
        kept.push(row.clone());
    }
    let mut daily: Vec<Value> = by_day
        .iter()
        .map(|(day, bucket)| bucket.to_json(json!({"day": day})))
        .collect();
    daily.sort_by(|a, b| {
        a.get("day").and_then(Value::as_str).unwrap_or("")
            .cmp(b.get("day").and_then(Value::as_str).unwrap_or(""))
    });
    let mut models: Vec<Value> = by_model
        .iter()
        .map(|(model, bucket)| bucket.to_json(json!({"model": model})))
        .collect();
    models.sort_by_key(|item| -item.get("total").and_then(Value::as_i64).unwrap_or(0));
    let mut accounts: Vec<Value> = by_account
        .iter()
        .map(|(account, bucket)| bucket.to_json(json!({"account": account})))
        .collect();
    accounts.sort_by_key(|item| -item.get("total").and_then(Value::as_i64).unwrap_or(0));
    json!({
        "source": view.get("source").cloned().unwrap_or(json!("local-workbuddy-jsonl")),
        "since": view.get("since").cloned().unwrap_or(json!("")),
        "until": view.get("until").cloned().unwrap_or(json!("")),
        "scannedFiles": view.get("scannedFiles").cloned().unwrap_or(json!(0)),
        "filtered": {field: value},
        "totals": totals.to_json(json!({})),
        "daily": daily,
        "models": models,
        "accounts": accounts,
        "dailyBreakdown": kept,
    })
}

// ─── 积分 / 调用量统计（§5） ───

/// 官方计费明细行（`get-user-request-usage`）。字段名有多套别名，逐个兜底。
fn normalize_usage_row(row: &Value) -> Option<(String, String, i64, String, String)> {
    let text = |keys: &[&str]| -> Option<String> {
        keys.iter().find_map(|key| {
            row.get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
    };
    let request_id = text(&["requestId", "request_id", "id"])?;
    let usage_date = text(&["usageDate", "usage_date", "date", "requestTime", "request_time"])?;
    let credit = row
        .get("credit")
        .and_then(Value::as_f64)
        .map(|value| value.round() as i64)
        .or_else(|| row.get("credit").and_then(Value::as_i64))
        .unwrap_or(0);
    let model = text(&["model", "modelName", "model_name"]).unwrap_or_else(|| "unknown".to_string());
    let day = usage_date
        .split(' ')
        .next()
        .unwrap_or(&usage_date)
        .to_string();
    Some((request_id, day, credit, model, usage_date))
}

/// 一天一账号的聚合行
#[derive(Default, Clone)]
struct CreditDay {
    credits: i64,
    requests: i64,
    models: BTreeMap<String, (i64, i64)>,
}

fn parse_usage_payload(payload: &Value) -> Vec<Value> {
    // 官方返回的行数组可能挂在 data / data.records / data.list / data.items / rows 下
    let candidates = [
        payload.get("data").and_then(|d| d.get("records")),
        payload.get("data").and_then(|d| d.get("list")),
        payload.get("data").and_then(|d| d.get("items")),
        payload.get("data").and_then(|d| d.get("rows")),
        payload.get("data").and_then(Value::as_array).map(|_| &Value::Null),
        payload.get("records"),
        payload.get("list"),
        payload.get("rows"),
    ];
    for candidate in candidates.into_iter().flatten() {
        if let Some(items) = candidate.as_array() {
            return items.clone();
        }
    }
    if let Some(items) = payload.get("data").and_then(Value::as_array) {
        return items.clone();
    }
    Vec::new()
}

/// 拉取某账号某时间区间的计费明细（接口失败 → Err，由调用方决定是否降级）。
async fn fetch_usage_rows(
    account: &BuddyAccount,
    start: &str,
    end: &str,
) -> Result<Vec<Value>, String> {
    let mut rows: Vec<Value> = Vec::new();
    for page in 1..=50 {
        let body = json!({
            "startTime": start,
            "endTime": end,
            "pageNum": page,
            "pageSize": 200,
        });
        let payload = super::api::travel_request(
            reqwest::Method::POST,
            "/billing/meter/get-user-request-usage",
            Some(body),
            &account.access_token,
            account.uid.as_deref(),
            account.enterprise_id.as_deref(),
            account.domain.as_deref(),
        )
        .await?;
        let page_rows = parse_usage_payload(&payload);
        let count = page_rows.len();
        rows.extend(page_rows);
        if count < 200 {
            break;
        }
    }
    Ok(rows)
}

/// 积分 + 调用量的按天统计（带缓存：今日 60 秒 TTL，历史日视为不可变）。
#[tauri::command]
pub async fn buddy_credit_usage_stats(
    account_id: Option<String>,
    days: Option<i64>,
    force: Option<bool>,
) -> Result<Value, String> {
    let days = clamp_days(days);
    let since = local_day(-(days - 1));
    let accounts: Vec<BuddyAccount> = match account_id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) => super::store::load_account(BuddyPlatform::Workbuddy, id)
            .into_iter()
            .collect(),
        None => super::store::list_accounts(BuddyPlatform::Workbuddy),
    };
    if accounts.is_empty() {
        return Err("没有可统计的 WorkBuddy 账号".to_string());
    }

    let cache_name = "credit-usage-cache.json";
    let mut cache = read_cache(cache_name).unwrap_or(json!({"version": 1, "accounts": {}}));
    let today = local_day(0);
    let mut all_days: BTreeMap<String, CreditDay> = BTreeMap::new();
    let mut account_rows: Vec<Value> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut per_model: BTreeMap<String, (i64, i64)> = BTreeMap::new();

    for account in &accounts {
        let uid = account.uid.clone().unwrap_or_else(|| account.id.clone());
        let entry = cache
            .get("accounts")
            .and_then(|items| items.get(&uid))
            .cloned()
            .unwrap_or(json!({"days": {}}));
        // 只补"缺失或今天（未定稿）"的日期
        let cached_days = entry.get("days").cloned().unwrap_or(json!({}));
        let missing: Vec<String> = (0..days)
            .map(|offset| local_day(-offset))
            .filter(|day| day.as_str() >= since.as_str())
            .filter(|day| {
                let cached = cached_days.get(day);
                let is_today = day == &today;
                let fresh = cached
                    .and_then(|item| item.get("queriedAt").and_then(Value::as_i64))
                    .map(|at| now_ms() - at < TODAY_TTL_MS)
                    .unwrap_or(false);
                is_today || cached.is_none() || (is_today && !fresh)
            })
            .collect();
        let mut days_map = cached_days.as_object().cloned().unwrap_or_default();
        if !missing.is_empty() && force != Some(true) {
            let start = format!("{} 00:00:00", missing.iter().min().cloned().unwrap_or(today.clone()));
            let end = format!("{} 23:59:59", today);
            match fetch_usage_rows(account, &start, &end).await {
                Ok(rows) => {
                    // 清零待更新的日期，避免重复累加
                    for day in &missing {
                        days_map.insert(
                            day.clone(),
                            json!({"credits": 0, "requests": 0, "final": day != &today, "queriedAt": now_ms(), "models": {}}),
                        );
                    }
                    for row in rows {
                        let Some((_, day, credit, model, _)) = normalize_usage_row(&row) else { continue };
                        if day < since {
                            continue;
                        }
                        let Some(entry) = days_map.get_mut(&day).and_then(Value::as_object_mut) else {
                            continue;
                        };
                        let credits = entry.get("credits").and_then(Value::as_i64).unwrap_or(0) + credit;
                        let requests = entry.get("requests").and_then(Value::as_i64).unwrap_or(0) + 1;
                        entry.insert("credits".into(), json!(credits));
                        entry.insert("requests".into(), json!(requests));
                        if let Some(models) = entry.get_mut("models").and_then(Value::as_object_mut) {
                            let current = models.get(&model).cloned().unwrap_or(json!({"credits": 0, "requests": 0}));
                            let c = current.get("credits").and_then(Value::as_i64).unwrap_or(0) + credit;
                            let r = current.get("requests").and_then(Value::as_i64).unwrap_or(0) + 1;
                            models.insert(model, json!({"credits": c, "requests": r}));
                        }
                    }
                }
                Err(error) => errors.push(format!("{}: {}", account.email, error)),
            }
        }
        if let Some(accounts_map) = cache.get_mut("accounts").and_then(Value::as_object_mut) {
            accounts_map.insert(uid.clone(), json!({"days": days_map, "updatedAt": now_ms()}));
        }

        let mut account_credits = 0i64;
        let mut account_requests = 0i64;
        for (day, value) in days_map.iter() {
            if day.as_str() < since.as_str() {
                continue;
            }
            let credits = value.get("credits").and_then(Value::as_i64).unwrap_or(0);
            let requests = value.get("requests").and_then(Value::as_i64).unwrap_or(0);
            account_credits += credits;
            account_requests += requests;
            let entry = all_days.entry(day.clone()).or_default();
            entry.credits += credits;
            entry.requests += requests;
            if let Some(models) = value.get("models").and_then(Value::as_object) {
                for (model, stats) in models {
                    let c = stats.get("credits").and_then(Value::as_i64).unwrap_or(0);
                    let r = stats.get("requests").and_then(Value::as_i64).unwrap_or(0);
                    let day_entry = all_days.entry(day.clone()).or_default();
                    let slot = day_entry.models.entry(model.clone()).or_insert((0, 0));
                    slot.0 += c;
                    slot.1 += r;
                    let global = per_model.entry(model.clone()).or_insert((0, 0));
                    global.0 += c;
                    global.1 += r;
                }
            }
        }
        account_rows.push(json!({
            "accountId": account.id,
            "uid": uid,
            "name": if account.email.trim().is_empty() { account.id.clone() } else { account.email.clone() },
            "credits": account_credits,
            "requests": account_requests,
        }));
    }
    cache["version"] = json!(1);
    write_cache(cache_name, &cache);

    let daily: Vec<Value> = all_days
        .iter()
        .map(|(day, value)| {
            json!({
                "day": day,
                "credits": value.credits,
                "requests": value.requests,
                "models": value.models.iter().map(|(model, (c, r))| json!({
                    "model": model, "credits": c, "requests": r,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let totals_credits: i64 = all_days.values().map(|value| value.credits).sum();
    let totals_requests: i64 = all_days.values().map(|value| value.requests).sum();
    let mut models: Vec<Value> = per_model
        .iter()
        .map(|(model, (credits, requests))| json!({"model": model, "credits": credits, "requests": requests}))
        .collect();
    models.sort_by_key(|item| -item.get("credits").and_then(Value::as_i64).unwrap_or(0));
    account_rows.sort_by_key(|item| -item.get("credits").and_then(Value::as_i64).unwrap_or(0));

    Ok(json!({
        "source": "official-billing",
        "since": since,
        "until": today,
        "totals": {"credits": totals_credits, "requests": totals_requests},
        "daily": daily,
        "accounts": account_rows,
        "models": models,
        "dailyBreakdown": all_days.iter().flat_map(|(day, value)| {
            value.models.iter().map(|(model, (credits, requests))| json!({
                "day": day, "model": model, "credits": credits, "requests": requests,
            })).collect::<Vec<_>>()
        }).collect::<Vec<_>>(),
        "errors": errors,
    }))
}

/// 清空统计缓存（前端"强制刷新"用）。
#[tauri::command]
pub async fn buddy_stats_clear_cache() -> Result<bool, String> {
    for name in ["token-stats-cache.json", "credit-usage-cache.json"] {
        let path = cache_path(name);
        if path.is_file() {
            std::fs::remove_file(&path).map_err(|e| format!("删除缓存失败: {}", e))?;
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_usage_in_nested_records_with_aliases() {
        let record = json!({
            "type": "assistant",
            "message": {"usage": {"prompt_tokens": 10, "completion_tokens": 20,
                "cache_read_input_tokens": 3, "cache_creation_input_tokens": 4}}
        });
        assert_eq!(find_usage(&record), Some((10, 20, 3, 4)));
        let camel = json!({"usage": {"inputTokens": 5, "outputTokens": 7}});
        assert_eq!(find_usage(&camel), Some((5, 7, 0, 0)));
        assert_eq!(find_usage(&json!({"a": 1})), None);
    }

    #[test]
    fn normalizes_usage_rows_with_aliases() {
        let row = json!({"request_id": "r1", "request_time": "2026-09-18 10:00:00", "credit": 2.6, "model": "m1"});
        let normalized = normalize_usage_row(&row).unwrap();
        assert_eq!(normalized.0, "r1");
        assert_eq!(normalized.1, "2026-09-18");
        assert_eq!(normalized.2, 3, "credit 应四舍五入");
        assert_eq!(normalized.3, "m1");
    }

    #[test]
    fn aggregates_buckets_into_all_dimensions() {
        let mut buckets: BTreeMap<(String, String, String), TokenBucket> = BTreeMap::new();
        buckets
            .entry(("2026-09-18".into(), "u1".into(), "m1".into()))
            .or_insert_with(TokenBucket::new)
            .add((10, 5, 1, 0));
        buckets
            .entry(("2026-09-18".into(), "u1".into(), "m2".into()))
            .or_insert_with(TokenBucket::new)
            .add((1, 2, 0, 0));
        buckets
            .entry(("2026-09-19".into(), "u2".into(), "m1".into()))
            .or_insert_with(TokenBucket::new)
            .add((100, 200, 0, 0));
        let view = aggregate_tokens(&buckets, "2026-09-01", 3);
        assert_eq!(view["totals"]["input"], 111);
        assert_eq!(view["totals"]["output"], 207);
        assert_eq!(view["totals"]["calls"], 3);
        assert_eq!(view["daily"].as_array().unwrap().len(), 2);
        assert_eq!(view["models"].as_array().unwrap().len(), 2);
        assert_eq!(view["models"][0]["model"], "m1", "按总量降序");
        assert_eq!(view["dailyBreakdown"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn filter_recomputes_view_without_rescanning() {
        let mut buckets: BTreeMap<(String, String, String), TokenBucket> = BTreeMap::new();
        buckets
            .entry(("2026-09-18".into(), "u1".into(), "m1".into()))
            .or_insert_with(TokenBucket::new)
            .add((10, 5, 0, 0));
        buckets
            .entry(("2026-09-18".into(), "u2".into(), "m1".into()))
            .or_insert_with(TokenBucket::new)
            .add((7, 7, 0, 0));
        let view = aggregate_tokens(&buckets, "2026-09-01", 2);
        let filtered = filter_token_view(&view, "account", "u2");
        assert_eq!(filtered["totals"]["input"], 7);
        assert_eq!(filtered["dailyBreakdown"].as_array().unwrap().len(), 1);
        assert_eq!(filtered["filtered"]["account"], "u2");
    }

    #[test]
    fn resolves_account_from_record_then_path() {
        let known = vec!["uid-abc".to_string()];
        let record = json!({"accountUid": "from-record"});
        assert_eq!(resolve_account(&record, Path::new("x.jsonl"), &known), "from-record");
        let record = json!({});
        assert_eq!(
            resolve_account(&record, Path::new("/tmp/uid-abc/session.jsonl"), &known),
            "uid-abc"
        );
        assert_eq!(resolve_account(&record, Path::new("/tmp/other.jsonl"), &known), "unknown");
    }
}
