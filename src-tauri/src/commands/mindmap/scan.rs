//! 扫描项目目录，生成供 AI 分析用的文本上下文：
//! 1. 目录树（限制深度与条目数，跳过依赖/构建产物目录）
//! 2. 关键文件内容（manifest、README、入口文件等，逐文件截断）

use std::path::Path;
use walkdir::WalkDir;

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".nuxt",
    "out",
    "coverage",
    ".idea",
    ".vscode",
    "__pycache__",
    ".venv",
    "venv",
    ".turbo",
    ".cache",
    ".expo",
    "DerivedData",
    ".gradle",
];

const MAX_TREE_DEPTH: usize = 5;
const MAX_TREE_LINES: usize = 400;

/// 关键文件（按优先级顺序读取，控制总量）。
const KEY_FILES: &[&str] = &[
    "README.md",
    "package.json",
    "Cargo.toml",
    "go.mod",
    "pyproject.toml",
    "requirements.txt",
    "tsconfig.json",
    "vite.config.ts",
    "vite.config.js",
    "next.config.js",
    "next.config.ts",
    "nuxt.config.ts",
    "nuxt.config.js",
    "pom.xml",
    "build.gradle",
    "composer.json",
    "Gemfile",
    "src/main.ts",
    "src/main.tsx",
    "src/main.js",
    "src/index.ts",
    "src/index.tsx",
    "src/app.ts",
    "src/app.tsx",
    "src/main.rs",
];

const MAX_FILE_CHARS: usize = 3000;
const MAX_TOTAL_FILE_CHARS: usize = 24000;

/// 取项目名（路径末段）。
pub fn project_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn should_skip_dir(name: &str) -> bool {
    SKIP_DIRS.iter().any(|d| *d == name) || (name.starts_with('.') && name != ".github")
}

/// 构建目录树文本（目录在前，文件在后）。
fn build_tree(root: &Path) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut budget = MAX_TREE_LINES;
    let entries: Vec<_> = WalkDir::new(root)
        .min_depth(1)
        .max_depth(MAX_TREE_DEPTH)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            // 跳过忽略目录内部的所有内容
            e.path()
                .components()
                .rev()
                .nth(1)
                .map(|c| c.as_os_str().to_string_lossy())
                .map(|s| !should_skip_dir(&s))
                .unwrap_or(true)
        })
        .collect();

    // 目录优先，其次文件，各自按名称排序
    let mut dirs: Vec<_> = entries.iter().filter(|e| e.file_type().is_dir()).collect();
    let mut files: Vec<_> = entries.iter().filter(|e| e.file_type().is_file()).collect();
    dirs.sort_by_key(|e| e.path().to_string_lossy().to_string());
    files.sort_by_key(|e| e.path().to_string_lossy().to_string());

    for e in dirs.into_iter().chain(files.into_iter()) {
        if budget == 0 {
            out.push("...（目录树已截断）".to_string());
            break;
        }
        let rel = e.path().strip_prefix(root).unwrap_or(e.path());
        let depth = rel.components().count().saturating_sub(1).min(MAX_TREE_DEPTH);
        let name = e.file_name().to_string_lossy().to_string();
        if e.file_type().is_dir() && should_skip_dir(&name) {
            continue;
        }
        let suffix = if e.file_type().is_dir() { "/" } else { "" };
        out.push(format!("{}{}{}", "  ".repeat(depth), name, suffix));
        budget -= 1;
    }
    out.join("\n")
}

/// 读取关键文件内容（截断、控制总量）。
fn key_files(root: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut total = 0usize;
    for rel in KEY_FILES {
        if total >= MAX_TOTAL_FILE_CHARS {
            break;
        }
        let path = root.join(rel);
        if !path.is_file() {
            continue;
        }
        let Ok(data) = std::fs::read_to_string(&path) else {
            continue;
        };
        if data.trim().is_empty() {
            continue;
        }
        let take = data.chars().take(MAX_FILE_CHARS).collect::<String>();
        let truncated = take.chars().count() < data.chars().count();
        total += take.chars().count();
        parts.push(format!(
            "=== {} ===\n{}{}\n",
            rel,
            take,
            if truncated { "\n...（已截断）" } else { "" }
        ));
    }
    parts.join("\n")
}

/// 生成完整扫描上下文。
pub fn scan_project(path: &str) -> Result<String, String> {
    let root = Path::new(path);
    if !root.is_dir() {
        return Err(format!("项目目录不存在: {}", path));
    }
    let name = project_name(path);
    let tree = build_tree(root);
    let files = key_files(root);
    Ok(format!(
        "项目名称：{name}\n项目路径：{path}\n\n## 目录结构\n{tree}\n\n## 关键文件\n{files}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_name_from_path() {
        assert_eq!(project_name("E:/pro/demo"), "demo");
        assert_eq!(project_name("E:/pro/demo/"), "demo");
    }

    #[test]
    fn skips_known_dirs() {
        assert!(should_skip_dir("node_modules"));
        assert!(should_skip_dir(".git"));
        assert!(!should_skip_dir("src"));
    }
}
