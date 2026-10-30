use super::*;

/// 把 `add_subtree` 的嵌套 `children` 展开成扁平的 add ops。
///
/// **前序遍历**是硬要求：子节点的 `parentId` 指向刚生成的父节点 id，ops 按数组顺序
/// 由前端逐条应用，父必须先于子出现，否则子节点会挂到一个还不存在的 id 上。
pub(super) fn agent_flatten_subtree(
    children: &[serde_json::Value],
    parent: &str,
    depth: usize,
    ops: &mut Vec<serde_json::Value>,
) {
    if depth >= AGENT_MAX_SUBTREE_DEPTH || ops.len() >= AGENT_MAX_OPS {
        return;
    }
    for n in children {
        if ops.len() >= AGENT_MAX_OPS {
            return;
        }
        let name = n
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if name.is_empty() {
            continue;
        }
        let id = crate::commands::mindmap::db::new_id("agent");
        ops.push(serde_json::json!({
            "action": "add",
            "id": id.clone(),
            "parentId": parent,
            "name": name,
            "detail": n.get("detail").and_then(|v| v.as_str()).unwrap_or(""),
            "kind": n.get("kind").and_then(|v| v.as_str()).unwrap_or("other"),
            "color": agent_valid_color(n.get("color").and_then(|v| v.as_str())),
            "sources": n.get("sources").and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|s| s.as_str()).filter(|s| !s.trim().is_empty()).take(3).map(|s| s.trim().to_string()).collect::<Vec<_>>())
                .unwrap_or_default(),
        }));
        if let Some(sub) = n.get("children").and_then(|v| v.as_array()) {
            agent_flatten_subtree(sub, &id, depth + 1, ops);
        }
    }
}

/// 校验 AI 给的颜色为 #RRGGBB；不合法返回 None（调用方省略该字段，画布用默认色）。
pub(super) fn agent_valid_color(color: Option<&str>) -> Option<String> {
    let hex = color?.trim().strip_prefix('#')?;
    if hex.len() == 6 && hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Some(format!("#{}", hex.to_ascii_lowercase()))
    } else {
        None
    }
}

/// Agent 系统提示词：角色 + 工具使用规则 + 字段约束。
pub(super) fn agent_prompt(doc_name: &str) -> String {
    format!(
        r#"你是思维导图「{name}」的智能体助手，工作在导图应用的右侧对话栏里。用户会要求你分析、修改或重组这张导图。

你能**直接改动这张导图**：下面每个写工具都会立刻落到画布上，不需要用户确认（用户可用 Ctrl+Z 撤销）。所以可以边生成回复边连续操作，不必先征求同意、也不要把改动计划写在回复里等用户执行。

规则：
1. 涉及导图内容时优先使用工具；修改前先用 get_document_overview 了解结构，必要时 find_nodes / get_subtree 确认细节。引用的 id 必须来自工具返回结果，不要臆造。
2. 新增节点有三种方式：**多层结构一律用 add_subtree**（一次调用提交整棵嵌套树，children 里可继续嵌 children，系统按层级生成并挂好）；「同一父节点下加一批平级兄弟」用 add_child_nodes；要新起一条主线（无父节点）用 create_root_node。三者都不要提供 id，系统会生成并在工具结果里返回。
3. kind 取值：root|module|component|service|route|config|file|task|requirement|constraint|risk|other。color 是 #RRGGBB。
4. 删除（delete_nodes）与移动（move_nodes）也直接生效，不会弹确认框。所以动手前自己核一遍 id：删除会连同整棵子树一起消失；move 不能把节点挂到它自己的子孙下（会被拒绝）。
5. 不改图的分析（总结、找重复与缺口、回答问题）直接回答，引用节点名称。
6. 用户用 @ 引用的文件会以「用户引用的文件内容」附在消息里。要求分析某个文件的业务逻辑时：先通读给出的内容，再按「入口/流程/分支/关键数据/边界与异常」组织成节点落到图上，并用 add_child_nodes / add_subtree 的 sources 把被分析的文件锚到相关节点（项目相对路径或绝对路径均可）。
7. 把一份**带层级编号的文本**（「一、」「（一）」「1.」「1.1」「①」等）转成导图时：严格按原文层级**展开到最深一层**，不要合并、不要省略任何一级；`name` 只放标题，条目下的说明性正文放进 `detail`（不要把整段说明塞进 name，名称过长会毁掉画布可读性）。
8. 目标节点有歧义时（get_document_overview 里出现多个同名节点，例如有好几个「扩展」）：优先用**用户当前选中的节点**；没有选中节点就先问用户是哪一个，不要猜。
9. 全程用中文，简洁，可用 Markdown。
10. **每次调用工具前，先在你自己的回复里用一到两句话说明「接下来要做什么、为什么」**（这段会原样展示给用户的「思考过程」面板）。不要只发工具调用不说话，也不要把行动计划憋到最后一次性说完。"#,
        name = doc_name
    )
}

