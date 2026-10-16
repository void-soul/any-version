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
//   · 产出后**过一遍规范化**（`store::normalize_naming`）：命名改合规 + 外键补索引。
//     这两类都是 error 级（阻断保存 / 导出），而提示词只是概率性约束 —— 只靠提示词，
//     用户会拿到「满屏报错且导不出来」的设计。规范化与 `store::validate` 共用同一套规则，
//     不会两套口径漂移；它只改明显的规范问题，业务语义仍由 AI 决定。

use super::file::new_document;
use super::models::{DbDesignDocument, DbDesignNode, DbDesignRelation, DbField};
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
    /// 最近的对话轮次（滚动窗口，前端只留最近几轮）。
    ///
    /// **只存文本，不存模型吐过的整份 JSON**：下一轮模型看到的是更新后的当前设计，
    /// 旧 JSON 里九成是重复的表定义，存起来只烧 token。
    #[serde(default)]
    pub history: Vec<ChatTurn>,
    /// 已经追问了几轮。写进提示词是为了让模型遵守「3 轮后必须出结果」——
    /// 光在提示词里写死上限而每次都发同样的话，模型数不清自己问过几轮。
    #[serde(default)]
    pub clarify_round: u32,
}

/// 一轮对话（用户或助手）。
#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ChatTurn {
    /// `"user"` 或 `"assistant"`
    pub role: String,
    pub text: String,
}

/// 模型这一轮的产出：**要么给设计，要么反过来问你**。
///
/// 用 `tag = "kind"` 让前端能直接 `switch (outcome.kind)`：
/// `{"kind":"design","document":{…},"decisions":[…]}` /
/// `{"kind":"clarify","question":"…","options":[…]}`。
///
/// `decisions` 与 `document` **分开放**，而不是让模型把 decisions 写进文档里再回吐：
/// 它每轮都要重写整份文档，很可能把你之前写的决策丢了或改写。真正的所有权在前端 ——
/// 保留原文档的决策，只追加本轮新增的。
#[derive(serde::Serialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AiOutcome {
    /// 产出了一份设计（可能带本轮的决策摘要）
    Design {
        document: DbDesignDocument,
        /// 本轮「为什么这么改」。模型没给就是空数组，**不算错误**。
        decisions: Vec<String>,
    },
    /// 业务语义上有歧义，停下来问用户（此时不落图，文档一个字都不改）
    Clarify {
        question: String,
        /// 候选项，供用户点一下就答；模型没给就是空数组（自由输入）。
        options: Vec<String>,
    },
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
///
/// **示例必须自身通过 `store::validate`**，否则模型会忠实照抄违规写法。
/// 这是踩过两次的坑：第一次示例用 `id` 作主键、第二次用 `title`/`content`/`status`
/// 这类通用裸名 —— 两次用户看到的都是「AI 照着示例写出来的违规字段」。
/// 测试 `example_document_passes_our_own_validator` 会把示例过一遍真实校验器，
/// 以后新增任何规则都会自动被这条测试覆盖。
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
          { "name": "user_id", "type": { "base": "bigint" }, "pk": true, "autoIncrement": true, "comment": "用户主键" },
          { "name": "user_name", "type": { "base": "varchar", "length": 64 }, "unique": true, "comment": "登录名" },
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
          { "name": "article_id", "type": { "base": "bigint" }, "pk": true, "autoIncrement": true, "comment": "文章主键" },
          { "name": "user_id", "type": { "base": "bigint" }, "comment": "作者用户，引用 user.user_id" },
          { "name": "article_title", "type": { "base": "varchar", "length": 128 }, "nullable": false, "comment": "标题" },
          { "name": "article_content", "type": { "base": "text" }, "comment": "正文" },
          { "name": "article_status", "type": { "base": "enum", "values": ["draft", "published", "archived"] }, "nullable": false, "comment": "状态" }
        ],
        "indexes": [ { "name": "idx_article_user", "kind": "index", "fields": ["user_id"] } ]
      }
    }
  ],
  "relations": [
    {
      "from": { "node": "article", "field": "user_id" },
      "to": { "node": "user", "field": "user_id" },
      "kind": "1-n",
      "onDelete": "CASCADE"
    }
  ]
}
"#;

