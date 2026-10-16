// 数据库设计器（C2）：画「表 / 视图 / 函数」组成的图，改的是设计文件，导出才生成 SQL。
//
// 分期见 docs/design/db-designer.md：
//   P1 数据模型 + 校验 + 导出（本目录已实现）
//   P2 画布 UI（React Flow）  P3 反向工程  P4 Agent 助手 + 内置技能
pub mod ai;
pub mod common_fields;
pub mod commands;
pub mod export;
pub mod file;
pub mod models;
pub mod reverse;
pub mod settings;
pub mod store;
pub mod sync;

#[cfg(test)]
mod tests {
    use super::models::*;
    use super::store::{normalize_naming, relation_fields, rename_field, validate};
    use super::export::{export_outline, export_sql, export_sql_with, ExportOptions};

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
                common_fields: Vec::new(),
                        decisions: Vec::new(),
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
                            ..field("user_id", "bigint")
                        }],
                        indexes: vec![],
                    }),
                    view: None,
                    ..Default::default()
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
                            DbField { pk: true, auto_increment: true, ..field("order_id", "bigint") },
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
                    ..Default::default()
                },
            ],
            relations: vec![DbDesignRelation {
                id: "r1".to_string(),
                name: "订单属于用户".to_string(),
                from: DbRelationEnd { node: "n_order".to_string(), field: "uid".to_string() },
                to: DbRelationEnd { node: "n_user".to_string(), field: "user_id".to_string() },
                kind: "1-n".to_string(),
                on_delete: "CASCADE".to_string(),
                on_update: "CASCADE".to_string(),
                mirror: false,
            }],
            updated_at: String::new(),
        }
    }

    #[test]
fn naming_rules_reject_bare_id_and_generic_names() {
    let mut doc = sample("mysql");
    // 主键应为 order_id，守着 id 就应报错
    doc.nodes[1].table.as_mut().unwrap().fields[0].name = "id".to_string();
    doc.relations[0].to.field = "id".to_string();
    let errs = validate(&doc).errors.join("\n");
    assert!(errs.contains("主键应命名为 order_id"), "{}", errs);

    // 裸 name 需要补表名前缀
    let mut doc = sample("mysql");
    doc.nodes[1].table.as_mut().unwrap().fields[1].name = "name".to_string();
    let errs = validate(&doc).errors.join("\n");
    assert!(errs.contains("应改为 order_name"), "{}", errs);
}

#[test]
fn foreign_key_without_index_is_rejected() {
    let mut doc = sample("mysql");
    // 抽掉 uid 的索引：每次关联都要全表扫
    doc.nodes[1].table.as_mut().unwrap().indexes.clear();
    let errs = validate(&doc).errors.join("\n");
    assert!(errs.contains("没有索引"), "{}", errs);
}

#[test]
fn normalize_fixes_pk_name_cascades_references_and_adds_fk_index() {
    let mut doc = sample("mysql");
    // 回归当初的不合规样本：两张表主键都是 id、还有裸 name、外键无索引
    doc.nodes[0].table.as_mut().unwrap().fields[0].name = "id".to_string();
    doc.nodes[1].table.as_mut().unwrap().fields[0].name = "id".to_string();
    doc.nodes[1].table.as_mut().unwrap().fields[1].name = "name".to_string();
    doc.nodes[1].table.as_mut().unwrap().indexes.clear();
    doc.relations[0].to.field = "id".to_string();

    let rep = normalize_naming(&mut doc);
    assert_eq!(rep.renamed_pk.len(), 2, "{:?}", rep.renamed_pk);
    assert_eq!(rep.added_index.len(), 1, "{:?}", rep.added_index);
    // 主键改名后指向它的关联端必须跟着改，否则变成悬空引用
    assert_eq!(doc.relations[0].to.field, "user_id");
    let r = validate(&doc);
    assert!(r.is_ok(), "{:?}", r.errors);
}

