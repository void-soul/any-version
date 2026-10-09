// 数据库设计器（C2）：画「表 / 视图 / 函数」组成的图，改的是设计文件，导出才生成 SQL。
//
// 分期见 docs/design/db-designer.md：
//   P1 数据模型 + 校验 + 导出（本目录已实现）
//   P2 画布 UI（React Flow）  P3 反向工程  P4 Agent 助手 + 内置技能
pub mod commands;
pub mod export;
pub mod file;
pub mod models;
pub mod reverse;
pub mod store;

#[cfg(test)]
mod tests {
    use super::models::*;
    use super::store::{relation_fields, rename_field, validate};
    use super::export::{export_outline, export_sql};

    fn ty(base: &str) -> DbLogicalType {
        DbLogicalType { base: base.to_string(), ..Default::default() }
    }

    fn field(name: &str, base: &str) -> DbField {
        DbField {
            name: name.to_string(),
            r#type: ty(base),
            nullable: false,
            default: None,
            comment: String::new(),
            pk: false,
            auto_increment: false,
            unique: false,
        }
    }

    /// 两份表 + 一条 1-n 关联的最小文档
    fn sample(dialect: &str) -> DbDesignDocument {
        let mut uid = field("uid", "bigint");
        uid.comment = "下单人".to_string();
        let mut no = field("no", "varchar");
        no.r#type = DbLogicalType { base: "varchar".into(), length: Some(32), ..Default::default() };
        no.unique = true;
        no.comment = "订单号".to_string();
        DbDesignDocument {
            id: "d1".to_string(),
            name: "订单库".to_string(),
            description: String::new(),
            dialect: dialect.to_string(),
            folder_id: None,
            nodes: vec![
                DbDesignNode {
                    id: "n_user".to_string(),
                    kind: "table".to_string(),
                    name: "users".to_string(),
                    comment: "用户表".to_string(),
                    x: 0.0,
                    y: 0.0,
                    table: Some(DbTableBody {
                        fields: vec![DbField {
                            pk: true,
                            auto_increment: true,
                            ..field("id", "bigint")
                        }],
                        indexes: vec![],
                    }),
                    view: None,
                },
                DbDesignNode {
                    id: "n_order".to_string(),
                    kind: "table".to_string(),
                    name: "orders".to_string(),
                    comment: "订单主表".to_string(),
                    x: 200.0,
                    y: 0.0,
                    table: Some(DbTableBody {
                        fields: vec![
                            DbField { pk: true, auto_increment: true, ..field("id", "bigint") },
                            no,
                            uid,
                        ],
                        indexes: vec![DbIndex {
                            name: "idx_orders_uid".to_string(),
                            kind: "index".to_string(),
                            fields: vec!["uid".to_string()],
                        }],
                    }),
                    view: None,
                },
            ],
            relations: vec![DbDesignRelation {
                id: "r1".to_string(),
                name: "订单属于用户".to_string(),
                from: DbRelationEnd { node: "n_order".to_string(), field: "uid".to_string() },
                to: DbRelationEnd { node: "n_user".to_string(), field: "id".to_string() },
                kind: "1-n".to_string(),
                on_delete: "CASCADE".to_string(),
                on_update: "CASCADE".to_string(),
            }],
            updated_at: String::new(),
        }
    }

    #[test]
    fn sample_document_is_valid() {
        let r = validate(&sample("mysql"));
        assert!(r.is_ok(), "errors={:?}", r.errors);
    }

    #[test]
    fn validate_catches_duplicate_names_dangling_fields_and_bad_types() {
        let mut doc = sample("mysql");
        doc.nodes[1].name = "users".to_string(); // 与 users 重名
        let r = validate(&doc);
        assert!(r.errors.iter().any(|e| e.contains("节点名")), "{:?}", r.errors);

        let mut doc = sample("mysql");
        doc.relations[0].to.field = "nope".to_string();
        let r = validate(&doc);
        assert!(r.errors.iter().any(|e| e.contains("不存在的字段")), "{:?}", r.errors);

        let mut doc = sample("mysql");
        doc.nodes[0].table.as_mut().unwrap().fields[0].r#type.base = "nvarchar".to_string();
        let r = validate(&doc);
        assert!(r.errors.iter().any(|e| e.contains("不在白名单")), "{:?}", r.errors);
    }

