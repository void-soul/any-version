use super::*;

/// 建根节点：parentId 必须是 null（前端按 null 判定「无父节点」），kind 为 root。
#[test]
fn create_root_node_builds_a_parentless_root() {
    let ops = agent_build_ops(
        "create_root_node",
        &serde_json::json!({ "name": "新主线", "detail": "说明" }),
    )
    .unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["action"], "add");
    assert!(
        ops[0]["parentId"].is_null(),
        "根节点不能有父节点: {}",
        ops[0]
    );
    assert_eq!(ops[0]["name"], "新主线");
    assert_eq!(ops[0]["kind"], "root");
    assert!(agent_build_ops(
        "create_root_node",
        &serde_json::json!({ "detail": "没名字" })
    )
    .is_err());
}

/// 写 ops 同步进内存快照：模型「改完立刻再查」时读工具必须看得到自己的改动，
/// 否则它会以为没生效而反复重试。
#[test]
fn write_ops_are_reflected_in_the_snapshot() {
    let mut full = crate::commands::mindmap::models::DocumentFull {
        document: crate::commands::mindmap::models::MindmapDocument {
            id: "doc".into(),
            name: "导图".into(),
            description: String::new(),
            source_type: "manual".into(),
            source_desc: String::new(),
            folder_id: None,
            node_count: 0,
            sticker_count: 0,
            created_at: String::new(),
            updated_at: String::new(),
            background_texture: "dots".into(),
            layout_dir: "lr".into(),
            ai_imports: 0,
            project_dir: None,
            ai_input_tokens: 0,
            ai_output_tokens: 0,
        },
        nodes: vec![node_for_snapshot("r", None, "根")],
        links: vec![],
        stickers: vec![],
    };

    // 加子节点 → 快照里能查到，且大纲能列出它
    let ops = agent_build_ops(
        "add_child_nodes",
        &serde_json::json!({ "parentId": "r", "nodes": [{ "name": "子节点" }] }),
    )
    .unwrap();
    agent_apply_ops_to_snapshot(&mut full, &ops);
    let child_id = ops[0]["id"].as_str().unwrap().to_string();
    assert!(full.nodes.iter().any(|n| n.id == child_id));
    let outline = agent_tool_overview(&full);
    assert!(outline["outline"].as_str().unwrap().contains("子节点"));

    // 移动 → 父节点变了
    let ops = agent_build_ops(
        "create_root_node",
        &serde_json::json!({ "name": "另一条主线" }),
    )
    .unwrap();
    let new_root = ops[0]["id"].as_str().unwrap().to_string();
    agent_apply_ops_to_snapshot(&mut full, &ops);
    agent_apply_ops_to_snapshot(
        &mut full,
        &agent_build_ops(
            "move_nodes",
            &serde_json::json!({ "ids": [child_id.clone()], "newParentId": new_root.clone() }),
        )
        .unwrap(),
    );
    assert_eq!(
        full.nodes
            .iter()
            .find(|n| n.id == child_id)
            .unwrap()
            .parent_id
            .as_deref(),
        Some(new_root.as_str())
    );

    // 删除 → 连同子树一起从快照消失
    agent_apply_ops_to_snapshot(
        &mut full,
        &agent_build_ops(
            "delete_nodes",
            &serde_json::json!({ "ids": [new_root.clone()] }),
        )
        .unwrap(),
    );
    assert!(!full.nodes.iter().any(|n| n.id == new_root));
    assert!(!full.nodes.iter().any(|n| n.id == child_id));
}

fn node_for_snapshot(
    id: &str,
    parent: Option<&str>,
    name: &str,
) -> crate::commands::mindmap::models::MindmapNode {
    crate::commands::mindmap::models::MindmapNode {
        id: id.into(),
        document_id: "doc".into(),
        parent_id: parent.map(|p| p.into()),
        name: name.into(),
        detail: String::new(),
        kind: if parent.is_none() { "root" } else { "other" }.into(),
        color: "#f59e0b".into(),
        sources: vec![],
        position_x: 0.0,
        position_y: 0.0,
    }
}

