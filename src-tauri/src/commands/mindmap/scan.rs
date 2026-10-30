//! 扫描项目目录，生成供 AI 分析用的文本上下文（借鉴 Archify 的「仓库证据」思路）：
//! 1. 技术栈与依赖摘要（解析常见 manifest，突出框架）
//! 2. 顶层目录规模统计（帮助 AI 判断模块边界与规模）
//! 3. README 结构摘要（标题大纲 + 首段）
//! 4. 项目标记（CI/容器化/API 定义/数据库迁移/路由等证据）
//! 5. 目录树（限制深度与条目数，跳过依赖/构建产物目录）
//! 6. 关键文件内容（manifest、README、入口文件等，逐文件截断）

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
const MAX_ENTRIES: usize = 2000;

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

/// 高价值依赖名 → 技术栈标签（用于在证据中突出框架/核心库）。
const FRAMEWORK_LABELS: &[(&str, &str)] = &[
    ("react", "React"),
    ("react-dom", "React"),
    ("react-native", "React Native"),
    ("vue", "Vue"),
    ("next", "Next.js"),
    ("nuxt", "Nuxt"),
    ("svelte", "Svelte"),
    ("solid-js", "Solid"),
    ("angular", "Angular"),
    ("sveltekit", "SvelteKit"),
    ("tauri", "Tauri"),
    ("electron", "Electron"),
    ("express", "Express"),
    ("fastify", "Fastify"),
    ("koa", "Koa"),
    ("nestjs", "NestJS"),
    ("axum", "Axum"),
    ("actix-web", "Actix"),
    ("rocket", "Rocket"),
    ("tokio", "Tokio"),
    ("serde", "serde"),
    ("egui", "egui"),
    ("yew", "Yew"),
    ("dioxus", "Dioxus"),
    ("flask", "Flask"),
    ("django", "Django"),
    ("fastapi", "FastAPI"),
    ("spring-boot", "Spring Boot"),
    ("spring-web", "Spring"),
    ("gin", "Gin"),
    ("echo", "Echo"),
    ("fiber", "Fiber"),
    ("jquery", "jQuery"),
    ("tailwindcss", "Tailwind"),
    ("typescript", "TypeScript"),
    ("zustand", "Zustand"),
    ("redux", "Redux"),
    ("sqlx", "SQLx"),
    ("diesel", "Diesel"),
    ("prisma", "Prisma"),
    ("bevy", "Bevy"),
    ("winit", "winit"),
    ("leptos", "Leptos"),
    ("iced", "Iced"),
];

/// 项目标记：精确文件名（存在即检测到）。
const MARKER_FILES: &[&str] = &[
    "openapi.yaml",
    "openapi.yml",
    "swagger.json",
    "swagger.yaml",
    ".env.example",
    "Dockerfile",
    "docker-compose.yml",
    "docker-compose.yaml",
    "Makefile",
    "build.rs",
    "router.ts",
    "router.js",
    "src/router.ts",
    "src/router.js",
    "src/App.tsx",
    "src/App.js",
    "prisma/schema.prisma",
];

/// 项目标记：路径前缀（目录式证据，rel 以该前缀开头即检测到）。
const MARKER_PREFIXES: &[&str] = &[
    ".github/workflows/",
    "docs/",
    "prisma/",
    "migrations/",
    "db/migrations/",
    "k8s/",
    "helm/",
    "deploy/",
    "manifests/",
    "scripts/",
    "tests/",
    "test/",
    "__tests__/",
    "routes/",
    "src/routes/",
    "src/controllers/",
    "src/services/",
    "src/pages/",
    "src/components/",
];

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

// ─── 条目收集（一次遍历，供目录树/规模/标记复用） ───

struct Entry {
    rel: String,
    is_dir: bool,
}

fn collect_entries(root: &Path) -> Vec<Entry> {
    // Project-authored ignore rules (.gitignore / .claudeignore / .git/info/exclude)
    // take precedence: anything they ignore is never scanned or read.
    let ignore = crate::commands::mindmap::ignore_rules::IgnoreRules::load(root);
    let mut out: Vec<Entry> = Vec::new();
    let mut iter = WalkDir::new(root)
        .min_depth(1)
        .max_depth(MAX_TREE_DEPTH)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            // 剪枝：不进入被忽略的目录（隐藏目录除 .github 外）
            if e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            if e.file_type().is_dir() && should_skip_dir(&name) {
                return false;
            }
            true
        });
    loop {
        if out.len() >= MAX_ENTRIES {
            break;
        }
        match iter.next() {
            Some(Ok(e)) => {
                let rel = e
                    .path()
                    .strip_prefix(root)
                    .unwrap_or(e.path())
                    .to_string_lossy()
                    .replace('\\', "/");
                if rel.is_empty() {
                    continue;
                }
                // Project ignore rules: skip ignored files and prune ignored
                // directories (their contents are ignored implicitly).
                let is_dir = e.file_type().is_dir();
                if ignore.is_ignored(&rel, is_dir) {
                    if is_dir {
                        iter.skip_current_dir();
                    }
                    continue;
                }
                out.push(Entry { rel, is_dir });
            }
            Some(Err(_)) => continue,
            None => break,
        }
    }
    out
}

