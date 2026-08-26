//! API 接口测试平台模块：项目-模块-接口三级结构，Rust 后端发起请求。
//!
//! 核心能力：
//! - 请求执行（reqwest，变量渲染 + 随机变量）
//! - 单元测试（断言式，逐条结果）
//! - 压力测试（tokio 并发引擎，实时进度 + 完整统计报告，报告落库）
//! - 导入导出（Postman v2.1 / Swagger / OpenAPI、Nest / Nuxt / Spring 框架扫描）
//!
//! 数据存储：独立 SQLite（~/.any-version/api.db），与 tasks.db 同一风格。

pub mod commands;
pub mod db;
pub mod exec;
pub mod import;
pub mod import_ai;
pub mod loadtest;
pub mod models;
pub mod render;

pub use commands::*;
