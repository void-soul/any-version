//! interaction responsibility.
use super::*;

/// 流式派发事件（前端 listen 接收，替代轮询）
#[derive(Serialize, Clone)]
pub struct CollabDeltaPayload {
    pub room_id: String,
    pub msg_id: String,
    pub delta: String,
}

/// 活动状态推送（思考中/调用工具等），不写入消息内容，仅前端实时显示
#[derive(Serialize, Clone)]
pub struct CollabActivityPayload {
    pub room_id: String,
    pub msg_id: String,
    pub activity: String,
}

/// 工具询问用户选择时推送，前端显示交互式按钮
#[derive(Serialize, Clone)]
pub struct CollabPromptPayload {
    pub room_id: String,
    pub msg_id: String,
    pub question: String,
    pub options: Vec<String>,
}

#[derive(Serialize, Clone)]
pub struct CollabMsgUpdatedPayload {
    pub room_id: String,
    pub message: CollabMessage,
}

/// 派发生命周期控制（取消 / 超时）
#[derive(Clone)]
pub(super) struct DispatchCtrl {
    pub(super) child: Arc<Mutex<Option<Child>>>,
    pub(super) cancel: Arc<AtomicBool>,
}

pub(super) static DISPATCH_STATE: OnceLock<Mutex<HashMap<String, DispatchCtrl>>> = OnceLock::new();
pub(super) const DISPATCH_TIMEOUT_SECS: u64 = 1800;

/// 活动派发追踪：防止同一房间+工具的并发派发（TOCTOU 竞态防护）
// 用 tokio::sync::Mutex：其守卫是 Send，可在 async 命令中安全跨 await 持有，
// 避免 std MutexGuard 导致 Tauri 命令的 Future 非 Send 而无法编译。
pub(super) static ACTIVE_DISPATCHES: OnceLock<tokio::sync::Mutex<HashSet<String>>> =
    OnceLock::new();

/// 压缩回调：dispatch_to_tool 完成后通过 oneshot 通知 collab_compact_session
pub(super) static COMPACT_CALLBACKS: OnceLock<
    Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>,
> = OnceLock::new();

/// Prompt 响应控制：子进程的 stdin 句柄 + 待响应标记
pub(super) struct PromptCtrl {
    pub(super) stdin: Arc<Mutex<Option<ChildStdin>>>,
    /// (问题文本, 首次检测时间) — None 表示无待响应问题
    pub(super) pending: Arc<Mutex<Option<(String, Instant)>>>,
}

pub(super) static PROMPT_STATE: OnceLock<Mutex<HashMap<String, PromptCtrl>>> = OnceLock::new();
pub(super) const PROMPT_AUTO_RESPOND_SECS: u64 = 120;

/// 检测一行文本是否为工具的交互式询问
pub(super) fn detect_prompt(line: &str) -> Option<(String, Vec<String>)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    // y/n 模式
    if lower.contains("[y/n]") || lower.contains("(y/n)") || lower.contains("{y/n}") {
        let question = trimmed
            .trim_end_matches(|c: char| !c.is_alphanumeric())
            .to_string();
        return Some((question, vec!["y".to_string(), "n".to_string()]));
    }
    // yes/no 模式
    if lower.contains("(yes/no)") || lower.contains("[yes/no]") || lower.contains("(yes/no/cancel)")
    {
        let question = trimmed
            .trim_end_matches(|c: char| !c.is_alphanumeric())
            .to_string();
        return Some((question, vec!["yes".to_string(), "no".to_string()]));
    }
    // 常见问题模式
    let question_patterns = [
        "do you want",
        "would you like",
        "are you sure",
        "continue?",
        "proceed?",
        "confirm?",
        "overwrite?",
        "press enter",
        "press any key",
        "choose an option",
        "select an option",
        "enter your choice",
        "enter selection",
        "please select",
        "please choose",
        "please enter",
        "please provide",
    ];
    for pat in question_patterns {
        if lower.contains(pat) {
            return Some((trimmed.to_string(), vec!["y".to_string(), "n".to_string()]));
        }
    }
    None
}

/// 发送 prompt 事件到前端，并记录 pending 状态
pub(super) fn emit_prompt(
    app: &tauri::AppHandle,
    room_id: &str,
    msg_id: &str,
    question: &str,
    options: Vec<String>,
) {
    eprintln!(
        "[collab] ⚠ Prompt 检测到: question='{}', options={:?}, msg={}",
        question, options, msg_id
    );
    // 记录 pending 状态
    if let Some(map) = PROMPT_STATE.get() {
        let g = map.lock();
        if let Some(ctrl) = g.get(msg_id) {
            let mut p = ctrl.pending.lock();
            *p = Some((question.to_string(), Instant::now()));
        }
    }
    let _ = app.emit(
        "collab:prompt",
        CollabPromptPayload {
            room_id: room_id.to_string(),
            msg_id: msg_id.to_string(),
            question: question.to_string(),
            options,
        },
    );
}

/// 向子进程 stdin 写入响应。返回 true 表示成功写入；false 表示无待响应状态或 stdin 已关闭。
/// 无论写入是否成功，都会清除 pending 状态（避免 stdin 关闭后反复重试）。
pub(super) fn write_stdin_response(msg_id: &str, response: &str) -> bool {
    if let Some(map) = PROMPT_STATE.get() {
        let g = map.lock();
        if let Some(ctrl) = g.get(msg_id) {
            // 清除 pending 状态（无论写入结果如何，不再重试）
            *ctrl.pending.lock() = None;
            // 在同一把锁内检查并写入 stdin，避免 TOCTOU 竞态
            let mut guard = ctrl.stdin.lock();
            if let Some(ref mut stdin) = *guard {
                let _ = stdin.write_all(format!("{}\n", response).as_bytes());
                let _ = stdin.flush();
                eprintln!(
                    "[collab] ✓ prompt 已响应: response='{}', msg={}",
                    response, msg_id
                );
                return true;
            }
            eprintln!("[collab] ⚠ stdin 已关闭，无法响应: msg={}", msg_id);
            return false;
        }
    }
    eprintln!("[collab] ⚠ 未找到 prompt 状态: msg={}", msg_id);
    false
}
