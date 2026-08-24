//! 调用 AI 分析项目并生成节点图（learn_graph）。

use std::time::Duration;
use serde_json;
use super::models::*;
use super::scan;

// ─── 供应商 + 模型解析（类似 translate.rs 逻辑，复用 AI 配置）───

fn resolve_provider_model(
    provider_id: &Option<String>,
    model_id: &Option<String>,
) -> Result<(crate::commands::ai::models::AiProvider, String), String> {
    let cfg = crate::commands::ai::config::load_ai_config();
    let provider = if let Some(pid) = provider_id {
        cfg.providers
            .iter()
            .find(|p| &p.id == pid)
            .cloned()
            .ok_or_else(|| format!("未找到供应商: {}", pid))?
    } else {
        cfg.providers
            .iter()
            .find(|p| !p.api_key.is_empty() && !p.openai_url.is_empty())
            .cloned()
            .ok_or_else(|| "没有配置了 OpenAI 端点和 API Key 的供应商".to_string())?
    };
    if provider.openai_url.is_empty() {
        return Err(format!("供应商「{}」未配置 OpenAI 兼容端点", provider.name));
    }
    if provider.api_key.is_empty() {
        return Err(format!("供应商「{}」未配置 API Key", provider.name));
    }
    let model = model_id
        .clone()
        .or_else(|| provider.active_model_id.clone())
        .or_else(|| provider.models.first().map(|m| m.id.clone()))
        .ok_or_else(|| format!("供应商「{}」未配置任何模型", provider.name))?;
    Ok((provider, model))
}

/// 构建 chat/completions 请求 URL。
fn completion_url(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        format!("{}/chat/completions", trimmed)
    } else {
        format!("{}/v1/chat/completions", trimmed)
    }
}

