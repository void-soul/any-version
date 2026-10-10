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
///
/// **必须**带 `rename_all = "camelCase"`：前端 invoke 传的是 `providerId` /
/// `modelId` / `runId` / `currentDoc`，漏了这个属性 serde 会静默全部变 None
/// —— 后果是模型回落全局默认（用户选了 A 模型实际跑了 B 模型）、「停止」失效。
/// 思维导图模块的入参结构体全都有（`mindmap/models.rs`），这里漏了。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiDesignInput {
    /// 需求描述（用户输入的原始文本）
    pub text: String,
    /// 设计名（可空，AI 自己起）
    pub name: Option<String>,
    pub dialect: Option<String>,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    /// 画布上的当前设计（前端原样传入）。非空时提示词带它作基线：
    /// AI 基于它修改并输出完整更新后的设计；是「新建」还是「修改」由模型
    /// 看需求自己判断（系统提示里写明了两种情况的处理方式）。
    #[serde(default)]
    pub current_doc: Option<DbDesignDocument>,
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

// 设计文件的「标准格式」：JSON Schema + 完整示例，直接进提示词。
// 比堆一堆散文规则稳 —— 规则写多了模型容易漏执行，照着格式抄不会。
// **schema 必须与 `models.rs`（serde camelCase）保持同步**：那边加/改字段，这里同步改。
const FORMAT_SCHEMA: &str = r#"{
  "name": "设计名（字符串）",
  "dialect": "mysql | postgres | sqlite",
  "nodes": [
    {
      "kind": "table | view",
      "name": "表名（小写蛇形，不允许重名）",
      "comment": "表的中文业务说明（可选）",
      "table": {
        "fields": [
          {
            "name": "字段名（小写蛇形）",
            "type": { "base": "varchar", "length": 32 },
            "comment": "字段的中文业务含义（可选）",
            "pk": true,
            "nullable": true,
            "default": "默认值（可选，字符串）",
            "autoIncrement": true,
            "unique": true
          }
        ],
        "indexes": [
          { "name": "idx_name", "kind": "index | unique | fulltext", "fields": ["col1", "col2"] }
        ]
      },
      "view": { "sql": "视图定义 SQL（仅 kind=view 时填）" }
    }
  ],
  "relations": [
    {
      "from": { "node": "子表（外键所在端）的节点 id 或表名", "field": "外键字段名" },
      "to": { "node": "父表（被引用端）的节点 id 或表名", "field": "被引用字段名（通常主键）" },
      "kind": "1-1 | 1-n | n-n",
      "onDelete": "CASCADE（可选）"
    }
  ]
}
"#;

/// 完整最小示例：两张表 + 一条关联，把 schema 里「可选字段怎么落」演示出来。
const FORMAT_EXAMPLE: &str = r#"{
  "name": "文章管理系统",
  "dialect": "mysql",
  "nodes": [
    {
      "kind": "table",
      "name": "user",
      "comment": "用户：系统账号",
      "table": {
        "fields": [
          { "name": "id", "type": { "base": "bigint" }, "pk": true, "autoIncrement": true, "comment": "主键" },
          { "name": "username", "type": { "base": "varchar", "length": 64 }, "unique": true, "comment": "登录名" },
          { "name": "created_at", "type": { "base": "datetime" }, "nullable": false, "comment": "创建时间" }
        ]
      }
    },
    {
      "kind": "table",
      "name": "article",
      "comment": "文章：用户发布的内容",
      "table": {
        "fields": [
          { "name": "id", "type": { "base": "bigint" }, "pk": true, "autoIncrement": true, "comment": "主键" },
          { "name": "user_id", "type": { "base": "bigint" }, "comment": "作者用户" },
          { "name": "title", "type": { "base": "varchar", "length": 128 }, "nullable": false, "comment": "标题" },
          { "name": "content", "type": { "base": "text" }, "comment": "正文" },
          { "name": "status", "type": { "base": "enum", "values": ["draft", "published", "archived"] }, "nullable": false, "comment": "状态" }
        ],
        "indexes": [ { "name": "idx_article_user", "kind": "index", "fields": ["user_id"] } ]
      }
    }
  ],
  "relations": [
    {
      "from": { "node": "article", "field": "user_id" },
      "to": { "node": "user", "field": "id" },
      "kind": "1-n",
      "onDelete": "CASCADE"
    }
  ]
}
"#;