/// 构建目录树文本（目录在前，文件在后）。
fn build_tree(entries: &[Entry]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut budget = MAX_TREE_LINES;
    let mut dirs: Vec<&Entry> = entries.iter().filter(|e| e.is_dir).collect();
    let mut files: Vec<&Entry> = entries.iter().filter(|e| !e.is_dir).collect();
    dirs.sort_by(|a, b| a.rel.cmp(&b.rel));
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    for e in dirs.into_iter().chain(files.into_iter()) {
        if budget == 0 {
            out.push("...（目录树已截断）".to_string());
            break;
        }
        let depth = e
            .rel
            .split('/')
            .count()
            .saturating_sub(1)
            .min(MAX_TREE_DEPTH);
        let name = e.rel.rsplit('/').next().unwrap_or(&e.rel);
        out.push(format!(
            "{}{}{}",
            "  ".repeat(depth),
            name,
            if e.is_dir { "/" } else { "" }
        ));
        budget -= 1;
    }
    out.join("\n")
}

/// 顶层目录规模：一级目录名 → 其下（限深）文件数，按文件数降序。
fn top_dir_stats(entries: &[Entry]) -> String {
    let mut map: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for e in entries {
        if e.is_dir || !e.rel.contains('/') {
            continue; // 跳过目录与根目录下的散文件（散文件没有归属目录）
        }
        let top = e.rel.split('/').next().unwrap_or("").to_string();
        if !top.is_empty() {
            *map.entry(top).or_default() += 1;
        }
    }
    let mut out: Vec<(String, usize)> = map.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out.truncate(20);
    out.iter()
        .map(|(n, c)| format!("{}/ · {} files", n, c))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 项目标记：检测 CI/容器化/API 定义/数据库迁移/路由/前端入口等证据。
fn project_markers(root: &Path, entries: &[Entry]) -> String {
    let mut found: Vec<String> = Vec::new();
    for f in MARKER_FILES {
        if root.join(f).is_file() {
            found.push((*f).to_string());
        }
    }
    for e in entries {
        for p in MARKER_PREFIXES {
            if e.rel.starts_with(p) && !found.iter().any(|s| s == p.trim_end_matches('/')) {
                found.push(p.trim_end_matches('/').to_string());
            }
        }
    }
    found.sort();
    if found.is_empty() {
        return "（未检测到明显的 CI/部署/API 定义/路由标记）".to_string();
    }
    format!("检测到：{}", found.join("、"))
}

/// 依赖清单：从常见 manifest 提取依赖名（去版本），识别框架，控制数量。
fn scan_deps(root: &Path) -> String {
    let mut deps: Vec<String> = Vec::new();
    let mut push_unique = |name: String| {
        if !name.is_empty() && !deps.iter().any(|d| d == &name) {
            deps.push(name);
        }
    };
    // package.json：dependencies / devDependencies
    if let Ok(data) = std::fs::read_to_string(root.join("package.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) {
            for sec in ["dependencies", "devDependencies"] {
                if let Some(map) = v.get(sec).and_then(|x| x.as_object()) {
                    for (name, ver) in map {
                        push_unique(format!("{}@{}", name, ver.as_str().unwrap_or("?")));
                    }
                }
            }
        }
    }
    // Cargo.toml：任意 [dependencies]/[dev-dependencies] 段
    if let Ok(data) = std::fs::read_to_string(root.join("Cargo.toml")) {
        let mut in_deps = false;
        for line in data.lines() {
            let t = line.trim();
            if t.starts_with('[') {
                in_deps = t.starts_with("[dependencies") || t.starts_with("[dev-dependencies");
                continue;
            }
            if in_deps && !t.is_empty() && !t.starts_with('#') {
                let name = t
                    .split(['=', ':'])
                    .next()
                    .map(|s| s.trim().trim_matches('"').to_string())
                    .unwrap_or_default();
                let name = name.trim_end_matches(".version").to_string();
                if !name.is_empty() && !name.contains(' ') && !name.contains('{') {
                    push_unique(name);
                }
            }
        }
    }
    // pyproject.toml：[project] dependencies / [tool.poetry.dependencies]
    if let Ok(data) = std::fs::read_to_string(root.join("pyproject.toml")) {
        for line in data.lines() {
            let t = line.trim();
            if t.starts_with("dependencies") || t.starts_with("dependencies ") {
                continue;
            }
            let cleaned = t.trim_start_matches('"').trim_start_matches('\'');
            let name = cleaned
                .split(['=', '<', '>', '!', '[', ';', ' ', '"', '\''])
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .to_string();
            if !name.is_empty()
                && !name.contains(']')
                && !name.contains('{')
                && !name.contains("tool")
                && name != "."
            {
                push_unique(name);
            }
        }
    }
    // requirements.txt
    if let Ok(data) = std::fs::read_to_string(root.join("requirements.txt")) {
        for line in data.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') || t.starts_with("-r") || t.starts_with("--") {
                continue;
            }
            let name = t
                .split(['=', '<', '>', '!', '[', ';', ' '])
                .next()
                .unwrap_or("")
                .to_string();
            push_unique(name);
        }
    }
    // go.mod
    if let Ok(data) = std::fs::read_to_string(root.join("go.mod")) {
        for line in data.lines() {
            let t = line.trim();
            if t.is_empty()
                || t.starts_with("//")
                || t.starts_with("module ")
                || t.starts_with("go ")
            {
                continue;
            }
            let name = t.split_whitespace().next().unwrap_or("").to_string();
            if name != "require" && !name.is_empty() && !name.ends_with(')') {
                push_unique(name);
            }
        }
    }
    // composer.json：require / require-dev
    if let Ok(data) = std::fs::read_to_string(root.join("composer.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) {
            for sec in ["require", "require-dev"] {
                if let Some(map) = v.get(sec).and_then(|x| x.as_object()) {
                    for (name, ver) in map {
                        push_unique(format!("{}@{}", name, ver.as_str().unwrap_or("?")));
                    }
                }
            }
        }
    }
    // Gemfile：gem 'name'
    if let Ok(data) = std::fs::read_to_string(root.join("Gemfile")) {
        for line in data.lines() {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix("gem ") {
                let name = rest
                    .trim()
                    .trim_start_matches(|c| c == '\'' || c == '"')
                    .split(|c| c == '\'' || c == '"' || c == ',')
                    .next()
                    .unwrap_or("")
                    .to_string();
                push_unique(name);
            }
        }
    }

    deps.truncate(40);
    if deps.is_empty() {
        return String::new();
    }
    let labels: Vec<&str> = FRAMEWORK_LABELS
        .iter()
        .filter(|(n, _)| {
            deps.iter()
                .any(|d| d == n || d.starts_with(&format!("{}@", n)))
        })
        .map(|(_, l)| *l)
        .collect();
    let mut out = String::new();
    if !labels.is_empty() {
        out.push_str(&format!("重点框架/库：{}\n", labels.join("、")));
    }
    out.push_str(&format!(
        "依赖清单（前 {} 项）：{}",
        deps.len(),
        deps.join("、")
    ));
    out
}

