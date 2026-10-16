// 增量更新：把模型给的「表级补丁」应用到当前设计。
//
// 为什么是**表级**而不是字段级操作集 / JSON Patch：
//   · 模型不用算数组下标（算错就静默改错别人的字段，比报错难发现得多）；
//   · 操作集小且完备，漏报一张表的结果是「那张表不变」而不是「被清空」；
//   · 代价是改一个字段要重发整张表 —— 相比重发整个设计文档仍然省得多。
//
// 合并策略是**保守**的：模型提到的字段才动，**它没提到的字段一律保留**。
// 最坏的失败模式是「你手工加的字段静默消失」，所以宁可多几个字段（你能自己删）。
//
// 全程不碰坐标：位置是用户的（模型给的 x/y 会被丢掉），布局永远由前端算。

use super::models::{DbDesignNode, DbDesignRelation, DbDesignDocument};

/// 模型给的表级补丁。
#[derive(serde::Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DesignPatch {
    /// 新增或更新的表（**完整定义**，含全部字段）
    #[serde(default)]
    pub upsert_tables: Vec<DbDesignNode>,
    /// 要删除的表：给表名或 id 都认
    #[serde(default)]
    pub remove_tables: Vec<String>,
    /// 新增或更新的关联（完整定义）
    #[serde(default)]
    pub upsert_relations: Vec<DbDesignRelation>,
    /// 要删除的关联：给 id 或两端「表.字段」都认
    #[serde(default)]
    pub remove_relations: Vec<String>,
}

/// 应用结果：除了最终文档，还要告诉用户**改了什么**。
#[derive(serde::Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct PatchApplied {
    pub added_tables: Vec<String>,
    pub updated_tables: Vec<String>,
    pub removed_tables: Vec<String>,
    pub added_relations: usize,
    pub removed_relations: usize,
    /// 因端点表不存在而被丢弃的关联（模型幻觉出一个不存在的表时会发生）
    pub dropped_relations: Vec<String>,
}

