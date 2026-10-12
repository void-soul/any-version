// 设计文档的校验与结构维护（改名级联、关联字段反查）。
//
// 校验集中在这一处：导入文件、保存文档、导出 SQL 前都走 `validate`。
// 输出两级 —— errors 阻断，warnings 放行但要提示（与思维导图导入端同构）。
use super::models::*;

/// 校验结果：`errors` 阻断保存/导出，`warnings` 放行但要提示。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct ValidationReport {
    /// 阻断级：文档不能保存 / 不能导出
    pub errors: Vec<String>,
    /// 提示级：可以保存，但用户应该知道
    pub warnings: Vec<String>,
}

impl ValidationReport {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// 逻辑类型白名单（与 docs/design/db-designer.md §2.3 一致）
const BASE_TYPES: [&str; 18] = [
    "int", "bigint", "smallint", "tinyint", "decimal", "float", "double", "char", "varchar",
    "text", "date", "time", "datetime", "timestamp", "boolean", "json", "uuid", "blob",
];

pub fn validate(doc: &DbDesignDocument) -> ValidationReport {
    let mut r = ValidationReport::default();

    if !DIALECTS.contains(&doc.dialect.as_str()) {
        r.errors.push(format!("未知方言 {}（可选：{}）", doc.dialect, DIALECTS.join(" / ")));
    }

    // ── 节点 ──
    let mut seen_names: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for n in &doc.nodes {
        if n.name.trim().is_empty() {
            r.errors.push(format!("节点 {} 没有名称", n.id));
        }
        *seen_names.entry(n.name.to_lowercase()).or_insert(0) += 1;
        if !NODE_KINDS.contains(&n.kind.as_str()) {
            r.errors.push(format!("节点「{}」的 kind={} 不在允许列表", n.name, n.kind));
        }
        match n.kind.as_str() {
            "table" => {
                if n.table.is_none() {
                    r.errors.push(format!("表节点「{}」缺 table 内容", n.name));
                    continue;
                }
                let t = n.table.as_ref().unwrap();
                if t.fields.is_empty() {
                    r.errors.push(format!("表「{}」一个字段都没有", n.name));
                }
                let mut field_names: std::collections::HashMap<String, usize> =
                    std::collections::HashMap::new();
                for f in &t.fields {
                    *field_names.entry(f.name.to_lowercase()).or_insert(0) += 1;
                    if f.name.trim().is_empty() {
                        r.errors.push(format!("表「{}」有未命名字段", n.name));
                        continue;
                    }
                    if !BASE_TYPES.contains(&f.r#type.base.as_str()) && f.r#type.base != "enum" {
                        r.errors.push(format!(
                            "表「{}」字段「{}」的类型 {} 不在白名单",
                            n.name, f.name, f.r#type.base
                        ));
                    }
                    if f.r#type.base == "enum" && f.r#type.values.is_empty() {
                        r.errors.push(format!(
                            "表「{}」字段「{}」是 enum 但没给取值",
                            n.name, f.name
                        ));
                    }
                    if f.r#type.base == "varchar" && f.r#type.length.is_none() {
                        r.warnings.push(format!(
                            "表「{}」字段「{}」的 varchar 没给长度（导出时按 255 兜底）",
                            n.name, f.name
                        ));
                    }
                    if f.r#type.base == "decimal" && f.r#type.precision.is_none() {
                        r.warnings.push(format!(
                            "表「{}」字段「{}」的 decimal 没给精度（导出时按 10,2 兜底）",
                            n.name, f.name
                        ));
                    }
                    if f.pk && f.nullable {
                        r.errors.push(format!(
                            "表「{}」字段「{}」是主键却允许 NULL",
                            n.name, f.name
                        ));
                    }
                }
                for (name, cnt) in field_names {
                    if cnt > 1 {
                        r.errors.push(format!("表「{}」字段「{}」重复 {}", n.name, name, cnt));
                    }
                }
                let pks = t.fields.iter().filter(|f| f.pk).count();
                if pks > 1 {
                    // 复合主键是合法需求，但连线引用它时要小心 —— 提示而不是拦
                    r.warnings.push(format!("表「{}」有 {} 个主键字段（复合主键）", n.name, pks));
                }
                for idx in &t.indexes {
                    if !INDEX_KINDS.contains(&idx.kind.as_str()) {
                        r.errors.push(format!(
                            "表「{}」索引「{}」的 kind={} 不在允许列表",
                            n.name, idx.name, idx.kind
                        ));
                    }
                    if idx.fields.is_empty() {
                        r.errors.push(format!("表「{}」索引「{}」没有字段", n.name, idx.name));
                    }
                    for f in &idx.fields {
                        if !t.fields.iter().any(|x| x.name == *f) {
                            r.errors.push(format!(
                                "表「{}」索引「{}」引用了不存在的字段「{}」",
                                n.name, idx.name, f
                            ));
                        }
                    }
                }
            }
            "view" => {
                let sql = n.view.as_ref().map(|v| v.sql.trim()).unwrap_or("");
                if sql.is_empty() {
                    r.errors.push(format!("视图「{}」没有 SQL", n.name));
                }
            }
            _ => {}
        }
    }
    for (name, cnt) in seen_names {
        if cnt > 1 {
            r.errors.push(format!("节点名「{}」重复 {}", name, cnt));
        }
    }

    // ── 关联 ──
    for rel in &doc.relations {
        if !RELATION_KINDS.contains(&rel.kind.as_str()) {
            r.errors.push(format!("关联「{}」的 kind={} 不在允许列表", rel.id, rel.kind));
        }
        for (end, side) in [(&rel.from, "from"), (&rel.to, "to")] {
            let Some(node) = doc.node(&end.node) else {
                r.errors.push(format!(
                    "关联「{}」的 {} 指向不存在的节点 {}",
                    rel.id, side, end.node
                ));
                continue;
            };
            if node.kind != "table" {
                r.errors.push(format!(
                    "关联「{}」的 {} 指向「{}」，但它不是表（{}）",
                    rel.id, side, node.name, node.kind
                ));
                continue;
            }
            if !doc.fields_of(&end.node).iter().any(|f| f.name == end.field) {
                r.errors.push(format!(
                    "关联「{}」的 {} 引用了表「{}」里不存在的字段「{}」",
                    rel.id, side, node.name, end.field
                ));
            }
        }
    }