/// 从「最新往回」收集会话历史直到字符预算用尽，返回保持时间正序的 (role, content)。
/// 空内容的 assistant 行（纯 ops 载荷）不进上下文——ops 已经体现在用户可见的
/// 后续对话里，回放进提示词只会重复占预算。
pub(super) fn agent_history_window(
    history: &[AgentMessageRow],
    budget: usize,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut left = budget;
    for m in history.iter().rev() {
        if (m.role != "user" && m.role != "assistant") || m.content.trim().is_empty() {
            continue;
        }
        let cost = m.content.chars().count() + 8;
        if cost > left {
            break;
        }
        left -= cost;
        out.push((m.role.clone(), m.content.clone()));
    }
    out.reverse();
    out
}

/// 节点 id 及其全部子孙 id（含自身）。带已访问去重：脏数据成环时不会死循环。
pub(super) fn agent_descendant_ids(nodes: &[MindmapNode], root_id: &str) -> Vec<String> {
    let mut out = vec![root_id.to_string()];
    let mut i = 0;
    while i < out.len() {
        let cur = out[i].clone();
        i += 1;
        for n in nodes {
            if n.parent_id.as_deref() == Some(cur.as_str()) && !out.iter().any(|x| x == &n.id) {
                out.push(n.id.clone());
            }
        }
    }
    out
}

pub(super) fn agent_node_brief(n: &MindmapNode, with_detail: bool) -> serde_json::Value {
    let mut v =
        serde_json::json!({ "id": n.id, "parentId": n.parent_id, "name": n.name, "kind": n.kind });
    if with_detail {
        v["detail"] = serde_json::json!(n.detail);
        v["color"] = serde_json::json!(n.color);
    }
    v
}

/// 节点在导图里的可读路径（`根 > … > 自身`）。
///
/// 拿它去分辨**同名节点**：图里常有 N 个都叫「扩展」的节点，只给 id 与 parent_id
/// 的话模型没法判断该挂到哪一个（parent_id 是不透明的 id）。
pub(super) fn agent_node_path(
    nodes: &[crate::commands::mindmap::models::MindmapNode],
    id: &str,
) -> String {
    let mut chain = vec![id.to_string()];
    let mut cursor = id.to_string();
    // 防环兜底：父指针异常时最多回溯 32 层
    for _ in 0..32 {
        let parent = nodes
            .iter()
            .find(|n| n.id == cursor)
            .and_then(|n| n.parent_id.clone());
        match parent {
            Some(p) if !p.is_empty() => {
                chain.push(p.clone());
                cursor = p;
            }
            _ => break,
        }
    }
    chain.reverse();
    chain
        .iter()
        .filter_map(|i| nodes.iter().find(|n| n.id == *i).map(|n| n.name.as_str()))
        .collect::<Vec<_>>()
        .join(" > ")
}

/// 读工具：文档大纲（压缩表示，一行一个节点）。
///
/// 输出带 `path` 列（`根 > … > 节点名`）：同名节点很常见（比如多个「扩展」），
/// 没有路径模型只能瞎猜一个 id。
pub(super) fn agent_tool_overview(full: &DocumentFull) -> serde_json::Value {
    let mut outline = String::from("id | parent_id | path | name\n");
    for n in &full.nodes {
        outline.push_str(&format!(
            "{} | {} | {} | {}\n",
            n.id,
            n.parent_id.as_deref().unwrap_or("-"),
            agent_node_path(&full.nodes, &n.id),
            n.name
        ));
    }
    serde_json::json!({ "document": full.document.name, "total": full.nodes.len(), "outline": outline })
}

