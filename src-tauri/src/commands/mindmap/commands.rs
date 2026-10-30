//! Tauri 命令：思维导图 CRUD + AI 生成 + 导出。

use super::models::*;
use crate::commands::ai;
use tauri::Emitter;

// ─── Progreso en vivo (eventos a la UI) ───

/// Emite un evento de progreso del importador IA a la interfaz.
/// El frontend escucha "mm-ai-progress" y pinta el log paso a paso.
/// `extra` aporta los campos estructurados del paso (round/reason/files/views/view/count/detail).
fn emit_progress(app: &Option<tauri::AppHandle>, step: &str, extra: serde_json::Value) {
    if let Some(handle) = app {
        let mut payload = extra;
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("step".into(), serde_json::json!(step));
        }
        let _ = handle.emit("mm-ai-progress", payload);
    }
}

/// 截断长文本用于事件回传（思考/工具参数可能很长，别把事件缓冲撑爆）。
fn truncate_for_log(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    let head: String = chars[..max_chars].iter().collect();
    format!("{}…（已截断，共 {} 字）", head, chars.len())
}

// ─── AI 运行取消（类似 IDE 中断构建） ───

/// 取消时返回的标准错误文本（前端据此识别为「用户主动取消」而非失败）。
const ERR_CANCELLED: &str = "已取消";

type CancelFlag = std::sync::Arc<std::sync::atomic::AtomicBool>;
type CancelRegistry = std::sync::Mutex<std::collections::HashMap<String, CancelFlag>>;
type AskSender = tokio::sync::oneshot::Sender<serde_json::Value>;
type AskRegistry = std::sync::Mutex<std::collections::HashMap<String, AskSender>>;

/// 进程级 AI 运行取消标志注册表：每个 run_id 一把独立 AtomicBool，避免并行运行互相误伤。
fn cancel_registry() -> &'static CancelRegistry {
    static CANCELS: std::sync::OnceLock<CancelRegistry> = std::sync::OnceLock::new();
    CANCELS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn ai_cancel_flag(run_id: &str) -> CancelFlag {
    let mut g = cancel_registry().lock().unwrap();
    g.entry(run_id.to_string())
        .or_insert_with(|| std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)))
        .clone()
}

/// 运行结束后从同一个注册表移除该 run_id 的 flag（新导入会重新创建干净标志）。
fn ai_drop_flag(run_id: &str) {
    cancel_registry().lock().unwrap().remove(run_id);
}

fn is_cancelled(flag: &std::sync::atomic::AtomicBool) -> bool {
    flag.load(std::sync::atomic::Ordering::Relaxed)
}

/// 取消检查：已取消时推送 cancel 事件并返回 Err(已取消)。
fn cancel_err(
    app: &Option<tauri::AppHandle>,
    flag: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    if is_cancelled(flag) {
        emit_progress(app, "cancel", serde_json::json!({}));
        return Err(ERR_CANCELLED.into());
    }
    Ok(())
}

// ─── AI 询问通道（ask-user）───
// AI 在生成中遇到信息不足/歧义时，可输出 {"ask": {...}} 向用户提问；
// 后端注册一个 oneshot 发送端（按 run_id 索引），阻塞等待前端 mm_ai_answer 回填，
// 然后把用户回答追加进提示词继续生成。取消时（mm_ai_cancel）会向该通道发送
// Null 标记把等待解除，避免用户点「停止」后运行卡死在提问处。
fn ask_registry() -> &'static AskRegistry {
    static ASKS: std::sync::OnceLock<AskRegistry> = std::sync::OnceLock::new();
    ASKS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn ask_register(run_id: &str, tx: AskSender) {
    ask_registry()
        .lock()
        .unwrap()
        .insert(run_id.to_string(), tx);
}

/// 用户回填询问答案（mm_ai_answer 调用）。无等待中的询问时返回 Err。
fn ask_send_answer(run_id: &str, answer: serde_json::Value) -> Result<(), String> {
    match ask_registry().lock().unwrap().remove(run_id) {
        Some(tx) => tx.send(answer).map_err(|_| "询问已结束".into()),
        None => Err("当前没有等待中的询问".into()),
    }
}

/// 取消时解除等待中的询问（mm_ai_cancel 调用，尽力而为）。
fn ask_send_cancel(run_id: &str) {
    if let Some(tx) = ask_registry().lock().unwrap().remove(run_id) {
        let _ = tx.send(serde_json::Value::Null);
    }
}