// ── 命名规范与外键索引 ──
    validate_naming(doc, &mut r);

    r
}

/// 通用裸名字段黑名单：这些词单独出现时无法区分属于哪张表，
/// 多表共存时必然出现 `name`/`status` 这类同名列，联表查询时要靠表名硬猜。
///
/// 判定口径是「不含下划线的单词」才查表，所以 `user_name`、`order_status` 天然豁免；
/// `created_at` 这类审计字段因为带下划线也不受影响。
const GENERIC_FIELD_NAMES: [&str; 42] = [
    "id", "name", "title", "label", "desc", "description", "price", "amount", "status", "type",
    "code", "value", "content", "text", "url", "uri", "phone", "email", "address", "age",
    "sex", "gender", "count", "num", "number", "total", "size", "weight", "height", "color",
    "comment", "note", "remark", "date", "time", "year", "month", "day", "sort", "order", "flag",
    "enabled",
];

/// 表名去复数，得到「实体名」——主键与字段前缀都用它。
/// 只覆盖常见英文复数形态，遇到不规则名词（person/people）原样返回，
/// 那时校验会提示期望值，用户按实际语义调整即可。
fn singularize(name: &str) -> String {
    let n = name.trim();
    if n.is_empty() {
        return String::new();
    }
    if let Some(stem) = n.strip_suffix("ies") {
        return format!("{}y", stem);
    }
    if let Some(stem) = n.strip_suffix("ses") {
        // addresses → address、statuses → status
        return stem.to_string();
    }
    if let Some(stem) = n.strip_suffix('s') {
        if !stem.ends_with('s') {
            return stem.to_string();
        }
    }
    n.to_string()
}