#[test]
fn agent_valid_color_accepts_only_rrggbb() {
    assert_eq!(
        agent_valid_color(Some("#FF8800")).as_deref(),
        Some("#ff8800")
    );
    assert_eq!(
        agent_valid_color(Some(" #A1B2C3 ")).as_deref(),
        Some("#a1b2c3")
    );
    // 非法：缺 #、长度不对、非十六进制
    assert_eq!(agent_valid_color(Some("FF8800")), None);
    assert_eq!(agent_valid_color(Some("#F80")), None);
    assert_eq!(agent_valid_color(Some("#GG0000")), None);
    assert_eq!(agent_valid_color(None), None);
    assert_eq!(agent_valid_color(Some("")), None);
}

#[test]
fn agent_history_window_keeps_order_and_respects_budget() {
    fn row(role: &str, content: &str) -> AgentMessageRow {
        AgentMessageRow {
            id: String::new(),
            session_id: String::new(),
            role: role.into(),
            content: content.into(),
            ops_json: "[]".into(),
            created_at: String::new(),
        }
    }
    let history = vec![
        row("user", "第一条"),
        row("assistant", "第一条回答"),
        // 纯 ops 载荷行：不进上下文
        row("assistant", ""),
        row("user", "第二条"),
    ];
    let win = agent_history_window(&history, 10_000);
    assert_eq!(win.len(), 3);
    assert_eq!(win[0], ("user".to_string(), "第一条".to_string()));
    assert_eq!(win[2], ("user".to_string(), "第二条".to_string()));

    // 预算只够两条时，从最新往回保留
    let win = agent_history_window(&history, ("第二条".chars().count() + 8) as usize);
    assert_eq!(win, vec![("user".to_string(), "第二条".to_string())]);
}

/// 外部 JSON 导入：parent 引用、未知父级、kind/color 规范化。
#[test]
fn import_json_builds_tree_and_normalizes_bad_fields() {
    let json = serde_json::json!({
        "name": "订单系统",
        "layoutDir": "tb",
        "nodes": [
            { "id": "root", "name": "订单系统", "kind": "root", "color": "#f8fafc", "detail": "总览" },
            { "id": "a", "name": "下单", "parent_id": "root", "kind": "unknown-kind", "color": "red" },
            { "id": "b", "name": "支付", "parentId": "a" },
            { "id": "c", "name": "", "parent_id": "不存在的父级" }
        ]
    });
    let mut nodes = json_to_mindmap_nodes(&json, "doc-1", "imp_");
    let mut warnings = Vec::new();
    normalize_imported_nodes(&mut nodes, &mut warnings);

    assert_eq!(nodes.len(), 4);
    // 根节点：没有父
    assert!(nodes
        .iter()
        .any(|n| n.name == "订单系统" && n.parent_id.is_none()));
    // parent_id / parentId 都要认
    let pay = nodes.iter().find(|n| n.name == "支付").unwrap();
    let order = nodes.iter().find(|n| n.name == "下单").unwrap();
    assert_eq!(pay.parent_id.as_deref(), Some(order.id.as_str()));
    // 引用了不存在的父级 → 自动成为新根，不能丢节点
    let orphan = nodes.iter().find(|n| n.name == "未命名").unwrap();
    assert!(orphan.parent_id.is_none());
    // kind 越界 / color 非法 → 规范化并记 warning
    assert_eq!(order.kind, "other");
    assert_eq!(order.color, "#94a3b8");
    assert!(warnings.iter().any(|w| w.contains("kind")));
    assert!(warnings.iter().any(|w| w.contains("color")));
}

/// 空 nodes / 缺 nodes 必须报错，而不是建出一份空白文档。
#[test]
fn import_json_rejects_empty_nodes() {
    let json = serde_json::json!({ "nodes": [] });
    assert!(json_to_mindmap_nodes(&json, "doc-1", "imp_").is_empty());
    assert!(serde_json::json!({ "name": "x" }).get("nodes").is_none());
}

#[test]
fn agent_tools_spec_is_valid_openai_tools_array() {
    let spec: serde_json::Value =
        serde_json::from_str(AGENT_TOOLS_SPEC).expect("工具集必须是合法 JSON");
    let arr = spec.as_array().expect("工具集必须是数组");
    assert!(arr.len() >= 7);
    for t in arr {
        let name = t
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|v| v.as_str());
        assert!(
            name.is_some_and(|n| !n.is_empty()),
            "每个工具必须有 function.name"
        );
    }
}

