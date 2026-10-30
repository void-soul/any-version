//! 收藏模块的 Tauri 命令。
//!
//! 全部**只读**：不调用任何平台的写入接口。

use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;

use super::bilibili;
use super::check;
use super::classify::{self, BATCH_SIZE};
use super::db::{self, NewFavorite};
use super::github;
use super::organize;
use super::zhihu;

// ─── 任务调度：谁可以和谁同时跑 ───
//
// 三个平台的导入各拉各的接口、互不相干，**允许同时跑**（用户点三个「导入」就该三个一起跑）；
// 但「导入」与「加工」（AI 归类 / 失效检测）必须互斥：加工要扫全库挑条目，
// 和正在写入的导入抢同一份数据，也会把刚导入、还没稳定的条目算进去。
//
// 因此这里维护两张表（都按**任务名**分开记，不再是一个全局 bool）：
// - RUNNING：当前正在跑的任务，用于互斥判定；
// - CANCEL：用户点过「停止」的任务，用于循环里的中断检查。
//
// 用一张全局 bool 的时代，任一任务入口都会把它清掉 —— 并发下这等于顺手抹掉别人的
// 停止请求；点「停止 GitHub」也会把同时跑的 B站/知乎一起停掉。

const TASK_GITHUB: &str = "github";
const TASK_BILIBILI: &str = "bilibili";
const TASK_ZHIHU: &str = "zhihu";
const TASK_CLASSIFY: &str = "classify";
const TASK_CHECK: &str = "check";
const ALL_TASKS: [&str; 5] = [
    TASK_GITHUB,
    TASK_BILIBILI,
    TASK_ZHIHU,
    TASK_CLASSIFY,
    TASK_CHECK,
];

/// 当前正在跑的任务名（同一时刻最多：三个导入，或单独一个加工任务）。
static RUNNING: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// 被用户点「停止」的任务名。
static CANCEL: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// 锁被毒化时照样取内容：这里存的是任务名列表，坏掉一个也不会让收藏模块不可用。
fn tasks_lock<'a>(
    g: std::sync::PoisonError<std::sync::MutexGuard<'a, Vec<String>>>,
) -> std::sync::MutexGuard<'a, Vec<String>> {
    g.into_inner()
}

fn is_import_task(task: &str) -> bool {
    matches!(task, TASK_GITHUB | TASK_BILIBILI | TASK_ZHIHU)
}

fn task_label(task: &str) -> &'static str {
    match task {
        TASK_GITHUB => "GitHub 导入",
        TASK_BILIBILI => "B站导入",
        TASK_ZHIHU => "知乎导入",
        TASK_CLASSIFY => "AI 归类",
        TASK_CHECK => "失效检测",
        _ => "其他任务",
    }
}

/// 两个任务能否同时跑：只有「导入 × 导入」可以，其余一律互斥（含同名任务自身）。
fn tasks_conflict(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    !(is_import_task(a) && is_import_task(b))
}

/// 任务开工：与在跑的任务互斥时直接报错，界面上按钮也是灰的，双保险。
fn begin_task(task: &str) -> Result<(), String> {
    let mut g = RUNNING.lock().unwrap_or_else(tasks_lock);
    if let Some(other) = g.iter().find(|t| tasks_conflict(task, t)) {
        return Err(format!(
            "「{}」正在运行，请等它结束（或点「停止」）后再开始",
            task_label(other)
        ));
    }
    g.push(task.to_string());
    Ok(())
}

/// 任务收工（成功失败都要调，否则后面的任务会被永久挡住）。
fn end_task(task: &str) {
    let mut g = RUNNING.lock().unwrap_or_else(tasks_lock);
    if let Some(pos) = g.iter().position(|t| t == task) {
        g.remove(pos);
    }
}

/// 清掉本任务的停止标记：上一次点过停止，不该影响这一次。
fn reset_cancel(task: &str) {
    let mut g = CANCEL.lock().unwrap_or_else(tasks_lock);
    g.retain(|t| t != task);
}