/// 构建 AI 系统提示词。
fn system_prompt() -> String {
    r#"你是一位资深软件架构分析师。下面是一个项目的目录结构树和关键文件内容。
请分析该项目，生成一个「父-子」树形结构 JSON，用于帮助新开发者快速理解项目。

要求：
1. 输出一个合法 JSON 对象，包含：
   - "summary": 一段 markdown 文字（项目的整体介绍，300 字以内）
   - "nodes": 数组，每个节点包含：
     - "id": 唯一字符串（如 "n1","n2",...）
     - "name": 节点显示名称（简短，<20 字）
     - "parent_id": 父节点 id 或 null（根节点为 null）
     - "description": 一句话概述（<80 字）
     - "detail": 详细说明（markdown，这次分析、建议的阅读顺序、注意事项等，200 字以内）
     - "kind": 类型（module/lib/component/class/function/service/route/config/file/entry/other 之一）

2. 根节点 (parent_id=null) 应该是项目名，如 "my-app"。
3. 节点总数控制在 15-50 个，按目录/模块/组件这些有意义的逻辑结构划分，
   不要每个文件一个节点。
4. 对核心模块（如 src/xxx、lib/xxx）要适当展开；脚手架文件可合并。
5. detail 字段用中文写作，为阅读者提供学习路径建议。
6. 只输出 JSON，不要代码块标记（不要 ```json），不要任何额外文字。
7. 必须输出合法 JSON——字段名带双引号，字符串用双引号，无尾逗号。"#
        .to_string()
}

/// 安全解析 AI 返回的 JSON（处理代码块标记等）。
fn parse_ai_json(text: &str) -> Result<serde_json::Value, String> {
    let trimmed = text.trim();
    // 找到第一个 '{' 到最后一个 '}'
    let start = trimmed.find('{').ok_or("响应中没有 JSON 对象")?;
    let end = trimmed.rfind('}').ok_or("响应中没有 JSON 对象")?;
    let json_str = &trimmed[start..=end];
    serde_json::from_str(json_str).map_err(|e| format!("JSON 解析失败: {}", e))
}

/// 将 serde_json::Value 转换为 Vec<LearnNode>。
fn json_to_nodes(json: &serde_json::Value) -> Vec<LearnNode> {
    let arr = match json.get("nodes").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .enumerate()
        .map(|(i, v)| LearnNode {
            id: v
                .get("id")
                .and_then(|id| id.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("n{}", i + 1)),
            name: v
                .get("name")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "未知".to_string()),
            parent_id: v
                .get("parent_id")
                .and_then(|p| p.as_str())
                .filter(|s| !s.is_empty() && *s != "null")
                .map(|s| s.to_string()),
            description: v
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string(),
            detail: v
                .get("detail")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string(),
            kind: v
                .get("kind")
                .and_then(|k| k.as_str())
                .unwrap_or("other")
                .to_string(),
            position_x: 0.0,
            position_y: 0.0,
        })
        .collect()
}

/// 主命令：生成项目学习结构。
#[tauri::command]
pub async fn learn_generate(input: GenerateLearnInput) -> Result<LearnGraph, String> {
    let project_path = input.project_path.trim().to_string();
    if project_path.is_empty() {
        return Err("项目路径不能为空".to_string());
    }

    let project_name = scan::project_name(&project_path);

    // 1. 扫描项目
    let context = scan::scan_project(&project_path)?;

    // 2. 解析供应商/模型
    let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;

    // 3. 构建请求
    let url = completion_url(&provider.openai_url);
    let body = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system_prompt() },
            { "role": "user", "content": context }
        ],
        "stream": false,
        "temperature": 0.3,
    });

    eprintln!("[learn] 发送分析请求 (供应商: {}, 模型: {}, 项目: {})", provider.name, model, project_name);

    // 4. 发送请求
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", provider.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(Duration::from_secs(180))
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    if !status.is_success() {
        let msg = value
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("未知错误");
        return Err(format!("AI 接口错误 ({}): {}", status.as_u16(), msg));
    }

    // 5. 提取文本
    let content = value
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|ch| ch.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if content.is_empty() {
        return Err("AI 返回空结果".to_string());
    }

    // 6. 解析结构
    let parsed = parse_ai_json(&content).map_err(|e| {
        eprintln!("[learn] JSON 解析失败 (项目: {}, 响应长度: {} 字符)", project_name, content.len());
        format!("AI 返回的 JSON 无法解析: {}", e)
    })?;

    let summary = parsed
        .get("summary")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let mut nodes = json_to_nodes(&parsed);

    // 确保有根节点
    let has_root = nodes.iter().any(|n| n.parent_id.is_none());
    if !has_root {
        nodes.insert(
            0,
            LearnNode {
                id: "root".to_string(),
                name: project_name.clone(),
                parent_id: None,
                description: "项目根节点".to_string(),
                detail: summary.clone(),
                kind: "root".to_string(),
                position_x: 0.0,
                position_y: 0.0,
            },
        );
    }

    let now = chrono::Utc::now().to_rfc3339();
    let graph = LearnGraph {
        project_path: project_path.clone(),
        project_name,
        summary,
        generated_at: now,
        nodes,
    };

    // 7. 持久化
    super::store::save_graph(&graph)?;

    Ok(graph)
}

// ─── 从文本生成需求（非项目模式） ───

/// 从一段文本中提取结构化需求的系统提示词。
fn text_to_requirements_system_prompt() -> String {
    r#"你是一位资深产品经理兼软件架构师。用户会提供一段文字（可能来自会议纪要、需求文档、聊天记录、头脑风暴等）。
请从中提取并组织为结构化的需求树形图，方便团队理解、拆解和跟踪。

要求：
1. 输出一个合法 JSON 对象，包含：
   - "summary": 一段 markdown 文字（整体概述，200 字以内）
   - "nodes": 数组，每个节点包含：
     - "id": 唯一字符串（如 "n1","n2",...）
     - "name": 节点显示名称（简短，<20 字）
     - "parent_id": 父节点 id 或 null（根节点为 null）
     - "description": 一句话概述（<80 字）
     - "detail": 详细说明（markdown，含需求细节、验收标准、注意事项等，200 字以内）
     - "kind": 类型，从以下选择：
       - "root": 根节点（项目/产品名）
       - "module": 功能模块
       - "requirement": 具体需求/用户故事
       - "task": 可执行任务
       - "constraint": 约束条件/非功能需求
       - "risk": 风险/依赖
       - "other": 其他

2. 根节点 (parent_id=null) 应该是项目名或主题标题。
3. 节点总数控制在 8-30 个。把用户提到的所有明确需求、隐含需求、约束条件都组织进去。
4. detail 字段用中文写作，包含验收条件或实施建议。
5. 只输出 JSON，不要代码块标记（不要 ```json），不要任何额外文字。
6. 必须输出合法 JSON——字段名带双引号，字符串用双引号，无尾逗号。"#
        .to_string()
}

