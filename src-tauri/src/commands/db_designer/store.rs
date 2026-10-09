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

    r
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
