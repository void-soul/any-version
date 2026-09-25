//! 收藏 / 星标聚合模块（只读导入 + AI 归类 + 失效检测）。
//!
//! 设计见 `docs/plans/2026-09-22-favorites-module-design.md`，
//! 第一批只做 GitHub star（B站 / 知乎后续接入，共用同一套本地库与 UI）。
//!
//! 硬规则：**只读取平台数据，不反向操作平台**（不做 unstar / 取消收藏 / 新建收藏夹）。

pub mod db;
pub mod settings;
pub mod github;
pub mod classify;
pub mod cookie_expiry;
pub mod check;
pub mod wbi;
pub mod bilibili;
pub mod zhihu;
pub mod commands;
pub mod agent;

pub use commands::*;
/// 界面设置命令（宽度 / 上次用的模型）与 db 命令平级导出，lib.rs 注册处无需带子模块名。
/// 必须用 glob：`#[tauri::command]` 生成的 `__cmd__*` helper 宏只有 glob 才会一并带出，
/// 逐个列出函数名会漏掉宏，`generate_handler!` 就找不到它们。
pub use settings::*;
