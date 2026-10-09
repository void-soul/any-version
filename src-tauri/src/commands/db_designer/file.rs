// 设计文件的读写：**纯文件**，不进应用数据库。
//
// 用户决定（2026-10-09）：设计文档就是一份 `.dbdesign.json`，好处有三条 ——
//   1. 内置 Agent / 外部 Agent / 脚本都能直接读写，不用经过应用；
//   2. 可以随项目一起提交进版本库，设计和代码同仓；
//   3. 应用只是「一个编辑器」，不持有数据，卸载/换机器不丢设计。
// 因此这里只做「路径 → 文档」「文档 → 路径」，没有文档表、没有文件夹。
use std::fs;
use std::path::{Path, PathBuf};

use super::export::{export_outline, export_sql};
use super::models::*;
use super::store::validate;

/// 设计文件后缀：`订单库.dbdesign.json`
pub const FILE_SUFFIX: &str = ".dbdesign.json";

/// 读设计文件。JSON 非法才报错；内容校验交给 `validate`（允许带着 warning 打开）。
pub fn load_file(path: &str) -> Result<DbDesignDocument, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("读取设计文件失败 {}: {}", path, e))?;
    // 容忍 UTF-8 BOM：Windows 上的编辑器 / Agent 脚本写出的文件常带 BOM，
    // 而 serde_json 不认，会直接报「expected value at line 1 column 1」。
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    let doc: DbDesignDocument = serde_json::from_str(raw)
        .map_err(|e| format!("设计文件不是合法 JSON：{}（{}）", e, path))?;
    Ok(doc)
}

/// 写设计文件。**校验不通过就拒绝写盘** —— 别把坏设计落成一个打不开的文件。
pub fn save_file(path: &str, doc: &DbDesignDocument) -> Result<(), String> {
    let report = validate(doc);
    if !report.is_ok() {
        return Err(format!("设计文档校验未通过，未写入：\n- {}", report.errors.join("\n- ")));
    }
    write_json(path, doc)
}

/// 强制写盘（不做校验）：给「导入后先落盘、再让用户修」这类场景留的口子。
pub fn write_json(path: &str, doc: &DbDesignDocument) -> Result<(), String> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("创建目录失败 {}: {}", parent.display(), e))?;
        }
    }
    let json = serde_json::to_string_pretty(doc).map_err(|e| format!("序列化失败: {}", e))?;
    fs::write(path, json).map_err(|e| format!("写入失败 {}: {}", path, e))?;
    Ok(())
}

/// 导出 SQL 到设计文件同目录的 `<主名>.sql`（`订单库.dbdesign.json` → `订单库.sql`）。
pub fn export_sql_file(path: &str) -> Result<String, String> {
    let doc = load_file(path)?;
    let sql = export_sql(&doc)?;
    let out = sibling_path(path, "sql");
    fs::write(&out, &sql).map_err(|e| format!("写入 SQL 失败 {}: {}", out.display(), e))?;
    Ok(out.to_string_lossy().to_string())
}

/// 导出结构说明到同目录的 `<主名>.md`。
pub fn export_outline_file(path: &str) -> Result<String, String> {
    let doc = load_file(path)?;
    let out = sibling_path(path, "md");
    fs::write(&out, export_outline(&doc))
        .map_err(|e| format!("写入结构说明失败 {}: {}", out.display(), e))?;
    Ok(out.to_string_lossy().to_string())
}

/// 同目录同名、换后缀：`a/x.dbdesign.json` + `sql` → `a/x.sql`
fn sibling_path(path: &str, ext: &str) -> PathBuf {
    let p = PathBuf::from(path);
    let stem = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("design")
        .trim_end_matches(".dbdesign");
    p.with_file_name(format!("{}.{}", stem, ext))
}

/// 新建一份空设计（前端「新建」按钮用）。
pub fn new_document(name: &str, dialect: &str) -> DbDesignDocument {
    DbDesignDocument {
        id: format!("dbd_{}", chrono::Utc::now().timestamp_millis()),
        name: if name.trim().is_empty() {
            "未命名设计".to_string()
        } else {
            name.to_string()
        },
        description: String::new(),
        dialect: if DIALECTS.contains(&dialect) {
            dialect.to_string()
        } else {
            "mysql".to_string()
        },
        folder_id: None,
        nodes: Vec::new(),
        relations: Vec::new(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::db_designer::models::{
        DbDesignNode, DbField, DbLogicalType, DbTableBody,
    };

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kira-dbd-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn one_table_doc(dialect: &str) -> DbDesignDocument {
        let mut doc = new_document("订单库", dialect);
        doc.nodes.push(DbDesignNode {
            id: "n1".to_string(),
            kind: "table".to_string(),
            name: "orders".to_string(),
            comment: "订单主表".to_string(),
            x: 0.0,
            y: 0.0,
            table: Some(DbTableBody {
                fields: vec![DbField {
                    name: "id".to_string(),
                    r#type: DbLogicalType { base: "bigint".into(), ..Default::default() },
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
        doc
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = temp_dir("round");
        let path = dir.join("订单库.dbdesign.json");
        let p = path.to_string_lossy().to_string();
        let doc = one_table_doc("postgres");
        save_file(&p, &doc).unwrap();
        let back = load_file(&p).unwrap();
        assert_eq!(back.name, "订单库");
        assert_eq!(back.nodes.len(), 1);
        assert_eq!(back, {
            let mut d = doc.clone();
            d.updated_at = back.updated_at.clone();
            d
        });
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_refuses_invalid_documents() {
        let dir = temp_dir("invalid");
        let p = dir.join("bad.dbdesign.json").to_string_lossy().to_string();
        let mut doc = one_table_doc("mysql");
        doc.relations.push(crate::commands::db_designer::models::DbDesignRelation {
            id: "r".to_string(),
            name: String::new(),
            from: crate::commands::db_designer::models::DbRelationEnd {
                node: "n1".to_string(),
                field: "ghost".to_string(),
            },
            to: crate::commands::db_designer::models::DbRelationEnd {
                node: "n1".to_string(),
                field: "id".to_string(),
            },
            kind: "1-n".to_string(),
            on_delete: String::new(),
            on_update: String::new(),
            mirror: false,
        });
        assert!(save_file(&p, &doc).is_err());
        assert!(!Path::new(&p).exists(), "校验失败就不该落盘");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_writes_sibling_sql_and_outline() {
        let dir = temp_dir("export");
        let p = dir.join("订单库.dbdesign.json").to_string_lossy().to_string();
        save_file(&p, &one_table_doc("mysql")).unwrap();

        let sql_path = export_sql_file(&p).unwrap();
        assert!(sql_path.ends_with("订单库.sql"), "{}", sql_path);
        assert!(!sql_path.contains("dbdesign"), "{}", sql_path);
        let sql = fs::read_to_string(&sql_path).unwrap();
        assert!(sql.contains("CREATE TABLE `orders`"), "{}", sql);

        let md_path = export_outline_file(&p).unwrap();
        assert!(md_path.ends_with("订单库.md"), "{}", md_path);
        let md = fs::read_to_string(&md_path).unwrap();
        assert!(md.contains("orders"), "{}", md);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_document_falls_back_to_known_dialect() {
        assert_eq!(new_document("", "mongodb").dialect, "mysql");
        assert_eq!(new_document("x", "sqlite").dialect, "sqlite");
        assert_eq!(new_document("", "mysql").name, "未命名设计");
    }
}