    #[test]
    fn rename_field_cascades_into_relations_and_indexes() {
        let mut doc = sample("mysql");
        let touched = rename_field(&mut doc, "n_order", "uid", "user_id").unwrap();
        assert_eq!(touched, 1, "关联里的字段名应被级联修改");
        assert_eq!(doc.relations[0].from.field, "user_id");
        assert!(doc.nodes[1]
            .table
            .as_ref()
            .unwrap()
            .indexes[0]
            .fields
            .contains(&"user_id".to_string()));
        // 改名后仍然自洽
        assert!(validate(&doc).is_ok());
    }

    #[test]
    fn rename_field_rejects_empty_and_duplicate() {
        let mut doc = sample("mysql");
        assert!(rename_field(&mut doc, "n_order", "uid", "").is_err());
        assert!(rename_field(&mut doc, "n_order", "uid", "no").is_err());
        assert!(rename_field(&mut doc, "n_order", "nope", "x").is_err());
    }

    #[test]
    fn relation_fields_lists_only_fields_used_by_relations() {
        let doc = sample("mysql");
        assert_eq!(relation_fields(&doc, "n_order"), vec!["uid".to_string()]);
        assert_eq!(relation_fields(&doc, "n_user"), vec!["id".to_string()]);
    }

    #[test]
    fn mysql_export_has_backticks_inline_comments_and_foreign_keys() {
        let sql = export_sql(&sample("mysql")).unwrap();
        assert!(sql.contains("CREATE TABLE `orders` ("), "{}", sql);
        assert!(sql.contains("`no` VARCHAR(32) NOT NULL COMMENT"), "{}", sql);
        assert!(sql.contains("`id` BIGINT NOT NULL AUTO_INCREMENT"), "{}", sql);
        assert!(sql.contains("PRIMARY KEY (`id`)"), "{}", sql);
        assert!(sql.contains("CONSTRAINT `fk_orders_uid` FOREIGN KEY (`uid`) REFERENCES `users` (`id`)"), "{}", sql);
        // 外键只能建在 from（多）端：父表不该被挂上外键
        assert!(!sql.contains("REFERENCES `orders`"), "父表被误建外键: {}", sql);
        assert!(sql.contains("ON DELETE CASCADE"), "{}", sql);
        assert!(sql.contains("KEY `idx_orders_uid` (`uid`)"), "{}", sql);
        assert!(sql.contains("COMMENT='订单主表'"), "{}", sql);
    }

    #[test]
    fn postgres_export_uses_quotes_serial_and_comment_statements() {
        let sql = export_sql(&sample("postgres")).unwrap();
        assert!(sql.contains("CREATE TABLE \"orders\" ("), "{}", sql);
        assert!(sql.contains("\"id\" BIGSERIAL"), "{}", sql);
        assert!(sql.contains("COMMENT ON TABLE \"orders\" IS '订单主表';"), "{}", sql);
        assert!(sql.contains("ALTER TABLE \"orders\" ADD CONSTRAINT \"fk_orders_uid\""), "{}", sql);
        // 索引在建表语句之外
        assert!(sql.contains("CREATE INDEX \"idx_orders_uid\" ON \"orders\" (\"uid\");"), "{}", sql);
    }

    #[test]
    fn sqlite_export_inlines_autoincrement_pk_and_lowers_comments() {
        let sql = export_sql(&sample("sqlite")).unwrap();
        assert!(sql.contains("\"id\" INTEGER PRIMARY KEY AUTOINCREMENT"), "{}", sql);
        assert!(!sql.contains("PRIMARY KEY (\"id\")"), "SQLite 单列自增主键不该再有独立 PK 行: {}", sql);
        assert!(sql.contains("FOREIGN KEY (\"uid\") REFERENCES \"users\" (\"id\")"), "{}", sql);
        assert!(sql.contains("-- 表 orders：订单主表"), "{}", sql);
    }

    #[test]
    fn export_refuses_invalid_documents() {
        let mut doc = sample("mysql");
        doc.relations[0].from.field = "ghost".to_string();
        assert!(export_sql(&doc).is_err());

        // 未知方言（例如旧文件里残留的 mongodb）也要被挡住，而不是导出半截 SQL
        let mut doc = sample("mysql");
        doc.dialect = "mongodb".to_string();
        let err = export_sql(&doc).unwrap_err();
        assert!(err.contains("未知方言"), "{}", err);
        assert!(validate(&doc).errors.iter().any(|e| e.contains("未知方言")));

        // 结构说明不挑方言，任何时候都能出
        let outline = export_outline(&doc);
        assert!(outline.contains("orders"), "{}", outline);
        assert!(outline.contains("订单属于用户") || outline.contains("1-n"), "{}", outline);
    }

