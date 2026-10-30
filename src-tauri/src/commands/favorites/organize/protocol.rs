use super::*;

/// 把分类概览压成 prompt（关注分组不参与整理，已在调用方过滤）。
///
/// 每行只有 `ID|名称|上级ID`：整理只改父子关系，条目数 / 路径 / 示例都不发，
/// 分类再多也不会把上下文撑爆（路径由前端预览用，不参与模型判断）。
pub fn build_prompt(cats: &[CategoryBrief]) -> String {
    let mut selected: Vec<&CategoryBrief> = cats.iter().filter(|c| c.kind != "follow").collect();
    if selected.len() > MAX_PROMPT_CATEGORIES {
        selected.sort_by(|a, b| a.path.cmp(&b.path));
        selected.truncate(MAX_PROMPT_CATEGORIES);
    }

    let mut lines = Vec::with_capacity(selected.len() + 2);
    lines.push(format!(
        "共 {} 个分类，请做整体分层整理（上级ID 为 (根) 表示根级；只输出 JSON）：",
        selected.len()
    ));
    lines.push("ID|名称|上级ID".to_string());
    for c in selected {
        let parent = c
            .parent_id
            .map(|p| p.to_string())
            .unwrap_or_else(|| "(根)".to_string());
        lines.push(format!("{}|{}|{}", c.id, clamp(&c.name, 30), parent));
    }
    lines.join("\n")
}

pub(super) fn clamp(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => &text[..end],
        None => text,
    }
}

/// 解析模型输出 → (新建分组, 移动建议)。
///
/// 分组 key 会按出现顺序映射成负数占位 ID（-1、-2…），这样 move 的 parentId
/// 可以直接引用 key；真正落库时才由 [`apply`] 分配真实 ID。
pub fn parse_plan(raw: &str) -> Result<(Vec<PlannedGroup>, Vec<CategoryMove>), String> {
    let json_text = crate::commands::favorites::classify::strip_fence(raw)
        .ok_or_else(|| "模型输出里没有找到 JSON".to_string())?;
    let value: serde_json::Value =
        serde_json::from_str(&json_text).map_err(|e| format!("模型输出的 JSON 无法解析: {}", e))?;

    let raw_groups: Vec<RawGroup> = match value.get("groups") {
        None => Vec::new(),
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|e| format!("模型输出的 groups 无法解析: {}", e))?,
    };
    let raw_placements: Vec<RawPlacement> = match value.get("placements") {
        None => Vec::new(),
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|e| format!("模型输出的 placements 无法解析: {}", e))?,
    };
    if value.get("groups").is_none() && value.get("placements").is_none() {
        return Err("模型输出缺少 groups / placements 数组".to_string());
    }
    // 旧版逐条 moves 格式已废弃：静默忽略会得到「无需整理」的假结果，宁可明确报错
    if value.get("placements").is_none() && value.get("moves").is_some() {
        return Err("模型输出使用了旧版 moves 格式，请重新生成".to_string());
    }

    // key → 负数占位 ID
    let mut key_to_temp: HashMap<String, i64> = HashMap::new();
    let mut groups: Vec<PlannedGroup> = Vec::with_capacity(raw_groups.len());
    for (index, g) in raw_groups.iter().enumerate() {
        let key = g.key.trim();
        if key.is_empty() {
            return Err("新建分组缺少 key".to_string());
        }
        if key_to_temp.contains_key(key) {
            return Err(format!("新建分组的 key 重复: {}", key));
        }
        let name = g.name.trim();
        if name.is_empty() {
            return Err("新建分组缺少 name".to_string());
        }
        let temp_id = -(index as i64 + 1);
        key_to_temp.insert(key.to_string(), temp_id);
        groups.push(PlannedGroup {
            temp_id,
            name: name.to_string(),
            parent_id: None,
            parent_name: None,
            depth: 0,
        });
    }
    // 第二遍解析 parentId（此时所有 key 已登记）
    for (group, raw_group) in groups.iter_mut().zip(raw_groups.iter()) {
        group.parent_id = resolve_parent(&raw_group.parent_id, &key_to_temp)?;
    }

    // 同一目标父级下的一批 ID → 多条 CategoryMove（reason 留空，预览里不展示）
    let total: usize = raw_placements.iter().map(|p| p.ids.len()).sum();
    let mut moves = Vec::with_capacity(total);
    for p in raw_placements {
        let parent_id = resolve_parent(&p.parent_id, &key_to_temp)?;
        for id in p.ids {
            moves.push(CategoryMove {
                id,
                parent_id,
                reason: String::new(),
            });
        }
    }
    Ok((groups, moves))
}
