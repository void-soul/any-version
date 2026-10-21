//! 分类层级整理：让 AI 检查「该做子类的分类被建成了平级」的问题。
//!
//! 能力边界（与用户诉求一致）：
//! - **可以新建缺失的分组**（例如平级出现 `Rust`/`Python`，但缺少 `编程语言` 这个父级）；
//! - **不删除、不改名、不改已有分类 ID**；条目关联全部挂在 `category_id` 上，
//!   已有 ID 不变，条目就不会成为孤儿；
//! - 只调整已有分类的 `parent_id`。
//!
//! 流程：读分类概览 → 喂给模型 → 解析「新建分组 + 移动建议」→ 本地校验
//! （成环 / 深度 / 同级重名）→ 前端预览勾选 → 只应用通过的改动。

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::db::{self, CategoryBrief, CategoryMove};

/// 整理后允许的最大层级（根级算第 1 级）。
pub const MAX_DEPTH: usize = 3;
/// 单次最多把多少个分类喂给模型（太多会挤爆上下文）。
const MAX_PROMPT_CATEGORIES: usize = 400;

pub const SYSTEM_PROMPT: &str = "你是收藏分类整理助手。用户会给出当前的分类列表，每行格式：\n\
`ID|名称|上级ID|直属条目数|完整路径|示例条目`\n\
请判断哪些分类放错了层级，并把它们移动到合适的父分类下。\n\
如果缺少一个合适的父分类，你可以新建分组（只建分组，不删除、不改名、不改已有 ID）。\n\
硬规则（必须全部遵守）：\n\
1. moves 里只能使用列表里已有的分类 ID；新建分组放在 groups 里；\n\
2. 禁止删除、重命名、合并已有分类；已有分类 ID 永远不变；\n\
3. groups 中的 parentId 只能是 null（根级）或列表里已有的分类 ID；新建分组最多一层，不要嵌套新建；\n\
4. 如果目标父级下已有同名分类，不要再新建同名分组，直接把条目/分类移到那个已有分类下；\n\
5. 不能形成环：移动后的分类不能落在它自己或它的子孙下面；整理后层级最多 3 级；\n\
6. 不能制造同级重名；已经处于合理层级的分类不要动；\n\
7. 只输出严格 JSON，不要任何解释文字：\n\
{\"groups\":[{\"key\":\"lang\",\"name\":\"编程语言\",\"parentId\":null}],\n\
 \"moves\":[{\"id\":2,\"parentId\":\"lang\",\"reason\":\"Rust 属于编程语言\"},{\"id\":3,\"parentId\":1,\"reason\":\"...\"}]}\n\
8. moves.parentId 可以是 null / 已有分类 ID / groups 里某个 key；没有需要调整时输出\n\
{\"groups\":[],\"moves\":[]}。";

/// 方案里的新建分组（`tempId` 为负数占位，真正落库时才分配真实 ID）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedGroup {
    pub temp_id: i64,
    pub name: String,
    /// null（根级）或已有分类 ID
    pub parent_id: Option<i64>,
    #[serde(default)]
    pub parent_name: Option<String>,
    #[serde(default)]
    pub depth: usize,
}

/// 一条通过校验的移动建议（前端预览与落库都用它）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedMove {
    pub id: i64,
    pub name: String,
    /// 当前完整路径（如 `编程语言/Rust`）
    pub path: String,
    pub from_parent_id: Option<i64>,
    pub from_parent_name: Option<String>,
    /// 可能为负数占位（指向本次新建的分组）
    pub to_parent_id: Option<i64>,
    pub to_parent_name: Option<String>,
    /// 移动后的层级（根级 = 1）
    pub depth: usize,
    pub reason: String,
}

/// 未通过校验的建议：如实返回原因，前端可以展示，不静默丢掉。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedMove {
    pub id: i64,
    pub name: String,
    pub problem: String,
}

