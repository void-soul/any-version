// 数据库设计器的 Tauri 命令（P1）。
//
// 形态：**纯文件 + 无状态**。文档不进应用数据库，前端持有当前文档对象，
// 后端负责「读文件 / 写文件 / 生成产物 / 校验 / 结构维护」这几件确定性工作。
// 新增命令必须同步注册到 `lib.rs` 的 invoke_handler（漏注册 = 前端运行时报错）。
use super::export::{export_outline, export_sql};
use super::file::{
    export_outline_file, export_sql_file, load_file, new_document, save_file, write_json,
};
use super::models::DbDesignDocument;
use super::store::{rename_field, validate, ValidationReport};

/// 新建一份空设计（前端「新建」用）
#[tauri::command]
pub fn dbd_new_document(name: String, dialect: String) -> DbDesignDocument {
    new_document(&name, &dialect)
}

/// 打开设计文件
#[tauri::command]
pub fn dbd_open_file(path: String) -> Result<DbDesignDocument, String> {
    load_file(&path)
}

/// 保存设计文件（校验不通过则不写盘）
#[tauri::command]
pub fn dbd_save_file(path: String, doc: DbDesignDocument) -> Result<(), String> {
    save_file(&path, &doc)
}

/// 强制保存（跳过校验）。仅用于「先落盘、再慢慢修」的场景。
#[tauri::command]
pub fn dbd_write_file(path: String, doc: DbDesignDocument) -> Result<(), String> {
    write_json(&path, &doc)
}

/// 校验当前文档（保存前先看一眼，前端可只显示 warnings）
#[tauri::command]
pub fn dbd_validate(doc: DbDesignDocument) -> ValidationReport {
    validate(&doc)
}

/// 把文档里所有「镜像父表主键」的关系同步到当前主键定义。
///
/// 前端每次改字段（加/删字段、改主键、改类型）后调一次：父表主键变了，子表的
/// 外键副本与连线跟着变。规则见 `sync::sync_mirrored_relations`。
#[tauri::command]
pub fn dbd_sync_relations(doc: DbDesignDocument) -> DbDesignDocument {
    let mut next = doc;
    super::sync::sync_mirrored_relations(&mut next);
    next
}

/// 生成建表语句文本（前端负责另存为）
#[tauri::command]
pub fn dbd_export_sql(doc: DbDesignDocument) -> Result<String, String> {
    export_sql(&doc)
}

/// 直接把 SQL 写到设计文件同目录的 `<主名>.sql`，返回落盘路径
#[tauri::command]
pub fn dbd_export_sql_file(path: String) -> Result<String, String> {
    export_sql_file(&path)
}

/// 结构说明（Markdown 文本）
#[tauri::command]
pub fn dbd_export_outline(doc: DbDesignDocument) -> String {
    export_outline(&doc)
}

/// 结构说明写到同目录的 `<主名>.md`
#[tauri::command]
pub fn dbd_export_outline_file(path: String) -> Result<String, String> {
    export_outline_file(&path)
}

/// 改字段名（**唯一**入口）：级联更新关联与索引，返回新文档
#[tauri::command]
pub fn dbd_rename_field(
    doc: DbDesignDocument,
    node_id: String,
    old: String,
    new: String,
) -> Result<DbDesignDocument, String> {
    let mut next = doc;
    rename_field(&mut next, &node_id, &old, &new)?;
    Ok(next)
}

/// 从 SQLite 库文件反推设计（只读打开，不改库）
#[tauri::command]
pub fn dbd_reverse_sqlite(path: String) -> Result<DbDesignDocument, String> {
    let name = std::path::Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("SQLite 设计")
        .to_string();
    super::reverse::reverse_sqlite(&path, &name)
}

/// 从 DDL 文本（mysqldump / pg_dump / 任意 .sql）反推设计
#[tauri::command]
pub fn dbd_reverse_ddl_text(text: String, dialect: String, name: String) -> Result<DbDesignDocument, String> {
    super::reverse::reverse_ddl(&text, &dialect, &name)
}

/// 从 DDL 文件反推设计
#[tauri::command]
pub fn dbd_reverse_ddl_file(path: String, dialect: String) -> Result<DbDesignDocument, String> {
    // 中文 Windows 的 Navicat / mysqldump 默认导出 GBK，read_to_string 会直接抛
    // "stream did not contain valid UTF-8" —— 用户看不懂。改成先按字节读，
    // 非 UTF-8 时给一句能照做的提示（另存为 UTF-8）。
    let bytes = std::fs::read(&path).map_err(|e| format!("读取失败 {}: {}", path, e))?;
    let text = String::from_utf8(bytes).map_err(|_| {
        format!(
            "{} 不是 UTF-8 编码（中文 Windows 的 Navicat / mysqldump 常默认导出 GBK）。\n\
             请在导出的字符集里选 UTF-8（Navicat：编码 = UTF-8）后重新导出。",
            path
        )
    })?;
    let name = std::path::Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("DDL 设计")
        .to_string();
    super::reverse::reverse_ddl(&text, &dialect, &name)
}
