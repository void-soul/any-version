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
