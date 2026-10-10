// 数据库设计器的 AI 助手：给一段需求，产出一份 `.dbdesign.json`。
//
// 设计要点：
//   · **只返回文档，不落盘**。应用拿到后走正常的「打开 → 校验 → 用户决定是否保存」链路，
//     避免 AI 直接覆盖用户正在画的设计（应用里替换当前内容是要红色确认的）。
//   · 复用 AI 模块已有的供应商 / 模型解析与流式通道（`ai::channel::stream_chat_with_resume`），
//     不自建请求逻辑 —— 超时、重试、断点续写、宽容 JSON 解析都在那一层。
//   · **流式构图**：流式过程中每写完一张表就推一份部分文档（`dbd-ai-partial`），
//     前端边收边画，与思维导图观感一致。
//   · 不强依赖 tool-call：只有部分网关支持，通道内部会在不支持时降级为纯文本协议。
//   · 产出后**不做**校验修补：校验规则只有一份（`store::validate`），
//     错误交给应用展示给用户在画布上改，比我们在后端瞎猜怎么修更可靠。

use super::file::new_document;
use super::models::{DbDesignDocument, DbDesignNode, DbDesignRelation};
use crate::commands::ai;

/// 前端传进来的生成请求
#[derive(serde::Deserialize)]
pub struct AiDesignInput {
    /// 需求描述（用户输入的原始文本）
    pub text: String,
    /// 设计名（可空，AI 自己起）
    pub name: Option<String>,
    pub dialect: Option<String>,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    /// 本次运行的 id（前端生成）。用于「停止」，不填则不能中途停。
    #[serde(default)]
    pub run_id: Option<String>,
}

/// 已请求停止的 run_id。用 HashSet 而不是每任务一个 AtomicBool：命令是异步的、
/// 「停止」走的是另一条命令，只能按 run_id 在全局里对上号（同思维导图 mm_ai_cancel 的思路）。
fn cancelled_runs() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    use std::sync::{Mutex, OnceLock};
    static SET: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    SET.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

/// 停止一次生成。重复调用无害；run_id 不存在也不会报错。
#[tauri::command]
pub fn dbd_ai_cancel(run_id: String) -> Result<(), String> {
    cancelled_runs()
        .lock()
        .map_err(|_| "取消状态被占用".to_string())?
        .insert(run_id);
    Ok(())
}

fn is_cancelled(run_id: &str) -> bool {
    cancelled_runs()
        .lock()
        .map(|s| s.contains(run_id))
        .unwrap_or(false)
}

/// 把进度推给前端（`dbd-ai-progress`）。事件是「即发即弃」的，
/// 前端用模块级缓冲订阅（切面板/keep-alive 也不会丢）。
fn emit_progress(app: &Option<tauri::AppHandle>, step: &str, extra: serde_json::Value) {
    use tauri::Emitter as _; // Tauri 2：emit 挂在 Emitter trait 上，不在 AppHandle 的固有方法里
    if let Some(handle) = app {
        let _ = handle.emit(
            "dbd-ai-progress",
            serde_json::json!({ "step": step, "data": extra }),
        );
    }
}

/// 生成过程的钩子：转发流式进度 + 响应「停止」。
struct DbdHooks {
    app: Option<tauri::AppHandle>,
    run_id: String,
}

impl ai::channel::ChannelHooks for DbdHooks {
    fn on_progress(&self, step: &str, extra: serde_json::Value) {
        emit_progress(&self.app, step, extra);
    }
    fn check_cancel(&self) -> Result<(), String> {
        if is_cancelled(&self.run_id) {
            return Err("已取消".to_string());
        }
        Ok(())
    }
}

const SYSTEM_PROMPT: &str = "\
你是数据库设计助手。根据用户给的需求，产出一份 any-version 数据库设计器的设计文件（JSON）。

硬性要求：
1. 只输出 JSON 对象本身，不要 Markdown 代码围栏，不要任何解释文字。
2. 顶层：name（设计名）、dialect（mysql|postgres|sqlite）、nodes、relations。
   id 可以省略（省略了我来补），不要为了凑字段编一个。
