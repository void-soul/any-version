// 任务模块：任务计划 + 每日复盘日志。
// 数据存储使用独立的 SQLite 数据库（~/.any-version/tasks.db）。
//
// 核心设计约定（借鉴 Daily Log）：
// 以 progress（0-100）作为完成度的唯一真相来源，status 由 progress 派生，
// 任何写进度的路径都必须经过 apply_progress()，禁止外部直接设置 status/completed_at。

pub mod db;
pub mod models;
pub mod commands;

pub use commands::*;