/// 表名/ id 的匹配：**忽略大小写**。
///
/// 为什么：`order` 与 `Order` 在 MySQL 里是同一张表（列名不区分大小写），
/// 而模型很可能把大小写改了。不匹配上的话会出现「新建一张 Order、旧的 order 还在」，
/// 校验会以「表名重复」报错，用户看到的是一张凭空多出来的表。
fn same_node(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// 在文档里找一张表：先按 id，再按名字（忽略大小写）。
fn find_node(doc: &DbDesignDocument, key: &str) -> Option<usize> {
    if let Some(i) = doc.nodes.iter().position(|n| n.id == key) {
        return Some(i);
    }
    doc.nodes.iter().position(|n| same_node(&n.name, key))
}

/**
 * 保守合并一张表：模型提到的字段才动，**它没提到的字段一律保留**。
 *
 * 逐项决定「谁说了算」：
 * | 字段 | 来源 | 为什么 |
 * |---|---|---|
 * | 表名 | **模型** | 模型改名的意图要尊重（id 不变，所以关联不会断） |
 * | 注释 / 标签 |模型 | 同上，属于设计内容 |
 * | 颜色 | 模型非空才用 | 你手动挑的颜色不该被一次生成清掉 |
 * | 字段（同名） | **模型** | 它就是来改这个的 |
 * | 字段（它没提） | **保留** | 核心：不能静默弄丢你手工加的东西 |
 * | 索引 | 同字段逻辑 | 同上 |
 * | 坐标 x/y | **永远保留** | 布局是前端的事，模型给的坐标一律丢掉 |
 */
fn merge_table(existing: &DbDesignNode, incoming: &DbDesignNode) -> DbDesignNode {
    let mut base = existing.clone();
    base.name = incoming.name.clone();
    base.comment = incoming.comment.clone();
    base.tags = incoming.tags.clone();
    if !incoming.color.trim().is_empty() {
        base.color = incoming.color.clone();
    }
    // 坐标：模型给的一律不要 —— 它不知道你把表拖到哪了
    base.x = existing.x;
    base.y = existing.y;

    if let (Some(eb), Some(mut ib)) = (base.table.as_mut(), incoming.table.as_ref()) {
        // 字段：同名的用新定义（并保持原位置），新名字追加到末尾
        for f in ib.fields.iter() {
            match eb.fields.iter().position(|x| x.name == f.name) {
                Some(i) => eb.fields[i] = f.clone(),
                None => eb.fields.push(f.clone()),
            }
        }
        // 索引同理
        for ix in ib.indexes.iter() {
            match eb.indexes.iter().position(|x| same_node(&x.name, &ix.name)) {
                Some(i) => eb.indexes[i] = ix.clone(),
                None => eb.indexes.push(ix.clone()),
            }
        }
    }
    base
}

/// 关联的两端是否指向同一处（`node` 与 `field` 都算）。
fn same_rel(a: &DbDesignRelation, b: &DbDesignRelation) -> bool {
    a.from.node == b.from.node
        && a.to.node == b.to.node
        && a.from.field == b.from.field
        && a.to.field == b.to.field
}

/// 把关联端点上写的**表名**回认成节点 id。
///
/// 模型经常写 `"from": {"node": "order", ...}` 而不是内部 id —— 回认不了就是悬空引用，
/// 校验会以「关联指向不存在的节点」把整份设计拦下。
fn resolve_relation_ids(doc: &DbDesignDocument, rel: &DbDesignRelation) -> DbDesignRelation {
    let mut out = rel.clone();
    for end in [&mut out.from, &mut out.to] {
        if doc.nodes.iter().any(|n| n.id == end.node) {
            continue; // 已经是 id
        }
        if let Some(i) = find_node(doc, &end.node) {
            end.node = doc.nodes[i].id.clone();
        }
    }
    out
}

/**
 * 把补丁应用到当前设计。**没被提到的表与关联一律原样保留** ——
 * 这条性质是整个增量方案的安全底线：模型漏报的结果是「那张表没变」，而不是「被清空」。
 */
pub fn apply_patch(
    base: &DbDesignDocument,
    patch: &DesignPatch,
) -> (DbDesignDocument, PatchApplied) {
    let mut doc = base.clone();
    let mut log = PatchApplied::default();

    // ── 1) 新增 / 更新表 ──
    for t in &patch.upsert_tables {
        let key = if t.id.trim().is_empty() { t.name.clone() } else { t.id.clone() };
        match find_node(&doc, &key).or_else(|| find_node(&doc, &t.name)) {
            Some(i) => {
                log.updated_tables.push(doc.nodes[i].name.clone());
                doc.nodes[i] = merge_table(&doc.nodes[i], t);
            }
            None => {
                // 新表：给它补 id（模型常常不写），坐标留 0 交给前端自动布局
                let mut nt = t.clone();
                if nt.id.trim().is_empty() {
                    nt.id = format!("n_{}", nt.name.trim());
                }
                nt.x = 0.0;
                nt.y = 0.0;
                log.added_tables.push(nt.name.clone());
                doc.nodes.push(nt);
            }
        }
    }

    // ── 2) 删除表（连带删掉它的关联，否则会留下悬空引用）──
    for key in &patch.remove_tables {
        if let Some(i) = find_node(&doc, key) {
            let id = doc.nodes[i].id.clone();
            let name = doc.nodes[i].name.clone();
            let before = doc.relations.len();
            doc.relations.retain(|r| r.from.node.clone() != id && r.to.node.clone() != id);
            log.removed_relations += before - doc.relations.len();
            doc.nodes.remove(i);
            log.removed_tables.push(name);
        }
    }

    // ── 3) 新增 / 更新关联 ──
    for r in &patch.upsert_relations {
        let rel = resolve_relation_ids(&doc, r);
        // 端点表不存在 = 模型幻觉出一个不存在的表：丢掉并记下来，别让整份设计校验失败
        let from_ok = doc.nodes.iter().any(|n| n.id == rel.from.node);
        let to_ok = doc.nodes.iter().any(|n| n.id == rel.to.node);
        if !from_ok || !to_ok {
            log.dropped_relations.push(format!(
                "{} -> {}",
                rel.from.node, rel.to.node
            ));
            continue;
        }
        match doc.relations.iter().position(|x| same_rel(x, &rel)) {
            Some(i) => doc.relations[i] = rel,
            None => {
                doc.relations.push(rel);
                log.added_relations += 1;
            }
        }
    }

    // ── 4) 删除关联：给 id 或「表.字段」都认 ──
    for key in &patch.remove_relations {
        let hit = doc.relations.iter().position(|r| {
            r.id == *key
                || format!("{}.{}", r.from.node, r.from.field) == *key
                || format!("{}.{}", r.to.node, r.to.field) == *key
                || r.name == *key
        });
        if let Some(i) = hit {
            doc.relations.remove(i);
            log.removed_relations += 1;
        }
    }

    (doc, log)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str) -> super::super::models::DbField {
        serde_json::from_value(serde_json::json!({
            "name": name, "type": { "base": "varchar", "length": 64 }, "nullable": false
        }))
        .unwrap()
    }

    fn table(id: &str, name: &str, fields: &[&str]) -> DbDesignNode {
        serde_json::from_value(serde_json::json!({
            "id": id, "kind": "table", "name": name,
            "table": { "fields": fields.iter().map(|f| field(f)).collect::<Vec<_>>() }
        }))
        .unwrap()
    }

    fn rel(id: &str, from: &str, ff: &str, to: &str, tf: &str, del: &str) -> DbDesignRelation {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "from": { "node": from, "field": ff },
            "to": { "node": to, "field": tf },
            "kind": "1-n", "onDelete": del
        }))
        .unwrap()
    }

    fn doc_of(nodes: Vec<DbDesignNode>) -> DbDesignDocument {
        let mut d = super::super::file::new_document("x", "mysql");
        d.nodes = nodes;
        d
    }

    fn names(doc: &DbDesignDocument) -> Vec<String> {
        let mut v: Vec<String> = doc.nodes.iter().map(|n| n.name.clone()).collect();
        v.sort();
        v
    }

    fn two_tables() -> DbDesignDocument {
        doc_of(vec![
            table("t1", "user", &["user_id"]),
            table("t2", "order", &["order_id", "user_id"]),
        ])
    }

    /// **核心不变量：补丁没提到的表一律原样保留。**
    ///
    /// 这是整个增量方案的安全底线 —— 模型漏报一张表，结果必须是「那张表没变」，
    /// 而不是「被清空」。最坏的失败模式是静默丢数据。
    #[test]
    fn untouched_tables_survive() {
        let patch = DesignPatch {
            upsert_tables: vec![table("t2", "order", &["order_id", "order_remark"])],
            ..Default::default()
        };
        let (out, _) = apply_patch(&two_tables(), &patch);
        assert_eq!(names(&out), vec!["order", "user"], "没提到的表必须全在");
    }

    /// 保守合并：模型没提到的字段要保留 —— 你手工加的字段不能被一次生成抹掉。
    #[test]
    fn merge_keeps_fields_the_model_omitted() {
        let base = doc_of(vec![table("t1", "order", &["order_id", "my_note", "another"])]);
        let patch = DesignPatch {
            upsert_tables: vec![table("t1", "order", &["order_id", "order_status"])],
            ..Default::default()
        };
        let (out, _) = apply_patch(&base, &patch);
        let fs: Vec<String> = out.nodes[0]
            .table
            .as_ref()
            .unwrap()
            .fields
            .iter()
            .map(|f| f.name.clone())
            .collect();
        assert_eq!(
            fs,
            vec!["order_id", "my_note", "another", "order_status"],
            "同名字段保持原位、新字段追加末尾、模型没提的字段必须留着"
        );
    }

    /// 改表名要尊重（id 不变，关联不断），且不能多出一张新表。
    #[test]
    fn renaming_a_table_updates_in_place() {
        let base = doc_of(vec![table("t1", "order", &["order_id"])]);
        let patch = DesignPatch {
            upsert_tables: vec![table("t1", "orders", &["order_id"])],
            ..Default::default()
        };
        let (out, log) = apply_patch(&base, &patch);
        assert_eq!(out.nodes.len(), 1, "改名不该多出一张表");
        assert_eq!(out.nodes[0].name, "orders");
        assert_eq!(log.updated_tables, vec!["order"], "记的是改之前的名字");
    }

    /// 表名大小写不同也算同一张表 —— 否则会出现「凭空多一张 Order」。
    #[test]
    fn table_name_match_ignores_case() {
        let base = doc_of(vec![table("t1", "order", &["order_id"])]);
        let patch = DesignPatch {
            upsert_tables: vec![table("", "Order", &["order_id"])],
            ..Default::default()
        };
        let (out, log) = apply_patch(&base, &patch);
        assert_eq!(out.nodes.len(), 1, "Order 与 order 应认成同一张表");
        assert_eq!(log.added_tables.len(), 0);
        assert_eq!(log.updated_tables.len(), 1);
    }

    /// 新表：模型没给 id 时要补上，否则 React key 撞车、关联也引用不到。
    #[test]
    fn new_table_gets_an_id() {
        let patch = DesignPatch {
            upsert_tables: vec![table("", "user", &["user_id"])],
            ..Default::default()
        };
        let (out, log) = apply_patch(&doc_of(vec![]), &patch);
        assert_eq!(out.nodes[0].id, "n_user");
        assert_eq!(log.added_tables, vec!["user"]);
    }

    /// 删表要连带删掉关联 —— 留着就是悬空引用，校验会以 error 把整份设计拦下。
    #[test]
    fn removing_a_table_removes_its_relations() {
        let mut base = two_tables();
        base.relations.push(rel("r1", "t2", "user_id", "t1", "user_id", ""));
        let patch = DesignPatch { remove_tables: vec!["user".into()], ..Default::default() };
        let (out, log) = apply_patch(&base, &patch);
        assert_eq!(names(&out), vec!["order"]);
        assert_eq!(out.relations.len(), 0, "删表必须连带删关联");
        assert_eq!(log.removed_tables, vec!["user"]);
        assert_eq!(log.removed_relations, 1);
    }

    /// 模型多半写表名而不是内部 id 当关联端点 —— 回认不了就是悬空引用。
    #[test]
    fn relation_endpoints_given_as_names_are_resolved() {
        let patch = DesignPatch {
            upsert_relations: vec![rel("r1", "order", "user_id", "user", "user_id", "")],
            ..Default::default()
        };
        let (out, log) = apply_patch(&two_tables(), &patch);
        assert_eq!(out.relations.len(), 1);
        assert_eq!(out.relations[0].from.node, "t2", "表名要回认成 id");
        assert_eq!(out.relations[0].to.node, "t1");
        assert_eq!(log.dropped_relations.len(), 0);
    }

    /// 模型幻觉出不存在的表时丢掉这条关联并记下来，而不是让整份设计校验失败。
    #[test]
    fn relation_to_a_missing_table_is_dropped_and_logged() {
        let base = doc_of(vec![table("t1", "user", &["user_id"])]);
        let patch = DesignPatch {
            upsert_relations: vec![rel("r1", "t1", "user_id", "nope", "id", "")],
            ..Default::default()
        };
        let (out, log) = apply_patch(&base, &patch);
        assert_eq!(out.relations.len(), 0, "端点表不存在的关联不能留");
        assert_eq!(log.dropped_relations.len(), 1, "要记下来让用户知道");
    }

    /// 重复 upsert 同一条关联应更新而不是追加两条。
    #[test]
    fn upserting_the_same_relation_updates_it() {
        let mut base = two_tables();
        base.relations.push(rel("r1", "t2", "user_id", "t1", "user_id", ""));
        let patch = DesignPatch {
            upsert_relations: vec![rel("r1", "t2", "user_id", "t1", "user_id", "CASCADE")],
            ..Default::default()
        };
        let (out, log) = apply_patch(&base, &patch);
        assert_eq!(out.relations.len(), 1, "不能追加出第二条一样的关联");
        assert_eq!(out.relations[0].on_delete, "CASCADE", "要更新成新定义");
        assert_eq!(log.added_relations, 0);
    }

    /// 删关联：给 id 或「表.字段」都认。
    #[test]
    fn relations_can_be_removed_by_id_or_field_path() {
        let mut base = two_tables();
        base.relations.push(rel("r1", "t2", "user_id", "t1", "user_id", ""));
        for key in ["r1", "t2.user_id"] {
            let p = DesignPatch { remove_relations: vec![key.into()], ..Default::default() };
            let (out, _) = apply_patch(&base, &p);
            assert_eq!(out.relations.len(), 0, "用 {} 应该能删掉", key);
        }
    }

    /// **坐标永远归前端**：模型给的 x/y 一律丢掉，不能让它把表挪走。
    #[test]
    fn model_supplied_coordinates_are_ignored() {
        let mut existing = table("t1", "order", &["order_id"]);
        existing.x = 500.0;
        existing.y = 300.0;
        let base = doc_of(vec![existing]);
        let mut incoming = table("t1", "order", &["order_id"]);
        incoming.x = 9999.0;
        incoming.y = 8888.0;
        let patch = DesignPatch { upsert_tables: vec![incoming], ..Default::default() };
        let (out, _) = apply_patch(&base, &patch);
        assert_eq!(out.nodes[0].x, 500.0, "坐标不能被模型改");
        assert_eq!(out.nodes[0].y, 300.0);
    }

    /// 空补丁不该改变任何东西。
    #[test]
    fn empty_patch_is_a_no_op() {
        let (out, log) = apply_patch(&two_tables(), &DesignPatch::default());
        assert_eq!(out.nodes.len(), 2);
        assert_eq!(log.added_tables.len() + log.updated_tables.len() + log.removed_tables.len(), 0);
    }
}