    #[test]
    fn enum_becomes_check_constraint_outside_mysql() {
        let mut doc = sample("postgres");
        doc.nodes[0].table.as_mut().unwrap().fields.push(DbField {
            name: "status".to_string(),
            r#type: DbLogicalType {
                base: "enum".to_string(),
                values: vec!["new".to_string(), "paid".to_string()],
                ..Default::default()
            },
            nullable: false,
            default: None,
            comment: String::new(),
            pk: false,
            auto_increment: false,
            unique: false,
        });
        let sql = export_sql(&doc).unwrap();
        assert!(sql.contains("CHECK (\"status\" IN ('new', 'paid'))"), "{}", sql);

        let mut mysql = sample("mysql");
        mysql.nodes[0].table.as_mut().unwrap().fields.push(DbField {
            name: "status".to_string(),
            r#type: DbLogicalType {
                base: "enum".to_string(),
                values: vec!["new".to_string(), "paid".to_string()],
                ..Default::default()
            },
            nullable: false,
            default: None,
            comment: String::new(),
            pk: false,
            auto_increment: false,
            unique: false,
        });
        let sql = export_sql(&mysql).unwrap();
        assert!(sql.contains("ENUM('new', 'paid')"), "{}", sql);
    }

    #[test]
    fn many_to_many_generates_a_junction_table() {
        let mut doc = sample("mysql");
        doc.nodes.push(DbDesignNode {
            id: "n_tag".to_string(),
            kind: "table".to_string(),
            name: "tags".to_string(),
            comment: String::new(),
            x: 0.0,
            y: 200.0,
            table: Some(DbTableBody {
                fields: vec![DbField { pk: true, auto_increment: true, ..field("id", "bigint") }],
                indexes: vec![],
            }),
            view: None,
        });
        doc.relations.push(DbDesignRelation {
            id: "r2".to_string(),
            name: String::new(),
            from: DbRelationEnd { node: "n_order".to_string(), field: "id".to_string() },
            to: DbRelationEnd { node: "n_tag".to_string(), field: "id".to_string() },
            kind: "n-n".to_string(),
            on_delete: "CASCADE".to_string(),
            on_update: "CASCADE".to_string(),
        });
        let sql = export_sql(&doc).unwrap();
        assert!(sql.contains("CREATE TABLE `orders_tags` ("), "{}", sql);
        assert!(sql.contains("PRIMARY KEY (`orders_id`, `tags_id`)"), "{}", sql);
        assert!(sql.contains("REFERENCES `tags` (`id`)"), "{}", sql);
    }

    #[test]
    fn views_are_exported_in_every_relational_dialect() {
        let mut doc = sample("postgres");
        doc.nodes.push(DbDesignNode {
            id: "n_v".to_string(),
            kind: "view".to_string(),
            name: "v_orders".to_string(),
            comment: String::new(),
            x: 0.0,
            y: 0.0,
            table: None,
            view: Some(DbViewBody { sql: "SELECT * FROM orders;".to_string() }),
        });
        let sql = export_sql(&doc).unwrap();
        assert!(sql.contains("CREATE VIEW \"v_orders\" AS"), "{}", sql);
        assert!(!sql.contains("SELECT * FROM orders;;"), "视图末尾分号不该重复: {}", sql);

        let mut lite = doc.clone();
        lite.dialect = "sqlite".to_string();
        assert!(export_sql(&lite).unwrap().contains("CREATE VIEW \"v_orders\" AS"));

        let mut my = doc.clone();
        my.dialect = "mysql".to_string();
        assert!(export_sql(&my).unwrap().contains("CREATE VIEW `v_orders` AS"));
    }

    #[test]
    fn function_nodes_are_rejected_after_the_decision_to_drop_them() {
        // 用户决定：函数不做（SQLite 没有、MySQL/PG 差异大）。
        // 历史文件里若还留着 function 节点，校验要报错而不是静默忽略。
        let mut doc = sample("mysql");
        doc.nodes.push(DbDesignNode {
            id: "n_f".to_string(),
            kind: "function".to_string(),
            name: "total_amount".to_string(),
            comment: String::new(),
            x: 0.0,
            y: 0.0,
            table: None,
            view: None,
        });
        let r = validate(&doc);
        assert!(r.errors.iter().any(|e| e.contains("不在允许列表")), "{:?}", r.errors);
        assert!(export_sql(&doc).is_err());
        let _ = ty("decimal");
    }
}