/// 把 AI 输出的 ask 归一化为 { question, fields:[{key,label,type,options,default}] }。
fn normalize_ask(ask: &serde_json::Value) -> serde_json::Value {
    match ask {
        serde_json::Value::String(s) => serde_json::json!({ "question": s, "fields": [] }),
        serde_json::Value::Object(_) => {
            let mut obj = ask.as_object().unwrap().clone();
            if !obj.contains_key("question") {
                obj.insert(
                    "question".into(),
                    serde_json::Value::String("请补充以下信息".into()),
                );
            }
            if !obj.contains_key("fields")
                || !obj.get("fields").map(|v| v.is_array()).unwrap_or(false)
            {
                obj.insert("fields".into(), serde_json::json!([]));
            }
            serde_json::Value::Object(obj)
        }
        _ => serde_json::json!({ "question": "请补充以下信息", "fields": [] }),
    }
}

/// 把「询问 + 用户回答」格式化为追加进提示词的文本人话。
fn format_ask_answer(ask: &serde_json::Value, answer: &serde_json::Value) -> String {
    let mut out = String::new();
    let q = ask.get("question").and_then(|x| x.as_str()).unwrap_or("");
    if !q.is_empty() {
        out.push_str(&format!("问题：{}\n", q));
    }
    let fields = ask.get("fields").and_then(|x| x.as_array());
    match (fields, answer) {
        (Some(fs), a) if !fs.is_empty() => {
            out.push_str("回答：\n");
            for f in fs {
                let key = f.get("key").and_then(|x| x.as_str()).unwrap_or("");
                let label = f.get("label").and_then(|x| x.as_str()).unwrap_or(key);
                let val = a
                    .get(key)
                    .map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .unwrap_or_default();
                out.push_str(&format!("- {}：{}\n", label, val));
            }
        }
        (_, serde_json::Value::String(s)) if !s.trim().is_empty() => {
            out.push_str(&format!("回答：{}\n", s));
        }
        (_, a) if !a.is_null() => {
            out.push_str(&format!("回答：{}\n", a));
        }
        _ => {
            out.push_str("回答：（用户未填写，请基于现有信息合理推断并继续）\n");
        }
    }
    out
}

// ─── 工具 ───

/// 一次 AI 运行（路由/探索/逐视图生成）的 token 累计器：原子计数，异步任务间安全共享。
#[derive(Default)]
struct UsageAcc {
    requests: std::sync::atomic::AtomicU64,
    input: std::sync::atomic::AtomicU64,
    output: std::sync::atomic::AtomicU64,
    total: std::sync::atomic::AtomicU64,
}

impl UsageAcc {
    fn snapshot(&self) -> UsageStats {
        use std::sync::atomic::Ordering;
        UsageStats {
            requests: self.requests.load(Ordering::Relaxed),
            input_tokens: self.input.load(Ordering::Relaxed),
            output_tokens: self.output.load(Ordering::Relaxed),
            total_tokens: self.total.load(Ordering::Relaxed),
        }
    }

    /// 相对某个基准快照的增量（用于算「单个视图」的消耗，探索/路由等公共部分不计入）。
    fn diff(&self, base: &UsageStats) -> UsageStats {
        let s = self.snapshot();
        UsageStats {
            requests: s.requests.saturating_sub(base.requests),
            input_tokens: s.input_tokens.saturating_sub(base.input_tokens),
            output_tokens: s.output_tokens.saturating_sub(base.output_tokens),
            total_tokens: s.total_tokens.saturating_sub(base.total_tokens),
        }
    }
}

