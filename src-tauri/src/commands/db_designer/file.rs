// 设计文件的读写：**纯文件**，不进应用数据库。
//
// 用户决定（2026-10-09）：设计文档就是一份 `.dbdesign.json`，好处有三条 ——
//   1. 内置 Agent / 外部 Agent / 脚本都能直接读写，不用经过应用；
//   2. 可以随项目一起提交进版本库，设计和代码同仓；
//   3. 应用只是「一个编辑器」，不持有数据，卸载/换机器不丢设计。
// 因此这里只做「路径 → 文档」「文档 → 路径」，没有文档表、没有文件夹。
use std::fs;
use std::path::{Path, PathBuf};

use super::export::{export_outline, export_sql_with, ExportOptions};
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
///
/// **以传入的 `doc` 为准，不重新读磁盘** —— 与「复制到剪贴板」那条路径保持一致。
/// 早先这里是 `load_file(path)` 再导出，于是「改了没保存就点导出」拿到的是磁盘上的
/// 旧版本，而剪贴板那条路用的是内存里的当前设计：同一个按钮，两种结果。
pub fn export_sql_file(path: &str, doc: &DbDesignDocument, opts: ExportOptions) -> Result<String, String> {
    let sql = export_sql_with(doc, opts)?;
    let out = sibling_path(path, "sql");
    fs::write(&out, &sql).map_err(|e| format!("写入 SQL 失败 {}: {}", out.display(), e))?;
    Ok(out.to_string_lossy().to_string())
}

/// 导出 SQL 到**任意**指定路径（「另存为」用；不要求设计文件已落盘）。
pub fn export_sql_to(
    target: &str,
    doc: &DbDesignDocument,
    opts: ExportOptions,
) -> Result<String, String> {
    let sql = export_sql_with(doc, opts)?;
    if let Some(parent) = Path::new(target).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("创建目录失败 {}: {}", parent.display(), e))?;
        }
    }
    fs::write(target, &sql).map_err(|e| format!("写入 SQL 失败 {}: {}", target, e))?;
    Ok(target.to_string())
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

/// 生成设计文档 id。
///
/// ⚠ 必须带进程内自增序号：只用毫秒时间戳的话，**同一毫秒里建的两份设计会拿到
/// 同一个 id**（连点两次「新建」就够）。id 是 AI 会话的主键，撞了意味着两份设计
/// 共用同一份对话。`reverse.rs:16` 的注释为同一件事踩过坑（那儿是两张表同 id
/// 导致索引挂错表），写法照抄那边。
fn new_document_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("dbd_{}_{}", chrono::Utc::now().timestamp_millis(), seq)
}

/// 新建一份空设计（前端「新建」按钮用）。
pub fn new_document(name: &str, dialect: &str) -> DbDesignDocument {
    DbDesignDocument {
        id: new_document_id(),
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
        decisions: Vec::new(),
        common_fields: Vec::new(),
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
                    name: "order_id".to_string(),
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

        let sql_path = export_sql_file(&p, &one_table_doc("mysql"), ExportOptions::default()).unwrap();
        assert!(sql_path.ends_with("订单库.sql"), "{}", sql_path);
        assert!(!sql_path.contains("dbdesign"), "{}", sql_path);
        let sql = fs::read_to_string(&sql_path).unwrap();
        assert!(sql.contains("CREATE TABLE `orders`"), "{}", sql);

        // 导出以**传入的 doc** 为准：改了没保存也要能导出眼前这份
        let mut edited = one_table_doc("mysql");
        edited.name = "改过的名字".to_string();
        let sql2 = export_sql_file(&p, &edited, ExportOptions::default()).unwrap();
        assert!(
            fs::read_to_string(&sql2).unwrap().contains("改过的名字"),
            "不该导出磁盘上的旧版本"
        );

        // 另存为：写到任意路径，且不需要设计文件已落盘
        let custom = dir.join("别处.sql").to_string_lossy().to_string();
        let got = export_sql_to(&custom, &one_table_doc("postgres"), ExportOptions::default()).unwrap();
        assert_eq!(got, custom);
        assert!(fs::read_to_string(&custom).unwrap().contains("CREATE TABLE"));

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

    /// 回归：id 只有毫秒时间戳时，**同一毫秒连建两份设计会拿到同一个 id**。
    ///
    /// 后果不是「不好看」而是数据串联：id 是 AI 会话的主键，撞了就意味着
    /// 两份设计共用同一份对话（会话索引按 id 索引，见 `chat.rs`）。
    /// `reverse.rs:16` 的注释早就警告过这个模式 —— 那次是两张表同 id 导致
    /// 索引挂错表，这次的代价是两份设计互相看到对方的对话。
    ///
    /// 用循环而不是「建两份」：两份恰好同毫秒的概率虽低但非零，
    /// 单次断言会偶发通过，等于没测。
    #[test]
    fn documents_created_in_a_row_get_distinct_ids() {
        let ids: std::collections::HashSet<String> = (0..64)
            .map(|i| new_document(&format!("设计{}", i), "mysql").id)
            .collect();
        assert_eq!(ids.len(), 64, "64 份设计必须拿到 64 个不同的 id");
    }

    /// id 会被后端当会话 id 用（`chat.rs::chat_path` 只放行字母数字 `_` `-`），
    /// 所以它的字符集不能随时间格式变化而跑到白名单外面去。
    #[test]
    fn new_document_id_is_usable_as_a_session_id() {
        let id = new_document("x", "mysql").id;
        assert!(
            id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "会话 id {} 含 chat.rs 不放行的字符，保存时会失败",
            id
        );
    }
}
