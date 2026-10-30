//! Picky 模块：收藏 / 归档页面（与 Flutter 端 picky 保持同一数据接口）。
//!
//! 数据字段（camelCase，与 Flutter 模型 toJson 一致）：
//! - 页面信息：id / title / description / url / imageUrl / faviconUrl / createdAt / updatedAt / metaFetched
//! - 状态：refined（false=收藏中，true=已归档/炼化）
//! - 评论：id / bookmarkId / content / createdAt / updatedAt / parentId（树形，2 级）
//! - 标签：id / name / color / createdAt，书签-标签关联 bookmarkTags: { bookmarkId: [tagId...] }
//!
//! 云同步：S3 兼容存储（SigV4），state.json 结构与 Flutter 端完全一致，可跨端互通。
//! 收藏中的 lingzuCode / aiCopy 等个性化字段不做处理，但会原样保留（extra），
//! 避免与 Flutter 端来回同步时丢失其数据。

mod s3;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use tauri::{Emitter, Manager};

use crate::commands::config::get_data_dir;
pub use s3::PickySyncConfig;

mod models;
pub use models::*;
mod database;
use database::*;
mod browser;
use browser::*;
mod webview;
use webview::*;
mod metadata;
pub use metadata::*;
mod store;
use store::*;
mod sync_config;
use sync_config::*;
mod merge;
use merge::*;
mod bookmarks;
pub use bookmarks::*;
mod annotations;
pub use annotations::*;
mod sync;
pub use sync::*;

#[cfg(test)]
mod tests;