/// 思维导图 AI 导入的供应商/模型解析：优先级与翻译等其它 AI 功能一致——
/// 1. 显式传入（面板当前选择）
/// 2. 全局默认 AI 模型（全局设置中选择，存于 translate_config.json）——校验可用性，
///    失效时静默回退首个可用供应商
/// 3. 首个有 api_key 且配置了 OpenAI 端点的供应商
fn resolve_provider_model(
    pid: &Option<String>,
    mid: &Option<String>,
) -> Result<(ai::models::AiProvider, String), String> {
    let cfg = ai::config::load_ai_config();
    let default_cfg = ai::translate::load_translate_config();
    let (p, explicit_mid) = if let Some(id) = pid {
        (
            cfg.providers
                .iter()
                .find(|x| &x.id == id)
                .cloned()
                .ok_or_else(|| format!("未找到供应商: {}", id))?,
            mid.clone(),
        )
    } else {
        // 全局默认供应商可用（存在且配了端点与 key）才采用；否则回退首个可用供应商。
        // 默认模型只与默认供应商成对使用，避免跨供应商拼出不存在的模型 id。
        match (&default_cfg.provider_id, mid) {
            (Some(gpid), _) => match cfg.providers.iter().find(|x| &x.id == gpid) {
                Some(p) if !p.openai_url.is_empty() && !p.api_key.is_empty() => {
                    (p.clone(), mid.clone().or(default_cfg.model_id.clone()))
                }
                _ => (first_usable_provider(&cfg)?, None),
            },
            _ => (first_usable_provider(&cfg)?, None),
        }
    };
    if p.openai_url.is_empty() {
        return Err(format!("供应商 '{}' 未配置端点", p.name));
    }
    if p.api_key.is_empty() {
        return Err(format!("供应商 '{}' 未配置 Key", p.name));
    }
    let m = explicit_mid
        .or_else(|| p.active_model_id.clone())
        .or_else(|| p.models.first().map(|m| m.id.clone()))
        .ok_or("无可用模型")?;
    Ok((p, m))
}

/// 首个有 api_key 且配置了 OpenAI 端点的供应商（全局默认失效/缺省时的回退）。
fn first_usable_provider(cfg: &ai::models::AiConfig) -> Result<ai::models::AiProvider, String> {
    cfg.providers
        .iter()
        .find(|x| !x.api_key.is_empty() && !x.openai_url.is_empty())
        .cloned()
        .ok_or("无可用供应商".to_string())
}

/// 把 serde_json 的错误定位（行/列）换算为原文上下文窗口，便于直接看出坏在哪。
/// 宽容 JSON 解析已上提到共享通道（供 tool-call 降级等场景复用）。
use ai::channel::parse_json;

/// 节点 `kind` 的唯一真相。
///
/// **刻意只有这一份**：同一枚举此前散在 `validate_ai_nodes_json` 与
/// `normalize_imported_nodes` 两处局部常量里（连类型与顺序都不一致：
/// `[&str; 12]` vs `&[&str]`），外加内置技能 `validate.py` 的 KINDS 与
/// `SKILL.md` 的白名单散文 —— 一共四份。加一种 kind 只改其中一份时，
/// 「严格校验」与「导入归一化」会对同一个值给出相反判断，且没有任何编译期或
/// 测试期信号。`mindmap_node_kinds_match_the_builtin_skill` 守住另外三份。
pub(crate) const NODE_KINDS: [&str; 12] = [
    "root",
    "module",
    "component",
    "service",
    "route",
    "config",
    "file",
    "task",
    "requirement",
    "constraint",
    "risk",
    "other",
];

// ─── 文档 ───

// ─── 节点 ───

// ─── 贴纸 ───

// ─── 自由关系线 ───

// ─── 导出 ───

// ─── AI 生成 ───

// ─── AI 请求通道（共享实现见 crate::commands::ai::channel） ───
//
// TTFB 超时 / send 重试 / 流式断点续写 / SSE 消费 / stream_options 兼容等传输韧性
// 已收编到共享模块；本文件只保留两件事：
// 1. MmHooks：把通道进度事件接到 "mm-ai-progress"，把取消检查接到 run_id 取消标志；
// 2. call_ai_json：思维导图业务收尾（token 统计、stream done 事件、宽容 JSON 解析与日志）。

/// 思维导图 AI 通道钩子：进度 → "mm-ai-progress"；取消 → run_id AtomicBool。
struct MmHooks<'a> {
    app: &'a Option<tauri::AppHandle>,
    cancel: &'a std::sync::atomic::AtomicBool,
}

impl<'a> ai::channel::ChannelHooks for MmHooks<'a> {
    fn on_progress(&self, step: &str, extra: serde_json::Value) {
        emit_progress(self.app, step, extra);
    }
    fn check_cancel(&self) -> Result<(), String> {
        cancel_err(self.app, self.cancel)
    }
}