/// README 结构摘要：标题大纲（前 24 个）+ 首段（前 160 字）。
fn readme_structure(root: &Path) -> String {
    let path = root.join("README.md");
    if !path.is_file() {
        return String::new();
    }
    let Ok(data) = std::fs::read_to_string(&path) else {
        return String::new();
    };
    let headings: Vec<String> = data
        .lines()
        .take(160)
        .filter(|l| l.starts_with('#'))
        .map(|l| l.trim().to_string())
        .collect();
    let first_para: String = data
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("")
        .chars()
        .take(160)
        .collect();
    let mut out = String::new();
    if !headings.is_empty() {
        out.push_str(
            &headings
                .iter()
                .take(24)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    if !first_para.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("首段：{}", first_para));
    }
    out
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
            if truncated {
                "\n...（已截断）"
            } else {
                ""
            }
        ));
    }
    parts.join("\n")
}

// ─── 跨文件 import 统计（模块耦合，供 AI 判断分层） ───

const MAX_IMPORT_FILES: usize = 500;
const MAX_IMPORT_FILE_BYTES: u64 = 256 * 1024;

/// 项目文件集上下文：供 AI 证据锚定校验（存在性 + 内容相关度）使用。
pub struct ProjectFiles {
    /// 项目根目录（用于读取文件内容做相关度检查）
    pub root: std::path::PathBuf,
    /// 项目内全部文件（项目相对路径，含扩展名）
    pub files: std::collections::HashSet<String>,
}

