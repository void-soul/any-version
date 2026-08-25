// 需求模块：多项目 → 多模块 → 多图谱 的思维导图式结构。
// - models.rs   数据模型（项目/模块/节点/图谱）
// - db.rs        SQLite 持久化
// - scan.rs      项目目录扫描
// - commands.rs  Tauri 命令（CRUD + AI 生成 + 导出）
// - store.rs     旧 JSON 文件存储（保留兼容）
// - generate.rs  旧 AI 生成（已废弃，redirect 到新命令）

pub mod models;
pub mod db;
pub mod scan;
pub mod commands;
pub mod store;
pub mod generate;

pub use models::*;
pub use commands::*;