/// 用户是否点了本任务的「停止」。
fn is_cancelled(task: &str) -> bool {
    CANCEL
        .lock()
        .unwrap_or_else(tasks_lock)
        .iter()
        .any(|t| t == task)
}

/// 请求停止：`task` 为 None 时停止所有任务（前端的兜底用法）。
fn request_cancel(task: Option<&str>) {
    let mut g = CANCEL.lock().unwrap_or_else(tasks_lock);
    let targets: Vec<&str> = match task {
        Some(t) if !t.trim().is_empty() => vec![t],
        _ => ALL_TASKS.to_vec(),
    };
    for t in targets {
        if !g.iter().any(|x| x == t) {
            g.push(t.to_string());
        }
    }
}

/// 单次导入最多翻多少页：star 上千时防止一次跑太久（每页 100 条）。
const MAX_PAGES: usize = 100;

/// 导入 / 归类的实时进度事件（前端 `favorites-progress`）。
///
/// 字段全部可选、按阶段取用：导入阶段给抓取/新增计数与当前收藏夹，
/// 归类阶段给已归类/剩余/标签数。`done = true` 表示整趟结束（汇总值）。
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FavoritesProgress {
    /// import | classify | check
    stage: &'static str,
    /// 任务名（github / bilibili / zhihu / classify / check）：
    /// 多个导入会同时发进度，前端按它分行展示，否则后一个会盖掉前一个。
    task: String,
    source: Option<String>,
    /// 当前正在处理的收藏夹 / 阶段说明
    folder: Option<String>,
    message: Option<String>,
    fetched: Option<usize>,
    added: Option<usize>,
    updated: Option<usize>,
    skipped: Option<usize>,
    classified: Option<usize>,
    tags_written: Option<usize>,
    remaining: Option<usize>,
    /// 知乎专用：当前收藏夹已抓取条数 / 服务端报告的总数（Paging.Totals）
    folder_fetched: Option<usize>,
    folder_total: Option<i64>,
    /// 失效检测专用：已探测条数 / 本轮待探测总数
    checked: Option<usize>,
    check_total: Option<usize>,
    done: bool,
}

fn emit_progress(app: &tauri::AppHandle, progress: &FavoritesProgress) {
    use tauri::Emitter;
    // `task` 是前端分行的键：漏填（默认空串）时事件会被前端按任务名过滤掉，
    // 表现为「后端在跑、界面一行进度都没有」——不会报错，只能靠日志看出来。
    // 这里留一条兜底告警；真正的把关是 `every_progress_literal_sets_task` 那条测试。
    if progress.task.is_empty() {
        eprintln!(
            "[收藏] 进度事件缺 task（前端会丢弃它）: stage={}, source={:?}",
            progress.stage, progress.source
        );
    }
    let _ = app.emit("favorites-progress", progress);
}

