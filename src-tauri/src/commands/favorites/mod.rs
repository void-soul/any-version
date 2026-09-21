//! 收藏 / 星标聚合模块（只读导入 + AI 归类 + 失效检测）。
//!
//! 设计见 `docs/plans/2026-09-22-favorites-module-design.md`，
//! 第一批只做 GitHub star（B站 / 知乎后续接入，共用同一套本地库与 UI）。
//!
//! 硬规则：**只读取平台数据，不反向操作平台**（不做 unstar / 取消收藏 / 新建收藏夹）。

pub mod db;
pub mod github;
pub mod commands;

pub use commands::*;