/// 从文本生成需求结构。
#[tauri::command]
pub async fn learn_generate_from_text(input: GenerateFromTextInput) -> Result<LearnGraph, String> {
    let text = input.text.trim().to_string();
    if text.is_empty() {
        return Err("文本内容不能为空".to_string());
    }
    let title = if input.title.trim().is_empty() { "需求分析" } else { input.title.trim() };

    // 1. 解析供应商/模型
    let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;

    // 2. 构建请求
    let url = completion_url(&provider.openai_url);
    let body = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": text_to_requirements_system_prompt() },
            { "role": "user", "content": format!("请分析以下文字，提取结构化需求：\n\n{}", text) }
        ],
        "stream": false,
        "temperature": 0.3,
    });

    eprintln!("[learn] 从文本提取需求 (标题: {}, 文本长度: {})", title, text.len());

    // 3. 发送请求
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", provider.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(Duration::from_secs(180))
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    if !status.is_success() {
        let msg = value
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("未知错误");
        return Err(format!("AI 接口错误 ({}): {}", status.as_u16(), msg));
    }

    // 4. 提取文本
    let content = value
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|ch| ch.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if content.is_empty() {
        return Err("AI 返回空结果".to_string());
    }

    // 5. 解析结构
    let parsed = parse_ai_json(&content).map_err(|e| {
        eprintln!("[learn] 文本需求 JSON 解析失败 (标题: {}, 响应长度: {} 字符)", title, content.len());
        format!("AI 返回的 JSON 无法解析: {}", e)
    })?;

    let summary = parsed
        .get("summary")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let mut nodes = json_to_nodes(&parsed);

    // 确保有根节点
    let has_root = nodes.iter().any(|n| n.parent_id.is_none());
    if !has_root {
        nodes.insert(0, LearnNode {
            id: "root".to_string(),
            name: title.to_string(),
            parent_id: None,
            description: "需求根节点".to_string(),
            detail: summary.clone(),
            kind: "root".to_string(),
            position_x: 0.0,
            position_y: 0.0,
        });
    }

    // 6. 用文本内容 MD5 作为"路径"标识
    let hash_hex: String = md5::compute(text.as_bytes())
        .iter().take(8).map(|b| format!("{:02x}", b)).collect();
    let path_id = format!("text:{}", hash_hex);
    let now = chrono::Utc::now().to_rfc3339();
    let graph = LearnGraph {
        project_path: path_id,
        project_name: title.to_string(),
        summary,
        generated_at: now,
        nodes,
    };

    // 7. 持久化
    super::store::save_graph(&graph)?;

    Ok(graph)
}

// ─── 子树重新生成 ───

/// 构建子树分析的系统提示词。
fn subtree_system_prompt(node_name: &str, node_kind: &str, node_description: &str) -> String {
    format!(
        r#"你是一位资深软件架构分析师。现在需要你深入分析一个软件项目的特定模块/组件。

目标节点："{node_name}"
节点类型：{node_kind}
节点描述：{node_description}

请分析该模块的内部结构，生成「父-子」树形 JSON。

要求：
1. 只分析该节点**内部**的子结构（不涉及项目其他模块），输出一个合法 JSON 对象，包含：
   - "nodes": 数组，每个子节点包含：
     - "id": 唯一字符串（如 "c1","c2",...）
     - "name": 子节点显示名称（简短，<20 字）
     - "parent_id": 父节点 id（如 "c0"）或 null（根节点——即当前被分析的节点本身——为 null）
     - "description": 一句话概述（<80 字）
     - "detail": 详细说明（markdown，说明该子模块的作用、包含哪些文件/类/函数、如何阅读等，200 字以内）
     - "kind": 类型（module/lib/component/class/function/service/route/config/file/entry/other 之一）

2. 第一个节点 (parent_id=null) 必须是当前节点本身（名称保持 "{node_name}"）。
3. 子节点总数控制在 3-12 个，按该模块内部的子目录/子组件/子文件等划分。
4. detail 字段用中文写作，为阅读者提供学习路径建议。
5. 只输出 JSON，不要代码块标记，不要任何额外文字。
6. 必须输出合法 JSON——字段名带双引号，字符串用双引号，无尾逗号。"#
    )
}

/// 生成唯一的新节点 ID（基于现有节点 ID 避免冲突）。
fn fresh_node_id(existing_ids: &std::collections::HashSet<&str>) -> String {
    for i in 0u64.. {
        let candidate = format!("n{}", i);
        if !existing_ids.contains(candidate.as_str()) {
            return candidate;
        }
    }
    // fallback: use nanoid-style random
    format!("n{}", chrono::Utc::now().timestamp_millis())
}