/// 校验结果：通过的分组 / 移动，以及被拒绝的建议。
#[derive(Debug, Clone, Default)]
pub struct BuildOutcome {
    pub groups: Vec<PlannedGroup>,
    pub moves: Vec<PlannedMove>,
    pub skipped: Vec<SkippedMove>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryMovePlan {
    pub model: String,
    /// 本次送去判断的分类数
    pub considered: usize,
    pub groups: Vec<PlannedGroup>,
    pub moves: Vec<PlannedMove>,
    pub skipped: Vec<SkippedMove>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub applied: usize,
    pub created: usize,
    pub skipped: Vec<SkippedMove>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGroup {
    key: String,
    name: String,
    #[serde(default)]
    parent_id: Option<RawParent>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMove {
    id: i64,
    #[serde(default)]
    parent_id: Option<RawParent>,
    #[serde(default)]
    reason: String,
}

/// 模型输出里的父级引用：数字 = 已有分类 ID，字符串 = 新建分组的 key。
/// `#[serde(untagged)]` 会自动按 JSON 类型选择。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawParent {
    Id(i64),
    Key(String),
}

/// 把分类概览压成 prompt（关注分组不参与整理，已在调用方过滤）。
pub fn build_prompt(cats: &[CategoryBrief]) -> String {
    let mut selected: Vec<&CategoryBrief> = cats.iter().filter(|c| c.kind != "follow").collect();
    if selected.len() > MAX_PROMPT_CATEGORIES {
        // 分类特别多时优先保留条目多的，再按路径排回来，保证输出稳定
        selected.sort_by(|a, b| b.count.cmp(&a.count).then(a.id.cmp(&b.id)));
        selected.truncate(MAX_PROMPT_CATEGORIES);
        selected.sort_by(|a, b| a.path.cmp(&b.path));
    }

    let mut lines = Vec::with_capacity(selected.len() + 2);
    lines.push(format!(
        "共 {} 个分类，请整理层级（只输出 JSON）：",
        selected.len()
    ));
    lines.push("ID|名称|上级ID|直属条目数|完整路径|示例条目".to_string());
    for c in selected {
        let parent = c
            .parent_id
            .map(|p| p.to_string())
            .unwrap_or_else(|| "(根)".to_string());
        let samples = c
            .samples
            .iter()
            .take(3)
            .map(|s| clamp(s, 32))
            .collect::<Vec<_>>()
            .join("；");
        lines.push(format!(
            "{}|{}|{}|{}|{}|{}",
            c.id,
            clamp(&c.name, 30),
            parent,
            c.count,
            clamp(&c.path, 80),
            samples
        ));
    }
    lines.join("\n")
}

fn clamp(text: &str, max_chars: usize) -> &str {
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
    let json_text = super::classify::strip_fence(raw)
        .ok_or_else(|| "模型输出里没有找到 JSON".to_string())?;
    let value: serde_json::Value = serde_json::from_str(&json_text)
        .map_err(|e| format!("模型输出的 JSON 无法解析: {}", e))?;

    let raw_groups: Vec<RawGroup> = match value.get("groups") {
        None => Vec::new(),
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|e| format!("模型输出的 groups 无法解析: {}", e))?,
    };
    let raw_moves: Vec<RawMove> = match value.get("moves") {
        None => Vec::new(),
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|e| format!("模型输出的 moves 无法解析: {}", e))?,
    };
    if value.get("groups").is_none() && value.get("moves").is_none() {
        return Err("模型输出缺少 groups / moves 数组".to_string());
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

    let mut moves = Vec::with_capacity(raw_moves.len());
    for m in raw_moves {
        moves.push(CategoryMove {
            id: m.id,
            parent_id: resolve_parent(&m.parent_id, &key_to_temp)?,
            reason: m.reason.trim().to_string(),
        });
    }
    Ok((groups, moves))
}

fn resolve_parent(
    parent: &Option<RawParent>,
    key_to_temp: &HashMap<String, i64>,
) -> Result<Option<i64>, String> {
    match parent {
        None => Ok(None),
        Some(RawParent::Id(id)) => Ok(Some(*id)),
        Some(RawParent::Key(key)) => key_to_temp
            .get(key.trim())
            .copied()
            .map(Some)
            .ok_or_else(|| format!("parentId 引用了不存在的新建分组 key: {}", key)),
    }
}

/// 校验「新建分组 + 移动建议」。
///
/// 新建分组最多一层（只能挂到已有分类或根级），因此不会出现分组之间的成环；
/// 已有分类的移动仍会严格检查成环 / 深度 / 同级重名。
pub fn build_plan(
    cats: &[CategoryBrief],
    input_groups: &[PlannedGroup],
    input_moves: &[CategoryMove],
) -> BuildOutcome {
    let by_id: HashMap<i64, &CategoryBrief> = cats.iter().map(|c| (c.id, c)).collect();
    let current: HashMap<i64, Option<i64>> = cats.iter().map(|c| (c.id, c.parent_id)).collect();
    let mut skipped: Vec<SkippedMove> = Vec::new();

    // ── 新建分组：去重 + 深度校验 ──
    let mut accepted: Vec<PlannedGroup> = Vec::new();
    // 占位 ID → 真实已有 ID 或已接受分组的占位 ID
    let mut remap: HashMap<i64, i64> = HashMap::new();
    let mut seen_temps: HashSet<i64> = HashSet::new();

    for g in input_groups {
        if g.temp_id >= 0 || !seen_temps.insert(g.temp_id) {
            skipped.push(SkippedMove {
                id: g.temp_id,
                name: g.name.clone(),
                problem: "新建分组的临时 ID 无效或重复".to_string(),
            });
            continue;
        }
        let name = g.name.trim();
        if name.is_empty() {
            skipped.push(SkippedMove {
                id: g.temp_id,
                name: String::new(),
                problem: "新建分组名不能为空".to_string(),
            });
            continue;
        }
        let parent = match g.parent_id {
            None => None,
            Some(pid) if pid >= 0 => match by_id.get(&pid) {
                Some(p) if p.kind != "follow" => Some(pid),
                Some(_) => {
                    skipped.push(SkippedMove {
                        id: g.temp_id,
                        name: name.to_string(),
                        problem: "不能把新建分组挂到关注分组下".to_string(),
                    });
                    continue;
                }
                None => {
                    skipped.push(SkippedMove {
                        id: g.temp_id,
                        name: name.to_string(),
                        problem: format!("目标分类 {} 不存在", pid),
                    });
                    continue;
                }
            },
            Some(_) => {
                skipped.push(SkippedMove {
                    id: g.temp_id,
                    name: name.to_string(),
                    problem: "新建分组最多一层，只能挂到已有分类或根级".to_string(),
                });
                continue;
            }
        };

        // 目标父级下已有同名分类：直接复用，不再新建（避免制造重名）
        if let Some(existing) = by_id
            .values()
            .find(|c| c.parent_id == parent && c.name == name)
        {
            if existing.kind == "follow" {
                skipped.push(SkippedMove {
                    id: g.temp_id,
                    name: name.to_string(),
                    problem: "目标父级下已有同名关注分组".to_string(),
                });
                continue;
            }
            // 该分类本身也要被移走 → 不能复用它（否则移走后新分组仍会缺）
            if !input_moves.iter().any(|m| m.id == existing.id) {
                remap.insert(g.temp_id, existing.id);
                continue;
            }
        }
        // 和其他新建分组重名：复用先接受的那个
        if let Some(prev) = accepted
            .iter()
            .find(|a| a.parent_id == parent && a.name == name)
        {
            remap.insert(g.temp_id, prev.temp_id);
            continue;
        }

        let depth = match parent {
            None => 1,
            Some(pid) => depth_of(&current, pid).unwrap_or(1) + 1,
        };
        if depth > MAX_DEPTH {
            skipped.push(SkippedMove {
                id: g.temp_id,
                name: name.to_string(),
                problem: format!("新建后会超过 {} 级", MAX_DEPTH),
            });
            continue;
        }

        let mut accepted_group = g.clone();
        accepted_group.name = name.to_string();
        accepted_group.parent_id = parent;
        accepted_group.parent_name = parent
            .and_then(|pid| by_id.get(&pid))
            .map(|c| c.name.clone());
        accepted_group.depth = depth;
        accepted.push(accepted_group);
        remap.insert(g.temp_id, g.temp_id);
    }

    // 新建分组的父级（含祖先）不允许同时被移动：否则分组的深度会跟着变化
    let mut protected: HashSet<i64> = HashSet::new();
    for g in &accepted {
        let mut cur = g.parent_id;
        while let Some(pid) = cur {
            if !protected.insert(pid) {
                break;
            }
            cur = current.get(&pid).copied().flatten();
        }
    }

    // ── 移动建议：先解析目标，再做统一的树校验 ──
    let mut candidates: Vec<CategoryMove> = Vec::new();
    let mut seen_move_ids: HashSet<i64> = HashSet::new();
    for m in input_moves {
        if !seen_move_ids.insert(m.id) {
            skipped.push(SkippedMove {
                id: m.id,
                name: by_id.get(&m.id).map(|c| c.name.clone()).unwrap_or_default(),
                problem: "模型对同一个分类给出了多条移动建议，只保留第一条".to_string(),
            });
            continue;
        }
        let Some(cat) = by_id.get(&m.id) else {
            skipped.push(SkippedMove {
                id: m.id,
                name: String::new(),
                problem: "分类不存在（可能已被删除）".to_string(),
            });
            continue;
        };
        if cat.kind == "follow" {
            skipped.push(SkippedMove {
                id: m.id,
                name: cat.name.clone(),
                problem: "关注分组由人工维护，不参与自动整理".to_string(),
            });
            continue;
        }
        if protected.contains(&m.id) {
            skipped.push(SkippedMove {
                id: m.id,
                name: cat.name.clone(),
                problem: "新建分组的父级依赖它当前的层级，本次不移动它".to_string(),
            });
            continue;
        }

        let target = match m.parent_id {
            None => None,
            Some(pid) if pid < 0 => match remap.get(&pid) {
                Some(t) => Some(*t),
                None => {
                    skipped.push(SkippedMove {
                        id: m.id,
                        name: cat.name.clone(),
                        problem: "目标新建分组不可用".to_string(),
                    });
                    continue;
                }
            },
            Some(pid) => match by_id.get(&pid) {
                Some(p) if p.kind != "follow" => Some(pid),
                Some(_) => {
                    skipped.push(SkippedMove {
                        id: m.id,
                        name: cat.name.clone(),
                        problem: "不能把分类移到关注分组下面".to_string(),
                    });
                    continue;
                }
                None => {
                    skipped.push(SkippedMove {
                        id: m.id,
                        name: cat.name.clone(),
                        problem: format!("目标分类 {} 不存在", pid),
                    });
                    continue;
                }
            },
        };
        if target == cat.parent_id {
            continue;
        }
        if target == Some(m.id) {
            skipped.push(SkippedMove {
                id: m.id,
                name: cat.name.clone(),
                problem: "不能把分类移到自己下面".to_string(),
            });
            continue;
        }
        candidates.push(CategoryMove {
            id: m.id,
            parent_id: target,
            reason: m.reason.clone(),
        });
    }

    loop {
        // 每轮重建成树：accepted / candidates 都可能在上一轮被裁剪
        let group_names: HashMap<i64, String> = accepted
            .iter()
            .map(|g| (g.temp_id, g.name.clone()))
            .collect();
        let mut map = current.clone();
        for g in &accepted {
            map.insert(g.temp_id, g.parent_id);
        }
        for m in &candidates {
            map.insert(m.id, m.parent_id);
        }

        let mut invalid: HashMap<i64, String> = HashMap::new();
        for m in &candidates {
            match depth_of(&map, m.id) {
                None => {
                    invalid.insert(m.id, "会让分类层级成环".to_string());
                }
                Some(depth) if depth > MAX_DEPTH => {
                    invalid.insert(m.id, format!("整理后会超过 {} 级", MAX_DEPTH));
                }
                _ => {}
            }
        }
        if !invalid.is_empty() {
            record_skips(&mut skipped, &invalid, &by_id, &accepted);
            candidates.retain(|m| !invalid.contains_key(&m.id));
            if candidates.is_empty() && accepted.is_empty() {
                break;
            }
            continue;
        }

        // 分组本身不应超过层级（父级被移动的情况已被 protected 拦住，这里兜底）
        let mut drop_groups: HashMap<i64, String> = HashMap::new();
        for g in &accepted {
            if depth_of(&map, g.temp_id).map_or(true, |depth| depth > MAX_DEPTH) {
                drop_groups.insert(
                    g.temp_id,
                    format!("整理后会超过 {} 级，已取消该新建分组", MAX_DEPTH),
                );
            }
        }

        // 同级重名：优先保留已有/正在移动的条目分类，取消多余的新建分组
        if drop_groups.is_empty() {
            let mut children: HashMap<Option<i64>, HashMap<String, Vec<i64>>> = HashMap::new();
            for (cid, pid) in &map {
                let name = if *cid < 0 {
                    group_names.get(cid).cloned().unwrap_or_default()
                } else {
                    by_id.get(cid).map(|c| c.name.clone()).unwrap_or_default()
                };
                children.entry(*pid).or_default().entry(name).or_default().push(*cid);
            }
            for groups in children.values() {
                for ids in groups.values().filter(|ids| ids.len() > 1) {
                    for cid in ids {
                        if *cid < 0 {
                            drop_groups.insert(
                                *cid,
                                "与已有分类同名，已取消该新建分组".to_string(),
                            );
                        } else if candidates.iter().any(|m| m.id == *cid) {
                            invalid.insert(*cid, "目标父级下已有同名分类".to_string());
                        }
                    }
                }
            }
            for (id, problem) in &invalid {
                let name = by_id.get(id).map(|c| c.name.clone()).unwrap_or_default();
                skipped.push(SkippedMove {
                    id: *id,
                    name,
                    problem: problem.clone(),
                });
            }
            candidates.retain(|m| !invalid.contains_key(&m.id));
            if !invalid.is_empty() {
                if candidates.is_empty() && accepted.is_empty() {
                    break;
                }
                continue;
            }
        }

        if drop_groups.is_empty() {
            break;
        }
        for (temp, problem) in &drop_groups {
            skipped.push(SkippedMove {
                id: *temp,
                name: accepted
                    .iter()
                    .find(|g| g.temp_id == *temp)
                    .map(|g| g.name.clone())
                    .unwrap_or_default(),
                problem: problem.clone(),
            });
        }
        accepted.retain(|g| !drop_groups.contains_key(&g.temp_id));
        candidates.retain(|m| !matches!(m.parent_id, Some(pid) if drop_groups.contains_key(&pid)));
        if candidates.is_empty() && accepted.is_empty() {
            break;
        }
    }

    // 只保留被「通过的移动」引用到的新建分组
    let referenced: HashSet<i64> = candidates
        .iter()
        .filter_map(|m| m.parent_id)
        .filter(|pid| *pid < 0)
        .collect();
    let out_groups: Vec<PlannedGroup> = accepted
        .into_iter()
        .filter(|g| referenced.contains(&g.temp_id))
        .collect();

    let mut map = current.clone();
    for g in &out_groups {
        map.insert(g.temp_id, g.parent_id);
    }
    for m in &candidates {
        map.insert(m.id, m.parent_id);
    }
    let group_name = |id: i64| -> Option<String> {
        out_groups
            .iter()
            .find(|g| g.temp_id == id)
            .map(|g| g.name.clone())
    };
    let mut moves: Vec<PlannedMove> = candidates
        .iter()
        .map(|m| {
            let cat = by_id.get(&m.id).expect("候选 id 已校验存在");
            let to_parent_name = match m.parent_id {
                None => None,
                Some(pid) if pid < 0 => group_name(pid),
                Some(pid) => by_id.get(&pid).map(|c| c.name.clone()),
            };
            PlannedMove {
                id: m.id,
                name: cat.name.clone(),
                path: cat.path.clone(),
                from_parent_id: cat.parent_id,
                from_parent_name: cat
                    .parent_id
                    .and_then(|pid| by_id.get(&pid))
                    .map(|c| c.name.clone()),
                to_parent_id: m.parent_id,
                to_parent_name,
                depth: depth_of(&map, m.id).unwrap_or(0),
                reason: m.reason.trim().to_string(),
            }
        })
        .collect();
    moves.sort_by(|a, b| a.path.cmp(&b.path));
    BuildOutcome {
        groups: out_groups,
        moves,
        skipped,
    }
}

fn record_skips(
    skipped: &mut Vec<SkippedMove>,
    invalid: &HashMap<i64, String>,
    by_id: &HashMap<i64, &CategoryBrief>,
    accepted: &[PlannedGroup],
) {
    for (id, problem) in invalid {
        let name = by_id
            .get(id)
            .map(|c| c.name.clone())
            .or_else(|| accepted.iter().find(|g| g.temp_id == *id).map(|g| g.name.clone()))
            .unwrap_or_default();
        skipped.push(SkippedMove {
            id: *id,
            name,
            problem: problem.clone(),
        });
    }
}

/// 节点在最终树里的层级；成环或数据异常时返回 None。
fn depth_of(map: &HashMap<i64, Option<i64>>, id: i64) -> Option<usize> {
    let mut seen = HashSet::new();
    let mut cur = Some(id);
    let mut depth = 0usize;
    while let Some(node) = cur {
        if !seen.insert(node) {
            return None;
        }
        depth += 1;
        if depth > map.len() + 1 {
            return None;
        }
        cur = map.get(&node).copied().flatten();
    }
    Some(depth)
}

/// 只改上级与同级排序号；分类 ID / 名称 / 条目关联都不动。
fn set_parent(
    conn: &rusqlite::Connection,
    id: i64,
    parent: Option<i64>,
) -> Result<(), String> {
    let next: i32 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM favorite_category WHERE parent_id IS ?1",
            rusqlite::params![parent],
            |r| r.get(0),
        )
        .map_err(|e| format!("读取排序号失败: {e}"))?;
    conn.execute(
        "UPDATE favorite_category SET parent_id = ?1, sort_order = ?2 WHERE id = ?3",
        rusqlite::params![parent, next, id],
    )
    .map_err(|e| format!("调整分类层级失败: {e}"))?;
    Ok(())
}

