use super::*;

/// `common`：文档里启用的公共字段。**非空时**要在提示词里声明「这些字段导出时会自动附加，
/// 不要手动创建」—— 否则模型会自己在每张表里加一遍 created_at，
/// 而导出时又因为重名被跳过（表自己的赢），等于白写一遍，还让用户以为配置没生效。
pub(super) fn system_prompt(common: &[DbField]) -> String {
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
   - **通用裸名必须带表名前缀**：这份清单与应用侧校验器 `store.rs::GENERIC_FIELD_NAMES`
       **逐词对齐（42 个，加新词时两边一起改）**：
       id/name/title/label/desc/description/price/amount/status/type/code/value/content/text/url/uri/
       phone/email/address/age/sex/gender/count/num/number/total/size/weight/height/color/comment/
       note/remark/date/time/year/month/day/sort/order/flag/enabled
       这些词单独出现时无法分辨属于哪张表，要写成 article_title、article_status、user_name、
       product_price、order_date、user_enabled；
       **已经带下划线的（created_at、user_id、article_title）天然豁免，不用再加前缀**
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
7. 决策记录（**长期记忆**）：每轮都要在输出的 `decisions` 数组里写清**这次定下了什么约束、为什么**。
   好的例子：
     - 「订单明细冗余 product_name 与 product_price 快照，保证订单完成后仍能追溯下单时的商品与价格」
     - 「status 改成枚举并加入已关闭，因为财务要求保留关单记录」
   决策会**存进设计文件**，下次生成时你还会读到 —— 它是你与用户之间的长期记忆，所以只写
   **下次改动时需要知道的东西**。

   下面这些**不要写进 decisions**（没有信息量，且会稀释这条列表）：
   - **过程记录**：「其余 11 张表与 15 组关联原样保留，未做任何改动」「本轮新增 1 张表」
     —— 用户从界面上已经知道这次改了哪些，重复一遍只是噪声。
   - **规则复述**：「已遵守命名规范」「已按要求加上索引」—— 规范本来就写在你的提示词里。
   - **工具行为解释**：「导出 SQL 会自动加反引号」「我没改坐标，位置由界面决定」
     —— 那是实现细节，不是设计决策；下次生成不需要知道。
   判断标准：**这句话能不能帮你在几周后避免改错东西？** 不能，就别写。

8. 业务语义有歧义就先问，不要自己猜：
   - 必须先问（输出 `clarify`）：字段取值范围、可空性、唯一性、是否需要关联、多对多还是一对多
     —— 这些猜错就是做错业务。
   - 不要问（按上面规范自己定）：表名与字段命名、类型选择、索引。
   每次最多问 3 个问题。追问可以轮多，不要因为轮数停止提问；但若累计追问达 8 轮仍然没产出，我会明说「不要再问了」——
   那个上限是防「你不回答就无限问」的兜底，不是对正常澄清的限制。

9. **修改既有设计时默认只输出增量（patch）**，不要重发整份设计：
   - 画布上已有设计时，除非需求是「整个重做」，否则一律用 patch。省 token 也避免误改无关的表。
   - `upsertTables`：**新增或修改的表，给完整定义（含全部字段）**。没提到的表我会原样保留。
   - `removeTables`：要删的表名（或 id）。删表会连带删掉它的关联。
   - `upsertRelations` / `removeRelations`：同理，给完整关联定义或 id。
   - **合并是保守的**：你重提的字段才更新，**你没提到的字段一律保留** —— 所以不必列出没改动的字段，
     但也**不能用「没提到」来表达删除**（要删字段请把该表重提一次并写全剩余字段，或用 removeTables 删表）。
   - 表名与关联端点可以写表名，不必写内部 id（我会回认）。坐标 x/y 不要给（位置由界面决定）。
   - 只有画布上还没有任何表时才用 `document` 全量输出。

输出格式（**三选一**）：

   {{\"document\": {{设计文档}}, \"decisions\": [\"本轮为什么这么改…\"]}}

   {{\"patch\": {{\"upsertTables\": [], \"removeTables\": [], \"upsertRelations\": [], \"removeRelations\": []}}, \"decisions\": []}}

   {{\"clarify\": {{\"question\": \"问什么\", \"options\": [\"选项A\", \"选项B\"]}}}}

     —— 有歧义时只输出第三种，不要同时给设计，也不要猜一个先给出来。
{common_note}",
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
pub(super) fn common_note(common: &[DbField]) -> String {
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
pub(super) fn bool_default_hint(dialect: &str) -> &'static str {
    match dialect {
        "postgres" => "boolean 列的默认值写 \"false\" / \"true\"（PG 有原生 BOOLEAN 类型）",
        "sqlite" => "boolean 列的默认值写 \"0\" / \"1\"（SQLite 会导出成 INTEGER，没有布尔类型）",
        _ => "boolean 列的默认值写 \"0\" / \"1\"（MySQL 没有 boolean 类型，会导出成 TINYINT(1)）",
    }
}

/// 组装 user 消息。`current` 非空（有表）时为「修改」场景：带上当前设计作基线，
/// 由模型看需求自己判断是「基于它改」还是「与它无关的全新设计」（系统提示写明两种处理）。
pub(super) fn build_user_prompt(
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
            let who = if h.role == "assistant" {
                "AI"
            } else {
                "用户"
            };
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
pub(super) fn doc_to_prompt_json(d: &DbDesignDocument) -> serde_json::Value {
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
