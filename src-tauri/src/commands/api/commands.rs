//! API 模块的 Tauri 命令层：CRUD + 请求执行 + 单测 + 压测 + 导入导出。

use serde_json::Value;

use super::db::{self, now_ts};
use super::models::*;

/// 接口表的完整列清单（顺序与 endpoint_row 一致）。
const EP_COLS: &str = "id, project_id, module_id, name, method, url, headers, query_params, path_params, body, body_type, description, docs_md, timeout_ms, created_at, updated_at, body_form, body_urlencoded, body_graphql_query, body_graphql_variables, authorization, cookies, settings, response_comment, is_favorite, extract_rules";

mod projects;
pub use projects::*;
mod environments;
pub use environments::*;
mod modules;
pub use modules::*;
mod history;
pub use history::*;
mod endpoints;
pub use endpoints::*;
mod execution;
pub use execution::*;
mod unit_tests;
pub use unit_tests::*;
mod load_tests;
pub use load_tests::*;
mod interchange;
pub use interchange::*;
mod preset_headers;
pub use preset_headers::*;
