// 「镜像主键」的同步：父表（A）主键变化后，子表（B）跟着变。
//
// 为什么需要这个：拖线那一刻的复制是对的，但**之后**父表加/减主键，A 侧变了、
// B 侧还停在旧结构上，导出的 SQL 就会引用不存在的列。所以要有命令级的 reconcile，
// 而且必须放在后端 —— 复制规则只有一份真源，前端不复制（跟校验同理）。
//
// 判定依据是 `DbDesignRelation::mirror`：只有「从父表全部主键锚点拖出来」的那组关系
// 才是镜像。字段级手动拖的关系 mirror = false，父表后来加主键不会自动推给子表。
//
// 四条同步规则：
//   1. A 新增主键字段      → B 补上同名字段副本 + 补一条关系
//   2. A 取消某个主键      → B 删掉该副本字段（若副本已无任何关系）+ 删关系
//   3. A 主键字段类型变了  → B 副本类型跟着变（否则外键类型对不上，SQL 跑不起来）
//   4. B 已有同名字段      → 复用它（不复制、不覆盖用户自己的列）
use super::models::{DbDesignDocument, DbDesignRelation, DbField, DbTableBody};

/// 复制父表主键到子表时**不**带过去的属性：主键身份、自增、唯一都不该跟过来
/// （子表里它就是普通外键列）。
fn as_foreign_key_copy(src: &DbField) -> DbField {
    DbField {
        name: src.name.clone(),
        r#type: src.r#type.clone(),
        nullable: false,
        default: None,
        comment: src.comment.clone(),
        pk: false,
        auto_increment: false,
        unique: false,
    }
}

fn pk_fields(table: &DbTableBody) -> Vec<String> {
    table
        .fields
        .iter()
        .filter(|f| f.pk)
        .map(|f| f.name.clone())
        .collect()
}