/// `common`：文档里启用的公共字段。**非空时**要在提示词里声明「这些字段导出时会自动附加，
/// 不要手动创建」—— 否则模型会自己在每张表里加一遍 created_at，
/// 而导出时又因为重名被跳过（表自己的赢），等于白写一遍，还让用户以为配置没生效。
fn system_prompt(common: &[DbField]) -> String {
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
   - **default 一律写成字符串**（\"0\" / \"CURRENT_TIMESTAMP\" / \"false\"）：即使字段是boolean 或 int，
     也不要写 \"default\":false、\"default\":0 这种裸值（我这边按字符串拼进 SQL，裸值会让整份产出作废）
   - 不要写 \"VARCHAR(32)\" 这种整串，永远用上面的 type 对象
5. 约定：
   - 节点 id/x/y 可省略（我这边来补）；表名不允许重名
   - 每张表至少一个主键（字段 \"pk\":true）；\"autoIncrement\":true 必须是主键
   - **主键名禁止用 \"id\"**：一律用有业务含义的名字（表 user 用 user_id、表 order_item 用 order_item_id），
     外键则引用父表那个主键名。裸 id 会让联表 SQL 到处都是 user.id 这种分不清是谁的列
   - **通用裸名必须带表名前缀**：name/title/label/desc/description/price/amount/status/type/code/
     value/content/text/url/phone/email/address/count/total/size/color/comment/note/remark 这类词
     单独出现时无法分辨属于哪张表，要写成 article_title、article_status、user_name；
     **已经带下划线的（created_at、user_id、article_title）不用再加前缀**
   - **每个外键字段都要有索引**：relations 里的 from.field（非 n-n）必须在本表 indexes 里有一条
     kind 为 index 且包含该字段的记录，如 {{\"name\":\"idx_article_user\",\"kind\":\"index\",\"fields\":[\"user_id\"]}}
   - 主键只用字段 \"pk\":true 表达，不要再写 \"kind\":\"primary\" 的索引
   - indexes 只在需要查询加速或唯一约束时写
   - relations：from = 子表（外键所在端），to = 父表（被引用端）；不要写自关联；
     外键字段类型必须与父表被引用字段一致
   - 每张表、每个字段的 comment 用中文写清业务含义，这是产出里最值钱的部分
   - 表按真实实体来，不要为了显得完整而编造；字段类型拿不准时给保守类型
   - 修改场景：未涉及改动的表/关联**原样保留**，输出完整更新后的设计，不要 diff，不要省略

6. 上面第 5 条里的命名与索引规则是**硬性的**：产出后我会跑一遍自动修正（主键改名、
   通用裸名加表前缀、外键补索引），所以下面两点请务必做对：
   - **别依赖我兜底，请一次写对**。自动修正**只在目标名没被占用时才生效**：同一张表里
     若已经有 article_title，再写一个 title 就会撞名、修正被跳过，错误会原样留在你的产出里。
     所以不要同时出现 article_title 和 title 这种冲突命名。
   - **关联两端必须写**当前真实存在的字段名。自动修正会把改名**级联**到 relations 与 indexes，
     但它**不会凭空创建**字段 —— 指向一个不存在的字段名，我这边改不了。
7. 决策记录（**长期记忆**）：每次修改都要在输出的 `decisions` 数组里用一句话写清「为什么这么改」，\n   例如「status 改成枚举并加入已关闭，因为财务要求保留关单记录」。不要写「已遵守命名规范」这类\n   毫无信息的客套话。决策会存进设计文件，下次生成时你还会读到 —— 它是你与用户之间的长期记忆。\n\n8. 业务语义有歧义就先问，不要自己猜：\n   - 必须先问（输出 `clarify`）：字段取值范围、可空性、唯一性、是否需要关联、多对多还是一对多\n     —— 这些猜错就是做错业务。\n   - 不要问（按上面规范自己定）：表名与字段命名、类型选择、索引。\n   每次最多问 3 个问题；追问累计 3 轮后我会在提示词里明说「不要再问了」，那时必须直接给设计。\n\n输出格式（**二选一**）：\n   {{\"document\": {{设计文档}}, \"decisions\": [\"本轮为什么这么改…\"]}}\n   {{\"clarify\": {{\"question\": \"问什么\", \"options\": [\"选项A\", \"选项B\"]}}}}\n     —— 有歧义时只输出后者，不要同时给设计，也不要猜一个先给出来。\n{common_note}",
        schema = FORMAT_SCHEMA,
        example = FORMAT_EXAMPLE,
        common_note = common_note(common),
    )
}

/// 「公共字段已启用」的声明段。
///
/// **为什么必须有**：模型看到需求里说「记录创建时间」就会自己给每张表加 `created_at`，
/// 而导出时公共字段会因重名被跳过（表自己的定义优先）—— 模型写的那一份根本没进SQL，
/// 用户却会以为「配置了公共字段，怎么导出还是我自己写的那个」。一句话说清「这些字段我替你加」
/// 就能省掉这一整轮返工。
///
/// 没启用时返回空串，提示词里就不出现这一节（不要给模型留「也许有公共字段」的想象空间）。
fn common_note(common: &[DbField]) -> String {
    let names: Vec<String> = common
        .iter()
        .map(|f| f.name.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    if names.is_empty() {
        return String::new();
    }
    // 逐行拼而不是用 `\` 续行：续行会保留下一行的缩进，提示词里就多出一堆空格
    let lines = [
        "7. **所有表在导出时都会自动追加下面这些字段，你不要创建它们**：".to_string(),
        format!("   - {}", names.join("\u{3001}")),
        "   它们来自设计文件顶层的公共字段配置，由导出环节统一追加到每张表末尾。".to_string(),
        "   如果你也在表里写上同名字段，导出时会因重名被跳过（以表自己的定义为准），等于白写一遍，".to_string(),
        "   还会让用户误以为配置没生效 —— 而被跳过的那个字段名可能与你写的并不一样，排查起来更费劲。".to_string(),
    ];
    format!("\n{}\n", lines.join("\n"))
}

/// boolean 列的默认值该怎么写 —— **取决于目标方言**，所以这句不能进系统提示（那里不知道方言）。
///
/// `boolean` 只是设计器内部的跨方言抽象，落地时各家不同：MySQL 根本没有 BOOLEAN（导出成
/// `TINYINT(1)`）、SQLite 只有 INTEGER 存储类（也导出成 INTEGER），这两种默认值必须是 `0` / `1`；
/// 只有 PG 真有 BOOLEAN 类型，`false` / `true` 才对。写反了不会立刻报错
/// （PG 上 `DEFAULT 0` 才会直接报错），但导出的 DDL 语义会和列类型对不上。
fn bool_default_hint(dialect: &str) -> &'static str {
    match dialect {
        "postgres" => "boolean 列的默认值写 \"false\" / \"true\"（PG 有原生 BOOLEAN 类型）",
        "sqlite" => "boolean 列的默认值写 \"0\" / \"1\"（SQLite 会导出成 INTEGER，没有布尔类型）",
        _ => "boolean 列的默认值写 \"0\" / \"1\"（MySQL 没有 boolean 类型，会导出成 TINYINT(1)）",
    }
}

/// 组装 user 消息。`current` 非空（有表）时为「修改」场景：带上当前设计作基线，
/// 由模型看需求自己判断是「基于它改」还是「与它无关的全新设计」（系统提示写明两种处理）。
fn build_user_prompt(
    name_hint: &str,
    dialect: &str,
    text: &str,
    current: Option<&DbDesignDocument>,
    history: &[ChatTurn],
    clarify_round: u32,
) -> String {
    let bool_hint = bool_default_hint(dialect);
    let decisions = current.map(|d| d.decisions.as_slice()).unwrap_or(&[]);
    let mut head = String::new();
    // 已知决策：它在最前面，因为它是模型判断「该不该动某处」的前提
    if decisions.iter().any(|d| !d.trim().is_empty()) {
        head.push_str("已知设计决策与约束（用户与 AI 之前定下的，必须遵守）：\n");
        for d in decisions.iter().filter(|d| !d.trim().is_empty()) {
            head.push_str(&format!("- {}\n", d.trim()));
        }
        head.push_str("\n");
    }
    // 追问轮次：写清楚「你问过几轮」，否则提示词里那句「3 轮后必须产出」模型无从遵守
    if clarify_round > 0 {
        head.push_str(&format!(
            "注意：你已经追问过 {} 轮了。**这轮必须直接给出设计**，不要再输出 clarify。\n\n",
            clarify_round
        ));
    }
    // 最近几轮对话：滚动窗口，前端只送最近几轮。assistant 那轮记的是决策摘要而非整份 JSON
    let recent: Vec<&ChatTurn> = history
        .iter()
        .filter(|h| !h.text.trim().is_empty())
        .collect();
    if !recent.is_empty() {
        head.push_str("最近几轮对话：\n");
        for h in recent {
            let who = if h.role == "assistant" { "AI" } else { "用户" };
            head.push_str(&format!("{}：{}\n", who, h.text.trim()));
        }
        head.push_str("\n");
    }
    match current.filter(|d| !d.nodes.is_empty()) {
        None => format!(
            "{head}设计名：{name_hint}\n目标方言：{dialect}\n\n需求：\n{text}\n\n\
             请按系统提示的标准格式输出 JSON。字段命名用小写蛇形。\n{bool_hint}。"
        ),
        Some(doc) => format!(
            "{head}设计名：{name_hint}\n目标方言：{dialect}\n\n\
             当前设计（用户正在做的既有设计，作为基线）：\n{}\n\n\
             用户需求：\n{text}\n\n\
             判断：若需求是对当前设计的修改/扩展，则基于当前设计施加改动并输出完整更新后的设计 JSON\
             （未涉及改动的表/关联必须原样保留，不要省略）；\
             若需求是与当前设计无关的全新设计，则忽略当前设计、产出新设计。\
             按系统提示的标准格式输出完整 JSON。\n{bool_hint}。",
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

/// 把模型返回的 JSON 收成 [`AiOutcome`]：三种形状都要能认。
///
/// | 模型写了 | 怎么处理 |
/// |---|---|
/// | `{{"clarify":{{...}}}}` | 问问题，不落图 |
/// | `{{"document":{{...}},"decisions":[...]}}` | 正常产出 |
/// | `{{"name":...,"nodes":[...]}}`（旧形状） | 正常产出，`decisions` 当空 |
///
/// 第三行是为了向后兼容：模型不听话打出旧形状时仍然能用。
/// 格式不兼容不得导致整份产出作废（Q-0404 的教训）。
fn parse_outcome(json: serde_json::Value) -> Result<AiOutcome, String> {
    // 问题分支：先看有没有 clarify（不能直接把它当文档解析，否则会报错误的句子）
    if let Some(cl) = json.get("clarify").and_then(|c| c.as_object()) {
        let question = cl
            .get("question")
            .and_then(|q| q.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if question.is_empty() {
            return Err("AI 说要问问题却没写问问内容".into());
        }
        let options = cl
            .get("options")
            .and_then(|o| o.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();
        return Ok(AiOutcome::Clarify { question, options });
    }
    // 文档在包装层的 document 里，或者根本就是纯文档（旧形状）
    let doc_value = match json.get("document") {
        Some(d) => d.clone(),
        None => json.clone(),
    };
    let doc = parse_document(doc_value)?;
    let decisions = json
        .get("decisions")
        .and_then(|d| d.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    Ok(AiOutcome::Design { document: doc, decisions })
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

/// 生成设计文档。**同步返回完整文档**（当前是短请求，不必做流式进度）。
#[tauri::command]
pub async fn dbd_ai_generate(
    app: tauri::AppHandle,
    input: AiDesignInput,
) -> Result<AiOutcome, String> {
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
    let user = build_user_prompt(
        &name_hint,
        &dialect,
        text,
        input.current_doc.as_ref(),
        &input.history,
        input.clarify_round,
    );
    // 公共字段配置跟着「当前设计」走：新建设计没有配置，修改场景才需要声明
    let common: &[DbField] = input
        .current_doc
        .as_ref()
        .map(|d| d.common_fields.as_slice())
        .unwrap_or(&[]);

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
        &system_prompt(common),
        &user,
        0.3,
        |len, acc| {
            // 附带末尾 200 字符做预览：前端控制台实时显示「正在写什么」（与思维导图同款）
            let tail: String = acc.chars().rev().take(200).collect::<Vec<_>>().into_iter().rev().collect();
            emit_progress(&app_opt, "stream", serde_json::json!({ "length": len, "text": tail }));
            // 只有「又多出一个完整节点/关系」才推一次 —— 天然节流，
            // 不会每来 400 字符就刷一遍画布。
            // 已经出现 clarify 就不再推部分文档：追问分支的语义是「不落图」，
            // 中途推过去的部分表会留在画布上变成「半套设计」。
            if acc.contains("\"clarify\"") {
                return;
            }
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
            &system_prompt(common),
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

    // 三种形状都要能认：clarify / 包装的 design / 旧的裸文档
    let parsed = parse_outcome(json)?;
    // 追问分支：文档一个字都不动。
    // 流式期间可能已经推过部分表过去，所以前端要靠这个返回值把画布恢复到本轮开始前 ——
    // 语义是「这轮什么都没发生，只是我问了句话」。
    // 两个分支都直接返回
    match parsed {
        AiOutcome::Clarify { question, options } => {
            emit_progress(
                &Some(app.clone()),
                "clarify",
                serde_json::json!({ "question": question }),
            );
            return Ok(AiOutcome::Clarify { question, options });
        }
        AiOutcome::Design { document, decisions } => {
            // 落地前过一遍规范化：命名改合规 + 外键补索引。
            //
            // 为什么必须在这里做：命名规范是 error 级（阻断保存 / 导出），而提示词只是**概率性**
            // 约束 —— 改过示例与禁令之后模型仍会写出 type_id / title / status 这类裸名，
            // 用户拿到的就是「满屏报错且导不出来」的死锁。规范化是确定性的，且与校验共用同一套
            // 规则（singularize + GENERIC_FIELD_NAMES），不会出现两套口径漂移。
            let mut d = document;
            let normalized = super::store::normalize_naming(&mut d);
            let touched = normalized.renamed_pk.len()
                + normalized.renamed_field.len()
                + normalized.added_index.len();
            if touched > 0 || !normalized.skipped.is_empty() {
                tracing::info!(
                    "[dbd] AI 产出已规范化：主键改名 {}、字段改名 {}、补索引 {}、跳过 {}",
                    normalized.renamed_pk.len(),
                    normalized.renamed_field.len(),
                    normalized.added_index.len(),
                    normalized.skipped.len()
                );
            }
            return Ok(AiOutcome::Design { document: d, decisions });
        }
    }
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
        let p = build_user_prompt("N", "mysql", "需求甲", None, &[], 0);
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
        let p = build_user_prompt("N", "mysql", "加个字段", Some(&cur), &[], 0);
        assert!(p.contains("当前设计"), "修改场景必须带当前设计作基线");
        assert!(p.contains("orders"));
        assert!(!p.contains("\"x\""), "布局坐标不该进提示词");
        assert!(!p.contains("\"y\""));
    }

    /// 当前设计为空文档（没表）时按新建处理，不带基线。
    #[test]
    fn prompt_ignores_empty_current_doc() {
        let cur = new_document("cur", "mysql");
        let p = build_user_prompt("N", "mysql", "需求甲", Some(&cur), &[], 0);
        assert!(!p.contains("当前设计"));
    }

    /// **示例与提示词里都不允许出现裸 `id` 作主键**。
    ///
    /// 踩过的坑：这条禁令原先只存在于用户脑子里，提示词里根本没写，而 `FORMAT_EXAMPLE`
    /// 两张表的主键都叫 `id` —— 模型最强的模仿对象就是示例，于是生成的所有表主键都是 id，
    /// 「禁止 id」的要求看起来像没生效。测试把这条不变量钉住，防止有人再把示例改回去。
    #[test]
    fn prompt_and_example_forbid_bare_id_as_primary_key() {
        let p = system_prompt(&[]);
        // 禁令要在提示词里，且说得具体（「不许用 id / user_id 这类通用名」）
        assert!(
            p.contains("主键名"),
            "提示词必须写明主键命名的约束"
        );
        assert!(
            p.contains("\"id\""),
            "禁令要点名 id，只说「有含义的主键名」模型会忽略"
        );
        // 示例本身不能拿 id 当主键：示例比规则更有说服力
        let bare_id_pk = format!(
            r#"{{"name":"id","type":{{"base":"{}"}},"pk":true"#,
            "bigint"
        );
        assert!(
            !FORMAT_EXAMPLE.contains(&bare_id_pk),
            "示例里不能再用 id 作主键——模型照抄的就是示例"
        );
        // 但 schema 里字段名示例仍是 id：那里演示的是「字段怎么命名」，不是主键命名
        assert!(
            FORMAT_SCHEMA.contains("\"name\""),
            "schema 仍应演示字段结构"
        );
    }

    /// 提示词必须把**校验器管的每一类命名/索引规则**都写出来。
    ///
    /// 之前只写了主键那一条，于是模型压根不知道有「通用裸名」和「外键索引」两条规则 ——
    /// 它不是不听话，是没人告诉过它。规则与校验器脱节 = 稳定产出违规。
    #[test]
    fn prompt_states_every_naming_rule_the_validator_enforces() {
        let p = system_prompt(&[]);
        for (needle, why) in [
            ("主键名禁止用", "主键不能叫 id"),
            ("通用裸名必须带表名前缀", "name/title/status 这类要加表前缀"),
            ("已经带下划线的", "要说清 created_at 这类不必再加前缀，否则模型会过度加前缀"),
            ("每个外键字段都要有索引", "外键字段缺索引是 error 级，会阻断导出"),
        ] {
            assert!(p.contains(needle), "提示词缺少规则「{}」（{}）", needle, why);
        }
    }

    /// 提示词必须说明「产出后会自动修正」，并说清它的**失效条件**。
    ///
    /// 只说「会自动修」是有害的：模型会以为命名可以偷懒，而自动修正只在目标名没被占用时
    /// 才生效 —— 撞名就跳过，错误原样留下。关联两端同理：修正会级联改名，但不会凭空创建字段。
    #[test]
    fn prompt_explains_autofix_and_its_limits() {
        let p = system_prompt(&[]);
        assert!(
            p.contains("自动修正"),
            "要告诉模型产出后会自动规范化，否则它会以为命名可以偷懒"
        );
        assert!(
            p.contains("撞名") || p.contains("没被占用"),
            "必须说清自动修正的失效条件：目标名被占用时会跳过，错误会原样留下"
        );
        assert!(
            p.contains("不会凭空创建"),
            "必须说清关联指向不存在的字段时自动修正救不了"
        );
    }

    /// 回归（滑雪助手那份产出）：boolean 字段写 `"default": false` 曾让整份设计
    /// 报「invalid type: boolean `false`, expected a string」作废。宽容反序列化在
    /// `models::DbField::default` 上，这里钉住的是**整条解析链路**（含补 id / 端点回认）。
    #[test]
    fn parse_document_accepts_non_string_default() {
        let json = serde_json::json!({
            "name": "滑雪助手", "dialect": "mysql",
            "nodes": [{
                "name": "posture",
                "table": { "fields": [
                    { "name": "posture_id", "type": { "base": "bigint" }, "pk": true, "autoIncrement": true },
                    { "name": "need_dynamic_capture", "type": { "base": "boolean" }, "nullable": false, "default": false },
                    { "name": "posture_score", "type": { "base": "decimal", "precision": 5, "scale": 2 }, "default": 0 }
                ] }
            }]
        });
        let doc = parse_document(json).expect("非字符串默认值不该让整份产出作废");
        let fields = &doc.nodes[0].table.as_ref().unwrap().fields;
        assert_eq!(fields[1].default.as_deref(), Some("false"));
        assert_eq!(fields[2].default.as_deref(), Some("0"));
    }

    /// 提示词必须点名「default 写字符串」——产出里最常见的作废原因就是这个，
    /// 光靠schema 里那句「（可选，字符串）」模型会当成可以省略，不是「必须字符串」。
    #[test]
    fn prompt_requires_string_default() {
        let p = system_prompt(&[]);
        assert!(
            p.contains("default 一律写成字符串"),
            "提示词必须明确 default 的类型，否则模型会写裸 false / 裸 0"
        );
    }

    /// **boolean 默认值的写法必须按方言给**：MySQL / SQLite 的列其实是数值型，
    /// 让模型统一写 "false" 会导出成 `TINYINT(1) DEFAULT false`；反过来若对 PG 说 0/1，
    /// 导出的 `BOOLEAN DEFAULT 0` 在 PG 上是直接报错的。这句只能进 user 消息（那里才知道方言）。
    #[test]
    fn bool_default_hint_follows_dialect() {
        let mysql = build_user_prompt("N", "mysql", "需求甲", None, &[], 0);
        assert!(mysql.contains("TINYINT(1)"), "MySQL 要说明没有 boolean 类型: {}", mysql);
        assert!(mysql.contains("\"0\" / \"1\""), "{}", mysql);
        assert!(!mysql.contains("\"false\" / \"true\""), "{}", mysql);

        let pg = build_user_prompt("N", "postgres", "需求甲", None, &[], 0);
        assert!(pg.contains("\"false\" / \"true\""), "PG 有原生 BOOLEAN: {}", pg);

        let sqlite = build_user_prompt("N", "sqlite", "需求甲", None, &[], 0);
        assert!(sqlite.contains("INTEGER"), "{}", sqlite);

        // 修改场景同样要带上
        let mut cur = new_document("cur", "mysql");
        cur.nodes.push(DbDesignNode {
            name: "orders".into(),
            ..Default::default()
        });
        assert!(build_user_prompt("N", "mysql", "改字段", Some(&cur), &[], 0).contains("TINYINT(1)"));
    }

    /// 启用公共字段后，**提示词里必须出现「所有表导出时会自动追加、不要你创建」这段声明**。
    ///
    /// 不写的后果（模型看到「记录创建时间」就会自己加）���模型写的那份因重名被跳过、
    /// 根本没进 SQL，而用户会以为「配置了公共字段，导出怎么还是我自己写的那个」。
    #[test]
    fn prompt_declares_common_fields_when_enabled() {
        let cf = vec![
            DbField { name: "created_at".into(), ..Default::default() },
            DbField { name: "is_deleted".into(), ..Default::default() },
        ];
        let p = system_prompt(&cf);
        assert!(p.contains("所有表在导出时都会自动追加"), "{}", p);
        assert!(p.contains("不要创建它们"), "{}", p);
        // 字段名要列出来（模型得知道是哪些）
        assert!(p.contains("created_at"), "{}", p);
        assert!(p.contains("is_deleted"), "{}", p);
        // 不能留下未替换的占位符
        assert!(!p.contains("{common_note}"), "占位符没被替换: {}", p);
    }

    /// 回归（用户实测）：**画布上还没有表、只配了公共字段**时发起的 AI 生成，
    /// 提示词里也必须带那句声明。
    ///
    /// 起因：前端曾用「画布有表才把当前设计传给后端」当条件，公共字段配置跟着
    /// `currentDoc` 一起被挡在门外 —— 于是最自然的路径（刚配好公共字段就点生成）
    /// 恰恰拿不到声明，模型自己建了一遍 created_at，导出时又因重名被跳过。
    /// 空文档（`nodes: []`）+ `commonFields` 非空就是这条路径的输入形态。
    #[test]
    fn prompt_declares_common_fields_for_empty_design() {
        let json = serde_json::json!({
            "text": "设计一个订单系统",
            "dialect": "mysql",
            "currentDoc": {
                "name": "未命名设计",
                "dialect": "mysql",
                "nodes": [],
                "commonFields": [
                    { "name": "created_at", "type": { "base": "datetime" } },
                    { "name": "remark", "type": { "base": "varchar", "length": 500 } }
                ]
            }
        });
        let input: AiDesignInput = serde_json::from_value(json).expect("输入必须能解析");
        let common: &[DbField] = input
            .current_doc
            .as_ref()
            .map(|d| d.common_fields.as_slice())
            .unwrap_or(&[]);
        assert_eq!(common.len(), 2, "空文档里的公共字段也必须送到后端");
        let p = system_prompt(common);
        assert!(p.contains("所有表在导出时都会自动追加"), "{}", p);
        assert!(p.contains("created_at"), "{}", p);
        assert!(p.contains("remark"), "{}", p);
        // 同时确认：空文档仍不会被当成「修改场景」把基线塞进 user 消息
        let user = build_user_prompt(
            "未命名设计",
            "mysql",
            "设计一个订单系统",
            input.current_doc.as_ref(),
            &[],
            0,
        );
        assert!(!user.contains("当前设计"), "空文档不该带基线: {}", user);
    }

    /// 没启用时这一节**整段不出现** —— 别给模型留「也许有公共字段」的想象空间，
    /// 否则它会自己加一堆 created_at，而那些列一个都不会进 SQL。
    #[test]
    fn prompt_has_no_common_note_when_not_enabled() {
        let p = system_prompt(&[]);
        assert!(!p.contains("公共字段"), "未启用时不该提公共字段:\n{}", p);
        assert!(!p.contains("{common_note}"), "{}", p);
        // 只配了空名字的项视同没配
        let blank = vec![DbField { name: "   ".into(), ..Default::default() }];
        assert!(!system_prompt(&blank).contains("公共字段"));
    }

    /// 新协议的三种形状都要能认。兼容旧形状是硬要求：模型不听话时
    /// 不能让整份产出作废（Q-0404 的教训）。
    #[test]
    fn parse_outcome_accepts_all_three_shapes() {
        // 包装形式：文档 + 本轮决策
        let wrapped = parse_outcome(serde_json::json!({
            "document": { "name": "商品库", "dialect": "mysql",
                         "nodes": [{ "name": "product", "kind": "table", "table": { "fields": [] } }],
                         "relations": [] },
            "decisions": ["status 用枚举，因为业务只允许三种状态", "  ", "多余空行不应进来"]
        }))
        .expect("包装形式应该能认");
        match wrapped {
            AiOutcome::Design { document, decisions } => {
                assert_eq!(document.name, "商品库");
                assert_eq!(decisions.len(), 2, "空白决策该被过滤: {:?}", decisions);
            }
            other => panic!("应该是设计分支: {:?}", other),
        }

        // 旧形状：裸文档（模型忘了包装层）
        let bare = parse_outcome(serde_json::json!({
            "name": "旧形状", "dialect": "mysql",
            "nodes": [{ "name": "product", "kind": "table", "table": { "fields": [] } }],
            "relations": []
        }))
        .expect("旧形状也要能用");
        match bare {
            AiOutcome::Design { document, decisions } => {
                assert_eq!(document.name, "旧形状");
                assert!(decisions.is_empty(), "没给 decisions 就是空数组，不算错误");
            }
            other => panic!("应该是设计分支: {:?}", other),
        }
    }

    /// 追问分支：只当确实写了问题才算追问，否则返回错误会把一次正常产出直接报成“说要问却没写内容”。
    #[test]
    fn parse_outcome_reads_clarify_branch() {
        let ok = parse_outcome(serde_json::json!({
            "clarify": { "question": "订单状态要允许哪些取值？", "options": ["待付/已付/退款", " ", ""] }
        }))
        .expect("追问应该能认");
        match ok {
            AiOutcome::Clarify { question, options } => {
                assert_eq!(question, "订单状态要允许哪些取值？");
                assert_eq!(options, vec!["待付/已付/退款"], "空选项该被过滤: {:?}", options);
            }
            other => panic!("应该是追问分支: {:?}", other),
        }
        // 没选项也得能用（用户自由输入）
        let no_opts = parse_outcome(serde_json::json!({ "clarify": { "question": "怎么定？" } })).unwrap();
        assert!(matches!(no_opts, AiOutcome::Clarify { options, .. } if options.is_empty()));
        // 写了 clarify 却没写问题内容 = 协议破损，必须报错而不是当成正常产出
        assert!(parse_outcome(serde_json::json!({ "clarify": { "options": ["a"] } })).is_err());
    }

    /// **决策记录必须进提示词** —— 它是「AI 与用户之间的长期记忆」，
    /// 不进提示词就等于没存。
    #[test]
    fn prompt_carries_decisions_and_history() {
        let mut doc = new_document("订单库", "mysql");
        doc.decisions = vec!["status 含已关闭：财务要求保留关单记录".to_string()];
        let hist = vec![
            ChatTurn { role: "user".into(), text: "对库存顺序排".into() },
            ChatTurn { role: "assistant".into(), text: "已按 created_at 升序排序".into() },
        ];
        let p = build_user_prompt("订单库", "mysql", "再加一个字段", Some(&doc), &hist, 0);
        assert!(p.contains("已知设计决策与约束"), "{}", p);
        assert!(p.contains("财务要求保留关单记录"), "决策必须原文进提示词: {}", p);
        assert!(p.contains("最近几轮对话"), "{}", p);
        assert!(p.contains("已按 created_at 升序排序"), "历史要进提示词: {}", p);
        assert!(p.contains("用户：对库存顺序排"), "轮次要区分用户/AI: {}", p);

        // 没有决策与历史时不留空段（给模型看到无意义的标题只会占 token）
        let plain = build_user_prompt("新", "mysql", "设计一个商品库", None, &[], 0);
        assert!(!plain.contains("已知设计决策"), "{}", plain);
        assert!(!plain.contains("最近几轮对话"), "{}", plain);
    }

    /// 追问轮次要写进提示词，否则「达到上限就必须产出」这句口号签形同形会被模型忽略
    /// （它数不清自己问过几轮）。
    #[test]
    fn prompt_states_clarify_rule_and_round_count() {
        let p = system_prompt(&[]);
        assert!(p.contains("clarify"), "提示词要教会模型什么时候问: {}", p);
        assert!(p.contains("不要自己猜"), "{}", p);
        assert!(p.contains("decisions"), "提示词要要求写决策记录: {}", p);

        let round2 = build_user_prompt("N", "mysql", "继续", None, &[], 2);
        assert!(round2.contains("已经追问过 2 轮"), "{}", round2);
        assert!(round2.contains("不要再输出 clarify"), "{}", round2);
        // 第 1 轮（0 轮已追问）不该出现这句，否则模型第一次就被要求别问
        assert!(!build_user_prompt("N", "mysql", "开始", None, &[], 0).contains("已经追问过"));
    }

    /// **示例本身必须通过我们自己的校验器。**
    ///
    /// 这是踩过两次的坑才补上的测试：第一次示例用 `id` 作主键，第二次用
    /// `title`/`content`/`status` 这类通用裸名 —— 两次用户看到的都是「AI 照着示例
    /// 写出来的违规字段」，而提示词里已经把规则写得很清楚了。模型最强的模仿对象是
    /// 示例，示例违规就会**稳定地**产出违规。
    ///
    /// 走完整链路（`parse_document` 补 id、按表名回认关联端点 → `validate`），
    /// 与 AI 产出走的是同一条路。以后**新增任何校验规则，这条测试自动覆盖**，
    /// 不会再出现「规则加了但示例忘了改」。
    #[test]
    fn example_document_passes_our_own_validator() {
        let json: serde_json::Value =
            serde_json::from_str(FORMAT_EXAMPLE).expect("示例必须是合法 JSON");
        let doc = super::parse_document(json).expect("示例必须能被解析成设计文件");
        let report = super::super::store::validate(&doc);
        assert!(
            report.errors.is_empty(),
            "示例自己就违反了我们自己的规则，模型会照抄它：\n{}",
            report.errors.join("\n")
        );
    }

    /// AI 产出走完规范化后**必须能过校验** —— 这才是「命名约束生效」的可测定义。
    ///
    /// 光改提示词不够：模型仍会写出 type_id 这种主键、title/status 这种通用裸名，
    /// 而这两类都是 error 级，会阻断保存和导出。规范化是确定性的那一道。
    #[test]
    fn ai_output_passes_validation_after_normalizing() {
        // 故意构造一份「模型不听话」的产出：主键裸名 + 通用裸名字段 + 外键无索引
        let mut doc = new_document("文章库", "mysql");
        let mut parent = DbDesignNode {
            id: "t1".into(),
            name: "article_type".into(),
            kind: "table".into(),
            ..Default::default()
        };
        parent.table = Some(super::super::models::DbTableBody {
            fields: vec![
                serde_json::from_value(serde_json::json!({
                    "name": "type_id", "type": { "base": "bigint" }, "pk": true, "autoIncrement": true
                }))
                .unwrap(),
                serde_json::from_value(serde_json::json!({
                    "name": "name", "type": { "base": "varchar", "length": 32 }
                }))
                .unwrap(),
            ],
            indexes: vec![],
        });
        let mut child = DbDesignNode {
            id: "t2".into(),
            name: "article".into(),
            kind: "table".into(),
            ..Default::default()
        };
        child.table = Some(super::super::models::DbTableBody {
            fields: vec![
                serde_json::from_value(serde_json::json!({
                    "name": "article_id", "type": { "base": "bigint" }, "pk": true, "autoIncrement": true
                }))
                .unwrap(),
                serde_json::from_value(serde_json::json!({
                    "name": "article_title", "type": { "base": "varchar", "length": 128 }
                }))
                .unwrap(),
                serde_json::from_value(serde_json::json!({
                    "name": "type_id", "type": { "base": "bigint" }
                }))
                .unwrap(),
            ],
            indexes: vec![],
        });
        doc.nodes.push(parent);
        doc.nodes.push(child);
        doc.relations.push(DbDesignRelation {
            id: String::new(),
            name: String::new(),
            from: super::super::models::DbRelationEnd { node: doc.nodes[1].id.clone(), field: "type_id".into() },
            to: super::super::models::DbRelationEnd { node: doc.nodes[0].id.clone(), field: "type_id".into() },
            kind: "1-n".into(),
            on_delete: String::new(),
            on_update: String::new(),
            mirror: false,
        });

        // 规范化前：应当报出命名与索引两类错误（证明这份产出确实不合规）
        let before = super::super::store::validate(&doc);
        assert!(
            before.errors.iter().any(|e| e.contains("主键应命名为")),
            "裸名主键应被校验拦下：{:?}",
            before.errors
        );

        let report = super::super::store::normalize_naming(&mut doc);
        assert!(
            !report.renamed_pk.is_empty(),
            "主键应被改名：{:?}",
            report.renamed_pk
        );
        assert!(
            !report.renamed_field.is_empty(),
            "通用裸名字段应被改名：{:?}",
            report.renamed_field
        );
        assert!(
            !report.added_index.is_empty(),
            "外键字段应自动补索引：{:?}",
            report.added_index
        );
        // 改名必须级联到关联端点，否则关系会指向一个已经不存在的字段
        assert_eq!(
            doc.relations[0].to.field, "article_type_id",
            "父表主键改名后，关系指向父表的那一端必须同步"
        );
        // 子表外键 type_id 带下划线，通用裸名规则不碰它（外键名不必与父表主键同名），
        // 它仍指向父表改名后的 article_type_id —— 关系本身依然成立
        assert_eq!(doc.relations[0].from.field, "type_id");

        let after = super::super::store::validate(&doc);
        assert!(
            after.errors.is_empty(),
            "规范化后不应再有阻断级错误：{:?}",
            after.errors
        );
    }
}