/// 读工具：子树全文。
pub(super) fn agent_tool_subtree(full: &DocumentFull, root_id: &str) -> serde_json::Value {
    if !full.nodes.iter().any(|n| n.id == root_id) {
        return serde_json::json!({ "error": "节点不存在", "rootId": root_id });
    }
    let ids = agent_descendant_ids(&full.nodes, root_id);
    let nodes: Vec<serde_json::Value> = full
        .nodes
        .iter()
        .filter(|n| ids.iter().any(|i| i == &n.id))
        .map(|n| agent_node_brief(n, true))
        .collect();
    serde_json::json!({ "rootId": root_id, "count": nodes.len(), "nodes": nodes })
}

/// 读工具：查询节点（keyword 模糊 + name 精确，可只给一个）。
///
/// 结果带 `path`（`根 > … > 节点`）：图里同名节点很常见（多个「扩展」），
/// 只给 id 模型分不清该操作哪一个。
pub(super) fn agent_tool_find(
    full: &DocumentFull,
    keyword: &str,
    exact_name: &str,
) -> serde_json::Value {
    let kw = keyword.trim().to_lowercase();
    let exact = exact_name.trim();
    if kw.is_empty() && exact.is_empty() {
        return serde_json::json!({ "matches": [], "total": full.nodes.len() });
    }
    let matches: Vec<serde_json::Value> = full
        .nodes
        .iter()
        // 两个条件都给了就取交集（先按名字锁定，再用关键词收窄）
        .filter(|n| {
            let by_exact = exact.is_empty() || n.name == exact;
            let by_kw = kw.is_empty()
                || n.name.to_lowercase().contains(&kw)
                || n.detail.to_lowercase().contains(&kw);
            by_exact && by_kw
        })
        .take(30)
        .map(|n| {
            let mut brief = agent_node_brief(n, false);
            if let Some(o) = brief.as_object_mut() {
                o.insert(
                    "path".into(),
                    serde_json::json!(agent_node_path(&full.nodes, &n.id)),
                );
                let children = full
                    .nodes
                    .iter()
                    .filter(|c| c.parent_id.as_deref() == Some(n.id.as_str()))
                    .count();
                o.insert("childCount".into(), serde_json::json!(children));
            }
            brief
        })
        .collect();
    serde_json::json!({ "matches": matches, "total": full.nodes.len() })
}