/// 主键 / 字段命名规范 + 外键索引覆盖检查。
///
/// 单独成函数而不是塞进 `validate` 主干：这三组规则与结构性校验关注点不同，
/// 主干已经够长了；且它们是「规范性建议来源」，未来要按严重程度分级时会更好拆。
fn validate_naming(doc: &DbDesignDocument, r: &mut ValidationReport) {
    for n in &doc.nodes {
        if n.kind != "table" {
            continue;
        }
        let Some(t) = n.table.as_ref() else { continue };
        let entity = singularize(&n.name);
        let expect_pk = format!("{}_id", entity);
        let pks: Vec<&DbField> = t.fields.iter().filter(|f| f.pk).collect();

        if pks.len() == 1 {
            // 单字段主键必须精确等于 <实体>_id：这是最常见的来源（AI 生成、随手新建）
            if pks[0].name != expect_pk {
                r.errors.push(format!(
                    "表「{}」的主键应命名为 {}（当前是 {}）：主键叫 id 这类通用名时，多表联查无法分辨它属于谁",
                    n.name, expect_pk, pks[0].name
                ));
            }
        } else if pks.len() > 1 {
            // 复合主键不强求整体等于 <实体>_id，但不允许出现裸 id，且至少一个字段带前缀
            let names: Vec<&str> = pks.iter().map(|f| f.name.as_str()).collect();
            let has_bare_id = names.contains(&"id");
            let has_prefixed = names.iter().any(|n| n.contains('_'));
            if has_bare_id || !has_prefixed {
                r.errors.push(format!(
                    "表「{}」的复合主键（{}）里不能有裸 id，至少一个字段要带表名前缀（如 {}）",
                    n.name,
                    names.join(" / "),
                    expect_pk
                ));
            }
        }

        for f in &t.fields {
            // 主键的命名由上面的规则单独管，这里跳过 —— 否则同一条「主键叫 id」
            // 会被报两遍，校验面板里全是重复噪音
            if f.pk {
                continue;
            }
            let lower = f.name.to_lowercase();
            // 带下划线 = 已带前缀（user_name、created_at），天然合规
            if f.name.contains('_') {
                continue;
            }
            if !GENERIC_FIELD_NAMES.contains(&lower.as_str()) {
                continue;
            }
            r.errors.push(format!(
                "表「{}」字段「{}」是通用裸名，跨表同名时无法分辨归属，应改为 {}_{}",
                n.name, f.name, entity, f.name
            ));
        }
    }

    // ── 外键必须有索引 ──
    // 不建索引的话，每条关联都会让子表在被 join 时全表扫描。
    // n-n 关系不在此列：它导出时生成的是联合主键的中间表，天然有索引。
    for rel in &doc.relations {
        if rel.kind == "n-n" {
            continue;
        }
        let Some(child) = doc.node(&rel.from.node) else { continue };
        if child.kind != "table" {
            continue;
        }
        let Some(t) = child.table.as_ref() else { continue };
        let fk = &rel.from.field;
        if t.fields.iter().any(|f| f.pk && &f.name == fk) {
            continue; // 主键自带索引
        }
        let covered = t
            .indexes
            .iter()
            .any(|idx| idx.fields.iter().any(|c| c == fk));
        if !covered {
            r.errors.push(format!(
                "表「{}」的外键字段「{}」没有索引：与「{}」关联时会被全表扫描",
                child.name, fk, rel.to.node
            ));
        }
    }
}

/// 改字段名：**唯一**的改名入口 —— 同时把关联里引用旧名的两端改掉。
/// 返回被级联修改的关联条数。
pub fn rename_field(
    doc: &mut DbDesignDocument,
    node_id: &str,
    old: &str,
    new: &str,
) -> Result<usize, String> {
    if new.trim().is_empty() {
        return Err("新字段名不能为空".to_string());
    }
    let table = doc
        .nodes
        .iter_mut()
        .find(|n| n.id == node_id)
        .and_then(|n| n.table.as_mut())
        .ok_or_else(|| format!("找不到表节点 {}", node_id))?;
    if !table.fields.iter().any(|f| f.name == old) {
        return Err(format!("字段「{}」不存在", old));
    }
    if table.fields.iter().any(|f| f.name == new) {
        return Err(format!("字段「{}」已存在", new));
    }
    for f in table.fields.iter_mut() {
        if f.name == old {
            f.name = new.to_string();
        }
    }
    // 索引里也引用字段名
    for idx in table.indexes.iter_mut() {
        for f in idx.fields.iter_mut() {
            if f == old {
                *f = new.to_string();
            }
        }
    }
    // 关联两端的字段名
    let mut touched = 0;
    for rel in doc.relations.iter_mut() {
        for end in [&mut rel.from, &mut rel.to] {
            if end.node == node_id && end.field == old {
                end.field = new.to_string();
                touched += 1;
            }
        }
    }
    Ok(touched)
}

/// 参与关联的字段名集合（画布折叠态只显示「表名 + 注释 + 这些字段」）。
pub fn relation_fields(doc: &DbDesignDocument, node_id: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for rel in &doc.relations {
        for end in [&rel.from, &rel.to] {
            if end.node == node_id && !out.contains(&end.field) {
                out.push(end.field.clone());
            }
        }
    }
    out
}

/// n-n 关联自动生成的中间表名（单一真源：导出与校验都用这个规则）。
pub fn junction_table_name(doc: &DbDesignDocument, rel: &DbDesignRelation) -> String {
    let a = doc.node(&rel.from.node).map(|n| n.name.as_str()).unwrap_or("a");
    let b = doc.node(&rel.to.node).map(|n| n.name.as_str()).unwrap_or("b");
    format!("{}_{}", a, b)
}


/// 规范化结果：让调用方能告诉用户「改了什么」。
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizeReport {
    /// 主键改名：`表.旧名 → 新名`
    pub renamed_pk: Vec<String>,
    /// 通用裸名改名：`表.旧名 → 新名`
    pub renamed_field: Vec<String>,
    /// 补的外键索引：`表.字段`
    pub added_index: Vec<String>,
    /// 因目标名已被占用而跳过的改名
    pub skipped: Vec<String>,
}