#[test]
fn normalize_skips_rename_when_target_name_taken() {
    let mut doc = sample("mysql");
    // user_id 已存在：主键再改名会撞名，必须跳过而不是覆盖
    doc.nodes[0].table.as_mut().unwrap().fields[0].name = "id".to_string();
    doc.nodes[0].table.as_mut().unwrap().fields.push(field("user_id", "bigint"));
    doc.relations[0].to.field = "id".to_string();
    let rep = normalize_naming(&mut doc);
    assert!(rep.renamed_pk.is_empty(), "{:?}", rep.renamed_pk);
    assert!(!rep.skipped.is_empty(), "{:?}", rep.skipped);
}

    #[test] fn sample_document_is_valid() {
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
        assert_eq!(relation_fields(&doc, "n_user"), vec!["user_id".to_string()]);
    }

    #[test]
    fn mysql_export_has_backticks_inline_comments_and_foreign_keys() {
        let sql = export_sql(&sample("mysql")).unwrap();
        assert!(sql.contains("CREATE TABLE `orders` ("), "{}", sql);
        assert!(sql.contains("`no` VARCHAR(32) NOT NULL COMMENT"), "{}", sql);
        assert!(sql.contains("`order_id` BIGINT NOT NULL AUTO_INCREMENT"), "{}", sql);
        assert!(sql.contains("PRIMARY KEY (`order_id`)"), "{}", sql);
        assert!(sql.contains("CONSTRAINT `fk_orders_uid` FOREIGN KEY (`uid`) REFERENCES `users` (`user_id`)"), "{}", sql);
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
        assert!(sql.contains("\"order_id\" BIGSERIAL"), "{}", sql);
        assert!(sql.contains("COMMENT ON TABLE \"orders\" IS '订单主表';"), "{}", sql);
        assert!(sql.contains("ALTER TABLE \"orders\" ADD CONSTRAINT \"fk_orders_uid\""), "{}", sql);
        // 索引在建表语句之外
        assert!(sql.contains("CREATE INDEX \"idx_orders_uid\" ON \"orders\" (\"uid\");"), "{}", sql);
    }

    #[test]
    fn sqlite_export_inlines_autoincrement_pk_and_lowers_comments() {
        let sql = export_sql(&sample("sqlite")).unwrap();
        assert!(sql.contains("\"order_id\" INTEGER PRIMARY KEY AUTOINCREMENT"), "{}", sql);
        assert!(!sql.contains("PRIMARY KEY (\"order_id\")"), "SQLite 单列自增主键不该再有独立 PK 行: {}", sql);
        assert!(sql.contains("FOREIGN KEY (\"uid\") REFERENCES \"users\" (\"user_id\")"), "{}", sql);
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
            name: "user_status".to_string(),
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
        assert!(sql.contains("CHECK (\"user_status\" IN ('new', 'paid'))"), "{}", sql);

        let mut mysql = sample("mysql");
        mysql.nodes[0].table.as_mut().unwrap().fields.push(DbField {
            name: "user_status".to_string(),
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

    /// **boolean 的默认值要按方言落地**：`boolean` 只是内部抽象，MySQL / SQLite 的列
    /// 其实是数值型（TINYINT(1) / INTEGER），只有 PG 才有原生 BOOLEAN。
    ///
    /// AI 很爱写 `"default": "false"`（模型最自然的写法），原样透传就会得到
    /// `TINYINT(1) NOT NULL DEFAULT false` —— 能跑，但语义与列类型对不上；
    /// 而反过来在 PG 上写 `DEFAULT 0` 是直接报错的（boolean 列不接受 integer 默认值）。
    #[test]
    fn boolean_default_is_normalized_per_dialect() {
        let mut mk = |dialect: &str, def: &str| {
            let mut doc = sample(dialect);
            doc.nodes[0]
                .table
                .as_mut()
                .unwrap()
                .fields
                .push(DbField {
                    name: "order_paid".to_string(),
                    r#type: DbLogicalType {
                        base: "boolean".to_string(),
                        ..Default::default()
                    },
                    nullable: false,
                    default: Some(def.to_string()),
                    comment: String::new(),
                    pk: false,
                    auto_increment: false,
                    unique: false,
                });
            export_sql(&doc).unwrap()
        };
        // MySQL / SQLite：列是数值型 → 0 / 1
        let sql = mk("mysql", "false");
        assert!(sql.contains("DEFAULT 0"), "MySQL 应落成 0: {}", sql);
        assert!(!sql.contains("DEFAULT false"), "{}", sql);
        let sql = mk("mysql", "true");
        assert!(sql.contains("DEFAULT 1"), "{}", sql);
        let sql = mk("sqlite", "0");
        assert!(sql.contains("DEFAULT 0"), "{}", sql);
        // PG：原生 BOOLEAN → FALSE / TRUE
        let sql = mk("postgres", "false");
        assert!(sql.contains("DEFAULT FALSE"), "PG 应保留 FALSE: {}", sql);
        let sql = mk("postgres", "true");
        assert!(sql.contains("DEFAULT TRUE"), "{}", sql);
        // 认不出来的表达式原样透传，不擅自改写语义
        let sql = mk("mysql", "next_flag");
        assert!(sql.contains("DEFAULT next_flag"), "{}", sql);
    }

    /// **字符串型默认值必须自动补引号**：`"default": "enabled"` 原样拼出去是
    /// `DEFAULT enabled` —— 语法错误，**整份建表语句都跑不起来**。
    /// 这在 AI 产出里极常见（enum 字段的默认值基本都这么写）。
    #[test]
    fn string_default_gets_quoted_automatically() {
        let mk = |dialect: &str, base: &str, def: &str| {
            let mut doc = sample(dialect);
            doc.nodes[0].table.as_mut().unwrap().fields.push(DbField {
                name: "user_status".to_string(),
                r#type: DbLogicalType {
                    base: base.to_string(),
                    values: vec!["enabled".into(), "disabled".into()],
                    ..Default::default()
                },
                nullable: false,
                default: Some(def.to_string()),
                comment: String::new(),
                pk: false,
                auto_increment: false,
                unique: false,
            });
            export_sql(&doc).unwrap()
        };
        for dialect in ["mysql", "postgres", "sqlite"] {
            let sql = mk(dialect, "enum", "enabled");
            assert!(sql.contains("DEFAULT 'enabled'"), "{}: {}", dialect, sql);
            assert!(!sql.contains("DEFAULT enabled"), "{}: {}", dialect, sql);
        }
        assert!(mk("mysql", "varchar", "zhang").contains("DEFAULT 'zhang'"));
        // 内部单引号按 SQL 规则双写
        assert!(mk("mysql", "varchar", "it's").contains("DEFAULT 'it''s'"));
        // 已经带引号的绝不能二次加引号
        let q = mk("mysql", "enum", "'enabled'");
        assert!(q.contains("DEFAULT 'enabled'"), "{}", q);
        assert!(!q.contains("''enabled''"), "{}", q);
        // 关键字 / 函数 / 数字表达式不能被当成字符串套引号
        assert!(mk("mysql", "datetime", "CURRENT_TIMESTAMP").contains("DEFAULT CURRENT_TIMESTAMP"));
        assert!(mk("mysql", "datetime", "now()").contains("DEFAULT now()"));
        assert!(mk("postgres", "uuid", "uuid_generate_v4()").contains("DEFAULT uuid_generate_v4()"));
        // 数值类型永远不加引号
        assert!(mk("mysql", "decimal", "0").contains("DEFAULT 0"));
        // 日期没引号也补（SQLite 不认裸日期字面量）
        assert!(mk("sqlite", "date", "2026-01-01").contains("DEFAULT '2026-01-01'"));
    }

    /// 校验层要提前告诉用户「这个默认值缺引号」，且与导出层**同一口径**
    /// （两处各写一份必然漂移，漂移的后果是「校验说没问题、导出的 SQL 跑不起来」）。
    #[test]
    fn validate_warns_about_missing_quotes_in_string_default() {
        let mut doc = sample("mysql");
        doc.nodes[0].table.as_mut().unwrap().fields.push(DbField {
            name: "user_status".to_string(),
            r#type: DbLogicalType {
                base: "enum".to_string(),
                values: vec!["enabled".into(), "disabled".into()],
                ..Default::default()
            },
            nullable: false,
            default: Some("enabled".to_string()),
            comment: String::new(),
            pk: false,
            auto_increment: false,
            unique: false,
        });
        let rep = validate(&doc);
        assert!(
            rep.warnings.iter().any(|w| w.contains("缺少 SQL 引号") && w.contains("user_status")),
            "{:?}",
            rep.warnings
        );
        // 导出不报错（只是补引号），所以不能因此拦住保存/导出
        assert!(export_sql(&doc).is_ok());
        // 已经带引号的不该再警告
        let mut ok = doc.clone();
        ok.nodes[0].table.as_mut().unwrap().fields.last_mut().unwrap().default =
            Some("'enabled'".to_string());
        assert!(!validate(&ok)
            .warnings
            .iter()
            .any(|w| w.contains("缺少 SQL 引号")));
    }

    /// **MySQL 禁止 TEXT/BLOB/JSON 带默认值**（报错 1101），导出会跳过并留注释。
    /// AI 给 `posture_data json NOT NULL DEFAULT '{}'` 这类顺手写法很常见，
    /// 留着就会让整份 DDL 执行失败。
    #[test]
    fn mysql_skips_literal_default_on_text_blob_json() {
        let mut doc = sample("mysql");
        for (name, base, def) in [
            ("order_body", "text", "'无'"),
            ("order_raw", "blob", "'x'"),
            ("order_extra", "json", "'{}'"),
        ] {
            doc.nodes[0].table.as_mut().unwrap().fields.push(DbField {
                name: name.to_string(),
                r#type: DbLogicalType {
                    base: base.to_string(),
                    ..Default::default()
                },
                nullable: true,
                default: Some(def.to_string()),
                comment: String::new(),
                pk: false,
                auto_increment: false,
                unique: false,
            });
        }
        let sql = export_sql(&doc).unwrap();
        assert!(!sql.contains("DEFAULT '无'"), "MySQL text 列不该带默认值: {}", sql);
        assert!(!sql.contains("DEFAULT '{}'"), "MySQL json 列不该带默认值: {}", sql);
        assert!(!sql.contains("DEFAULT 'x'"), "MySQL blob 列不该带默认值: {}", sql);
        // 但要告诉用户「你写的默认值没进去」，而不是静默吞掉
        let rep = validate(&doc);
        assert!(
            rep.warnings.iter().any(|w| w.contains("不会导出") && w.contains("order_body")),
            "{:?}",
            rep.warnings
        );
    }

    /// **PG 的 json/jsonb 列默认值必须带 `::jsonb` 标注**，否则建表报
    /// `column is of type jsonb but default expression is of type text`。
    #[test]
    fn postgres_json_default_gets_type_cast() {
        let mut doc = sample("postgres");
        doc.nodes[0].table.as_mut().unwrap().fields.push(DbField {
            name: "order_extra".to_string(),
            r#type: DbLogicalType {
                base: "json".to_string(),
                ..Default::default()
            },
            nullable: true,
            default: Some("'{}'".to_string()),
            comment: String::new(),
            pk: false,
            auto_increment: false,
            unique: false,
        });
        let sql = export_sql(&doc).unwrap();
        assert!(sql.contains("DEFAULT '{}'::jsonb"), "{}", sql);
    }

    /// **MySQL 不允许 TEXT/BLOB 列进普通 / 唯一索引**（报错 1170）→ 跳过并留注释；
    /// 但 fulltext 索引本来就该建在文本列上，不能一起跳掉。
    #[test]
    fn mysql_skips_key_on_text_but_keeps_fulltext() {
        let mk = |kind: &str| {
            let mut doc = sample("mysql");
            doc.nodes[0].table.as_mut().unwrap().fields.push(DbField {
                name: "order_body".to_string(),
                r#type: DbLogicalType {
                    base: "text".to_string(),
                    ..Default::default()
                },
                nullable: true,
                default: None,
                comment: String::new(),
                pk: false,
                auto_increment: false,
                unique: false,
            });
            doc.nodes[0].table.as_mut().unwrap().indexes.push(DbIndex {
                name: format!("idx_orders_{}", kind),
                kind: kind.to_string(),
                fields: vec!["order_body".to_string()],
            });
            doc
        };
        let doc = mk("index");
        let sql = export_sql(&doc).unwrap();
        assert!(!sql.contains("`idx_orders_index`"), "MySQL text 索引建不了: {}", sql);
        assert!(sql.contains("已跳过索引 idx_orders_index"), "要留注释说明: {}", sql);
        // 级别是 warning 不是 error：卡住导出就是死锁（用户只能改类型或指定前缀长度）
        assert!(
            validate(&doc)
                .warnings
                .iter()
                .any(|w| w.contains("MySQL 不允许 TEXT/BLOB 列进普通或唯一索引")),
            "必须提示用户这个索引没进 DDL"
        );

        let doc = mk("fulltext");
        let sql = export_sql(&doc).unwrap();
        assert!(sql.contains("FULLTEXT KEY `idx_orders_fulltext`"), "{}", sql);
    }

    /// **一张表只能有一个自增列**（MySQL 报错 1067）。
    #[test]
    fn more_than_one_auto_increment_is_rejected() {
        let mut doc = sample("mysql");
        let f = |name: &str, ai: bool| DbField {
            name: name.to_string(),
            r#type: DbLogicalType {
                base: "bigint".to_string(),
                ..Default::default()
            },
            nullable: false,
            default: None,
            comment: String::new(),
            pk: false,
            auto_increment: ai,
            unique: false,
        };
        let t = doc.nodes[0].table.as_mut().unwrap();
        t.fields.push(f("order_seq", true));
        t.fields.push(f("order_seq2", true));
        assert!(
            validate(&doc)
                .errors
                .iter()
                .any(|e| e.contains("一张表只能有一个自增列")),
            "第二个自增列必须被拦住，否则 MySQL 建表直接失败"
        );
    }

    /// **索引名必须全库唯一**（PG / SQLite 的索引建在表外，重名报 already exists），
    /// MySQL 是表内唯一 —— 按最严口径判重。
    #[test]
    fn duplicate_index_names_across_tables_are_rejected() {
        let mut doc = sample("postgres");
        doc.nodes[1]
            .table
            .as_mut()
            .unwrap()
            .indexes
            .push(DbIndex {
                name: "idx_orders_uid".to_string(),
                kind: "index".to_string(),
                fields: vec!["uid".to_string()],
            });
        assert!(
            validate(&doc)
                .errors
                .iter()
                .any(|e| e.contains("全库唯一")),
            "同名索引跨表复用会让 PG / SQLite 建表失败"
        );
    }

    /// 全文索引在 PG / SQLite 上没有对应语法，导出降级成普通索引 —— 要提示，
    /// 别让用户以为建出了全文检索。
    #[test]
    fn fulltext_outside_mysql_warns_about_downgrade() {
        let mut doc = sample("postgres");
        doc.nodes[0].table.as_mut().unwrap().indexes.push(DbIndex {
            name: "idx_orders_title".to_string(),
            kind: "fulltext".to_string(),
            fields: vec!["title".to_string()],
        });
        assert!(
            validate(&doc)
                .warnings
                .iter()
                .any(|w| w.contains("降级为普通索引")),
            "全文索引降级必须提示"
        );
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
                fields: vec![DbField { pk: true, auto_increment: true, ..field("tag_id", "bigint") }],
                indexes: vec![],
            }),
            view: None,
            ..Default::default()
        });
        doc.relations.push(DbDesignRelation {
            id: "r2".to_string(),
            name: String::new(),
            from: DbRelationEnd { node: "n_order".to_string(), field: "order_id".to_string() },
            to: DbRelationEnd { node: "n_tag".to_string(), field: "tag_id".to_string() },
            kind: "n-n".to_string(),
            on_delete: "CASCADE".to_string(),
            on_update: "CASCADE".to_string(),
            mirror: false,
        });
        let sql = export_sql(&doc).unwrap();
        assert!(sql.contains("CREATE TABLE `orders_tags` ("), "{}", sql);
        // 中间表列名 = 父表名 + 外键字段名
        assert!(sql.contains("PRIMARY KEY (`orders_order_id`, `tags_tag_id`)"), "{}", sql);
        assert!(sql.contains("REFERENCES `tags` (`tag_id`)"), "{}", sql);
    }

    /// **导出选项：关掉外键后产物里一条 FOREIGN KEY 都不该有**，但表、字段、
    /// 索引、复合主键一个都不能少 —— 用户要的是「不要约束」，不是「少建东西」。
    ///
    /// 覆盖三种方言：MySQL/SQLite 的外键内联在建表语句里，PG 走 ALTER TABLE，
    /// 少处理任一处都会漏。
    #[test]
    fn export_without_foreign_keys_drops_every_fk_clause() {
        let no_fk = ExportOptions { foreign_keys: false };
        for dialect in ["mysql", "postgres", "sqlite"] {
            let doc = sample(dialect);
            let sql = export_sql_with(&doc, no_fk).unwrap();
            assert!(!sql.contains("FOREIGN KEY"), "{} 仍有 FOREIGN KEY:\n{}", dialect, sql);
            assert!(!sql.contains("REFERENCES"), "{} 仍有 REFERENCES:\n{}", dialect, sql);
            assert!(!sql.contains("ADD CONSTRAINT"), "{} 仍有 ADD CONSTRAINT:\n{}", dialect, sql);
            // 表与索引照旧
            assert!(sql.contains("CREATE TABLE"), "{}:\n{}", dialect, sql);
            assert!(sql.contains("PRIMARY KEY"), "{}:\n{}", dialect, sql);
            assert!(sql.contains("orders"), "{}:\n{}", dialect, sql);
            // 头部必须说明「外键是有意去掉的」，否则别人以为漏了
            assert!(
                sql.contains("未包含外键约束"),
                "{} 缺说明注释:\n{}",
                dialect,
                sql
            );
            // 默认路径不受影响（同一份文档走默认选项必须还带外键）
            let with_fk = export_sql(&doc).unwrap();
            assert!(with_fk.contains("FOREIGN KEY"), "{} 默认应带外键:\n{}", dialect, with_fk);
            assert!(!with_fk.contains("未包含外键约束"), "{}", dialect);
        }
    }

    /// n-n 中间表在关掉外键时**仍然要生成**（两列 + 复合主键），只是不带 FK。
    /// 连中间表都不生成就是丢数据 —— 用户说「不要外键约束」，没说「不要中间表」。
    #[test]
    fn export_without_foreign_keys_keeps_junction_table_body() {
        let mut doc = sample("mysql");
        doc.nodes.push(DbDesignNode {
            id: "n_tag".to_string(),
            kind: "table".to_string(),
            name: "tags".to_string(),
            comment: String::new(),
            x: 0.0,
            y: 0.0,
            table: Some(DbTableBody {
                fields: vec![DbField {
                    name: "tag_id".to_string(),
                    r#type: DbLogicalType { base: "bigint".to_string(), ..Default::default() },
                    nullable: false,
                    default: None,
                    comment: String::new(),
                    pk: true,
                    auto_increment: true,
                    unique: false,
                }],
                indexes: vec![],
            }),
            view: None,
            ..Default::default()
        });
        doc.relations.push(DbDesignRelation {
            id: "r2".to_string(),
            name: String::new(),
            from: DbRelationEnd { node: "n_order".to_string(), field: "order_id".to_string() },
            to: DbRelationEnd { node: "n_tag".to_string(), field: "tag_id".to_string() },
            kind: "n-n".to_string(),
            on_delete: "CASCADE".to_string(),
            on_update: String::new(),
            mirror: false,
        });
        let sql = export_sql_with(&doc, ExportOptions { foreign_keys: false }).unwrap();
        assert!(sql.contains("CREATE TABLE `orders_tags` ("), "中间表不该消失:\n{}", sql);
        assert!(sql.contains("PRIMARY KEY (`orders_order_id`, `tags_tag_id`)"), "{}", sql);
        assert!(!sql.contains("FOREIGN KEY"), "{}", sql);
        // 去掉外键后不能留下悬空逗号
        assert!(!sql.contains(",\n)"), "中间表末尾不该有悬空逗号:\n{}", sql);
    }

    /// 命令层的 `options` 是可选的：老调用方不传时必须等价于「带外键」。
    ///
    /// 顺带钉住 serde 的实际语义（写测试时才发现容易想反）：
    /// `null` / 字段缺失 → `None`（命令里 `unwrap_or_default()` 兜底），
    /// `{}` → `Some(默认值)`，两者最终都是「带外键」。
    #[test]
    fn export_options_defaults_to_including_foreign_keys() {
        assert!(ExportOptions::default().foreign_keys);
        let missing: Option<ExportOptions> = serde_json::from_value(serde_json::json!(null)).unwrap();
        assert_eq!(missing, None);
        let empty: Option<ExportOptions> = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(empty.unwrap().foreign_keys, "空对象应等价于默认（带外键）");
        let off: ExportOptions =
            serde_json::from_value(serde_json::json!({ "foreignKeys": false })).unwrap();
        assert!(!off.foreign_keys, "camelCase 的 foreignKeys 必须被识别");
        // snake_case **不被识别**：serde 默认忽略未知字段，于是 `foreign_keys: false`
        // 会被静默当成默认（带外键）。前端 invoke 必须用 camelCase —— 这条写进测试，
        // 因为「写错字段名不报错、只是开关没生效」是最难自己发现的坑。
        let snake: ExportOptions =
            serde_json::from_value(serde_json::json!({ "foreign_keys": false })).unwrap();
        assert!(snake.foreign_keys, "snake_case 会被忽略 → 落回默认（带外键）");
        // 命令层入口的兜底：None → 默认带外键
        let doc = sample("mysql");
        assert!(super::commands::dbd_export_sql(doc.clone(), None).unwrap().contains("FOREIGN KEY"));
        assert!(!super::commands::dbd_export_sql(doc, Some(ExportOptions { foreign_keys: false }))
            .unwrap()
            .contains("FOREIGN KEY"));
    }

    /// **公共字段：导出时自动附加到每张表末尾**；表里已有同名字段时**以表自己的为准**
    /// （不合并、不覆盖、只在 SQL 头部留一行说明）。
    #[test]
    fn common_fields_are_appended_and_table_own_field_wins() {
        let mut doc = sample("mysql");
        doc.common_fields = vec![
            DbField {
                name: "created_at".into(),
                r#type: DbLogicalType { base: "datetime".into(), ..Default::default() },
                nullable: false,
                default: Some("CURRENT_TIMESTAMP".into()),
                comment: "创建时间".into(),
                pk: false,
                auto_increment: false,
                unique: false,
            },
            DbField {
                name: "is_deleted".into(),
                r#type: DbLogicalType { base: "boolean".into(), ..Default::default() },
                nullable: false,
                default: Some("false".into()),
                comment: "是否逻辑删除".into(),
                pk: false,
                auto_increment: false,
                unique: false,
            },
        ];
        let sql = export_sql(&doc).unwrap();
        // 两张表都加上了
        assert_eq!(sql.matches("`created_at` DATETIME").count(), 2, "{}", sql);
        assert_eq!(sql.matches("`is_deleted` TINYINT(1)").count(), 2, "{}", sql);
        // boolean默认值按方言归一（MySQL → 0）
        assert!(sql.contains("DEFAULT 0"), "{}", sql);
        // 追加在表自身字段之后：created_at 出现在 orders 自己的字段之后
        let users = sql.find("CREATE TABLE `users`").unwrap();
        let orders = sql.find("CREATE TABLE `orders`").unwrap();
        assert!(users < orders);
        let own_field = sql[orders..].find("`uid` BIGINT").unwrap();
        let common_field = sql[orders..].find("`created_at`").unwrap();
        assert!(own_field < common_field, "公共字段应排在表自身字段之后");

        // 现在给「订单」表自己加一个 created_at（大小写还不同）→ 以表自己的为准
        let mut mixed = doc.clone();
        let t = mixed.nodes.iter_mut().find(|n| n.name == "orders").unwrap();
        t.table.as_mut().unwrap().fields.push(DbField {
            name: "CreatedAt".into(),
            r#type: DbLogicalType { base: "bigint".into(), ..Default::default() },
            nullable: false,
            default: None,
            comment: "我这表自己用 bigint 存".into(),
            pk: false,
            auto_increment: false,
            unique: false,
        });
        let sql2 = export_sql(&mixed).unwrap();
        let seg = &sql2[sql2.find("CREATE TABLE `orders`").unwrap()..];
        let head = &seg[..seg.find("\n)").unwrap()];
        assert!(head.contains("`CreatedAt` BIGINT"), "表自己的字段必须保留: {}", head);
        assert!(!head.contains("`created_at`"), "重名的公共字段必须跳过: {}", head);
        // 跳过要留痕：用户得知道「本来会加，是表里已有的那个赢了」
        assert!(
            sql2.contains("公共字段 created_at 因表内已有同名字段而跳过"),
            "{}",
            sql2
        );
        // 另一张没冲突的表照常加
        assert_eq!(sql2.matches("`created_at` DATETIME").count(), 1, "{}", sql2);
    }

    /// 回归：`CreatedAt`（表自己的）与 `created_at`（公共字段）只差一个下划线，
    /// 但 **MySQL 列名不区分大小写** —— 两个都写进 CREATE TABLE 会
    /// `Duplicate column name` 建表失败。所以重名判定必须去下划线 + 忽略大小写。
    /// （第一版只做了忽略大小写，被这条用例抓出来。）
    #[test]
    fn common_field_name_collision_ignores_case_and_underscores() {
        use super::common_fields::same_field_name;
        for (a, b) in [
            ("created_at", "CreatedAt"),
            ("created_at", "CREATED_AT"),
            ("created_at", " created_at "),
            ("is_deleted", "is-deleted"),
            ("remark", "Remark"),
        ] {
            assert!(same_field_name(a, b), "{a} 与 {b} 应算同一个字段");
        }
        for (a, b) in [("created_at", "updated_at"), ("remark", "remarks"), ("", "___")] {
            assert!(!same_field_name(a, b), "{a} 与 {b} 不该算同一个字段");
        }
    }

    /// 不启用公共字段时**完全不碰**用户的字段（缺省 = 空数组，向后兼容）。
    #[test]
    fn no_common_fields_means_no_change() {
        let doc = sample("mysql");
        assert!(doc.common_fields.is_empty());
        let sql = export_sql(&doc).unwrap();
        assert!(!sql.contains("created_at"), "{}", sql);
        assert!(!sql.contains("is_deleted"), "{}", sql);
        // 结构说明里也不该出现这一节
        let outline = export_outline(&doc);
        assert!(!outline.contains("公共字段"), "{}", outline);
    }

    /// 结构说明要单列公共字段：它是文档级配置，不写读者会以为导出时凭空多了几列。
    #[test]
    fn outline_lists_common_fields() {
        let mut doc = sample("postgres");
        doc.common_fields = vec![DbField {
            name: "remark".into(),
            r#type: DbLogicalType {
                base: "varchar".into(),
                length: Some(500),
                ..Default::default()
            },
            nullable: true,
            default: None,
            comment: "备注".into(),
            pk: false,
            auto_increment: false,
            unique: false,
        }];
        let md = export_outline(&doc);
        assert!(md.contains("## 公共字段"), "{}", md);
        assert!(md.contains("`remark` varchar(500)"), "{}", md);
        assert!(md.contains("以表自己的为准"), "{}", md);
    }

    /// 公共字段的**轻量**校验：类型越界/ 名称空 / 启用项重名要报错；
    /// 但**不套用**表级规则（「通用裸名」「外键索引」），否则用户统一叫 `remark` 会被拦。
    #[test]
    fn common_fields_get_light_validation_only() {
        let mut doc = sample("mysql");
        doc.common_fields = vec![DbField {
            name: "remark".into(),
            r#type: DbLogicalType { base: "varchar".into(), length: Some(500), ..Default::default() },
            nullable: true,
            default: None,
            comment: String::new(),
            pk: false,
            auto_increment: false,
            unique: false,
        }];
        let rep = validate(&doc);
        assert!(rep.errors.is_empty(), "裸名 remark 不该被拦: {:?}", rep.errors);

        doc.common_fields.push(DbField {
            name: "bad_type".into(),
            r#type: DbLogicalType { base: "不存在的类型".into(), ..Default::default() },
            ..Default::default()
        });
        doc.common_fields.push(DbField {
            name: "  ".into(),
            r#type: DbLogicalType { base: "int".into(), ..Default::default() },
            ..Default::default()
        });
        doc.common_fields.push(DbField {
            name: "REMARK".into(),
            r#type: DbLogicalType { base: "int".into(), ..Default::default() },
            ..Default::default()
        });
        let rep = validate(&doc);
        assert!(rep.errors.iter().any(|e| e.contains("不在白名单")), "{:?}", rep.errors);
        assert!(rep.errors.iter().any(|e| e.contains("字段名为空")), "{:?}", rep.errors);
        assert!(rep.errors.iter().any(|e| e.contains("公共字段名重复")), "{:?}", rep.errors);
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
            ..Default::default()
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
            ..Default::default()
        });
        let r = validate(&doc);
        assert!(r.errors.iter().any(|e| e.contains("不在允许列表")), "{:?}", r.errors);
        assert!(export_sql(&doc).is_err());
        let _ = ty("decimal");
    }
}
