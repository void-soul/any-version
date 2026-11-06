use super::*;

const MYSQL_DUMP: &str = r#"
    -- 订单库导出
    CREATE TABLE `users` (
      `id` BIGINT NOT NULL AUTO_INCREMENT COMMENT '主键',
      `name` VARCHAR(64) NOT NULL COMMENT '昵称',
      `status` enum('new','paid') NOT NULL DEFAULT 'new',
      PRIMARY KEY (`id`),
      UNIQUE KEY `uk_users_name` (`name`)
    ) ENGINE=InnoDB;

    CREATE TABLE `orders` (
      `id` BIGINT NOT NULL AUTO_INCREMENT,
      `no` varchar(32) NOT NULL,
      `uid` BIGINT NOT NULL,
      `amount` DECIMAL(12,2) NOT NULL DEFAULT 0,
      KEY `idx_orders_uid` (`uid`),
      CONSTRAINT `fk_orders_uid` FOREIGN KEY (`uid`) REFERENCES `users` (`id`) ON DELETE CASCADE ON UPDATE CASCADE
    );

    CREATE UNIQUE INDEX `uk_orders_no` ON `orders` (`no`);

    CREATE VIEW `v_paid` AS SELECT * FROM orders WHERE amount > 0;
    "#;

#[test]
fn mysql_ddl_becomes_tables_indexes_and_relations() {
    let doc = reverse_ddl(MYSQL_DUMP, "mysql", "订单库").unwrap();
    assert_eq!(doc.nodes.len(), 3, "2 表 + 1 视图: {:?}", doc.nodes);
    let users = doc.nodes.iter().find(|n| n.name == "users").unwrap();
    let fields = users.table.as_ref().unwrap().fields.clone();
    assert_eq!(fields.len(), 3);
    assert_eq!(fields[0].r#type.base, "bigint");
    assert!(fields[0].pk && fields[0].auto_increment && !fields[0].nullable);
    assert_eq!(fields[0].comment, "主键");
    assert_eq!(fields[1].r#type.length, Some(64));
    assert_eq!(fields[2].r#type.base, "enum");
    assert_eq!(
        fields[2].r#type.values,
        vec!["new".to_string(), "paid".to_string()]
    );
    // UNIQUE KEY 进索引表
    let idx = users.table.as_ref().unwrap().indexes.clone();
    assert_eq!(idx.len(), 1);
    assert_eq!(idx[0].kind, "unique");
    // 解析完统一做过命名规范化：裸名 name → user_name
    assert_eq!(idx[0].fields, vec!["user_name".to_string()]);

    let orders = doc.nodes.iter().find(|n| n.name == "orders").unwrap();
    let of = &orders.table.as_ref().unwrap().fields;
    assert_eq!(of[3].r#type.base, "decimal");
    assert_eq!(of[3].r#type.precision, Some(12));
    assert_eq!(of[3].default.as_deref(), Some("0"));
    // CREATE UNIQUE INDEX 也要收进来
    assert!(orders
        .table
        .as_ref()
        .unwrap()
        .indexes
        .iter()
        .any(|i| i.name == "uk_orders_no" && i.kind == "unique"));

    // 外键：从子表指向父表
    assert_eq!(doc.relations.len(), 1);
    let r = &doc.relations[0];
    assert_eq!(doc.node(&r.from.node).unwrap().name, "orders");
    assert_eq!(r.from.field, "uid");
    assert_eq!(doc.node(&r.to.node).unwrap().name, "users");
    assert_eq!(r.to.field, "user_id"); // 已规范化：主键 id → user_id
    assert_eq!(r.on_delete, "CASCADE");

    // 视图
    let v = doc.nodes.iter().find(|n| n.kind == "view").unwrap();
    assert_eq!(v.name, "v_paid");
    assert!(v.view.as_ref().unwrap().sql.contains("SELECT"));
}

#[test]
fn reverse_result_always_passes_validation() {
    let doc = reverse_ddl(MYSQL_DUMP, "mysql", "x").unwrap();
    let report = validate(&doc);
    assert!(report.is_ok(), "{:?}", report.errors);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn postgres_quoted_identifiers_and_alter_table_fk() {
    let ddl = r#"
        CREATE TABLE "public"."team" (
            "id" bigserial PRIMARY KEY,
            "title" varchar(80) NOT NULL
        );
        CREATE TABLE "member" (
            "id" bigint NOT NULL,
            "team_id" bigint NOT NULL
        );
        ALTER TABLE "member" ADD CONSTRAINT "fk_member_team" FOREIGN KEY ("team_id") REFERENCES "public"."team"("id") ON DELETE SET NULL;
        "#;
    let doc = reverse_ddl(ddl, "postgres", "团队").unwrap();
    let names: Vec<String> = doc.nodes.iter().map(|n| n.name.clone()).collect();
    assert!(names.contains(&"team".to_string()), "{:?}", names);
    let team = doc.nodes.iter().find(|n| n.name == "team").unwrap();
    assert!(team.table.as_ref().unwrap().fields[0].auto_increment);
    assert_eq!(doc.relations.len(), 1, "{:?}", doc.relations);
    assert_eq!(doc.relations[0].on_delete, "SET NULL");
}

#[test]
fn navicat_style_index_columns_with_asc_and_using_btree_are_parsed() {
    // Navicat 导出的索引列带排序方向，末尾还跟 USING BTREE：
    //   UNIQUE KEY `idx_51`(`openid` ASC, `user_status` ASC) USING BTREE
    // 踩过的坑：只脱反引号会留下「`openid` ASC」这种列名，校验判为引用不存在字段，
    // 整个逆向 DDL 直接失败（真实 dump 里 226 处一起报）。
    let ddl = r#"
        CREATE TABLE `base_user` (
          `openid` varchar(32) NOT NULL,
          `user_status` char(1) NULL DEFAULT '0',
          PRIMARY KEY (`openid`) USING BTREE,
          UNIQUE KEY `idx_51`(`openid` ASC, `user_status` ASC) USING BTREE
        ) ENGINE = InnoDB CHARACTER SET = utf8mb4;
        CREATE INDEX `idx_52` ON `base_user` (`user_status` DESC);
        "#;
    let doc = reverse_ddl(ddl, "mysql", "t").unwrap();
    let t = doc.nodes[0].table.as_ref().unwrap();
    let idx = t
        .indexes
        .iter()
        .find(|i| i.name == "idx_51")
        .expect("idx_51");
    // openid 是本表主键且属裸名 → 规范化为 base_user_id
    assert_eq!(
        idx.fields,
        vec!["base_user_id".to_string(), "user_status".to_string()]
    );
    assert_eq!(idx.kind, "unique");
    let idx2 = t
        .indexes
        .iter()
        .find(|i| i.name == "idx_52")
        .expect("idx_52");
    assert_eq!(idx2.fields, vec!["user_status".to_string()]);
}

#[test]
fn comments_are_stripped_and_quotes_do_not_split_statements() {
    let ddl = r#"
        -- CREATE TABLE fake (x int);
        /* CREATE TABLE fake2 (x int); */
        CREATE TABLE t (
            a VARCHAR(10) DEFAULT 'semi;colon',
            b INT
        );
        "#;
    let doc = reverse_ddl(ddl, "mysql", "t").unwrap();
    assert_eq!(
        doc.nodes.len(),
        1,
        "注释里的建表不该被解析: {:?}",
        doc.nodes.len()
    );
    assert_eq!(doc.nodes[0].table.as_ref().unwrap().fields.len(), 2);
    // 引号要**一起存**：设计文件里存的是 SQL 字面量，剥掉引号再导出就会得到
    // `DEFAULT semi;colon`（语法错误，而且分号还会把语句截断）。
    assert_eq!(
        doc.nodes[0].table.as_ref().unwrap().fields[0]
            .default
            .as_deref(),
        Some("'semi;colon'")
    );
}

#[test]
fn sqlite_db_is_reversed_with_fields_indexes_and_foreign_keys() {
    let dir = std::env::temp_dir().join(format!("kira-rev-sqlite-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("demo.db");
    {
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE users (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     name VARCHAR(64) NOT NULL,
                     created_at DATETIME
                 );
                 CREATE UNIQUE INDEX uk_users_name ON users(name);
                 CREATE TABLE orders (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     uid INTEGER NOT NULL REFERENCES users(id),
                     amount NUMERIC(12,2)
                 );
                 CREATE VIEW v_orders AS SELECT * FROM orders;",
        )
        .unwrap();
    }

    let doc = reverse_sqlite(&db.to_string_lossy(), "演示库").unwrap();
    assert_eq!(doc.dialect, "sqlite");
    let names: Vec<String> = doc.nodes.iter().map(|n| n.name.clone()).collect();
    assert!(names.contains(&"users".to_string()));
    assert!(names.contains(&"orders".to_string()));
    assert!(names.contains(&"v_orders".to_string()));

    let users = doc.nodes.iter().find(|n| n.name == "users").unwrap();
    let f = users.table.as_ref().unwrap().fields.clone();
    assert!(f[0].auto_increment && f[0].pk);
    assert_eq!(f[1].r#type.length, Some(64));
    assert!(users
        .table
        .as_ref()
        .unwrap()
        .indexes
        .iter()
        .any(|i| i.name == "uk_users_name" && i.kind == "unique"));

    // 内联 REFERENCES 与外键 pragma 都应还原成关联
    assert_eq!(doc.relations.len(), 1, "{:?}", doc.relations);
    assert_eq!(doc.relations[0].from.field, "uid");

    let report = validate(&doc);
    assert!(report.is_ok(), "{:?}", report.errors);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sqlite_reverse_rejects_non_sqlite_files() {
    let dir = std::env::temp_dir().join(format!("kira-rev-bad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("not.db");
    std::fs::write(&bad, b"this is not a sqlite file").unwrap();
    assert!(reverse_sqlite(&bad.to_string_lossy(), "x").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 用户给的 MySQL 片段：两条命名外键 + 一条命名 CHECK。
/// 故意保留第 2 条 FK 与 CHECK 之间**缺失的逗号**，作为容错回归样本。
const CONSTRAINT_DDL: &str = r#"
    CREATE TABLE `parents` (
      `parent_id` BIGINT NOT NULL AUTO_INCREMENT,
      PRIMARY KEY (`parent_id`)
    );
    CREATE TABLE `sports` (
      `sport_id` BIGINT NOT NULL AUTO_INCREMENT,
      PRIMARY KEY (`sport_id`)
    );
    CREATE TABLE `certificates` (
      `certificate_id` BIGINT NOT NULL AUTO_INCREMENT,
      `member_id` BIGINT NOT NULL,
      `sport_id` BIGINT NOT NULL,
      `amount_cents` BIGINT NOT NULL,
      PRIMARY KEY (`certificate_id`),
      KEY `idx_certificates_member` (`member_id`),
      KEY `idx_certificates_sport` (`sport_id`),
      CONSTRAINT `fk_certificates_member` FOREIGN KEY (`member_id`) REFERENCES `parents` (`parent_id`) ON DELETE CASCADE ON UPDATE RESTRICT,
      CONSTRAINT `fk_certificates_sport` FOREIGN KEY (`sport_id`) REFERENCES `sports` (`sport_id`) ON DELETE RESTRICT ON UPDATE RESTRICT
      CONSTRAINT `commerce_orders_chk_1` CHECK (`amount_cents` >= 0)
    );
    "#;

#[test]
fn named_fk_constraints_keep_names_and_actions() {
    let doc = reverse_ddl(CONSTRAINT_DDL, "mysql", "证书库").unwrap();
    assert_eq!(doc.relations.len(), 2, "{:?}", doc.relations);
    let member = doc.relations.iter().find(|r| r.from.field == "member_id").unwrap();
    assert_eq!(member.constraint_name, "fk_certificates_member");
    assert_eq!(member.on_delete, "CASCADE");
    assert_eq!(member.on_update, "RESTRICT");
    let sport = doc.relations.iter().find(|r| r.from.field == "sport_id").unwrap();
    assert_eq!(sport.constraint_name, "fk_certificates_sport");
    assert_eq!(sport.on_delete, "RESTRICT");
    assert_eq!(sport.on_update, "RESTRICT");
}

#[test]
fn named_check_survives_missing_comma_after_fk() {
    let doc = reverse_ddl(CONSTRAINT_DDL, "mysql", "证书库").unwrap();
    let table = doc.nodes.iter().find(|n| n.name == "certificates").unwrap().table.as_ref().unwrap();
    assert_eq!(table.checks.len(), 1, "缺逗号不该把 CHECK 吞掉: {:?}", table.checks);
    assert_eq!(table.checks[0].name, "commerce_orders_chk_1");
    assert!(table.checks[0].expression.contains("amount_cents"), "{:?}", table.checks);
}

#[test]
fn no_action_is_not_truncated_to_no() {
    let ddl = r#"
        CREATE TABLE parents (parent_id BIGINT PRIMARY KEY);
        CREATE TABLE children (
            child_id BIGINT PRIMARY KEY,
            parent_id BIGINT NOT NULL,
            KEY idx_children_parent (parent_id),
            CONSTRAINT fk_children_parent FOREIGN KEY (parent_id) REFERENCES parents (parent_id)
                ON DELETE NO ACTION ON UPDATE NO ACTION
        );
        "#;
    let doc = reverse_ddl(ddl, "mysql", "x").unwrap();
    let r = &doc.relations[0];
    assert_eq!(r.on_delete, "NO ACTION");
    assert_eq!(r.on_update, "NO ACTION");
    assert_eq!(r.constraint_name, "fk_children_parent");
}

#[test]
fn constraint_names_survive_export_then_reverse() {
    let doc = reverse_ddl(CONSTRAINT_DDL, "mysql", "证书库").unwrap();
    let sql = super::super::export::export_sql(&doc).unwrap();
    assert!(sql.contains("CONSTRAINT `fk_certificates_member`"), "{}", sql);
    assert!(sql.contains("CONSTRAINT `commerce_orders_chk_1` CHECK"), "{}", sql);
    let again = reverse_ddl(&sql, "mysql", "roundtrip").unwrap();
    let names: Vec<&str> = again.relations.iter().map(|r| r.constraint_name.as_str()).collect();
    assert!(names.contains(&"fk_certificates_member"), "{:?}", names);
    assert!(names.contains(&"fk_certificates_sport"), "{:?}", names);
    let table = again.nodes.iter().find(|n| n.name == "certificates").unwrap().table.as_ref().unwrap();
    assert!(table.checks.iter().any(|c| c.name == "commerce_orders_chk_1"), "{:?}", table.checks);
}

/// 守护：容错不能把正常 DDL 切出额外的 CHECK 或关系。
#[test]
fn well_formed_ddl_is_not_changed_by_comma_recovery() {
    let doc = reverse_ddl(MYSQL_DUMP, "mysql", "订单库").unwrap();
    assert_eq!(doc.relations.len(), 1, "{:?}", doc.relations);
    assert_eq!(doc.relations[0].constraint_name, "fk_orders_uid");
    let orders = doc.nodes.iter().find(|n| n.name == "orders").unwrap().table.as_ref().unwrap();
    assert!(orders.checks.is_empty(), "正常 DDL 不该凭空多出 CHECK: {:?}", orders.checks);
}

#[test]
fn mysql_check_constraints_and_literal_comment_markers_survive_roundtrip() {
    let sql = r#"
        CREATE TABLE sports (
            id BIGINT PRIMARY KEY,
            theme_color VARCHAR(16) DEFAULT '#21685c',
            sport_label VARCHAR(100) DEFAULT 'a--b/*c*/#d;it\'s,ok',
            quantity INT,
            CONSTRAINT sports_positive CHECK (`quantity` > 0),
            CHECK (`id` > 0),
            KEY order_id (id)
        );
        CREATE TABLE orders (id BIGINT PRIMARY KEY, KEY order_id (id));
    "#;
    let doc = reverse_ddl(sql, "mysql", "fixture").unwrap();
    assert_eq!(doc.nodes.len(), 2);
    let table = doc.nodes.iter().find(|node| node.name == "sports").unwrap().table.as_ref().unwrap();
    assert_eq!(table.fields.iter().filter(|field| field.name == "sport_id").count(), 1);
    assert_eq!(table.fields.iter().find(|field| field.name == "theme_color").unwrap().default.as_deref(), Some("'#21685c'"));
    assert_eq!(table.checks.len(), 2);
    assert_eq!(table.fields.iter().find(|field| field.name == "sport_label").unwrap().default.as_deref(), Some(r"'a--b/*c*/#d;it\'s,ok'"));
    assert!(table.checks.iter().any(|check| check.name == "sports_positive" && check.expression == "`quantity` > 0"));
    assert!(table.checks.iter().any(|check| check.expression == "`sport_id` > 0"));
    assert_eq!(table.indexes.len(), 1, "CHECK must not become an index");
    let saved = serde_json::to_string(&doc).unwrap();
    let restored = serde_json::from_str(&saved).unwrap();
    let exported = super::super::export::export_sql(&restored).unwrap();
    assert!(exported.contains("CONSTRAINT `sports_positive` CHECK (`quantity` > 0)"));
    let again = reverse_ddl(&exported, "mysql", "roundtrip").unwrap();
    assert_eq!(again.nodes[0].table.as_ref().unwrap().checks, table.checks);
    for dialect in ["postgres", "sqlite"] {
        let mut other = doc.clone(); other.dialect = dialect.into();
        assert!(validate(&other).errors.iter().any(|error| error.contains("全库唯一")));
    }
    let mut duplicate = doc;
    let body = duplicate.nodes[0].table.as_mut().unwrap();
    body.indexes.push(body.indexes[0].clone());
    assert!(validate(&duplicate).errors.iter().any(|error| error.contains("表内唯一")));
}

#[test]
fn check_renaming_preserves_literals_and_functions() {
    let mut doc = reverse_ddl("CREATE TABLE items (id INT PRIMARY KEY, status VARCHAR(20), CHECK (status <> 'status' AND length(status) > 0));", "mysql", "fixture").unwrap();
    let id = doc.nodes[0].id.clone();
    let field = doc.nodes[0].table.as_ref().unwrap().fields.iter().find(|field| field.name.ends_with("_status")).unwrap().name.clone();
    super::super::store::rename_field(&mut doc, &id, &field, "item_state").unwrap();
    let expression = &doc.nodes[0].table.as_ref().unwrap().checks[0].expression;
    assert_eq!(expression, "item_state <> 'status' AND length(item_state) > 0");
}

#[test]
#[ignore = "requires an explicitly supplied local DDL; never reads a user's file by default"]
fn local_mysql_ddl_fixture() {
    let path = std::env::var("KIRA_DBD_DDL_FIXTURE").expect("explicit fixture path");
    let text = std::fs::read_to_string(path).unwrap();
    let doc = reverse_ddl(&text, "mysql", "local fixture").unwrap();
    let checks: usize = doc.nodes.iter().filter_map(|node| node.table.as_ref()).map(|table| table.checks.len()).sum();
    let exported = super::super::export::export_sql(&doc).unwrap();
    let roundtrip = reverse_ddl(&exported, "mysql", "roundtrip").unwrap();
    let roundtrip_checks: usize = roundtrip.nodes.iter().filter_map(|node| node.table.as_ref()).map(|table| table.checks.len()).sum();
    assert_eq!(roundtrip.nodes.len(), doc.nodes.len());
    assert_eq!(roundtrip_checks, checks);
    assert!(checks > 0);
    println!("tables/views={}, checks={}, exported_bytes={}, roundtrip=passed", doc.nodes.len(), checks, exported.len());
}