3. 节点只有 table 和 view 两种 kind。表名用小写蛇形（users / order_items），不允许重名。
4. 字段类型是**逻辑类型对象**：{\"base\":\"varchar\",\"length\":32}。base 只能是：
   int bigint smallint tinyint decimal float double char varchar text date time
   datetime timestamp boolean json uuid blob enum。不要写 VARCHAR(32) 这种整串。
   decimal 要给 precision/scale，enum 要给 values。
5. 每张表至少一个主键（字段 pk:true）。自增 autoIncrement:true 必须同时是主键。
6. 关联：from = 子表（外键所在的那一端），to = 父表（被引用的那一端）。
   from/to 的 node 写节点 id（节点没写 id 时写表名也可以，我能认出来），
   field 是字段名。不要写自关联。
   外键字段的类型要和父表主键一致。
7. 主键只用字段 pk:true 表达，**不要**额外写 kind 为 primary 的索引。
   需要查询加速的写 kind 为 index / unique 的索引，fields 是列名数组。
8. 注释用中文写清楚业务含义（comment 字段），这是产出里最值钱的部分。
9. 表按真实实体来，不要为了显得完整而编造；字段类型拿不准时给保守类型，
   不要发明表里没有的属性。
";

/// 生成设计文档。**同步返回完整文档**（当前是短请求，不必做流式进度）。
#[tauri::command]
pub async fn dbd_ai_generate(
    app: tauri::AppHandle,
    input: AiDesignInput,
) -> Result<DbDesignDocument, String> {
    let run_id = input.run_id.clone().unwrap_or_default();
    let text = input.text.trim();
    if text.is_empty() {
        return Err("请先描述要设计什么（例如：一个订单系统，含用户、商品、订单、订单项）".to_string());
    }
    let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;

    let dialect = input
        .dialect
        .clone()
        .unwrap_or_else(|| "mysql".to_string());
    let name_hint = input
        .name
        .clone()
        .unwrap_or_else(|| "未命名设计".to_string());

    let user = format!(
        "设计名：{name_hint}\n目标方言：{dialect}\n\n需求：\n{text}\n\n\
         请按系统提示的格式输出 JSON。字段命名用小写蛇形。"
    );

    emit_progress(&Some(app.clone()), "start", serde_json::json!({}));
    let hooks = DbdHooks {
        app: Some(app.clone()),
        run_id: run_id.clone(),
    };
    // 走**流式**通道：AI 每写完一张表就往前端推一份部分文档（`dbd-ai-partial`），
    // 画布边收边画，不用等整篇到齐（100 张表要等几十秒，全程空白很难受）。
    // 通道要求 `Fn`（不可变捕获），「已推过多少个」只能用原子量记录
    let seen_nodes = std::sync::atomic::AtomicUsize::new(0);
    let seen_relations = std::sync::atomic::AtomicUsize::new(0);
    let app_opt = Some(app.clone());
    let outcome = ai::channel::stream_chat_with_resume(
        &hooks,
        &provider,
        &model,
        SYSTEM_PROMPT,
        &user,
        0.3,
        |len, acc| {
            emit_progress(&app_opt, "stream", serde_json::json!({ "length": len }));
            // 只有「又多出一个完整节点/关系」才推一次 —— 天然节流，
            // 不会每来 400 字符就刷一遍画布。
            let partial = extract_partial(acc);
            let grew = partial.nodes.len() > seen_nodes.load(std::sync::atomic::Ordering::Relaxed)
                || partial.relations.len() > seen_relations.load(std::sync::atomic::Ordering::Relaxed);
            if grew {
                seen_nodes.store(partial.nodes.len(), std::sync::atomic::Ordering::Relaxed);
                seen_relations.store(partial.relations.len(), std::sync::atomic::Ordering::Relaxed);
                emit_partial(&app_opt, &run_id, &partial);
            }
        },
    )
    .await
    .map_err(|e| format!("AI 生成失败：{}", e))?;

    // 兜底：个别网关不认 stream:true（直接回一个普通 JSON），SSE 消费会拿到空文本。
    // 这时退回非流式再取一次，不要让用户看到「AI返回空」。
    let text = if outcome.text.trim().is_empty() {
        ai::channel::complete_chat(
            &hooks,
            &provider,
            &model,
            SYSTEM_PROMPT,
            &user,
            0.3,
            None,
            ai::usage::tool_ids::DB_DESIGNER,
        )
        .await
        .map_err(|e| format!("AI 生成失败：{}", e))?
        .text
    } else {
        outcome.text
    };
    if text.trim().is_empty() {
        return Err("AI返回空".into());
    }
    // 收尾解析用通道的宽容解析（容忍 markdown 围栏、前后缀、非法转义）
    let json = ai::channel::parse_json(&text)
        .map_err(|e| format!("AI 返回的内容不是合法 JSON：{}", e))?;

    parse_document(json)
}

/// 把 AI 返回的 JSON 收成 DbDesignDocument：缺字段就补默认，结构不对就报清楚。
fn parse_document(json: serde_json::Value) -> Result<DbDesignDocument, String> {
    let mut doc: DbDesignDocument = match serde_json::from_value(json.clone()) {
        Ok(d) => d,
        Err(e) => {
            // 常见的容错：AI 把设计包在 {"design": ...} 或 {"document": ...} 里
            let nested = json
                .get("design")
                .or_else(|| json.get("document"))
                .or_else(|| json.get("dbdesign"))
                .cloned();
            match nested.and_then(|v| serde_json::from_value::<DbDesignDocument>(v).ok()) {
                Some(d) => d,
                None => {
                    // 解析失败时把 AI 原始输出落滚动日志（同思维导图的做法）：
                    // 只给用户看 "missing field `id`" 没法定位，得看它到底写了什么。
                    let head: String = json.to_string().chars().take(2000).collect();
                    tracing::error!(
                        "[dbd] AI 产出无法解析为设计文件（{}）；AI 原始输出开头：{}",
                        e,
                        head
                    );
                    return Err(format!("AI 返回的不是合法设计文件：{}", e));
                }
            }
        }
    };
    if doc.id.trim().is_empty() {
        doc.id = new_id("dbd");
    }
    if doc.name.trim().is_empty() {
        doc.name = "未命名设计".to_string();
    }
    if !crate::commands::db_designer::models::DIALECTS.contains(&doc.dialect.as_str()) {
        doc.dialect = "mysql".to_string();
    }
    if doc.nodes.is_empty() {
        return Err("AI 没有产出任何表。换个说法再试一次，或自己加表".to_string());
    }
    // 节点 id 兜底：AI 常漏 id（这时 `id` 反序列化为空串），坐标更是不会给 ——
    // 位置交给前端「自动布局」。编号要避开 AI 自己写过的 id，否则会覆盖它的引用。
    let mut node_ids: Vec<String> = doc.nodes.iter().map(|n| n.id.clone()).collect();
    fill_missing_ids(&mut node_ids, "t");
    for (n, id) in doc.nodes.iter_mut().zip(node_ids) {
        n.id = id;
    }
    let mut relation_ids: Vec<String> = doc.relations.iter().map(|r| r.id.clone()).collect();
    fill_missing_ids(&mut relation_ids, "r");
    for (r, id) in doc.relations.iter_mut().zip(relation_ids) {
        r.id = id;
    }

    // 关联端点写的是**表名**而不是 id 时（AI 漏写 id 的情况下很常见），按表名回认一次。
    // 只在端点指向不存在的节点时才改 —— 正常的 id 引用一律不动。
    let id_by_name: std::collections::HashMap<String, String> = doc
        .nodes
        .iter()
        .filter(|n| !n.name.trim().is_empty())
        .map(|n| (n.name.trim().to_lowercase(), n.id.clone()))
        .collect();
    for r in doc.relations.iter_mut() {
        for end in [&mut r.from, &mut r.to] {
            let key = end.node.trim().to_lowercase();
            if key.is_empty() || doc.nodes.iter().any(|n| n.id == end.node) {
                continue;
            }
            if let Some(real) = id_by_name.get(&key) {
                end.node = real.clone();
            }
        }
    }
    Ok(doc)
}

/// 复用 AI 模块的「供应商 + 模型」解析顺序：显式 provider → 全局默认 → 首个可用 → 活跃模型。
/// （与思维导图 / API 模块同一套逻辑，用户在哪配的模型都能用上。）
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

/// 从**可能还没写完**的 JSON 文本里，抠出已经写完整的数组元素。
///
/// 逐字符扫 + 括号配对（**字符串内部的括号不参与配对**），所以注释 / 默认值里的
/// 花括号不会打架；`]` 出现在数组层（depth==0）就说明这个数组结束了。
/// 这是「边收边落图」的关键：AI 每写完一张表，我们就能把它画上画布，
/// 不必等整个文档到齐（100 张表要等几十秒，全程空白很难受）。
fn complete_elements(text: &str, key: &str) -> Vec<serde_json::Value> {
    let needle = format!("\"{}\"", key);
    let Some(k) = text.find(&needle) else {
        return Vec::new();
    };
    let Some(start) = text[k..].find('[').map(|i| k + i) else {
        return Vec::new();
    };
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    let mut obj_start: Option<usize> = None;
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
        } else {
            match b {
                b'"' => in_str = true,
                b'{' => {
                    if depth == 0 {
                        obj_start = Some(i);
                    }
                    depth += 1;
                }
                b'}' => {
                    depth -= 1;
                    if depth <= 0 {
                        // 一个元素收尾了。解析失败就丢掉（多半是还没写完）
                        if let Some(s) = obj_start {
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text[s..=i]) {
                                out.push(v);
                            }
                        }
                        obj_start = None;
                        depth = 0;
                    }
                }
                b']' if depth == 0 => break,
                _ => {}
            }
        }
        i += 1;
    }
    out
}