/// 就地同步一个文档里所有镜像关系，返回改动了多少处（给调用方做提示 / 测试断言）。
pub fn sync_mirrored_relations(doc: &mut DbDesignDocument) -> usize {
    let mut changes = 0usize;

    // 先收集「有哪些 (子表, 父表) 是镜像组」，避免边改边借还 borrow。
    let mut groups: Vec<(String, String)> = Vec::new();
    for rel in doc.relations.iter() {
        if !rel.mirror {
            continue;
        }
        let key = (rel.from.node.clone(), rel.to.node.clone());
        if !groups.contains(&key) {
            groups.push(key);
        }
    }

    for (child_id, parent_id) in groups {
        // 父表 / 子表任一被删：镜像关系失去意义，整体清掉。
        let parent_pk = match doc.node(&parent_id).and_then(|n| n.table.as_ref()) {
            Some(t) => pk_fields(t),
            None => {
                let before = doc.relations.len();
                doc.relations
                    .retain(|r| !(r.mirror && r.from.node == child_id && r.to.node == parent_id));
                changes += before - doc.relations.len();
                continue;
            }
        };
        if doc.node(&child_id).and_then(|n| n.table.as_ref()).is_none() {
            let before = doc.relations.len();
            doc.relations
                .retain(|r| !(r.mirror && r.from.node == child_id && r.to.node == parent_id));
            changes += before - doc.relations.len();
            continue;
        }

        // 规则 1 + 3：父表每个主键 → 子表要么复用同名字段、要么补一个副本（类型跟随）。
        for pk in &parent_pk {
            let Some(src) = doc
                .fields_of(&parent_id)
                .iter()
                .find(|f| &f.name == pk)
                .cloned()
            else {
                continue;
            };
            // 已经为这个父字段建过镜像关系？按 to.field 认，**不**按字段名 ——
            // 用户可能把复制过去的副本改名成 user_id，那条关系仍要认得出它对应 id。
            let existing = doc
                .relations
                .iter()
                .find(|r| {
                    r.mirror
                        && r.from.node == child_id
                        && r.to.node == parent_id
                        && &r.to.field == pk
                })
                .map(|r| r.from.field.clone());

            match existing {
                Some(child_field) => {
                    // 已有副本：字段被手工删过就补回来，类型跟着父主键走。
                    let has_field = doc.fields_of(&child_id).iter().any(|f| f.name == child_field);
                    if !has_field {
                        let mut copy = as_foreign_key_copy(&src);
                        copy.name = child_field.clone();
                        if let Some(child) = doc.node_mut(&child_id).and_then(|n| n.table.as_mut()) {
                            child.fields.push(copy);
                            changes += 1;
                        }
                    } else if let Some(child) = doc.node_mut(&child_id).and_then(|n| n.table.as_mut()) {
                        if let Some(target) = child.fields.iter_mut().find(|f| f.name == child_field) {
                            if target.r#type != src.r#type {
                                target.r#type = src.r#type.clone();
                                changes += 1;
                            }
                        }
                    }
                }
                None => {
                    // 还没有这条引用：子表有同名字段就复用，没有就复制一份。
                    let child_has_field = doc.fields_of(&child_id).iter().any(|f| &f.name == pk);
                    if !child_has_field {
                        if let Some(child) = doc.node_mut(&child_id).and_then(|n| n.table.as_mut()) {
                            child.fields.push(as_foreign_key_copy(&src));
                            changes += 1;
                        }
                    }
                    doc.relations.push(DbDesignRelation {
                        id: format!("r_{}_{}", child_id, pk),
                        name: String::new(),
                        from: super::models::DbRelationEnd {
                            node: child_id.clone(),
                            field: pk.clone(),
                        },
                        to: super::models::DbRelationEnd {
                            node: parent_id.clone(),
                            field: pk.clone(),
                        },
                        kind: "1-n".to_string(),
                        on_delete: "RESTRICT".to_string(),
                        on_update: "RESTRICT".to_string(),
                        mirror: true,
                    });
                    changes += 1;
                }
            }
        }

        // 规则 2：父表不再是主键的镜像引用 → 删关系；副本字段若已无任何关系，一并删掉。
        let stale: Vec<String> = doc
            .relations
            .iter()
            .filter(|r| {
                r.mirror && r.from.node == child_id && r.to.node == parent_id && !parent_pk.contains(&r.to.field)
            })
            .map(|r| r.from.field.clone())
            .collect();
        for field in stale {
            doc.relations
                .retain(|r| !(r.mirror && r.from.node == child_id && r.from.field == field && r.to.node == parent_id));
            changes += 1;
            // 只有「没人再引用」的副本才删：用户自己写的同名列不能被误删。
            let still_used = doc
                .relations
                .iter()
                .any(|r| r.from.node == child_id && r.from.field == field);
            if !still_used {
                if let Some(child) = doc.node_mut(&child_id).and_then(|n| n.table.as_mut()) {
                    let before = child.fields.len();
                    child.fields.retain(|f| f.name != field);
                    changes += before - child.fields.len();
                }
            }
        }
    }

    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::db_designer::models::{DbDesignNode, DbLogicalType};

    fn table(id: &str, name: &str, fields: Vec<DbField>) -> DbDesignNode {
        DbDesignNode {
            id: id.into(),
            kind: "table".into(),
            name: name.into(),
            comment: String::new(),
            x: 0.0,
            y: 0.0,
            table: Some(DbTableBody {
                fields,
                indexes: Vec::new(),
            }),
            view: None,
            ..Default::default()
        }
    }

    fn field(name: &str, base: &str, pk: bool) -> DbField {
        DbField {
            name: name.into(),
            r#type: DbLogicalType {
                base: base.into(),
                ..Default::default()
            },
            nullable: false,
            default: None,
            comment: String::new(),
            pk,
            auto_increment: false,
            unique: false,
        }
    }

    fn mirror_rel(child: &str, parent: &str, fieldname: &str) -> DbDesignRelation {
        DbDesignRelation {
            id: format!("r_{}_{}", child, fieldname),
            name: String::new(),
            from: super::super::models::DbRelationEnd {
                node: child.into(),
                field: fieldname.into(),
            },
            to: super::super::models::DbRelationEnd {
                node: parent.into(),
                field: fieldname.into(),
            },
            kind: "1-n".into(),
            on_delete: "RESTRICT".into(),
            on_update: "RESTRICT".into(),
            mirror: true,
        }
    }

    fn doc() -> DbDesignDocument {
        DbDesignDocument {
            id: "d1".into(),
            name: "d".into(),
            description: String::new(),
            dialect: "mysql".into(),
            folder_id: None,
            nodes: vec![
                table("a", "users", vec![field("id", "bigint", true)]),
                table("b", "orders", vec![field("id", "bigint", true)]),
            ],
            relations: vec![mirror_rel("b", "a", "id")],
            updated_at: String::new(),
        }
    }

    /// A 新增一个主键 → B 自动补上副本字段与关系。
    #[test]
    fn parent_gains_pk_child_gets_copy_and_relation() {
        let mut d = doc();
        d.node_mut("a")
            .unwrap()
            .table
            .as_mut()
            .unwrap()
            .fields
            .push(field("tenant_id", "bigint", true));
        sync_mirrored_relations(&mut d);
        let b_fields: Vec<String> = d
            .fields_of("b")
            .iter()
            .map(|f| f.name.clone())
            .collect();
        assert_eq!(b_fields, vec!["id", "tenant_id"]);
        assert_eq!(d.relations.len(), 2);
        // 副本不能是主键、也不能自增
        let copied = d.fields_of("b").iter().find(|f| f.name == "tenant_id").unwrap();
        assert!(!copied.pk && !copied.auto_increment);
    }

    /// A 取消主键 → B 的副本字段与关系一起消失。
    #[test]
    fn parent_drops_pk_child_copy_is_removed() {
        let mut d = doc();
        d.node_mut("a")
            .unwrap()
            .table
            .as_mut()
            .unwrap()
            .fields
            .push(field("tenant_id", "bigint", true));
        sync_mirrored_relations(&mut d);
        d.node_mut("a")
            .unwrap()
            .table
            .as_mut()
            .unwrap()
            .fields
            .retain(|f| f.name != "tenant_id");
        sync_mirrored_relations(&mut d);
        let b_fields: Vec<String> = d
            .fields_of("b")
            .iter()
            .map(|f| f.name.clone())
            .collect();
        assert_eq!(b_fields, vec!["id"]);
        assert_eq!(d.relations.len(), 1);
    }

    /// A 主键类型变了 → B 副本跟着变（否则外键类型对不上）。
    #[test]
    fn parent_pk_type_change_propagates() {
        let mut d = doc();
        d.node_mut("a")
            .unwrap()
            .table
            .as_mut()
            .unwrap()
            .fields[0]
            .r#type
            .base = "int".into();
        sync_mirrored_relations(&mut d);
        assert_eq!(d.fields_of("b")[0].r#type.base, "int");
    }

    /// 非镜像（字段级手动拖的）关系不参与同步：A 加主键不该动 B。
    #[test]
    fn non_mirror_relation_is_left_alone() {
        let mut d = doc();
        d.relations[0].mirror = false;
        d.node_mut("a")
            .unwrap()
            .table
            .as_mut()
            .unwrap()
            .fields
            .push(field("tenant_id", "bigint", true));
        sync_mirrored_relations(&mut d);
        assert_eq!(d.fields_of("b").len(), 1);
        assert_eq!(d.relations.len(), 1);
    }

    /// B 已有同名字段 → 复用，不重复复制。
    #[test]
    fn existing_same_name_field_is_reused() {
        let mut d = doc();
        d.node_mut("a")
            .unwrap()
            .table
            .as_mut()
            .unwrap()
            .fields
            .push(field("tenant_id", "bigint", true));
        // B 已经有 tenant_id，但不是外键副本
        d.node_mut("b")
            .unwrap()
            .table
            .as_mut()
            .unwrap()
            .fields
            .push(field("tenant_id", "varchar", false));
        sync_mirrored_relations(&mut d);
        assert_eq!(d.fields_of("b").len(), 2);
        let existing = d.fields_of("b").iter().find(|f| f.name == "tenant_id").unwrap();
        // 用户自己的列类型不该被覆盖
        assert_eq!(existing.r#type.base, "varchar");
        assert_eq!(d.relations.len(), 2);
    }

    /// 父表被删 → 镜像关系整体清掉（不留下悬空引用）。
    #[test]
    fn deleting_parent_drops_mirror_relations() {
        let mut d = doc();
        d.nodes.retain(|n| n.id != "a");
        sync_mirrored_relations(&mut d);
        assert!(d.relations.is_empty());
    }

    /// 幂等：连续同步两次不应继续产生改动。
    #[test]
    fn sync_is_idempotent() {
        let mut d = doc();
        assert_eq!(sync_mirrored_relations(&mut d), 0);
        assert_eq!(sync_mirrored_relations(&mut d), 0);
    }

    /// 副本被改名（如 id → user_id）后同步**不能**又把 id 复制一份回来：
    /// 关系是按父字段认的，不是按字段名。
    #[test]
    fn renamed_copy_is_matched_by_parent_field() {
        let mut d = doc();
        // 子表那一列改名成 user_id
        d.node_mut("b")
            .unwrap()
            .table
            .as_mut()
            .unwrap()
            .fields[0]
            .name = "user_id".to_string();
        d.relations[0].from.field = "user_id".to_string();
        d.node_mut("a")
            .unwrap()
            .table
            .as_mut()
            .unwrap()
            .fields
            .push(field("tenant_id", "bigint", true));
        sync_mirrored_relations(&mut d);
        let names: Vec<String> = d.fields_of("b").iter().map(|f| f.name.clone()).collect();
        assert!(
            names.contains(&"user_id".to_string()) && !names.contains(&"id".to_string()),
            "改名后不该被复制回 id: {:?}",
            names
        );
        // 新主键 tenant_id 仍然要补上
        assert!(names.contains(&"tenant_id".to_string()));
        assert_eq!(d.relations.len(), 2);
    }
}
