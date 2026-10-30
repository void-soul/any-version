//! models responsibility.
use super::*;

// ─── 协作线程数据模型 ───

/// 引用卡：用户引用某段内容时记录来源与原文
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CollabReference {
    pub source_message_id: String,
    pub source_sender_name: String,
    pub excerpt: String,
}

/// 文件附件：用户在输入框 @ 选择文件，派发时把内容注入提示词
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CollabFileRef {
    pub path: String,
}

/// token 消耗（来自工具输出的 usage 字段；非所有工具都提供，故为 Option）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// 派发标记：本条消息触发了对某工具的派发
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CollabDispatch {
    pub tool_id: String,
    pub session_id: String,
    pub model: Option<String>,
    /// 派发总耗时（毫秒），收尾时回填
    pub duration_ms: Option<u64>,
    /// token 消耗；工具输出含 usage 时回填，否则 None
    pub usage: Option<TokenUsage>,
}

/// 上下文快照：压缩旧会话后生成的摘要，用于在新会话中恢复上下文
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ContextSnapshot {
    pub id: String,
    pub room_id: String,
    pub tool_id: String,
    pub summary: String,
    pub old_session_id: String,
    pub message_count: usize,
    pub created_at: String,
}

/// 派发输出归档（抄 orca `worker_terminal_archives`）：取消 / 超时 / 出错前把已产出的
/// 输出落一份。orca 的理由是「释放终端后输出就没了」，我们这里是「进程被杀后
/// 半截输出只存在于那条消息里，下一次派发会把 content 整个覆盖掉」。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DispatchArchive {
    pub id: String,
    pub room_id: String,
    pub tool_id: String,
    pub message_id: String,
    pub content: String,
    /// 归档原因：`error`（含超时 / 用户取消）
    pub reason: String,
    pub created_at: String,
}