/// 收集项目文件集（存在性校验 + 内容相关度校验用）。
pub fn collect_project_files(path: &str) -> Result<ProjectFiles, String> {
    let root = Path::new(path);
    if !root.is_dir() {
        return Err(format!("项目目录不存在: {}", path));
    }
    Ok(ProjectFiles {
        root: root.to_path_buf(),
        files: collect_entries(root)
            .into_iter()
            .filter(|e| !e.is_dir)
            .map(|e| e.rel)
            .collect(),
    })
}

/// CLAUDE.md / AGENTS.md: project-authored guidance for AI agents.
/// Included verbatim (truncated) — it describes what the project *is* and
/// how its modules relate, exactly the context a mindmap needs.
fn agent_docs(root: &Path) -> String {
    const DOCS: &[&str] = &["CLAUDE.md", "AGENTS.md", ".claude/CLAUDE.md"];
    let mut parts: Vec<String> = Vec::new();
    for name in DOCS {
        let path = root.join(name);
        if !path.is_file() {
            continue;
        }
        let Ok(data) = std::fs::read_to_string(&path) else {
            continue;
        };
        let t = data.trim();
        if t.is_empty() {
            continue;
        }
        const MAX_DOC_CHARS: usize = 4000;
        let take: String = t.chars().take(MAX_DOC_CHARS).collect();
        let truncated = t.chars().count() > MAX_DOC_CHARS;
        parts.push(format!(
            "=== {} ==={}\n{}",
            name,
            if truncated { "（已截断）" } else { "" },
            take
        ));
    }
    parts.join("\n\n")
}

/// 生成完整扫描上下文（仓库证据 + 目录树 + 关键文件）。
pub fn scan_project(path: &str) -> Result<String, String> {
    let root = Path::new(path);
    if !root.is_dir() {
        return Err(format!("项目目录不存在: {}", path));
    }
    let entries = collect_entries(root);
    scan_project_from_entries(path, root, &entries)
}

/// 从已收集的条目集构建扫描上下文（供缓存路径复用同一次目录遍历）。
fn scan_project_from_entries(path: &str, root: &Path, entries: &[Entry]) -> Result<String, String> {
    let name = project_name(path);
    let tree = build_tree(entries);
    let deps = scan_deps(root);
    let dirs = top_dir_stats(entries);
    let coupling = analyze_coupling(root, entries);
    let readme = readme_structure(root);
    let markers = project_markers(root, entries);
    let files = key_files(root);

    let mut parts = vec![format!("项目名称：{name}"), format!("项目路径：{path}")];
    if !deps.is_empty() {
        parts.push(format!("## 技术栈与依赖\n{deps}"));
    }
    if !dirs.is_empty() {
        parts.push(format!("## 目录规模\n{dirs}"));
    }
    if let Some(coupling) = coupling {
        parts.push(format!("## 模块耦合\n{coupling}"));
    }
    if !readme.is_empty() {
        parts.push(format!("## README 结构\n{readme}"));
    }
    parts.push(format!("## 项目标记\n{markers}"));
    let agents = agent_docs(root);
    if !agents.is_empty() {
        parts.push(format!("## 项目自述（CLAUDE.md / AGENTS.md）\n{agents}"));
    }
    parts.push(format!("## 目录结构\n{tree}"));
    parts.push(format!("## 关键文件\n{files}"));
    Ok(parts.join("\n\n"))
}

// ─── AI-driven deep read（多轮探索：AI 请求文件批次，工具读取并压缩内容） ───

/// 带用户自定义提示的扫描：`user_hint` 作为附加指令追加到扫描上下文末尾，
/// 例如让 AI 忽略特定目录（node_modules/debug 等）或聚焦某模块。
// ─── 扫描结果缓存（同一项目重复导入时跳过全盘扫描，加快二次分析） ───

/// 扫描器逻辑版本：目录树/统计/证据等产出的组织方式变更时递增，自动作废旧缓存。
const SCAN_CACHE_VERSION: u32 = 1;
/// 缓存目录：数据目录下 scan-cache/<指纹 md5>.json；上限 N 份，超出后淘汰最旧的 mtime。
const SCAN_CACHE_MAX_ENTRIES: usize = 16;

/// 缓存条目落盘格式。
#[derive(serde::Serialize, serde::Deserialize)]
struct ScanCacheEntry {
    /// 与指纹一同校验，防手改/串文件
    fingerprint: String,
    saved_at_ms: u128,
    context: String,
}

mod import_graph;
pub use import_graph::*;
mod deep_read;
pub use deep_read::*;
mod cache;
pub use cache::*;
#[cfg(test)]
mod tests;