/// 带校验修复循环的 AI 生成（借鉴 Archify 的 validate→repair）：
/// 校验失败时把诊断列表反馈给 AI 重新输出，最多 max_rounds 轮，
/// 保留错误数最少的一版。返回 (最终 JSON, 剩余校验错误, 实际调用轮数)；
/// 错误为空即完全通过，轮数 = 1 表示首次即通过。
/// project：项目文件集上下文，非空时校验 sources 证据真实性（存在性 + 内容相关度）。
/// 询问机制：AI 遇到信息不足/歧义时可输出 {"ask": {...}} 向用户提问（最多 3 次），
/// 后端推送 step=ask 事件并阻塞等待前端 mm_ai_answer 回填，再把回答追加进提示词继续。
const MAX_ASK_ROUNDS: usize = 3;

// ─── 类型路由器（借鉴 Archify 的 type router） ───

const VIEW_LABELS: &[(&str, &str)] = &[
    ("architecture", "架构"),
    ("workflow", "业务流"),
    ("dataflow", "数据流"),
    ("sequence", "时序"),
    ("lifecycle", "生命周期"),
];

/// Wspólne wymogi tresciowe dla kazdego widoku: moduly, funkcje, przeplywy.
/// (Wymusza produkt, nie strukture katalogow.)
const VIEW_SUBSTANCE_REQ: &str = r##"
TREŚĆ (obowiązkowa, ważniejsza niż struktura katalogów):
- Każdy węzeł-moduł to PRAWDZIWY moduł, który zaistniał w przeczytanych plikach (nazwa z kodu/katalogu, nie wymyślona). W detail napisz, CO ten moduł robi — konkretna funkcjonalność, nie „zarządza danymi".
- Pod każdym modułem dodaj 2-4 węzły-funkcje/operacje biznesowe, które ten moduł realizuje (np. „rejestracja użytkownika", „generowanie faktury", „odświeżanie tokenu") — z pliku lub kodu, który o tym świadeczy.
- Jeśli z przeczytanych plików wynika przepływ biznesowy (kto → co → z czym → rezultat), dodaj podkorzeń „przepływ" lub opisz go w detail modułu: wejście, kroki, wynik. Nazwij realne funkcje/endpointy/tabele, które widziałeś.
- detail każdego modułu: co robi, na czym polega implementacja (biblioteka, wzorzec, endpoint, model danych), z czym się łączy. Czerp z treści plików, nie z domysłów.
- Zakaz pustych ogólników typu „warstwa logiki", „moduł pomocniczy" — jeśli nie wiesz, co moduł robi, powiedz to wprost w detail, zamiast zmyślać."##;

/// 询问机制说明：AI 遇到信息不足/歧义时优先向用户提问（而非臆造）。
/// 输出 {"ask": {...}} 时由后端弹出表单让用户填写，回答会回填后继续生成；
/// 每轮最多问 1 次、整个生成最多问 3 次（后端封顶），问完必须基于回答继续产出节点。
const ASK_REQ: &str = r##"

询问机制（重要）：当且仅当信息不足以可靠生成、且存在实质性歧义时（例如：无法确定模块边界/某个关键业务流走向、需求文本缺少关键约束、同一功能有多种合理解释），不要臆造，而是改输出一个询问对象（不要输出 nodes）：
{"ask":{"question":"你要问用户的问题（一句话，说清缺什么、为什么需要）","fields":[{"key":"字段键（英文蛇形）","label":"字段名","type":"text","options":[],"default":""}]}}
type 取值：text（单行）、textarea（多行）、select（单选，配 options 选项数组）。fields 最多 4 个，能一句话问清就别用字段（fields 留空 []）。
用户会填写后你再基于回答继续生成。注意：只有在真正卡住、且猜错会显著影响结果时才提问；能从上下文合理推断的就直接推断并在 detail 中说明依据，不要为了提问而提问。"##;

// 探索参数已开放为全局设置（mindmap_settings.json），见 super::settings::ExplorerSettings；
// 此处保留 use 便于后续扩展。
use super::settings::ExplorerSettings;

// ─── 思维导图 Agent（右栏对话）───
//
// 与导入流程共用传输通道（ai::channel）与进度/取消/问答基础设施；区别在于：
// 导入是「单轮 JSON 出口」，Agent 是「多轮工具循环」——模型通过 tools 读写导图。
// 写操作后端只裁决不执行：ops 以事件发给前端，由前端走既有写路径应用
// （撤销快照 / 画布刷新 / 持久化保持单一来源），后端阻塞等待前端回填结果。

// 注意：Agent 交互轮数**不是**这里的常量，而是「思维导图设置」里的 `agentRounds`
// （见 `settings::ExplorerSettings::agent_rounds`）。两件事不要混：
//   · 探索轮数  = AI 导入时点单读本地文件的轮数（settings::explorer_rounds）
//   · Agent 轮数 = 右栏对话的单轮 LLM 交互次数（settings::agent_rounds）
/// 单批写 ops 的节点数上限。
const AGENT_MAX_OPS: usize = 50;
/// 会话历史的字符预算（粗略 4 字符 ≈ 1 token，只求「不会无限膨胀」，精确计数交给网关）。
const AGENT_HISTORY_CHAR_BUDGET: usize = 24_000;
/// Agent 工具集（OpenAI tools 格式）。
///
/// 每个工具对应「改画布」的一个专用动作（建根 / 查 / 加子 / 删 / 移父 / 改内容），
/// 名字与用途一一对应，模型不用猜该用哪个。
///
/// 写工具**直接生效、不等用户确认**（用户在界面上可用 Ctrl+Z 撤销），所以模型可以
/// 边生成回复边连续改画布；后端同时把变更同步进本轮的内存快照，改完立刻再查也看得到。
pub const AGENT_TOOLS_SPEC: &str = r##"[
  { "type": "function", "function": { "name": "get_document_overview", "description": "获取当前思维导图的大纲：每个节点的 id、父节点、路径（如 根 > 体育 > 扩展）与名称。用于了解整体结构；同名节点靠 path 区分。", "parameters": { "type": "object", "properties": {} } } },
  { "type": "function", "function": { "name": "find_nodes", "description": "查询节点：keyword 按名称/详情模糊匹配，name 按名称精确匹配（二选一即可，都给则取交集）。返回 id、名称、路径与子节点数——要往某个节点下挂内容时，先用它拿到 id。", "parameters": { "type": "object", "properties": { "keyword": { "type": "string", "description": "模糊关键词" }, "name": { "type": "string", "description": "精确节点名（区分同名节点时配合 path 看）" } } } } },
  { "type": "function", "function": { "name": "get_subtree", "description": "读取某个节点及其全部子孙的完整内容（名称、详情 Markdown、类型、颜色）。", "parameters": { "type": "object", "properties": { "root_id": { "type": "string", "description": "子树根节点 id" } }, "required": ["root_id"] } } },
  { "type": "function", "function": { "name": "create_root_node", "description": "新建一个根节点（没有父节点，画布上独立成一棵树）。导图为空、或要新起一条主线时用。", "parameters": { "type": "object", "properties": { "name": { "type": "string", "description": "节点名称（必填）" }, "detail": { "type": "string", "description": "节点说明，Markdown" }, "color": { "type": "string", "description": "#RRGGBB" } }, "required": ["name"] } } },
  { "type": "function", "function": { "name": "add_child_nodes", "description": "给指定父节点新增一批平级子节点。新节点不需要提供 id，系统会生成并在工具结果里返回。分析某个文件后落地为导图时，用 sources 把来源文件锚到节点上。", "parameters": { "type": "object", "properties": { "parent_id": { "type": "string", "description": "父节点 id（必须是大纲中真实存在的 id）" }, "nodes": { "type": "array", "items": { "type": "object", "properties": { "name": { "type": "string", "description": "节点名称（必填）" }, "detail": { "type": "string", "description": "节点说明，Markdown" }, "kind": { "type": "string", "description": "节点类型" }, "color": { "type": "string", "description": "#RRGGBB" }, "sources": { "type": "array", "items": { "type": "string" }, "description": "该节点对应的真实源码文件（项目相对路径或绝对路径），用于证据锚定" } }, "required": ["name"] } } }, "required": ["parent_id", "nodes"] } } },
  { "type": "function", "function": { "name": "add_subtree", "description": "一次调用给指定父节点新增一棵完整的多层子树（children 里可继续嵌 children，系统按层级生成并逐层挂好）。把带层级编号的文本（一、（一）、1.、1.1）或任何树形结构落成导图时用它——一次提交整棵树，不要逐层 add_child_nodes。", "parameters": { "type": "object", "properties": { "parent_id": { "type": "string", "description": "父节点 id（必须是大纲中真实存在的 id）" }, "children": { "type": "array", "description": "要新增的顶层节点列表", "items": { "type": "object", "properties": { "name": { "type": "string", "description": "节点名称（必填）" }, "detail": { "type": "string", "description": "节点说明，Markdown（条目的解释性正文放这里，不要塞进 name）" }, "kind": { "type": "string", "description": "节点类型" }, "color": { "type": "string", "description": "#RRGGBB" }, "sources": { "type": "array", "items": { "type": "string" }, "description": "该节点对应的真实源码文件，用于证据锚定" }, "children": { "type": "array", "description": "子节点，结构与本对象完全相同（name 必填，可选 detail/kind/color/sources/children），可继续嵌套", "items": { "type": "object" } } }, "required": ["name"] } } }, "required": ["parent_id", "children"] } } },
  { "type": "function", "function": { "name": "update_nodes", "description": "修改节点内容：批量改名称 / 详情（Markdown）/ 颜色，只传要改的字段。", "parameters": { "type": "object", "properties": { "updates": { "type": "array", "items": { "type": "object", "properties": { "id": { "type": "string" }, "name": { "type": "string" }, "detail": { "type": "string", "description": "新的详情 Markdown，整段替换" }, "color": { "type": "string", "description": "#RRGGBB" } }, "required": ["id"] } } }, "required": ["updates"] } } },
  { "type": "function", "function": { "name": "move_nodes", "description": "修改节点的父节点（重新挂载/搬家），ids 是要搬的节点，new_parent_id 是新父节点 id。会连同整棵子树一起搬。", "parameters": { "type": "object", "properties": { "ids": { "type": "array", "items": { "type": "string" } }, "new_parent_id": { "type": "string", "description": "新父节点 id" } }, "required": ["ids", "new_parent_id"] } } },
  { "type": "function", "function": { "name": "delete_nodes", "description": "删除节点及其整棵子树。", "parameters": { "type": "object", "properties": { "ids": { "type": "array", "items": { "type": "string" } }, "description": "要删除的节点 id（每个都连同子树一起删）" }, "required": ["ids"] } } }
]"##;