/// 汇总一条已累计的导入计数。
fn import_progress(
    stage: &'static str,
    source: &str,
    folder: Option<String>,
    message: Option<String>,
    result: &ImportResult,
    done: bool,
) -> FavoritesProgress {
    FavoritesProgress {
        stage,
        // 导入阶段任务名就是来源名（github / bilibili / zhihu）
        task: source.to_string(),
        source: Some(source.to_string()),
        folder,
        message,
        fetched: Some(result.fetched),
        added: Some(result.added),
        updated: Some(result.updated),
        skipped: Some(result.skipped),
        done,
        ..FavoritesProgress::default()
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub login: String,
    /// 本次抓到的条数
    pub fetched: usize,
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
    /// 命中删除墓碑而跳过的条数：**用户之前删过、平台上还在**，这次没有捞回来
    #[serde(default)]
    pub skipped_deleted: usize,
    pub cancelled: bool,
    /// 读取失败的收藏夹（`标题（原因）`）；其余收藏夹照常导入
    #[serde(default)]
    pub failed: Vec<String>,
}

/// 停止长任务（导入 / AI 归类 / 失效检测）。
///
/// 各任务在**下一个循环开始前**检查自己的停止标记，所以停止是「不再往下走」
/// 而不是中断进行中的请求。`task` 为空时停止全部（导入可以同时跑，通常按名字停其中一个）。
#[tauri::command]
pub fn fav_cancel(task: Option<String>) -> Result<(), String> {
    request_cancel(task.as_deref());
    Ok(())
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifyResult {
    /// 本轮归类的条目数
    pub classified: usize,
    /// 新写出的「条目-标签」对数（多标签会大于条目数）
    pub tags_written: usize,
    pub model: String,
    /// 还剩多少条没归类（下一轮继续）
    pub remaining: usize,
    /// 用户中途点了停止（已归类的部分保留）
    pub cancelled: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub checked: usize,
    pub gone: usize,
    pub redirect: usize,
    pub unknown: usize,
    /// 因限流提前中断（没跑完，下次再点会继续）
    pub aborted: bool,
    /// 用户点了停止（已检测的条目状态已落库）
    pub cancelled: bool,
    /// 库里有 GitHub 条目但没配 Token → 这批没查（其它来源照查）
    pub skipped_no_token: usize,
}

/// 一次并发探测多少个 URL（太小慢、太大容易被目标站按 IP 限流）。
const PROBE_CONCURRENCY: usize = 8;

/// 单趟导入最多翻多少页（内容接口每页 [`zhihu::PAGE_SIZE`] 条）。
///
/// 2032 条的收藏夹约 102 页，给到 2000 页足够用；上限的意义是防「服务端一直
/// 返回 is_end=false」时把我们拖进死循环。
const ZHIHU_MAX_PAGES: usize = 2000;

// ─── 分类树（多级，操作逻辑对齐启动模块） ───

/// 供应商：必须是用户**显式选的**那个，不做任何兜底。
///
/// 以前缺省时会回落到「AI 模块的默认供应商」，结果是「以为跑在 A 上，其实花的是 B 的额度」。
fn pick_explicit_provider<'a>(
    providers: &'a [crate::commands::ai::models::AiProvider],
    provider_id: &Option<String>,
) -> Result<&'a crate::commands::ai::models::AiProvider, String> {
    let pid = provider_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "请先选择供应商（收藏模块的归类不使用 AI 模块的默认供应商）".to_string())?;
    providers
        .iter()
        .find(|p| p.id == pid)
        .ok_or_else(|| format!("未找到供应商: {pid}"))
}

/// 模型：同样必须显式选，**不**回落供应商的激活模型 / 首个模型。
fn pick_explicit_model(
    provider: &crate::commands::ai::models::AiProvider,
    model_id: &Option<String>,
) -> Result<String, String> {
    model_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            format!(
                "请先选择模型（供应商「{}」的激活模型/首个模型不会自动沿用）",
                provider.name
            )
        })
}

/// 解析「用哪个供应商的哪个模型」——两项都必须显式指定。
fn resolve_ai_target(
    cfg: &crate::commands::ai::models::AiConfig,
    provider_id: &Option<String>,
    model_id: &Option<String>,
) -> Result<(crate::commands::ai::models::AiProvider, String), String> {
    let provider = pick_explicit_provider(&cfg.providers, provider_id)?.clone();
    let model = pick_explicit_model(&provider, model_id)?;
    if provider.openai_url.is_empty() {
        return Err(format!("供应商「{}」未配置 OpenAI 兼容端点", provider.name));
    }
    if provider.api_key.is_empty() {
        return Err(format!("供应商「{}」未配置 API Key", provider.name));
    }
    Ok((provider, model))
}

mod github_import;
pub use github_import::*;
mod bilibili_import;
pub use bilibili_import::*;
mod zhihu_import;
pub use zhihu_import::*;
mod classification;
pub use classification::*;
mod checking;
pub use checking::*;
mod content;
pub use content::*;
mod library;
pub use library::*;
#[cfg(test)]
mod tests;
