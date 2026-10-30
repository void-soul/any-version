//! 导入导出：Postman 标准数据、Swagger/OpenAPI、Nest/Nuxt/Spring 框架接口扫描。

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::models::{Authorization, FormDataItem, KeyValueItem, RequestSettings};

/// 导入候选（由各解析器产出，统一落库）。
#[derive(Debug, Clone, Default)]
pub struct EndpointDraft {
    /// 模块名（自动创建或匹配已有）
    pub module: String,
    pub name: String,
    pub method: String,
    pub url: String,
    pub headers: Vec<KeyValueItem>,
    pub query_params: Vec<KeyValueItem>,
    pub body: String,
    pub body_type: String,
    pub body_form: Vec<FormDataItem>,
    pub body_urlencoded: Vec<KeyValueItem>,
    pub body_graphql_query: String,
    pub body_graphql_variables: String,
    pub authorization: Authorization,
    /// 请求设置（cURL 的 -k/--http1.1 等会映射到这里）
    pub settings: RequestSettings,
    pub docs_md: String,
}

/// Postman 导入结果。
#[derive(Debug, Clone)]
pub struct PostmanImport {
    /// 集合级变量（导入为环境变量）
    pub variables: Vec<KeyValueItem>,
    pub drafts: Vec<EndpointDraft>,
}

// ─── 通用工具 ───

fn v_str(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

/// 从装饰器后文本提取函数名：取第一个 `(` 前最近的标识符。
fn extract_func_name(after: &str) -> String {
    let Some(paren_idx) = after.find('(') else {
        return String::new();
    };
    let before = &after[..paren_idx];
    let mut name = String::new();
    for ch in before.chars().rev() {
        if ch.is_alphanumeric() || ch == '_' {
            name.insert(0, ch);
        } else if !name.is_empty() {
            break;
        }
    }
    name
}

/// 确保路径以 `/` 开头（Nest/Spring 扫描产物为相对路径时补上）。
fn ensure_leading_slash(url: &str) -> String {
    if url.starts_with('/') {
        url.to_string()
    } else {
        format!("/{}", url)
    }
}

fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        if base.is_empty() {
            "/".to_string()
        } else {
            base.to_string()
        }
    } else if base.is_empty() {
        format!("/{}", path)
    } else {
        format!("{}/{}", base, path)
    }
}

/// 目录遍历（跳过常见依赖/构建目录，限深限文件数）。
fn walk_files(
    root: &Path,
    depth: usize,
    max_depth: usize,
    max_files: &mut usize,
    out: &mut Vec<PathBuf>,
) {
    if depth > max_depth || *max_files == 0 {
        return;
    }
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if *max_files == 0 {
            return;
        }
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if matches!(
                name.as_str(),
                "node_modules"
                    | ".git"
                    | "dist"
                    | "build"
                    | "target"
                    | ".nuxt"
                    | ".output"
                    | "coverage"
                    | "vendor"
                    | ".next"
                    | ".cache"
                    | ".idea"
            ) {
                continue;
            }
            walk_files(&path, depth + 1, max_depth, max_files, out);
        } else {
            *max_files -= 1;
            out.push(path);
        }
    }
}

fn kv_enabled(v: &Value) -> bool {
    v.get("disabled").and_then(|d| d.as_bool()).unwrap_or(false) == false
}

/// 剥掉 URL 里的查询串（`?` 及其后所有内容），只在 `url.query` 数组非空时用。
fn strip_query(url: &str) -> &str {
    match url.split_once('?') {
        Some((base, _)) => base,
        None => url,
    }
}

mod postman;
pub use postman::*;
mod swagger;
pub use swagger::*;
mod framework;
pub use framework::*;
mod curl;
pub use curl::*;
mod openapi;
pub use openapi::*;
#[cfg(test)]
mod tests;