/// 子树的最大嵌套深度（模型吐出病态结构时的兜底）。
const AGENT_MAX_SUBTREE_DEPTH: usize = 8;

/// 一轮 Agent 对话的产物。
struct AgentTurn {
    reply: String,
    rounds: usize,
}

// ─── 项目目录绑定与 @ 文件引用 ───

/// 列出文件的跳过目录（依赖/构建产物/缓存，@ 引用不该出现它们）。
///
/// 这里只列**具体目录名**，不再一刀切地跳过所有 `.` 开头的目录：`.github`、`.vscode`
/// 这些恰恰是常被引用的（workflow、tasks.json），一刀切会让它们永远搜不到。
/// 真正该挡掉的隐藏目录已逐条列在下面。
const PROJECT_SKIP_DIRS: [&str; 20] = [
    "node_modules",
    ".git",
    "target",
    "dist",
    "build",
    "out",
    "coverage",
    "__pycache__",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".cache",
    ".turbo",
    ".parcel-cache",
    ".gradle",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".venv",
    ".idea",
];
/// @ 引用候选的文件数上限。
///
/// 原为 800：大仓库遍历到上限即截断，而遍历是深度优先 —— 被截掉的是**整块子树**，
/// 用户会「明明有这个文件却搜不到」。候选清单只在绑定时拉取一次并缓存在前端，
/// 放宽到 4000 的传输代价可以接受。
const PROJECT_FILES_MAX: usize = 4000;
/// 目录遍历深度上限。
const PROJECT_WALK_MAX_DEPTH: usize = 12;
/// 单个附件文件进上下文的字符上限。
const AGENT_ATTACHMENT_CHARS: usize = 8_000;
/// 只引用了**一个**文件时的字符上限：分析单个文件的业务逻辑通常要读完整段代码，
/// 8k 会把后半段截掉；多文件时仍按 8k 控制总上下文。
const AGENT_ATTACHMENT_CHARS_SINGLE: usize = 24_000;
/// 单轮对话最多引用的文件数。
const AGENT_MAX_ATTACHMENTS: usize = 8;

// ─── 子树重新分析 ───

// ─── 初始化 ───

// ─── 文件夹 ───

mod documents;
pub use documents::*;
mod validation;
pub(crate) use validation::*;
mod transport;
use transport::*;
mod routing;
pub use routing::*;
mod agent_tools;
use agent_tools::*;
mod agent_session;
pub use agent_session::*;
mod project_files;
pub use project_files::*;
mod regenerate;
pub use regenerate::*;
#[cfg(test)]
mod agent_tests;
#[cfg(test)]
mod tests;