/// 把不合规命名就地改合规：主键改成 `<实体>_id`、通用裸名加表前缀、外键补索引。
///
/// **为什么必须有这个函数**：命名规范是 error 级（阻断保存/导出），但逆向解析别人的
/// DDL / SQLite 库、以及 AI 一次生成 hundreds 张表时，几乎不可能天然合规 —— 不做规范化，
/// 用户导入真实 DDL 后会陷入「满屏报错且导不出来」的死锁。所以外来数据在落地前先过一遍。
///
/// 改名是**先算全量映射再统一应用**：链式改名（`a.id → a_id` 而原表里已有 `a_id`）
/// 会撞名，所以映射算完要先检查目标名是否被占用，冲突的跳过并记进 `skipped`。
pub fn normalize_naming(doc: &mut DbDesignDocument) -> NormalizeReport {
    let mut report = NormalizeReport::default();
    let mut renames: Vec<(String, String, String)> = Vec::new(); // (node_id, old, new)

    for n in &doc.nodes {
        if n.kind != "table" {
            continue;
        }
        let Some(t) = n.table.as_ref() else { continue };
        let entity = singularize(&n.name);
        if entity.is_empty() {
            continue;
        }
        let occupied = |cand: &str| t.fields.iter().any(|f| f.name == cand);

        let pks: Vec<&DbField> = t.fields.iter().filter(|f| f.pk).collect();
        if pks.len() == 1 {
            let expect = format!("{}_id", entity);
            let cur = pks[0].name.clone();
            if cur != expect {
                if occupied(&expect) {
                    report.skipped.push(format!(
                        "{}.{} → {}（已存在同名字段，跳过）",
                        n.name, cur, expect
                    ));
                } else {
                    renames.push((n.id.clone(), cur.clone(), expect.clone()));
                    report.renamed_pk.push(format!("{}.{} → {}", n.name, cur, expect));
                }
            }
        }
        for f in &t.fields {
            if f.pk || f.name.contains('_') {
                continue;
            }
            let lower = f.name.to_lowercase();
            if !GENERIC_FIELD_NAMES.contains(&lower.as_str()) {
                continue;
            }
            let expect = format!("{}_{}", entity, lower);
            if occupied(&expect) {
                report.skipped.push(format!(
                    "{}.{} → {}（已存在同名字段，跳过）",
                    n.name, f.name, expect
                ));
                continue;
            }
            renames.push((n.id.clone(), f.name.clone(), expect.clone()));
            report.renamed_field.push(format!("{}.{} → {}", n.name, f.name, expect));
        }
    }

    if !renames.is_empty() {
        let lookup = |nid: &str, old: &str| -> Option<String> {
            renames
                .iter()
                .find(|(n, o, _)| n == nid && o == old)
                .map(|(_, _, new)| new.clone())
        };
        // 字段 + 同表索引列
        for n in doc.nodes.iter_mut() {
            if let Some(t) = n.table.as_mut() {
                let nid = n.id.clone();
                for f in t.fields.iter_mut() {
                    if let Some(new) = lookup(&nid, &f.name) {
                        f.name = new;
                    }
                }
                for idx in t.indexes.iter_mut() {
                    for c in idx.fields.iter_mut() {
                        if let Some(new) = lookup(&nid, c) {
                            *c = new;
                        }
                    }
                }
            }
        }
        // 关联两端：指向本表主键的那端也要跟着改名，否则会变成悬空引用
        for rel in doc.relations.iter_mut() {
            for end in [&mut rel.from, &mut rel.to] {
                if let Some(new) = lookup(&end.node, &end.field) {
                    end.field = new;
                }
            }
        }
    }

    // ── 外键补索引 ──
    let mut needed: Vec<(String, String)> = Vec::new();
    for rel in &doc.relations {
        if rel.kind == "n-n" {
            continue; // 中间表是联合主键，自带索引
        }
        let Some(child) = doc.node(&rel.from.node) else { continue };
        let Some(t) = child.table.as_ref() else { continue };
        if t.fields.iter().any(|f| f.pk && f.name == rel.from.field) {
            continue;
        }
        if t.indexes.iter().any(|idx| idx.fields.iter().any(|c| *c == rel.from.field)) {
            continue;
        }
        let key = (rel.from.node.clone(), rel.from.field.clone());
        if !needed.contains(&key) {
            needed.push(key);
        }
    }
    for (nid, field) in needed {
        if let Some(n) = doc.nodes.iter_mut().find(|n| n.id == nid) {
            if let Some(t) = n.table.as_mut() {
                let idx_name = format!("idx_{}_{}", n.name, field);
                if t.indexes.iter().any(|i| i.name == idx_name) {
                    continue;
                }
                t.indexes.push(DbIndex {
                    name: idx_name,
                    kind: "index".to_string(),
                    fields: vec![field.clone()],
                });
                report.added_index.push(format!("{}.{}", n.name, field));
            }
        }
    }

    report
}
