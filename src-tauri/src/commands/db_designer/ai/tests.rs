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
    assert!(
        doc.relations.iter().all(|r| !r.id.is_empty()),
        "关系 id 要补上"
    );
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
    assert_eq!(
        doc.nodes[0].id, "t1",
        "空 id 要补齐，否则前端 key 撞车、关联引用不到"
    );
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
    assert!(p.contains("主键名"), "提示词必须写明主键命名的约束");
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
        (
            "已经带下划线的",
            "要说清 created_at 这类不必再加前缀，否则模型会过度加前缀",
        ),
        (
            "每个外键字段都要有索引",
            "外键字段缺索引是 error 级，会阻断导出",
        ),
    ] {
        assert!(
            p.contains(needle),
            "提示词缺少规则「{}」（{}）",
            needle,
            why
        );
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
    assert!(
        mysql.contains("TINYINT(1)"),
        "MySQL 要说明没有 boolean 类型: {}",
        mysql
    );
    assert!(mysql.contains("\"0\" / \"1\""), "{}", mysql);
    assert!(!mysql.contains("\"false\" / \"true\""), "{}", mysql);

    let pg = build_user_prompt("N", "postgres", "需求甲", None, &[], 0);
    assert!(
        pg.contains("\"false\" / \"true\""),
        "PG 有原生 BOOLEAN: {}",
        pg
    );

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
        DbField {
            name: "created_at".into(),
            ..Default::default()
        },
        DbField {
            name: "is_deleted".into(),
            ..Default::default()
        },
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
    let blank = vec![DbField {
        name: "   ".into(),
        ..Default::default()
    }];
    assert!(!system_prompt(&blank).contains("公共字段"));
}

/// 回归（用户实测）：模型输出的 JSON **结尾多了一个花括号**，
/// `parse_json` 报 `trailing characters`，整份设计作废 —— 而里面的内容完全可用。
///
/// **回归（用户实测，2026-10-16）**：模型写长 JSON 时「忘了自己在写 JSON」——
/// 在 relations 中途直接跳去写 decisions 的内容，中间整段被跳过，
/// 结果花括号不平（实测 67 开 / 64 闭）、decisions 键整个消失。
///
/// 补括号救不回来（结构断在关键位置），但**断裂点之前那几张表是完整的**（流式落图一直在用它）。
#[test]
fn extract_partial_recovers_tables_before_the_break() {
    let text = concat!(
        r#"{"document":{"name":"x","dialect":"mysql","nodes":["#,
        r#"{"kind":"table","name":"user","table":{"fields":[{"name":"user_id","type":{"base":"bigint"},"pk":true}]}},"#,
        r#"{"kind":"table","name":"order_info","table":{"fields":[{"name":"order_info_id","type":{"base":"bigint"},"pk":true},"#,
        r#"{"name":"order_info_remark","type":{"base":"varchar","length":512},"nullable":true}]}}],"relations":["#,
        r#"{"from":{"node":"t9","field":"order_id"},"to":{"node":"order_info","field":"order"#,
        "/completed/cancelled、支付状态取 unpaid…",
    );
    // 严格解析与「截取第一个对象」都救不回来
    assert!(
        extract_first_json_object(text).is_none(),
        "结构断裂应救不回来"
    );
    // 但已完成的表能提取出来
    let doc = extract_partial(text);
    assert_eq!(doc.nodes.len(), 2, "断裂点之前的表必须能救回");
    assert_eq!(doc.nodes[0].name, "user");
    assert_eq!(doc.nodes[1].name, "order_info");
    let fields: Vec<String> = doc.nodes[1]
        .table
        .as_ref()
        .unwrap()
        .fields
        .iter()
        .map(|f| f.name.clone())
        .collect();
    assert!(
        fields.contains(&"order_info_remark".to_string()),
        "{:?}",
        fields
    );
}