/// 抓顶层 `"key": "value"` 里的 value（只找第一次出现，够用即可）。
fn grab_string(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let k = text.find(&needle)?;
    let rest = &text[k + needle.len()..];
    let a = rest.find('"')?;
    let b = rest[a + 1..].find('"').map(|i| a + 1 + i)?;
    Some(rest[a + 1..b].to_string())
}

/// 把「当前累积到的文本」尽力解析成一份**部分文档**（只含已经写完整的节点 / 关系）。
/// 解析不了的就当还没有 —— 下一 tick 内容更多时会再试。
fn extract_partial(text: &str) -> DbDesignDocument {
    let mut doc = new_document("", "mysql");
    for v in complete_elements(text, "nodes") {
        if let Ok(n) = serde_json::from_value::<DbDesignNode>(v) {
            doc.nodes.push(n);
        }
    }
    for v in complete_elements(text, "relations") {
        if let Ok(r) = serde_json::from_value::<DbDesignRelation>(v) {
            doc.relations.push(r);
        }
    }
    // 补 id：AI 漏写 id 时节点现在能解析通过了（`id` 改成缺了当空），但空 id 会让
    // 前端 React key 撞车、关联也引用不到 —— 所以这里按 t1/t2、r1/r2 补齐。
    // 与 parse_document 用同一套编号，流式落图和最终结果对得上。
    let mut node_ids: Vec<String> = doc.nodes.iter().map(|n| n.id.clone()).collect();
    fill_missing_ids(&mut node_ids, "t");
    for (n, id) in doc.nodes.iter_mut().zip(node_ids) {
        n.id = id;
    }
    let mut rel_ids: Vec<String> = doc.relations.iter().map(|r| r.id.clone()).collect();
    fill_missing_ids(&mut rel_ids, "r");
    for (r, id) in doc.relations.iter_mut().zip(rel_ids) {
        r.id = id;
    }
    if let Some(d) = grab_string(text, "dialect") {
        if crate::commands::db_designer::models::DIALECTS.contains(&d.as_str()) {
            doc.dialect = d;
        }
    }
    doc
}