/// 把一次写工具调用归一化为 ops 数组（每个受影响节点一条，camelCase 直达前端）。
pub(super) fn agent_build_ops(
    action: &str,
    args: &serde_json::Value,
) -> Result<Vec<serde_json::Value>, String> {
    let mut ops: Vec<serde_json::Value> = Vec::new();
    match action {
        // 建根节点：parentId 为 null，画布上独立成一棵树
        "create_root_node" => {
            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if name.is_empty() {
                return Err("create_root_node 缺少 name".into());
            }
            ops.push(serde_json::json!({
                "action": "add",
                "id": crate::commands::mindmap::db::new_id("agent"),
                "parentId": serde_json::Value::Null,
                "name": name,
                "detail": args.get("detail").and_then(|v| v.as_str()).unwrap_or(""),
                "kind": "root",
                "color": agent_valid_color(args.get("color").and_then(|v| v.as_str()))
                    .unwrap_or_else(|| "#f8fafc".to_string()),
                "sources": serde_json::Value::Array(vec![]),
            }));
        }
        "add_child_nodes" => {
            let parent = args
                .get("parentId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .trim()
                .to_string();
            let nodes = args
                .get("nodes")
                .and_then(|v| v.as_array())
                .ok_or("add_child_nodes 缺少 nodes")?;
            for n in nodes.iter().take(AGENT_MAX_OPS) {
                let name = n
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                ops.push(serde_json::json!({
                    "action": "add",
                    "id": crate::commands::mindmap::db::new_id("agent"),
                    "parentId": parent,
                    "name": name,
                    "detail": n.get("detail").and_then(|v| v.as_str()).unwrap_or(""),
                    "kind": n.get("kind").and_then(|v| v.as_str()).unwrap_or("other"),
                    "color": agent_valid_color(n.get("color").and_then(|v| v.as_str())),
                    // 证据锚定：分析文件后生成的节点带着来源文件（最多 3 个，与节点上限一致）
                    "sources": n.get("sources").and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|s| s.as_str()).filter(|s| !s.trim().is_empty()).take(3).map(|s| s.trim().to_string()).collect::<Vec<_>>())
                        .unwrap_or_default(),
                }));
            }
            if ops.is_empty() {
                return Err("add_child_nodes 没有有效节点（name 不能为空）".into());
            }
        }
        "add_subtree" => {
            let parent = args
                .get("parentId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .trim()
                .to_string();
            let children = args
                .get("children")
                .and_then(|v| v.as_array())
                .ok_or("add_subtree 缺少 children")?;
            agent_flatten_subtree(children, &parent, 0, &mut ops);
            if ops.is_empty() {
                return Err("add_subtree 没有有效节点（name 不能为空）".into());
            }
        }
        "update_nodes" => {
            for u in args
                .get("updates")
                .and_then(|v| v.as_array())
                .ok_or("update_nodes 缺少 updates")?
                .iter()
                .take(AGENT_MAX_OPS)
            {
                let id = u
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if id.is_empty() {
                    continue;
                }
                let mut op = serde_json::json!({ "action": "update", "id": id });
                for (key, field) in [("name", "name"), ("detail", "detail")] {
                    if let Some(v) = u.get(key).and_then(|v| v.as_str()) {
                        op[field] = serde_json::json!(v);
                    }
                }
                if let Some(c) = agent_valid_color(u.get("color").and_then(|v| v.as_str())) {
                    op["color"] = serde_json::json!(c);
                }
                if op.get("name").is_none()
                    && op.get("detail").is_none()
                    && op.get("color").is_none()
                {
                    continue;
                }
                ops.push(op);
            }
            if ops.is_empty() {
                return Err("update_nodes 没有有效更新".into());
            }
        }
        "delete_nodes" => {
            for id in args
                .get("ids")
                .and_then(|v| v.as_array())
                .ok_or("delete_nodes 缺少 ids")?
                .iter()
                .take(AGENT_MAX_OPS)
            {
                let id = id.as_str().unwrap_or("").trim();
                if !id.is_empty() {
                    ops.push(serde_json::json!({ "action": "delete", "id": id }));
                }
            }
            if ops.is_empty() {
                return Err("delete_nodes 没有有效 id".into());
            }
        }
        "move_nodes" => {
            let parent = args
                .get("newParentId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .trim()
                .to_string();
            if parent.is_empty() {
                return Err("move_nodes 缺少 newParentId".into());
            }
            for id in args
                .get("ids")
                .and_then(|v| v.as_array())
                .ok_or("move_nodes 缺少 ids")?
                .iter()
                .take(AGENT_MAX_OPS)
            {
                let id = id.as_str().unwrap_or("").trim();
                if !id.is_empty() {
                    ops.push(serde_json::json!({ "action": "move", "id": id, "parentId": parent }));
                }
            }
            if ops.is_empty() {
                return Err("move_nodes 没有有效 id".into());
            }
        }
        other => return Err(format!("未知写工具 {}", other)),
    }
    Ok(ops)
}

/// 写工具处理：构建 ops → 防环校验 → 事件发给前端 → 阻塞等待应用/确认结果。
/// 返回 (全部 ops, 给模型的工具结果)。回填复用问答通道（mm_ai_answer）：
/// 前端把应用/确认结果作为 answer 发回，取消时通道收到 Null。
/// 一次写工具调用：**直接生效**，不等用户确认。
///
/// 早先删除/移动要弹确认清单、后端阻塞等用户裁决（最长 10 分钟），模型只能干等；
/// 现在改成：发 ops 事件让前端立刻落图（界面有撤销快照，Ctrl+Z 可回退），
/// 后端不等回执、继续生成回复。
///
/// 同时把 ops 同步进**本轮的内存快照** `full`：模型常常「改完立刻再查」，
/// 看不到自己的改动就会以为没生效而反复重试。
pub(super) fn agent_handle_write(
    app: &Option<tauri::AppHandle>,
    cancel: &std::sync::atomic::AtomicBool,
    full: &mut DocumentFull,
    run_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Result<(Vec<serde_json::Value>, serde_json::Value), String> {
    cancel_err(app, cancel)?;
    let mut ops = agent_build_ops(name, args)?;
    // 防环：move 的目标父节点不能是被移动节点自身或其后代
    for op in ops
        .iter()
        .filter(|o| o.get("action").and_then(|v| v.as_str()) == Some("move"))
    {
        let id = op.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        let new_parent = op
            .get("parentId")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if agent_descendant_ids(&full.nodes, id)
            .iter()
            .any(|x| x == new_parent)
        {
            return Err("不能把节点移动到它自身或它的子孙节点下".into());
        }
    }
    emit_progress(
        app,
        "agentOps",
        serde_json::json!({
            "runId": run_id,
            "ops": ops,
        }),
    );
    agent_apply_ops_to_snapshot(full, &ops);
    // 结果回给模型时只给「干了什么」的摘要：完整 ops 太长，白耗 token
    let summary: Vec<String> = ops
        .iter()
        .map(|o| {
            let action = o.get("action").and_then(|v| v.as_str()).unwrap_or("?");
            let id = o.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let name = o.get("name").and_then(|v| v.as_str()).unwrap_or("");
            match action {
                "add" => format!("add {} → {}", name, id),
                "update" => format!("update {}", id),
                "delete" => format!("delete {}", id),
                "move" => format!(
                    "move {} → parent {}",
                    id,
                    o.get("parentId").and_then(|v| v.as_str()).unwrap_or("null")
                ),
                _ => format!("{} {}", action, id),
            }
        })
        .collect();
    let count = ops.len();
    let applied = std::mem::take(&mut ops);
    Ok((
        applied.clone(),
        serde_json::json!({ "status": "applied", "count": count, "changes": summary }),
    ))
}

/// 把已下发的 ops 应用到本轮的内存快照，让同一轮里后续的读工具看得到改动。
pub(super) fn agent_apply_ops_to_snapshot(full: &mut DocumentFull, ops: &[serde_json::Value]) {
    for op in ops {
        let action = op.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let id = op.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        match action {
            "add" => {
                if full.nodes.iter().any(|n| n.id == id) {
                    continue;
                }
                full.nodes
                    .push(crate::commands::mindmap::models::MindmapNode {
                        id: id.to_string(),
                        document_id: full.document.id.clone(),
                        parent_id: op
                            .get("parentId")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string()),
                        name: op
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        detail: op
                            .get("detail")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        kind: op
                            .get("kind")
                            .and_then(|v| v.as_str())
                            .unwrap_or("other")
                            .to_string(),
                        color: op
                            .get("color")
                            .and_then(|v| v.as_str())
                            .unwrap_or("#f59e0b")
                            .to_string(),
                        sources: op
                            .get("sources")
                            .and_then(|v| v.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|s| s.as_str())
                                    .map(|s| s.to_string())
                                    .collect()
                            })
                            .unwrap_or_default(),
                        position_x: 0.0,
                        position_y: 0.0,
                    });
            }
            "update" => {
                if let Some(n) = full.nodes.iter_mut().find(|n| n.id == id) {
                    if let Some(v) = op.get("name").and_then(|v| v.as_str()) {
                        n.name = v.to_string();
                    }
                    if let Some(v) = op.get("detail").and_then(|v| v.as_str()) {
                        n.detail = v.to_string();
                    }
                    if let Some(v) = op.get("color").and_then(|v| v.as_str()) {
                        n.color = v.to_string();
                    }
                }
            }
            "delete" => {
                let doomed: std::collections::HashSet<String> =
                    agent_descendant_ids(&full.nodes, id).into_iter().collect();
                full.nodes.retain(|n| !doomed.contains(&n.id));
            }
            "move" => {
                let new_parent = op
                    .get("parentId")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                if let Some(n) = full.nodes.iter_mut().find(|n| n.id == id) {
                    n.parent_id = new_parent;
                }
            }
            _ => {}
        }
    }
}