#[test]
fn agent_build_ops_normalizes_add_with_generated_ids() {
    let args = serde_json::json!({
        "parentId": "root-1",
        "nodes": [
            { "name": "模块A", "detail": "职责", "kind": "module", "color": "#00FF00" },
            { "name": "  " },
            { "color": "#00FF00" }
        ]
    });
    let ops = agent_build_ops("add_child_nodes", &args).unwrap();
    // 空名与缺名节点被跳过
    assert_eq!(ops.len(), 1);
    let op = &ops[0];
    assert_eq!(op["action"], "add");
    assert_eq!(op["parentId"], "root-1");
    assert_eq!(op["name"], "模块A");
    assert_eq!(op["color"], "#00ff00");
    // id 由后端生成，模型引用它时有据可依
    assert!(op["id"].as_str().is_some_and(|s| s.starts_with("agent_")));
}

/// `add_subtree` 的核心价值：一次调用产出整棵树的 ops，层级由**结构**保证。
/// 前序是硬要求——前端按数组顺序逐条应用，子节点的父必须先出现。
#[test]
fn add_subtree_expands_nested_children_in_preorder() {
    let args = serde_json::json!({
        "parentId": "扩展-id",
        "children": [{
            "name": "体育数字化",
            "children": [{
                "name": "智能训练与数据分析",
                "children": [
                    { "name": "高速运动场景AI动作捕捉", "detail": "解决实时性不足…" },
                    { "name": "可穿戴设备生理信号采集" }
                ]
            }]
        }]
    });
    let ops = agent_build_ops("add_subtree", &args).expect("展开应成功");
    assert_eq!(ops.len(), 4, "三层树应产出 4 条 add op");

    // 前序：父在子之前
    assert_eq!(ops[0]["name"], "体育数字化");
    assert_eq!(ops[0]["parentId"], "扩展-id");
    assert_eq!(ops[1]["name"], "智能训练与数据分析");
    assert_eq!(
        ops[1]["parentId"], ops[0]["id"],
        "二级要挂在一级新生成的 id 下"
    );
    assert_eq!(ops[2]["name"], "高速运动场景AI动作捕捉");
    assert_eq!(ops[2]["parentId"], ops[1]["id"]);
    assert_eq!(ops[3]["name"], "可穿戴设备生理信号采集");
    assert_eq!(ops[3]["parentId"], ops[1]["id"]);

    // 说明性正文进 detail，name 保持短标题
    assert_eq!(ops[2]["detail"], "解决实时性不足…");
    // id 必须各不相同（否则层级会塌成一片）
    let ids: std::collections::HashSet<&str> =
        ops.iter().filter_map(|o| o["id"].as_str()).collect();
    assert_eq!(ids.len(), 4);
}

/// 病态输入要被拦住：过深、过大、空名，都只是「少写」而不是崩或写脏数据。
#[test]
fn add_subtree_caps_depth_and_drops_empty_names() {
    // 深度超过上限：只写到上限为止，不 panic、不死循环
    fn nest(depth: usize) -> serde_json::Value {
        let mut node = serde_json::json!({ "name": format!("n{depth}") });
        if depth > 0 {
            node["children"] = serde_json::json!([nest(depth - 1)]);
        }
        node
    }
    let ops = agent_build_ops(
        "add_subtree",
        &serde_json::json!({ "parentId": "p", "children": [nest(15)] }),
    )
    .unwrap();
    assert!(
        ops.len() <= AGENT_MAX_SUBTREE_DEPTH,
        "深度必须被截断: {}",
        ops.len()
    );

    // 空名节点跳过（模型偶尔会吐 {"children": [...]}）
    let ops = agent_build_ops(
        "add_subtree",
        &serde_json::json!({
            "parentId": "p",
            "children": [{ "name": "  " }, { "name": "有效" }]
        }),
    )
    .unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["name"], "有效");
}

#[test]
fn agent_build_ops_rejects_invalid_input() {
    assert!(agent_build_ops("add_child_nodes", &serde_json::json!({ "nodes": [] })).is_err());
    assert!(agent_build_ops("add_subtree", &serde_json::json!({ "parentId": "p" })).is_err());
    assert!(agent_build_ops(
        "add_subtree",
        &serde_json::json!({ "parentId": "p", "children": [{ "detail": "没有名字" }] })
    )
    .is_err());
    assert!(agent_build_ops("delete_nodes", &serde_json::json!({ "ids": ["  "] })).is_err());
    assert!(agent_build_ops("move_nodes", &serde_json::json!({ "ids": ["a"] })).is_err());
    assert!(agent_build_ops("unknown", &serde_json::json!({})).is_err());
}