/// 推送一份「到目前为止画得出来的图」（`dbd-ai-partial`）。
/// 前端拿到就立刻落画布，所以这一路是**流式构图**，与思维导图观感一致。
fn emit_partial(app: &Option<tauri::AppHandle>, run_id: &str, doc: &DbDesignDocument) {
    use tauri::Emitter as _;
    if let Some(handle) = app {
        let _ = handle.emit(
            "dbd-ai-partial",
            serde_json::json!({ "runId": run_id, "doc": doc }),
        );
    }
}

/// 给一批元素补 id：缺的按 `t1` / `r1` 这样的序号补，并且**不和 AI 自己写的 id 撞车**
/// （AI 可能只给一部分元素写 id，编号撞上去会覆盖它的引用关系）。
fn fill_missing_ids(ids: &mut Vec<String>, prefix: &str) {
    let used: std::collections::HashSet<String> =
        ids.iter().filter(|s| !s.trim().is_empty()).cloned().collect();
    let mut seq = 1usize;
    for id in ids.iter_mut() {
        if !id.trim().is_empty() {
            continue;
        }
        loop {
            let cand = format!("{}{}", prefix, seq);
            seq += 1;
            if !used.contains(&cand) {
                *id = cand;
                break;
            }
        }
    }
}

/// 本地 id：AI 常常不写 `id`，而关联要按 id 引用节点，不补就全乱了。
fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{}_{}", prefix, seq)
}