/// 重新分析指定节点的子树，生成更细粒度的子节点并替换原 children。
#[tauri::command]
pub async fn learn_regenerate_node(input: RegenerateNodeInput) -> Result<LearnGraph, String> {
    let project_path = input.project_path.trim().to_string();
    if project_path.is_empty() {
        return Err("项目路径不能为空".to_string());
    }

    // 1. 加载现有图
    let mut graph = super::store::load_graph(&project_path)
        .ok_or_else(|| "未找到该项目的分析记录".to_string())?;

    // 2. 找到目标节点并提取信息（clone 以避免后续可变借用冲突）
    let node_idx = graph.nodes.iter().position(|n| n.id == input.node_id)
        .ok_or_else(|| format!("未找到节点: {}", input.node_id))?;
    let node_name = graph.nodes[node_idx].name.clone();
    let node_kind = graph.nodes[node_idx].kind.clone();
    let node_description = graph.nodes[node_idx].description.clone();

    // 3. 扫描项目获取上下文
    let context = scan::scan_project(&project_path)?;

    // 4. 解析供应商/模型
    let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;

    // 5. 构建请求
    let url = completion_url(&provider.openai_url);
    let body = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": subtree_system_prompt(&node_name, &node_kind, &node_description) },
            { "role": "user", "content": format!("项目背景：\n{context}\n\n请只分析「{node_name}」这个模块的内部子结构。") }
        ],
        "stream": false,
        "temperature": 0.3,
    });

    eprintln!("[learn] 重新分析子树「{node_name}」(供应商: {}, 模型: {})", provider.name, model);

    // 6. 发送请求
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", provider.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(Duration::from_secs(180))
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    if !status.is_success() {
        let msg = value
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("未知错误");
        return Err(format!("AI 接口错误 ({}): {}", status.as_u16(), msg));
    }

    // 7. 提取文本
    let content = value
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|ch| ch.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if content.is_empty() {
        return Err("AI 返回空结果".to_string());
    }

    // 8. 解析新子节点
    let parsed = parse_ai_json(&content).map_err(|e| {
        eprintln!("[learn] 子树 JSON 解析失败 (节点: {}, 响应长度: {} 字符)", node_name, content.len());
        format!("AI 返回的 JSON 无法解析: {}", e)
    })?;

    let mut new_children = json_to_nodes(&parsed);

    if new_children.is_empty() {
        return Err("AI 未生成任何子节点".to_string());
    }

    // 9. 重映射 ID：AI 返回的根节点 ID 替换为 target.id，其余用新鲜 ID
    let existing_id_strings: Vec<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();

    let root_ai_id = new_children
        .iter()
        .find(|n| n.parent_id.is_none())
        .map(|n| n.id.clone())
        .unwrap_or_else(|| new_children[0].id.clone());

    // AI 返回的描述可能更新了 target 自身——保留原 name / kind，更新 description / detail
    let root_new = new_children.iter().find(|n| n.parent_id.is_none());
    if let Some(root) = root_new {
        if !root.description.is_empty() {
            graph.nodes[node_idx].description = root.description.clone();
        }
        if !root.detail.is_empty() {
            graph.nodes[node_idx].detail = root.detail.clone();
        }
    }

    // 收集后代 ID
    let target_id = graph.nodes[node_idx].id.clone();
    let mut descendant_ids: Vec<String> = Vec::new();
    fn collect_descendants(nodes: &[LearnNode], pid: &str, out: &mut Vec<String>) {
        for n in nodes {
            if n.parent_id.as_deref() == Some(pid) {
                out.push(n.id.clone());
                collect_descendants(nodes, &n.id, out);
            }
        }
    }
    let tid = target_id.clone();
    collect_descendants(&graph.nodes, &tid, &mut descendant_ids);
    let descendant_set: std::collections::HashSet<String> = descendant_ids.into_iter().collect();
    graph.nodes.retain(|n| !descendant_set.contains(&n.id));

    // 插入新子节点（跳过根节点自身）
    let tid2 = target_id.clone();
    for child in &mut new_children {
        if child.id == root_ai_id || child.parent_id.is_none() {
            continue;
        }
        let existing_set: std::collections::HashSet<&str> = existing_id_strings.iter().map(|s| s.as_str()).collect();
        let new_id = fresh_node_id(&existing_set);
        if child.parent_id.as_deref() == Some(&root_ai_id) {
            child.parent_id = Some(tid2.clone());
        }
        child.id = new_id;
        graph.nodes.push(child.clone());
    }

    // 10. 更新生成时间
    graph.generated_at = chrono::Utc::now().to_rfc3339();

    // 11. 持久化
    super::store::save_graph(&graph)?;

    Ok(graph)
}