// 公共字段：一份「所有表都该有的字段」的清单 + 导出时的合并规则。
//
// 需求（用户原话）：「理论上每个表都应该有创建修改删除时间和人，以及备注、是否逻辑删除，
// 这些字段的名称应该统一，属于所有表都应该有的字段。但这样未免太武断」——
// 所以做成**可勾选的配置**：内置一份推荐清单，用户挑自己要的，名字统一在一处改。
//
// **为什么不写进每张表的 fields**：
//   · 那是「每张表自己的字段」，和「全局统一附加」性质不同 —— 混进去以后，
//     删一个公共字段要改 30 张表，改名要改 30 张表，还分不清哪些是配置带来的；
//   · 画布上也不该显示它们（节点卡片只显示「这张表自己的列」才有信息量）。
// 所以：配置存文档顶层，**导出时才追加**，并且**表里已有同名字段时以表自己的为准**
// （不合并、不覆盖、不报错 —— 大部分人本来就把这类字段叫 created_at，那是对的做法）。
//
// 文件里存的是**完整定义**（`DbField[]`，自包含），内置清单只是「新建配置时的初始值」。

use super::models::{DbField, DbLogicalType};

/// 内置推荐清单（新建配置时的默认值，用户可勾选 / 改名 / 改默认值）。
///
/// 字段名都带下划线 → 不会触发「通用裸名必须带表前缀」那条 error 级校验。
/// `remark` 是唯一的例外（它在裸名黑名单里），但公共字段不参与命名校验，见 `store::validate`。
pub fn builtin_common_fields() -> Vec<DbField> {
    let datetime = || DbLogicalType { base: "datetime".into(), ..Default::default() };
    let bigint = || DbLogicalType { base: "bigint".into(), ..Default::default() };
    let f = |name: &str, r#type: DbLogicalType, nullable: bool, default: Option<&str>, comment: &str| DbField {
        name: name.to_string(),
        r#type,
        nullable,
        default: default.map(|s| s.to_string()),
        comment: comment.to_string(),
        pk: false,
        auto_increment: false,
        unique: false,
    };
    vec![
        f("created_at", datetime(), false, Some("CURRENT_TIMESTAMP"), "创建时间"),
        f("updated_at", datetime(), false, Some("CURRENT_TIMESTAMP"), "修改时间"),
        f("deleted_at", datetime(), true, None, "删除时间（逻辑删除留痕，可空）"),
        f("created_by", bigint(), true, None, "创建人（用户 id，可空）"),
        f("updated_by", bigint(), true, None, "修改人（用户 id，可空）"),
        f("deleted_by", bigint(), true, None, "删除人（用户 id，可空）"),
        f(
            "remark",
            DbLogicalType { base: "varchar".into(), length: Some(500), ..Default::default() },
            true,
            None,
            "备注",
        ),
        f(
            "is_deleted",
            DbLogicalType { base: "boolean".into(), ..Default::default() },
            false,
            // 按字符串写：导出层会按方言归一（MySQL / SQLite → 0，PG → FALSE）
            Some("false"),
            "是否逻辑删除",
        ),
    ]
}

/// 字段名是否「算同一个」：**去下划线 + 忽略大小写**。
///
/// 为什么这么松（第一版只做了 trim + 忽略大小写，漏了一档就被抓出来了）：
///   · `CreatedAt` vs `created_at` 只差一个下划线，在人眼里是同一个东西；
///   · 而 **MySQL 的列名不区分大小写** —— 两个都写进CREATE TABLE 会直接
///     `Duplicate column name` 建表失败。所以「宽松比对」在这里不是宽容，是**防建表失败**。
///
/// PG 会把未加引号的标识符折叠成小写（`CreatedAt` → `createdat`），所以它俩能共存；
/// 但同一个导出文件要能在三种方言下都跑得起来，就不能指望这个差异。
pub fn same_field_name(a: &str, b: &str) -> bool {
    let norm = |s: &str| -> String {
        s.trim()
            .to_ascii_lowercase()
            .chars()
            .filter(|c| !matches!(c, '_' | '-' | ' '))
            .collect()
    };
    let (na, nb) = (norm(a), norm(b));
    // 全是下划线/空格的名字（"___"）归一后成空串，两个空串不该被判成同一个
    !na.is_empty() && na == nb
}

/// 这张表要追加哪些公共字段：**配置里启用的、且表里没有同名的**。
///
/// 返回 `(要追加的, 因重名被跳过的)`：后者会被写进导出 SQL 的头部注释 ——
/// 拿到 SQL 的人需要知道「这个字段本来是有的，是表里已有的那个赢了」。
pub fn common_fields_for(common: &[DbField], table_fields: &[DbField]) -> (Vec<DbField>, Vec<DbField>) {
    let mut add = Vec::new();
    let mut skipped = Vec::new();
    for c in common {
        if table_fields.iter().any(|f| same_field_name(&f.name, &c.name)) {
            skipped.push(c.clone());
        } else {
            add.push(c.clone());
        }
    }
    (add, skipped)
}