fn first_usable_provider(
    cfg: &ai::models::AiConfig,
) -> Result<ai::models::AiProvider, String> {
    cfg.providers
        .iter()
        .find(|x| !x.api_key.is_empty() && !x.openai_url.is_empty())
        .cloned()
        .ok_or_else(|| "还没有可用的 AI 供应商：请到 AI 模块配置一个（端点 + Key）".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 流式构图的核心：文本写到一半时，**只认已经写完的元素**。
    #[test]
    fn partial_nodes_ignore_unfinished_element() {
        let text = r#"{"dialect":"mysql","nodes":[
            {"id":"t1","name":"user","kind":"table","x":0,"y":0,
             "table":{"fields":[{"name":"id","type":{"base":"bigint"},"pk":true}]}},
            {"id":"t2","name":"order","kind":"tab"#;
        let els = complete_elements(text, "nodes");
        assert_eq!(els.len(), 1, "半张表不算：只应拿到 t1");
        assert_eq!(els[0]["name"], "user");
    }

    /// 字符串里的花括号不能参与配对（注释里写 `{"a":1}` 很常见）。
    #[test]
    fn partial_nodes_respect_string_braces() {
        let text = r#"{"nodes":[
            {"id":"t1","name":"a","kind":"table","x":0,"y":0,"comment":"形如 { x } 的花括号",
             "table":{"fields":[]}},
            {"id":"t2","name":"b","kind":"table","x":0,"y":0,"table":{"fields":[]}}]}"#;
        let els = complete_elements(text, "nodes");
        assert_eq!(els.len(), 2);
    }

    /// 空数组 / 找不到键都不炸，返回空。
    #[test]
    fn partial_nodes_handle_missing_key() {
        assert!(complete_elements("{}", "nodes").is_empty());
        assert!(complete_elements(r#"{"nodes":[]}"#, "nodes").is_empty());
    }

    /// 回归：模型整个漏写 id（顶层 / 节点 / 关系都没有）。
    /// 之前 `id` 是必填字段，`serde_json::from_value` 直接 "missing field `id`"，
    /// 整份产出作废；现在缺 id 按空处理，收尾补上。
    #[test]
    fn parse_document_tolerates_missing_ids() {
        let json = serde_json::json!({
            "name": "订单系统",
            "dialect": "mysql",
            "nodes": [
                {"name":"user","kind":"table","table":{"fields":[{"name":"id","type":{"base":"bigint"},"pk":true}]}},
                {"name":"order","kind":"table","table":{"fields":[{"name":"id","type":{"base":"bigint"},"pk":true},{"name":"user_id","type":{"base":"bigint"}}]}}
            ],
            "relations": [
                {"from":{"node":"order","field":"user_id"},"to":{"node":"user","field":"id"},"kind":"1-n"}
            ]
        });
        let doc = parse_document(json).expect("漏写 id 不该让整份产出作废");
        assert_eq!(doc.nodes.len(), 2);
        assert!(doc.nodes.iter().all(|n| !n.id.is_empty()), "节点 id 要补上");
        assert!(doc.relations.iter().all(|r| !r.id.is_empty()), "关系 id 要补上");
        assert!(!doc.id.is_empty(), "顶层 id 要补上");
        // 端点写的是表名（没 id 可用时模型多半这么干）→ 应回认成节点 id
        assert_eq!(doc.relations[0].from.node, doc.nodes[1].id);
        assert_eq!(doc.relations[0].to.node, doc.nodes[0].id);
    }

    /// 流式阶段同样要能抠出没写 id 的节点，否则就是「进度条在涨但画布空白」。
    #[test]
    fn extract_partial_tolerates_missing_ids() {
        let text = r#"{"dialect":"mysql","nodes":[
            {"name":"user","kind":"table","table":{"fields":[{"name":"id","type":{"base":"bigint"},"pk":true}]}},
            {"name":"order","kind":"t"#;
        let doc = extract_partial(text);
        assert_eq!(doc.nodes.len(), 1, "没写 id 的节点也要能画出来");
        assert_eq!(doc.nodes[0].id, "t1", "空 id 要补齐，否则前端 key 撞车、关联引用不到");
    }

    /// extract_partial 把节点、关系、方言一起收成一份可画的文档。
    #[test]
    fn extract_partial_builds_drawable_doc() {
        let text = r#"{"dialect":"postgres","nodes":[
            {"id":"t1","name":"user","kind":"table","x":0,"y":0,
             "table":{"fields":[{"name":"id","type":{"base":"bigint"},"pk":true}]}}],
            "relations":[{"id":"r1","name":"fk",
              "from":{"node":"t1","field":"id"},"to":{"node":"t2","field":"user_id"},
              "kind":"1-n","onDelete":"CASCADE"}]}"#;
        let doc = extract_partial(text);
        assert_eq!(doc.nodes.len(), 1);
        assert_eq!(doc.relations.len(), 1);
        assert_eq!(doc.dialect, "postgres");
    }
}