/// 应用整理方案：先按需创建新分组，再**只更新**已有分类的 `parent_id` / `sort_order`。
///
/// 应用前重新校验一次（前端传回的可能是用户勾选的子集），不合法的建议跳过并如实返回。
pub fn apply(
    conn: &rusqlite::Connection,
    groups: &[PlannedGroup],
    moves: &[CategoryMove],
) -> Result<ApplyResult, String> {
    let cats = db::category_briefs(conn)?;
    let plan = build_plan(&cats, groups, moves);
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("开启事务失败: {e}"))?;

    let mut applied = 0usize;
    let mut created = 0usize;
    let mut skipped = plan.skipped;
    let mut done_moves: HashSet<i64> = HashSet::new();

    // 先落「移向已有分类 / 根级」的移动：它们可能把某个同名分类挪走，
    // 从而腾出名字给待新建的分组（建分组时有同级重名校验）。
    for m in &plan.moves {
        if matches!(m.to_parent_id, Some(pid) if pid < 0) {
            continue;
        }
        set_parent(&tx, m.id, m.to_parent_id)?;
        applied += 1;
        done_moves.insert(m.id);
    }

    let mut temp_to_real: HashMap<i64, i64> = HashMap::new();
    for g in &plan.groups {
        // 并发/数据变化兜底：同名分类已经存在就复用，绝不重复建
        let real = match db::find_child_id(&tx, g.parent_id, &g.name)? {
            Some(id) => id,
            None => match db::create_category(&tx, &g.name, g.parent_id, "topic") {
                Ok(id) => {
                    created += 1;
                    id
                }
                Err(err) if err.contains("同名") => {
                    // 同名分类仍占着位置（它自己也要移到别的新建分组下）：
                    // 宁可跳过这条新建，也不去改名或删除任何已有分类
                    skipped.push(SkippedMove {
                        id: g.temp_id,
                        name: g.name.clone(),
                        problem: err,
                    });
                    continue;
                }
                Err(err) => return Err(err),
            },
        };
        temp_to_real.insert(g.temp_id, real);
    }

    // 再落「移向新建分组」的移动
    for m in &plan.moves {
        if done_moves.contains(&m.id) {
            continue;
        }
        let parent = match m.to_parent_id {
            Some(pid) if pid < 0 => match temp_to_real.get(&pid) {
                Some(real) => Some(*real),
                None => {
                    skipped.push(SkippedMove {
                        id: m.id,
                        name: m.name.clone(),
                        problem: "目标新建分组未创建成功".to_string(),
                    });
                    continue;
                }
            },
            other => other,
        };
        set_parent(&tx, m.id, parent)?;
        applied += 1;
    }
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(ApplyResult {
        applied,
        created,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        apply, build_plan, build_prompt, parse_plan, PlannedGroup, MAX_DEPTH,
    };
    use crate::commands::favorites::db::{self, CategoryBrief, CategoryMove};

    fn cat(id: i64, parent_id: Option<i64>, name: &str, kind: &str) -> CategoryBrief {
        CategoryBrief {
            id,
            parent_id,
            name: name.to_string(),
            kind: kind.to_string(),
            count: 0,
            path: name.to_string(),
            samples: vec![],
        }
    }

    fn mv(id: i64, parent_id: Option<i64>) -> CategoryMove {
        CategoryMove {
            id,
            parent_id,
            reason: "测试".to_string(),
        }
    }

    fn group(temp_id: i64, name: &str, parent_id: Option<i64>) -> PlannedGroup {
        PlannedGroup {
            temp_id,
            name: name.to_string(),
            parent_id,
            parent_name: None,
            depth: 0,
        }
    }

    #[test]
    fn prompt_lists_id_path_and_samples() {
        let mut rust = cat(2, None, "Rust", "topic");
        rust.path = "Rust".to_string();
        rust.count = 12;
        rust.samples = vec!["tokio".into(), "serde".into()];
        let prompt = build_prompt(&[cat(1, None, "编程语言", "topic"), rust]);
        assert!(prompt.contains("ID|名称|上级ID|直属条目数|完整路径|示例条目"));
        assert!(prompt.contains("2|Rust|(根)|12|Rust|tokio；serde"));
    }

    #[test]
    fn prompt_skips_follow_groups() {
        let prompt = build_prompt(&[cat(1, None, "主题", "topic"), cat(2, None, "关注", "follow")]);
        assert!(prompt.contains("1|主题"));
        assert!(!prompt.contains("2|关注"));
    }

    #[test]
    fn parses_groups_and_moves_with_key_references() {
        let raw = r#"{"groups":[{"key":"lang","name":"编程语言","parentId":null}],
                      "moves":[{"id":2,"parentId":"lang","reason":"Rust 属于编程语言"}]}"#;
        let (groups, moves) = parse_plan(raw).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].temp_id, -1);
        assert_eq!(groups[0].name, "编程语言");
        assert_eq!(groups[0].parent_id, None);
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].id, 2);
        assert_eq!(moves[0].parent_id, Some(-1));
    }

    #[test]
    fn parses_plain_and_fenced_with_empty_arrays() {
        let (groups, moves) = parse_plan(r#"{"groups":[],"moves":[]}"#).unwrap();
        assert!(groups.is_empty() && moves.is_empty());

        let (_, moves) = parse_plan("```json\n{\"groups\":[],\"moves\":[{\"id\":3,\"parentId\":null}]}\n```")
            .unwrap();
        assert_eq!(moves[0].parent_id, None);
    }

    #[test]
    fn parse_rejects_garbage_and_unknown_keys() {
        assert!(parse_plan("抱歉").is_err());
        assert!(parse_plan(r#"{"foo":1}"#).is_err());
        assert!(parse_plan(r#"{"groups":[],"moves":[{"id":2,"parentId":"nope"}]}"#).is_err());
        assert!(parse_plan(r#"{"groups":[{"key":"a","name":"A"},{"key":"a","name":"B"}],"moves":[]}"#).is_err());
    }

    #[test]
    fn accepts_moving_flat_category_under_existing_parent() {
        let cats = vec![cat(1, None, "编程语言", "topic"), cat(2, None, "Rust", "topic")];
        let out = build_plan(&cats, &[], &[mv(2, Some(1))]);
        assert_eq!(out.moves.len(), 1);
        assert_eq!(out.moves[0].to_parent_id, Some(1));
        assert_eq!(out.moves[0].to_parent_name.as_deref(), Some("编程语言"));
        assert_eq!(out.moves[0].depth, 2);
        assert!(out.groups.is_empty());
    }

    /// 缺少父级时，AI 可以新建一个分组，再把平级分类移进去。
    #[test]
    fn creates_missing_parent_group_and_moves_into_it() {
        let cats = vec![cat(2, None, "Rust", "topic"), cat(3, None, "Python", "topic")];
        let groups = vec![group(-1, "编程语言", None)];
        let out = build_plan(&cats, &groups, &[mv(2, Some(-1)), mv(3, Some(-1))]);
        assert_eq!(out.groups.len(), 1);
        assert_eq!(out.groups[0].name, "编程语言");
        assert_eq!(out.groups[0].depth, 1);
        assert_eq!(out.moves.len(), 2);
        assert!(out.moves.iter().all(|m| m.to_parent_id == Some(-1)));
        assert!(out.moves.iter().all(|m| m.to_parent_name.as_deref() == Some("编程语言")));
    }

    /// 目标父级下已有同名分类：复用已有分类，不重复新建。
    #[test]
    fn reuses_existing_same_name_group_instead_of_creating() {
        let cats = vec![cat(1, None, "编程语言", "topic"), cat(2, None, "Rust", "topic")];
        let groups = vec![group(-1, "编程语言", None)];
        let out = build_plan(&cats, &groups, &[mv(2, Some(-1))]);
        assert!(out.groups.is_empty(), "同名分类已存在，应复用而不是新建");
        assert_eq!(out.moves[0].to_parent_id, Some(1));
    }

    /// AI 可以在已有分类下补建子分组。
    #[test]
    fn creates_group_under_existing_parent() {
        let cats = vec![cat(1, None, "技术", "topic"), cat(2, None, "Rust", "topic")];
        let groups = vec![group(-1, "编程语言", Some(1))];
        let out = build_plan(&cats, &groups, &[mv(2, Some(-1))]);
        assert_eq!(out.groups.len(), 1);
        assert_eq!(out.groups[0].parent_id, Some(1));
        assert_eq!(out.groups[0].depth, 2);
        assert_eq!(out.moves[0].depth, 3);
    }

    /// 一个建议被拒（例如新建分组的父级本身要移动）不应连累同组的其他有效建议。
    #[test]
    fn keeps_group_when_some_moves_are_rejected() {
        let cats = vec![cat(1, None, "Rust", "topic"), cat(2, None, "Go", "topic")];
        // 新分组挂在 Rust 下面，同时又想把 Rust 移到新分组里 → 成环，该移动被拒
        let groups = vec![group(-1, "编程语言", Some(1))];
        let out = build_plan(&cats, &groups, &[mv(1, Some(-1)), mv(2, Some(-1))]);
        assert!(out.skipped.iter().any(|s| s.id == 1));
        assert_eq!(out.groups.len(), 1, "另一个有效移动仍需要这个分组");
        assert_eq!(out.moves.len(), 1);
        assert_eq!(out.moves[0].id, 2);
    }

    /// 同名分类正要被移走时，不应把新分组复用到它身上（否则移走后新分组就没了）。
    #[test]
    fn creates_group_when_same_name_category_moves_away() {
        let cats = vec![
            cat(1, None, "技术", "topic"),
            cat(2, None, "Rust", "topic"),
            cat(3, None, "Go", "topic"),
        ];
        let groups = vec![group(-1, "Rust", None)];
        let out = build_plan(&cats, &groups, &[mv(2, Some(1)), mv(3, Some(-1))]);
        assert_eq!(out.groups.len(), 1);
        assert_eq!(out.groups[0].name, "Rust");
        assert!(out.moves.iter().any(|m| m.id == 3 && m.to_parent_id == Some(-1)));
    }

    #[test]
    fn rejects_cycle() {
        let cats = vec![cat(1, None, "A", "topic"), cat(2, Some(1), "B", "topic")];
        let out = build_plan(&cats, &[], &[mv(1, Some(2))]);
        assert!(out.moves.is_empty());
        assert!(out.skipped.iter().any(|s| s.problem.contains("成环")));
    }

    #[test]
    fn rejects_duplicate_sibling_name() {
        let cats = vec![
            cat(1, None, "技术", "topic"),
            cat(2, Some(1), "Rust", "topic"),
            cat(3, None, "Rust", "topic"),
        ];
        let out = build_plan(&cats, &[], &[mv(3, Some(1))]);
        assert!(out.moves.is_empty(), "不能把第二个 Rust 塞进已有同名子级");
        assert!(out.skipped.iter().any(|s| s.problem.contains("同名")));
    }

    #[test]
    fn rejects_too_deep() {
        let cats = vec![
            cat(1, None, "A", "topic"),
            cat(2, Some(1), "B", "topic"),
            cat(3, Some(2), "C", "topic"),
            cat(4, None, "D", "topic"),
        ];
        let out = build_plan(&cats, &[], &[mv(4, Some(3))]);
        assert!(out.moves.is_empty());
        assert!(out.skipped.iter().any(|s| s.problem.contains("超过")));
        assert_eq!(MAX_DEPTH, 3);
    }

    #[test]
    fn rejects_new_group_too_deep() {
        let cats = vec![
            cat(1, None, "A", "topic"),
            cat(2, Some(1), "B", "topic"),
            cat(3, Some(2), "C", "topic"),
        ];
        let groups = vec![group(-1, "D", Some(3))];
        let out = build_plan(&cats, &groups, &[mv(3, Some(-1))]);
        assert!(out.groups.is_empty());
        assert!(out.moves.is_empty());
        assert!(out.skipped.iter().any(|s| s.problem.contains("超过")));
    }

    #[test]
    fn skips_follow_groups_and_unknown_ids() {
        let cats = vec![cat(1, None, "关注", "follow"), cat(2, None, "Rust", "topic")];
        let out = build_plan(&cats, &[], &[mv(2, Some(1)), mv(99, Some(2))]);
        assert!(out.moves.is_empty());
        assert_eq!(out.skipped.len(), 2);
    }

    #[test]
    fn ignores_noop_moves() {
        let cats = vec![cat(1, None, "A", "topic"), cat(2, Some(1), "B", "topic")];
        let out = build_plan(&cats, &[], &[mv(2, Some(1))]);
        assert!(out.moves.is_empty());
        assert!(out.skipped.is_empty());
    }

    #[test]
    fn ignores_unreferenced_groups() {
        let cats = vec![cat(1, None, "Rust", "topic")];
        let groups = vec![group(-1, "编程语言", None), group(-2, "多余分组", None)];
        let out = build_plan(&cats, &groups, &[mv(1, Some(-1))]);
        assert_eq!(out.groups.len(), 1, "没有被移动引用的新建分组不应创建");
        assert_eq!(out.groups[0].temp_id, -1);
    }

    /// 新建分组后落库：新分组拿到真实 ID；已有分类 ID 不变，条目关联一条不少。
    #[test]
    fn apply_creates_group_and_keeps_existing_ids_and_links() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO favorite (id, source, external_id, url, title, created_at, updated_at) \
             VALUES (1, 'github', '1', 'https://x', 'o/r', 'now', 'now')",
            [],
        )
        .unwrap();
        let rust = db::create_category(&conn, "Rust", None, "topic").unwrap();
        db::set_item_categories(&conn, 1, &[rust]).unwrap();

        let groups = vec![group(-1, "编程语言", None)];
        let result = apply(&conn, &groups, &[mv(rust, Some(-1))]).unwrap();
        assert_eq!(result.applied, 1);
        assert_eq!(result.created, 1);

        let (group_id, name): (i64, String) = conn
            .query_row(
                "SELECT id, name FROM favorite_category WHERE name = '编程语言'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(name, "编程语言");
        let (id, rust_parent): (i64, Option<i64>) = conn
            .query_row(
                "SELECT id, parent_id FROM favorite_category WHERE name = 'Rust'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(id, rust, "已有分类 ID 不能被改掉");
        assert_eq!(rust_parent, Some(group_id));

        let (favorite_id, category_id): (i64, i64) = conn
            .query_row(
                "SELECT favorite_id, category_id FROM favorite_item_category",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(favorite_id, 1);
        assert_eq!(category_id, rust, "条目关联仍指向原分类 ID");
    }

    /// 同名分类先移走，再新建同名分组：应用顺序要能腾出名字，不能报唯一冲突。
    #[test]
    fn apply_moves_away_same_name_before_creating_group() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::migrate(&conn).unwrap();
        let tech = db::create_category(&conn, "技术", None, "topic").unwrap();
        let rust = db::create_category(&conn, "Rust", None, "topic").unwrap();
        let go = db::create_category(&conn, "Go", None, "topic").unwrap();

        let groups = vec![group(-1, "Rust", None)];
        let result = apply(&conn, &groups, &[mv(rust, Some(tech)), mv(go, Some(-1))]).unwrap();
        assert_eq!(result.applied, 2);
        assert_eq!(result.created, 1, "旧 Rust 移走后，新建同名分组应成功");

        let new_rust: i64 = conn
            .query_row(
                "SELECT id FROM favorite_category WHERE name = 'Rust' AND id != ?1",
                [rust],
                |r| r.get(0),
            )
            .unwrap();
        let go_parent: Option<i64> = conn
            .query_row(
                "SELECT parent_id FROM favorite_category WHERE id = ?1",
                [go],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(go_parent, Some(new_rust), "Go 应移入新建的 Rust 分组");
        assert_eq!(result.skipped.len(), 0);
    }

    /// 前端预览用的结构必须是 camelCase（toParentId / tempId 等）。
    #[test]
    fn plan_serializes_as_camel_case() {
        let cats = vec![cat(2, None, "Rust", "topic")];
        let groups = vec![group(-1, "编程语言", None)];
        let out = build_plan(&cats, &groups, &[mv(2, Some(-1))]);
        let plan = super::CategoryMovePlan {
            model: "m".to_string(),
            considered: 1,
            groups: out.groups,
            moves: out.moves,
            skipped: out.skipped,
        };
        let value = serde_json::to_value(&plan).unwrap();
        assert_eq!(value["groups"][0]["tempId"], -1);
        assert_eq!(value["moves"][0]["toParentId"], -1);
        assert!(value["moves"][0].get("to_parent_id").is_none());
    }
}