fn system_prompt() -> String {
    format!(
        "你是数据库设计助手。根据用户的需求，产出 any-version 数据库设计器的设计文件（JSON）。
若请求附带「当前设计」，还要能基于它修改。

【输出规则】
1. 只输出 JSON 对象本身，不要 Markdown 代码围栏，不要任何解释文字。
2. 设计文件遵循下面的标准格式（字段名全部 camelCase，标注「可选」的字段可省略）：

{schema}

3. 完整最小示例：

{example}

4. type.base 白名单：int bigint smallint tinyint decimal float double char varchar text \
date time datetime timestamp boolean json uuid blob enum
   - decimal 必须给 precision/scale（如 {{\"base\":\"decimal\",\"precision\":10,\"scale\":2}}）
   - enum 必须给 values（取值数组）
   - 不要写 \"VARCHAR(32)\" 这种整串，永远用上面的 type 对象
5. 约定：
   - 节点 id/x/y 可省略（我这边来补）；表名不允许重名
   - 每张表至少一个主键（字段 \"pk\":true）；\"autoIncrement\":true 必须是主键
   - 主键只用字段 \"pk\":true 表达，不要再写 \"kind\":\"primary\" 的索引
   - indexes 只在需要查询加速或唯一约束时写
   - relations：from = 子表（外键所在端），to = 父表（被引用端）；不要写自关联；
     外键字段类型必须与父表被引用字段一致
   - 每张表、每个字段的 comment 用中文写清业务含义，这是产出里最值钱的部分
   - 表按真实实体来，不要为了显得完整而编造；字段类型拿不准时给保守类型
   - 修改场景：未涉及改动的表/关联**原样保留**，输出完整更新后的设计，不要 diff，不要省略",
        schema = FORMAT_SCHEMA,
        example = FORMAT_EXAMPLE,
    )
}

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

    // 画布有表时把当前设计带进提示词：AI 基于它修改（或判断出是全新设计则忽略）
    let user = build_user_prompt(&name_hint, &dialect, text, input.current_doc.as_ref());

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
        &system_prompt(),
        &user,
        0.3,
        |len, acc| {
            // 附带末尾 200 字符做预览：前端控制台实时显示「正在写什么」（与思维导图同款）
            let tail: String = acc.chars().rev().take(200).collect::<Vec<_>>().into_iter().rev().collect();
            emit_progress(&app_opt, "stream", serde_json::json!({ "length": len, "text": tail }));
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
            &system_prompt(),
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

/// 组装 user 消息。`current` 非空（有表）时为「修改」场景：带上当前设计作基线，
/// 由模型看需求自己判断是「基于它改」还是「与它无关的全新设计」（系统提示写明两种处理）。
fn build_user_prompt(
    name_hint: &str,
    dialect: &str,
    text: &str,
    current: Option<&DbDesignDocument>,
) -> String {
    match current.filter(|d| !d.nodes.is_empty()) {
        None => format!(
            "设计名：{name_hint}\n目标方言：{dialect}\n\n需求：\n{text}\n\n\
             请按系统提示的标准格式输出 JSON。字段命名用小写蛇形。"
        ),
        Some(doc) => format!(
            "设计名：{name_hint}\n目标方言：{dialect}\n\n\
             当前设计（用户正在做的既有设计，作为基线）：\n{}\n\n\
             用户需求：\n{text}\n\n\
             判断：若需求是对当前设计的修改/扩展，则基于当前设计施加改动并输出完整更新后的设计 JSON\
             （未涉及改动的表/关联必须原样保留，不要省略）；\
             若需求是与当前设计无关的全新设计，则忽略当前设计、产出新设计。\
             按系统提示的标准格式输出完整 JSON。",
            doc_to_prompt_json(doc)
        ),
    }
}

/// 当前设计进提示词前，剥掉布局坐标等纯前端状态：省 token，
/// 也避免模型把坐标抄回来（布局是前端的事，落图后前端会自己排）。
fn doc_to_prompt_json(d: &DbDesignDocument) -> serde_json::Value {
    let mut v = serde_json::to_value(d).unwrap_or(serde_json::Value::Null);
    if let Some(obj) = v.as_object_mut() {
        obj.remove("updatedAt");
        obj.remove("folderId");
    }
    if let Some(nodes) = v.get_mut("nodes").and_then(|n| n.as_array_mut()) {
        for n in nodes {
            if let Some(o) = n.as_object_mut() {
                o.remove("x");
                o.remove("y");
            }
        }
    }
    v
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

    /// 回归：前端 invoke 传 camelCase（providerId/modelId/runId/currentDoc）。
    /// 结构体漏 `rename_all = "camelCase"` 时 serde 静默全变 None ——
    /// 用户选的模型被忽略、实际跑全局默认，「停止」也失效（Q-0367 用户报障）。
    #[test]
    fn input_accepts_camel_case_fields() {
        let v = serde_json::json!({
            "text": "x",
            "providerId": "p1",
            "modelId": "m1",
            "runId": "r1",
            "currentDoc": { "name": "cur", "nodes": [ { "name": "t" } ] }
        });
        let input: AiDesignInput = serde_json::from_value(v).expect("camelCase 字段必须被识别");
        assert_eq!(input.provider_id.as_deref(), Some("p1"));
        assert_eq!(input.model_id.as_deref(), Some("m1"));
        assert_eq!(input.run_id.as_deref(), Some("r1"));
        assert!(input.current_doc.is_some());
    }

    /// 新建场景（无当前设计）：提示词不带基线，只发需求。
    #[test]
    fn prompt_generate_mode_has_no_current_design() {
        let p = build_user_prompt("N", "mysql", "需求甲", None);
        assert!(p.contains("需求甲"));
        assert!(!p.contains("当前设计"));
    }

    /// 修改场景：当前设计要进提示词，但布局坐标（x/y）必须剥掉。
    #[test]
    fn prompt_modify_mode_carries_current_doc_without_positions() {
        let mut cur = new_document("cur", "mysql");
        let mut node = DbDesignNode {
            name: "orders".into(),
            ..Default::default()
        };
        node.x = 12.0;
        node.y = 34.0;
        cur.nodes.push(node);
        let p = build_user_prompt("N", "mysql", "加个字段", Some(&cur));
        assert!(p.contains("当前设计"), "修改场景必须带当前设计作基线");
        assert!(p.contains("orders"));
        assert!(!p.contains("\"x\""), "布局坐标不该进提示词");
        assert!(!p.contains("\"y\""));
    }

    /// 当前设计为空文档（没表）时按新建处理，不带基线。
    #[test]
    fn prompt_ignores_empty_current_doc() {
        let cur = new_document("cur", "mysql");
        let p = build_user_prompt("N", "mysql", "需求甲", Some(&cur));
        assert!(!p.contains("当前设计"));
    }
}