/// 一张表都没写完就崩了 → 真的报错（降级没有意义）。
#[test]
fn extract_partial_is_empty_when_nothing_finished() {
    let doc =
        extract_partial(r#"{"document":{"name":"x","nodes":[{"kind":"table","name":"u","tab"#);
    assert!(doc.nodes.is_empty(), "半张表不算数");
}

/// 与 Q-0404 同源：模型的格式噪声不应毁掉可用产出。
#[test]
fn extract_json_tolerates_trailing_braces() {
    // 真实失败样本（摘自用户 1.log）：结尾多一个 }
    let text = r#"{"document":{"name":"n","dialect":"mysql","nodes":[{"kind":"table","name":"user","table":{"fields":[]}}],"relations":[]},"decisions":["x"]}}"#;
    let v = extract_first_json_object(text).expect("多一个括号也要能取到完整对象");
    assert_eq!(v["decisions"][0], "x");
    assert_eq!(v["document"]["name"], "n");
    // 且取到的对象能通过真正的业务解析（不是截出半个）
    let outcome = parse_outcome(v).expect("取到的必须是可用的设计");
    assert!(matches!(outcome, AiOutcome::Design { .. }));
}

/// 字符串里的括号不得参与配对：注释里写 `{"a":1}` 是常事，
/// 若参与了配对，第一个真对象就被提前切掉（真实下获得的是半个 JSON）。
#[test]
fn extract_json_respects_braces_inside_strings() {
    let text = r#"{"document":{"name":"n","comment":"形如 { x } 的花括号","dialect":"mysql","nodes":[{"kind":"table","name":"user","table":{"fields":[]}}],"relations":[]}}"#;
    let v = extract_first_json_object(text).expect("字符串里的括号应被忽略");
    assert_eq!(v["document"]["comment"], "形如 { x } 的花括号");
}

/// 完全没有 JSON 时才给错（不能为了容错返回一个空对象）。
#[test]
fn extract_json_gives_up_cleanly() {
    assert!(extract_first_json_object("没有 JSON 的文本").is_none());
    assert!(
        extract_first_json_object(r#"{"a":"没结尾"#).is_none(),
        "未闭合的对象不能拿去"
    );
}

/// 决策记录只能写「约束与理由」，不能混进过程记录与工具行为解释。
///
/// 依据（用户实测）：真实产出的 decisions 里出现了这两类噪声：
///  · 「其余 11 张表与 15 组关联原样保留，未做任何改动」 —— 过程记录，
///    而这信息用户从 toast 就能看到（PatchApplied 已报「更新 1 张表」），写进 decisions 只是噪声。
///  · 「避开 MySQL 保留字，导出 SQL 无需额外反引号」 —— 工具行为解释，
///    而导出器本来就会自动加反引号（quote），这个因果关系本身就是错的。
#[test]
fn prompt_excludes_process_notes_from_decisions() {
    let p = system_prompt(&[]);
    // 三类噪声要明确禁止，不然模型会继续写
    assert!(p.contains("过程记录"), "要禁止把过程写进 decisions: {}", p);
    assert!(p.contains("规则复述"), "要禁止复述规范: {}", p);
    assert!(p.contains("工具行为"), "要禁止解释工具行为: {}", p);
    // 并给了可操作的判孖标准（不是抽象的「请不要写干脆」）
    assert!(p.contains("几周后避免改错"), "要给判孖标准: {}", p);
    // 并给正反例，不然模型不知道该写什么
    assert!(p.contains("冗余"), "要给好的示例: {}", p);
}

/// **防退化**：只接受错误数严格变少的版本。
///
/// 这一行就是自校验的全部价值很多：模型完全可能把对的地方改错，
/// 没有这道闸就是付两轮 token 换一份更糟的设计。
#[test]
fn accept_repair_only_when_errors_strictly_decrease() {
    assert!(accept_repair(2, 3));
    assert!(accept_repair(0, 1));
    // 持平也拒绝：原地踏步意味着这轮 token 白花（设计一点没变好）
    assert!(!accept_repair(3, 3));
    // 变差拒绝
    assert!(!accept_repair(5, 3));
}

/// 重修提示词里必须带上错误清单（不带就无从修正），且带上当前设计。
#[test]
fn repair_prompt_carries_errors_and_current_design() {
    let mut doc = new_document("商品库", "mysql");
    doc.nodes.push(DbDesignNode {
        id: "t1".into(),
        name: "product".into(),
        kind: "table".into(),
        ..Default::default()
    });
    let errors = vec!["关联 r1 指向了不存在的字段 product_nope".to_string()];
    let p = build_repair_prompt(&doc, &errors, 1);
    assert!(
        p.contains("product_nope"),
        "错误清单必须原文进提示词: {}",
        p
    );
    assert!(
        p.contains("product"),
        "当前设计必须进提示词（模型靠它才能改）: {}",
        p
    );
    assert!(
        p.contains("原样保留"),
        "要告诉它不要借机拆掉无关的部分: {}",
        p
    );
    assert!(p.contains("1."), "错误要编号方便对照改: {}", p);
    // 第一轮不说「已经过 N 轮」（那句只对重试意义）
    assert!(!p.contains("进行过 1 轮"));
    // 第二轮要提醒它已经修过一次
    let p2 = build_repair_prompt(&doc, &errors, 2);
    assert!(p2.contains("进行过 1 轮"), "重试要说明已经过一轮: {}", p2);
    assert!(
        p2.contains("一次性全部修好"),
        "重试要防止它只改一部分: {}",
        p2
    );
}

/// 不能只看字面：它不参与重修判定。
#[test]
fn repair_prompt_says_nothing_about_warnings() {
    let doc = new_document("x", "mysql");
    let p = build_repair_prompt(&doc, &["一个错误".to_string()], 1);
    assert!(
        !p.contains("warning"),
        "warnings 不阻断导出，为它们花钱是浪费: {}",
        p
    );
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
        AiOutcome::Design {
            document,
            decisions,
            ..
        } => {
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
        AiOutcome::Design {
            document,
            decisions,
            ..
        } => {
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
            assert_eq!(
                options,
                vec!["待付/已付/退款"],
                "空选项该被过滤: {:?}",
                options
            );
        }
        other => panic!("应该是追问分支: {:?}", other),
    }
    // 没选项也得能用（用户自由输入）
    let no_opts =
        parse_outcome(serde_json::json!({ "clarify": { "question": "怎么定？" } })).unwrap();
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
        ChatTurn {
            role: "user".into(),
            text: "对库存顺序排".into(),
        },
        ChatTurn {
            role: "assistant".into(),
            text: "已按 created_at 升序排序".into(),
        },
    ];
    let p = build_user_prompt("订单库", "mysql", "再加一个字段", Some(&doc), &hist, 0);
    assert!(p.contains("已知设计决策与约束"), "{}", p);
    assert!(
        p.contains("财务要求保留关单记录"),
        "决策必须原文进提示词: {}",
        p
    );
    assert!(p.contains("最近几轮对话"), "{}", p);
    assert!(
        p.contains("已按 created_at 升序排序"),
        "历史要进提示词: {}",
        p
    );
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

/// 提示词里的通用裸名词表必须与校验器真源**逐词一致**。
///
/// 这里已经漂移过一次：提示词只列了 21 个词，而 `store::GENERIC_FIELD_NAMES` 有 42 个。
/// 后果不是报错，而是模型写出 `age` / `enabled` / `uri` 这类名字时不会被提醒 ——
/// 校验器只查 `store.rs` 那份，于是全靠 `normalize_naming` 兜底**静默改名**，
/// 用户看到的字段名和它要的不是一个字，却说不出哪里不对。
///
/// 比对的是**顺序敏感的完整清单**（提示词里那段 `a/b/c` 按 `/` 切开），
/// 不是「每个词都出现过」—— 后者会因为 `id`、`age` 之类是其它词的子串而漏检。
#[test]
fn prompt_lists_every_generic_field_name() {
    use super::super::store::GENERIC_FIELD_NAMES;
    let prompt = super::system_prompt(&[]);
    let start = prompt
        .find("id/name/title")
        .expect("提示词里应有通用裸名词表");
    let seg = &prompt[start..];
    let tail = "flag/enabled";
    let end = seg
        .find(tail)
        .expect("提示词里的裸名词表应写到 flag/enabled");
    let listed: Vec<&str> = seg[..end + tail.len()].split('/').map(str::trim).collect();
    assert_eq!(
        listed,
        GENERIC_FIELD_NAMES.to_vec(),
        "裸名词表与 store.rs 不一致 —— 两边必须同时改：\
             给 store.rs 加词就补进 system_prompt 第 5 条，删词就两边一起删"
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
        from: super::super::models::DbRelationEnd {
            node: doc.nodes[1].id.clone(),
            field: "type_id".into(),
        },
        to: super::super::models::DbRelationEnd {
            node: doc.nodes[0].id.clone(),
            field: "type_id".into(),
        },
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
