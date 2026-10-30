//! 分类层级整体整理：让 AI 通盘设计分类树，而不是只修明显放错的分类。
//!
//! 能力边界（与用户诉求一致）：
//! - **可以新建缺失的分组**（例如平级出现 `Rust`/`Python`，但缺少 `编程语言` 这个父级）；
//! - **不删除、不改名、不改已有分类 ID**；条目关联全部挂在 `category_id` 上，
//!   已有 ID 不变，条目就不会成为孤儿；
//! - 调整已有分类的 `parent_id`：只要目标层级更合理，即使当前层级「能用」也可以移动。
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

pub const SYSTEM_PROMPT: &str = "你是收藏分类整理助手。用户会给出当前全部分类，每行格式：\n\
`ID|名称|上级ID`，其中上级ID 为 (根) 表示根级。\n\
请做**整体分层整理**：先通盘设计一套清晰的层级（根级主题大类 + 必要的子类），\n\
再为每个分类判断最合适的父级，并输出需要改变的父子关系。\n\
不要只挑明显放错的分类：当前层级虽然能用、但换到更合适的父级下会让整体结构更清晰的，也要移动。\n\
如果缺少合适的父分类/主题分组，可以新建分组（只建分组，不删除、不改名、不改已有 ID）。\n\
输出要精简：按「目标父级」分组列出分类 ID，不要逐条写理由或解释。\n\
硬规则（必须全部遵守）：\n\
1. placements 里的 ids 只能使用列表里已有的分类 ID；每个 ID 最多出现一次；不要输出 reason；\n\
2. 禁止删除、重命名、合并已有分类；已有分类 ID 永远不变；\n\
3. groups 中的 parentId 只能是 null（根级）或列表里已有的分类 ID；新建分组最多一层，不要嵌套新建；\n\
4. 如果目标父级下已有同名分类，不要再新建同名分组，直接把分类移到那个已有分类下；\n\
5. 不能形成环：移动后的分类不能落在它自己或它的子孙下面；整理后层级最多 3 级；\n\
6. 不能制造同级重名；\n\
7. 从整体结构出发：根级尽量收敛为少量并列主题，避免大量分类平铺在根级；同一父级下按主题归类；\n\
   已经在最合适父级下的分类不要列出，避免无意义的层级变动；\n\
8. 只输出严格 JSON，不要任何解释文字；placements.parentId 可以是 null / 已有分类 ID / groups 里某个 key：\n\
{\"groups\":[{\"key\":\"lang\",\"name\":\"编程语言\",\"parentId\":null}],\n\
 \"placements\":[{\"parentId\":\"lang\",\"ids\":[2,3]},{\"parentId\":1,\"ids\":[4,5]},{\"parentId\":null,\"ids\":[6]}]}\n\
9. 没有需要调整时输出 {\"groups\":[],\"placements\":[]}。";

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

/// 一组「要移到同一个父级下」的分类 ID。
/// 相比逐条 `{id,parentId,reason}`，按目标父级聚合能把输出体积压到原来的 1/3 左右，
/// 400 个分类时不会因为输出过长而超时。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPlacement {
    #[serde(default)]
    parent_id: Option<RawParent>,
    #[serde(default)]
    ids: Vec<i64>,
}

/// 模型输出里的父级引用：数字 = 已有分类 ID，字符串 = 新建分组的 key。
/// `#[serde(untagged)]` 会自动按 JSON 类型选择。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawParent {
    Id(i64),
    Key(String),
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
                children
                    .entry(*pid)
                    .or_default()
                    .entry(name)
                    .or_default()
                    .push(*cid);
            }
            for groups in children.values() {
                for ids in groups.values().filter(|ids| ids.len() > 1) {
                    for cid in ids {
                        if *cid < 0 {
                            drop_groups
                                .insert(*cid, "与已有分类同名，已取消该新建分组".to_string());
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
            .or_else(|| {
                accepted
                    .iter()
                    .find(|g| g.temp_id == *id)
                    .map(|g| g.name.clone())
            })
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
fn set_parent(conn: &rusqlite::Connection, id: i64, parent: Option<i64>) -> Result<(), String> {
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

mod protocol;
pub use protocol::*;
#[cfg(test)]
mod tests;
