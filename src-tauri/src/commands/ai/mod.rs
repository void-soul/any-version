// 模块聚合：由原 commands/ai.rs（3114 行）按功能拆分而来。
// 所有 #[tauri::command] 通过 glob 重新导出，lib.rs 的 commands::ai::* 注册保持不变。

pub mod models;
pub mod config;
pub mod detect;
pub mod skills;
pub mod mcp;
pub mod usage;
pub mod provider;
pub mod launch;
pub mod terminal;
pub mod sessions;
pub mod cache;
pub mod tools;
pub mod tool_paths;
pub mod collab;

pub use models::*;
pub use config::*;
pub use detect::*;
pub use skills::*;
pub use usage::*;
pub use provider::*;
pub use launch::*;
pub use terminal::*;
pub use sessions::*;
pub use cache::*;
pub use tools::*;
pub use tool_paths::*;