/// 线程中的一条消息
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CollabMessage {
    pub id: String,
    pub room_id: String,
    /// "user" 或工具 id
    pub sender: String,
    /// 展示名："我" / "Claude Code"
    pub sender_name: String,
    /// 展示头像（emoji 或单字符），来自工具 config.avatar；旧消息无此字段时按 sender 回退
    #[serde(default)]
    pub avatar: Option<String>,
    pub content: String,
    pub references: Vec<CollabReference>,
    pub files: Vec<CollabFileRef>,
    pub dispatch: Option<CollabDispatch>,
    /// 工具回复回链到触发它的用户消息
    pub reply_to: Option<String>,
    /// 工具消息状态："running" | "done" | "error"
    pub status: Option<String>,
    pub created_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CollabRoom {
    pub id: String,
    pub name: String,
    pub project_path: String,
    /// 输入域策略（抄 open-tag `authorization.md` 的 sealed 模式）：
    /// - `open`（默认）：任何来源的文本都能进上下文；
    /// - `sealed`：只有**人类**消息与 [`CollabRoom::allowed_sources`] 里列出的工具
    ///   能进上下文，其余 agent 的引用一律剔除。
    ///
    /// 用途：多个 agent 互喂时，一个 agent 的输出（可能含不可信指令）会被当作另一个
    /// agent 的「事实」读进去。sealed 把这条链路收口到可信来源。
    #[serde(default = "default_input_policy")]
    pub input_policy: String,
    /// sealed 模式下允许进入上下文的来源（tool id 列表）
    #[serde(default)]
    pub allowed_sources: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub(super) fn default_input_policy() -> String {
    "open".to_string()
}

/// 输入域判定：该来源的文本能否进入这个房间的上下文。
///
/// 人类（"user" / "我"）与空来源永远放行 —— sealed 只约束 agent 之间互喂，
/// 把人挡在外面没有意义。
pub fn input_allowed(room: Option<&CollabRoom>, source_sender: &str) -> bool {
    let Some(room) = room else {
        return true;
    };
    if room.input_policy != "sealed" {
        return true;
    }
    let sender = source_sender.trim();
    if sender.is_empty() || sender == "user" || sender == "我" {
        return true;
    }
    room.allowed_sources.iter().any(|s| s == sender)
}

/// 按房间的输入域策略过滤引用卡（引用卡是最容易绕过输入域的入口）。
pub fn filter_references_by_policy(
    room: Option<&CollabRoom>,
    room_msgs: &[CollabMessage],
    refs: Vec<CollabReference>,
) -> Vec<CollabReference> {
    refs.into_iter()
        .filter(|r| {
            // 引用卡只存了展示名，来源 agent 的 id 要回消息表去查
            let sender = room_msgs
                .iter()
                .find(|m| m.id == r.source_message_id)
                .map(|m| m.sender.as_str())
                .unwrap_or("");
            let ok = input_allowed(room, sender);
            if !ok {
                eprintln!(
                    "[collab] 输入域拦截：sealed 房间剔除了来自 {} 的引用",
                    sender
                );
            }
            ok
        })
        .collect()
}

/// 协作任务（任务流 E：open/claimed/in_progress/in_review/done），agent 可通过总线创建/认领/交接/完成
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CollabTask {
    pub id: String,
    pub room_id: String,
    pub title: String,
    /// 任务详情 / 验收标准
    #[serde(default)]
    pub description: String,
    /// open | claimed | in_progress | in_review | done
    #[serde(default = "default_task_status")]
    pub status: String,
    /// 认领者 tool id
    #[serde(default)]
    pub assignee: Option<String>,
    /// 创建者 tool id（人或工具）
    #[serde(default)]
    pub created_by: String,
    /// 父任务（用于拆分/交接链）
    #[serde(default)]
    pub parent_task: Option<String>,
    /// 终态结果（抄 orca `worker_done --outcome`）：`done` | `failed` | `abandoned`。
    /// 只有走到 `done` / `failed` / `abandoned` 时才填 —— 「完成」和「做失败了」在协同里
    /// 是两件完全不同的事，光靠 status 分不出来（orca 的权威来自 dispatch 的 outcome）。
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

pub(super) fn default_task_status() -> String {
    "open".to_string()
}

/// 任务的合法状态（orca 用 SQL CHECK 约束，我们这是同一份约束的 Rust 表达）。
pub const TASK_STATUSES: &[&str] = &["open", "claimed", "in_progress", "in_review", "done"];

/// 任务的终态结果取值（与 [`CollabTask::outcome`] 对应）。
pub const TASK_OUTCOMES: &[&str] = &["done", "failed", "abandoned"];

/// 允许的状态迁移（**状态机**：不让「open 直接跳 in_review」这类跳步写进存储）。
///
/// `open` → 认领 / 直接开工 / 直接完成；`done` 只保留「重开」一条出路，
/// 免得一个已完成的任务被别的 agent 悄悄改回进行中。
pub fn allowed_task_transitions(from: &str) -> &'static [&'static str] {
    match from {
        "open" => &["claimed", "in_progress", "done"],
        "claimed" => &["in_progress", "open", "done"],
        "in_progress" => &["in_review", "done", "open"],
        "in_review" => &["in_progress", "done", "open"],
        "done" => &["open"],
        _ => &[],
    }
}

/// 迁移是否被允许（未知状态一律拒绝，避免写进一个谁都不认识的状态）。
pub fn can_transition_task(from: &str, to: &str) -> bool {
    allowed_task_transitions(from).contains(&to)
}

/// 认领任务：**锁内条件更新**（抄 open-tag `core.ts:840` 的原子 CAS）。
///
/// 旧实现是「find 到就直接赋值」，两个 agent 同时认领时后写的静默覆盖先写的 ——
/// 双方都拿到 `ok: true`，于是同一件事被做两遍。CAS 的判据就是
/// `assignee 为空 或 已经是自己`；被别人占着就直接拒绝并说清是谁占的。
pub fn claim_task(
    store: &mut CollabStore,
    room_id: &str,
    task_id: &str,
    by: &str,
) -> Result<CollabTask, String> {
    let Some(task) = store
        .tasks
        .iter_mut()
        .find(|t| t.id == task_id && t.room_id == room_id)
    else {
        return Err("not found".to_string());
    };
    if let Some(holder) = task.assignee.as_deref() {
        if holder == by {
            // 自己重复认领是幂等的（重放 / 双击不该报错）
            return Ok(task.clone());
        }
        return Err(format!(
            "task_already_claimed: {} 已被 {} 认领，不能重复认领",
            task_id, holder
        ));
    }
    if !can_transition_task(&task.status, "claimed") {
        return Err(format!(
            "invalid_transition: {} → claimed（当前状态 {}）",
            task.status, task.status
        ));
    }
    task.assignee = Some(by.to_string());
    task.status = "claimed".to_string();
    task.outcome = None;
    task.updated_at = now_str();
    Ok(task.clone())
}

/// 置任务终态：校验 outcome 取值并落库（终态要带结果，光有 status 分不出成功/失败）。
pub fn finish_task(
    store: &mut CollabStore,
    room_id: &str,
    task_id: &str,
    outcome: &str,
) -> Result<CollabTask, String> {
    if !TASK_OUTCOMES.contains(&outcome) {
        return Err(format!(
            "invalid_outcome: {}（可选 {})",
            outcome,
            TASK_OUTCOMES.join(" / ")
        ));
    }
    let Some(task) = store
        .tasks
        .iter_mut()
        .find(|t| t.id == task_id && t.room_id == room_id)
    else {
        return Err("not found".to_string());
    };
    if !can_transition_task(&task.status, "done") {
        return Err(format!(
            "invalid_transition: {} → done（当前状态 {}）",
            task.status, task.status
        ));
    }
    task.status = "done".to_string();
    task.outcome = Some(outcome.to_string());
    task.updated_at = now_str();
    Ok(task.clone())
}

/// 交接任务：显式转移 assignee（不是认领，不受「已被认领」限制），
/// 但**只有当前持有者或创建者**能转 —— 否则任意 agent 都能把别人的任务抢走。
pub fn handoff_task(
    store: &mut CollabStore,
    room_id: &str,
    task_id: &str,
    to: &str,
    by: &str,
) -> Result<CollabTask, String> {
    let Some(task) = store
        .tasks
        .iter_mut()
        .find(|t| t.id == task_id && t.room_id == room_id)
    else {
        return Err("not found".to_string());
    };
    let holder = task.assignee.clone().unwrap_or_default();
    if !holder.is_empty() && holder != by && !task.created_by.is_empty() && task.created_by != by {
        return Err(format!(
            "not_authorized: {} 当前由 {} 持有，只有持有者或创建者能交接",
            task_id, holder
        ));
    }
    // 交接换的是人、不是阶段：已开工的任务不能因为换人就退回 claimed，
    // 已完成的任务不能再派活。
    if task.status == "done" {
        return Err(format!(
            "invalid_transition: 任务 {task_id} 已完成，不能再交接"
        ));
    }
    if task.status == "open" {
        task.status = "claimed".to_string();
    }
    task.assignee = Some(to.to_string());
    task.outcome = None;
    task.updated_at = now_str();
    Ok(task.clone())
}

#[derive(Serialize, Deserialize, Default)]
pub struct CollabStore {
    /// Schema 版本号（未来结构变更时用于迁移判定）
    #[serde(default)]
    pub version: u32,
    pub rooms: Vec<CollabRoom>,
    pub messages: HashMap<String, Vec<CollabMessage>>,
    /// 房间+工具 → 是否已有会话（用于续聊判断）
    pub tool_sessions: HashMap<String, String>,
    /// 房间+工具 → 上下文快照（压缩后生成，新会话首次派发时注入）
    #[serde(default)]
    pub context_snapshots: HashMap<String, ContextSnapshot>,
    /// 派发轮次：同一发送者在静默窗口内的多条消息合并为一次派发
    #[serde(default)]
    pub turns: Vec<CollabTurn>,
    /// 回复授权槽位（持久化替代内存 ACTIVE_DISPATCHES，重启后可恢复防重复派发）
    #[serde(default)]
    pub reply_grants: Vec<ReplyGrant>,
    /// agent 在线状态（每个工具的当前运行状态）
    #[serde(default)]
    pub agents: Vec<CollabAgentStatus>,
    /// 任务流（E）
    #[serde(default)]
    pub tasks: Vec<CollabTask>,
    /// 派发输出归档（释放前留存，见 [`DispatchArchive`]）
    #[serde(default)]
    pub dispatch_archives: Vec<DispatchArchive>,
}

/// 回复授权槽位（回复协调用）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ReplyGrant {
    pub id: String,
    /// 触发消息 id（通常为 turn.trigger_message_id 或用户消息 id）
    pub message_id: String,
    pub tool_id: String,
    /// primary（主回复，唯一）| directed（被 @ 协同者）| supplemental（补充/纠正）
    pub slot: String,
    /// none | reserved | active | consumed | released
    pub status: String,
}

/// agent 在线状态
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CollabAgentStatus {
    pub tool_id: String,
    /// offline | online | thinking | working
    pub status: String,
    pub current_room: Option<String>,
    /// 最近一次心跳（UTC 时间戳字符串）
    pub last_heartbeat: String,
}

/// Turn 静默窗口（毫秒）：窗口内同一发送者的新消息合并进同一轮次
pub(super) const TURN_WINDOW_MS: u64 = 800;
/// 唤醒深度上限：防止工具互 @ 形成无限循环风暴
pub(super) const MAX_AGENT_WAKE_DEPTH: u32 = 4;
/// 同一根消息的最大唤醒次数
pub(super) const MAX_AGENT_WAKES_PER_ROOT: u32 = 6;

/// **全局并行 agent 上限**（orca 恰恰没有这个上限，我们必须自己补）。
///
/// orca 的并发约束只落在 git 子进程与磁盘上（`git-admission-state.ts`：general 2~4 /
/// network 3），agent 进程本身不限流 —— 它在远端跑，打满的是别人的配额。
/// 我们是**本地**桌面工具：同时开 N 个 Codex / Claude Code 会打满 CPU 与供应商配额，
/// 所以派发入口必须有硬上限。取 4，与 orca 的 `GENERAL_CAP` 同量级。
pub(super) const MAX_PARALLEL_AGENTS: usize = 4;

pub(super) static AGENT_SLOTS: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
