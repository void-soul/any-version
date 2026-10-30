use super::{apply, build_plan, build_prompt, parse_plan, PlannedGroup, MAX_DEPTH, SYSTEM_PROMPT};
use crate::commands::favorites::db::{self, CategoryBrief, CategoryMove};

fn cat(id: i64, parent_id: Option<i64>, name: &str, kind: &str) -> CategoryBrief {
    CategoryBrief {
        id,
        parent_id,
        name: name.to_string(),
        kind: kind.to_string(),
        path: name.to_string(),
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

/// prompt 只带 ID / 名称 / 上级 ID：上下文要小，路径与示例不发给模型。
#[test]
fn prompt_lists_only_id_name_and_parent() {
    let prompt = build_prompt(&[
        cat(1, None, "编程语言", "topic"),
        cat(2, Some(1), "Rust", "topic"),
    ]);
    assert!(prompt.contains("ID|名称|上级ID"));
    assert!(prompt.contains("1|编程语言|(根)"));
    assert!(prompt.contains("2|Rust|1"));
    assert!(prompt.contains("整体分层整理"));
    assert!(!prompt.contains("示例"));
    assert!(!prompt.contains("条目数"));
}

/// 整理要求覆盖整棵树：不能只让模型挑「明显放错」的分类。
#[test]
fn system_prompt_requires_whole_tree_organization() {
    assert!(SYSTEM_PROMPT.contains("整体分层整理"));
    assert!(SYSTEM_PROMPT.contains("不要只挑明显放错的分类"));
    assert!(SYSTEM_PROMPT.contains("根级尽量收敛"));
    assert!(!SYSTEM_PROMPT.contains("只判断哪些分类放错了层级"));
}

#[test]
fn prompt_skips_follow_groups() {
    let prompt = build_prompt(&[
        cat(1, None, "主题", "topic"),
        cat(2, None, "关注", "follow"),
    ]);
    assert!(prompt.contains("1|主题"));
    assert!(!prompt.contains("2|关注"));
}

#[test]
fn parses_grouped_placements_with_key_references() {
    let raw = r#"{"groups":[{"key":"lang","name":"编程语言","parentId":null}],
                      "placements":[{"parentId":"lang","ids":[2,3]}]}"#;
    let (groups, moves) = parse_plan(raw).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].temp_id, -1);
    assert_eq!(groups[0].name, "编程语言");
    assert_eq!(groups[0].parent_id, None);
    // 一组 placement 展开成多条移动，reason 由本地留空
    assert_eq!(moves.len(), 2);
    assert_eq!(moves[0].id, 2);
    assert_eq!(moves[0].parent_id, Some(-1));
    assert_eq!(moves[1].id, 3);
    assert_eq!(moves[1].parent_id, Some(-1));
    assert!(moves[0].reason.is_empty());
}

#[test]
fn parses_plain_and_fenced_with_empty_arrays() {
    let (groups, moves) = parse_plan(r#"{"groups":[],"placements":[]}"#).unwrap();
    assert!(groups.is_empty() && moves.is_empty());

    let (_, moves) = parse_plan(
        "```json\n{\"groups\":[],\"placements\":[{\"parentId\":null,\"ids\":[3]}]}\n```",
    )
    .unwrap();
    assert_eq!(moves[0].parent_id, None);
}

#[test]
fn parse_rejects_garbage_and_unknown_keys() {
    assert!(parse_plan("抱歉").is_err());
    assert!(parse_plan(r#"{"foo":1}"#).is_err());
    // 旧版逐条 moves 格式已不再接受，避免提示词与解析器格式不一致
    assert!(parse_plan(r#"{"groups":[],"moves":[{"id":2,"parentId":1}]}"#).is_err());
    assert!(parse_plan(r#"{"groups":[],"placements":[{"parentId":"nope","ids":[2]}]}"#).is_err());
    assert!(parse_plan(
        r#"{"groups":[{"key":"a","name":"A"},{"key":"a","name":"B"}],"placements":[]}"#
    )
    .is_err());
}

#[test]
fn accepts_moving_flat_category_under_existing_parent() {
    let cats = vec![
        cat(1, None, "编程语言", "topic"),
        cat(2, None, "Rust", "topic"),
    ];
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
    let cats = vec![
        cat(2, None, "Rust", "topic"),
        cat(3, None, "Python", "topic"),
    ];
    let groups = vec![group(-1, "编程语言", None)];
    let out = build_plan(&cats, &groups, &[mv(2, Some(-1)), mv(3, Some(-1))]);
    assert_eq!(out.groups.len(), 1);
    assert_eq!(out.groups[0].name, "编程语言");
    assert_eq!(out.groups[0].depth, 1);
    assert_eq!(out.moves.len(), 2);
    assert!(out.moves.iter().all(|m| m.to_parent_id == Some(-1)));
    assert!(out
        .moves
        .iter()
        .all(|m| m.to_parent_name.as_deref() == Some("编程语言")));
}

/// 目标父级下已有同名分类：复用已有分类，不重复新建。
#[test]
fn reuses_existing_same_name_group_instead_of_creating() {
    let cats = vec![
        cat(1, None, "编程语言", "topic"),
        cat(2, None, "Rust", "topic"),
    ];
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
    assert!(out
        .moves
        .iter()
        .any(|m| m.id == 3 && m.to_parent_id == Some(-1)));
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
    let cats = vec![
        cat(1, None, "关注", "follow"),
        cat(2, None, "Rust", "topic"),
    ];
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
