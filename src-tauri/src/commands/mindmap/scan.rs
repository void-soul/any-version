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
    ("react", "React"), ("react-dom", "React"), ("react-native", "React Native"),
    ("vue", "Vue"), ("next", "Next.js"), ("nuxt", "Nuxt"), ("svelte", "Svelte"),
    ("solid-js", "Solid"), ("angular", "Angular"), ("sveltekit", "SvelteKit"),
    ("tauri", "Tauri"), ("electron", "Electron"),
    ("express", "Express"), ("fastify", "Fastify"), ("koa", "Koa"), ("nestjs", "NestJS"),
    ("axum", "Axum"), ("actix-web", "Actix"), ("rocket", "Rocket"), ("tokio", "Tokio"),
    ("serde", "serde"), ("egui", "egui"), ("yew", "Yew"), ("dioxus", "Dioxus"),
    ("flask", "Flask"), ("django", "Django"), ("fastapi", "FastAPI"),
    ("spring-boot", "Spring Boot"), ("spring-web", "Spring"), ("gin", "Gin"),
    ("echo", "Echo"), ("fiber", "Fiber"), ("jquery", "jQuery"), ("tailwindcss", "Tailwind"),
    ("typescript", "TypeScript"), ("zustand", "Zustand"), ("redux", "Redux"),
    ("sqlx", "SQLx"), ("diesel", "Diesel"), ("prisma", "Prisma"),
    ("bevy", "Bevy"), ("winit", "winit"), ("leptos", "Leptos"), ("iced", "Iced"),
];

/// 项目标记：精确文件名（存在即检测到）。
const MARKER_FILES: &[&str] = &[
    "openapi.yaml", "openapi.yml", "swagger.json", "swagger.yaml",
    ".env.example", "Dockerfile", "docker-compose.yml", "docker-compose.yaml",
    "Makefile", "build.rs", "router.ts", "router.js", "src/router.ts", "src/router.js",
    "src/App.tsx", "src/App.js", "prisma/schema.prisma",
];

/// 项目标记：路径前缀（目录式证据，rel 以该前缀开头即检测到）。
const MARKER_PREFIXES: &[&str] = &[
    ".github/workflows/", "docs/", "prisma/", "migrations/", "db/migrations/",
    "k8s/", "helm/", "deploy/", "manifests/", "scripts/",
    "tests/", "test/", "__tests__/", "routes/", "src/routes/",
    "src/controllers/", "src/services/", "src/pages/", "src/components/",
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
        let depth = e.rel.split('/').count().saturating_sub(1).min(MAX_TREE_DEPTH);
        let name = e.rel.rsplit('/').next().unwrap_or(&e.rel);
        out.push(format!("{}{}{}", "  ".repeat(depth), name, if e.is_dir { "/" } else { "" }));
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
    out.iter().map(|(n, c)| format!("{}/ · {} files", n, c)).collect::<Vec<_>>().join("\n")
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
            if t.is_empty() || t.starts_with("//") || t.starts_with("module ") || t.starts_with("go ") {
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
        .filter(|(n, _)| deps.iter().any(|d| d == n || d.starts_with(&format!("{}@", n))))
        .map(|(_, l)| *l)
        .collect();
    let mut out = String::new();
    if !labels.is_empty() {
        out.push_str(&format!("重点框架/库：{}\n", labels.join("、")));
    }
    out.push_str(&format!("依赖清单（前 {} 项）：{}", deps.len(), deps.join("、")));
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
        out.push_str(&headings.iter().take(24).cloned().collect::<Vec<_>>().join("\n"));
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
            if truncated { "\n...（已截断）" } else { "" }
        ));
    }
    parts.join("\n")
}

// ─── 跨文件 import 统计（模块耦合，供 AI 判断分层） ───

const MAX_IMPORT_FILES: usize = 500;
const MAX_IMPORT_FILE_BYTES: u64 = 256 * 1024;

/// 源文件扩展名 → 语言（决定 import 解析方式）。
fn lang_of(rel: &str) -> Option<&'static str> {
    let ext = rel.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" | "vue" | "svelte" => Some("js"),
        "rs" => Some("rust"),
        "py" => Some("py"),
        "go" => Some("go"),
        "java" | "kt" | "kts" => Some("jvm"),
        "php" => Some("php"),
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" => Some("c"),
        "rb" => Some("rb"),
        _ => None,
    }
}

/// 最后一个完整引号串的内容（模块说明符一般在行尾；未闭合的引号忽略）。
fn last_quoted(line: &str) -> Option<String> {
    let mut quote: Option<char> = None;
    let mut open = 0usize;
    let mut last: Option<(usize, usize)> = None;
    for (idx, ch) in line.char_indices() {
        match quote {
            None => {
                if ch == '\'' || ch == '"' {
                    quote = Some(ch);
                    open = idx + ch.len_utf8();
                }
            }
            Some(q) => {
                if ch == q {
                    quote = None;
                    last = Some((open, idx));
                }
            }
        }
    }
    let (s, e) = last?;
    Some(line[s..e].to_string())
}

/// 从一行代码提取 import 目标（各语言行级解析，尽力而为）。
fn extract_line_imports(line: &str, lang: &str, out: &mut Vec<String>) {
    let t = line.trim();
    if t.is_empty() || t.starts_with("//") || t.starts_with('#') && lang != "c" {
        return;
    }
    match lang {
        "js" => {
            if t.starts_with("import")
                || t.starts_with("export")
                || t.contains(" from ")
                || t.contains("require(")
            {
                if let Some(s) = last_quoted(t) {
                    out.push(s);
                }
            }
        }
        "rust" => {
            if let Some(rest) = t.strip_prefix("use ") {
                let seg = rest
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .split(" as ")
                    .next()
                    .unwrap_or("")
                    .trim();
                if !seg.is_empty() {
                    out.push(seg.to_string());
                }
            }
        }
        "py" => {
            if let Some(rest) = t.strip_prefix("import ") {
                out.push(rest.split_whitespace().next().unwrap_or("").to_string());
            } else if let Some(rest) = t.strip_prefix("from ") {
                out.push(rest.split_whitespace().next().unwrap_or("").to_string());
            }
        }
        "go" => {
            if t.starts_with("import") || t.starts_with('"') {
                if let Some(s) = last_quoted(t) {
                    let s = s.trim_matches('"');
                    if !s.is_empty() && !s.starts_with('.') {
                        out.push(s.to_string());
                    }
                }
            }
        }
        "jvm" => {
            if t.starts_with("import ") {
                let rest = t["import ".len()..].trim_end_matches(';').trim().to_string();
                if !rest.is_empty() && !rest.contains('*') {
                    out.push(rest);
                }
            }
        }
        "php" => {
            if let Some(rest) = t.strip_prefix("use ") {
                let seg = rest.split([';', '\\']).next().unwrap_or("").trim();
                if !seg.is_empty() && !seg.contains('{') {
                    out.push(seg.to_string());
                }
            }
        }
        "c" => {
            if let Some(rest) = t.strip_prefix("#include \"") {
                if let Some(end) = rest.find('"') {
                    out.push(rest[..end].to_string());
                }
            }
        }
        "rb" => {
            if t.starts_with("require_relative ") {
                if let Some(s) = last_quoted(t) {
                    out.push(s);
                }
            }
        }
        _ => {}
    }
}

/// go.mod 的 module 前缀（用于判定 Go 内部导入）。
fn go_module_prefix(root: &Path) -> Option<String> {
    let data = std::fs::read_to_string(root.join("go.mod")).ok()?;
    data.lines()
        .find(|l| l.trim_start().starts_with("module "))
        .map(|l| l.split_whitespace().nth(1).unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 各 crate 的 src 根（Cargo.toml 所在目录的 src/，存在才计）。
fn crate_src_dirs(entries: &[Entry]) -> Vec<String> {
    let mut out = Vec::new();
    for e in entries {
        if e.is_dir {
            continue;
        }
        if e.rel.ends_with("Cargo.toml") {
            let dir = e.rel.trim_end_matches("Cargo.toml").trim_end_matches('/');
            // 根级 Cargo.toml（rel 为空）的 crate src 就是 "src"
            let src = if dir.is_empty() { "src".to_string() } else { format!("{}/src", dir) };
            if !out.contains(&src) {
                out.push(src);
            }
        }
    }
    out
}

/// Rust 文件在 crate 内的模块路径（commands/network.rs → [commands, network]；mod/lib/main.rs 表示所在目录模块）。
fn rust_module_path(file_rel: &str, crate_src: &str) -> Vec<String> {
    let rel = file_rel.strip_prefix(crate_src).unwrap_or(file_rel).trim_start_matches('/');
    let mut parts: Vec<String> = rel.split('/').map(|s| s.to_string()).collect();
    if parts.is_empty() {
        return parts;
    }
    let last = parts.pop().unwrap_or_default();
    if last != "mod.rs" && last != "lib.rs" && last != "main.rs" {
        parts.push(last.trim_end_matches(".rs").to_string());
    }
    parts
}

/// 目录路径 → 模块键（取前 2 段）。
fn dir_module_key(dir_rel: &str) -> String {
    let segs: Vec<&str> = dir_rel.split('/').filter(|s| !s.is_empty()).collect();
    match segs.len() {
        0 => String::new(),
        1 => segs[0].to_string(),
        _ => format!("{}/{}", segs[0], segs[1]),
    }
}

/// 文件 → 模块键（非 Rust：所在目录的前 2 段；Rust：crate src + 模块路径）。
fn file_module_key(file_rel: &str, lang: Option<&str>, crate_srcs: &[String]) -> String {
    if lang == Some("rust") {
        if let Some(cs) = crate_srcs.iter().find(|c| file_rel.starts_with(c.as_str())) {
            let mp = rust_module_path(file_rel, cs);
            return if mp.is_empty() { cs.clone() } else { format!("{}/{}", cs, mp.join("/")) };
        }
    }
    let idx = file_rel.rfind('/').map(|i| i + 1).unwrap_or(0);
    dir_module_key(&file_rel[..idx])
}

/// 解析 import 目标为模块键（内部依赖才返回 Some）。
fn resolve_import_key(
    root: &Path,
    file_rel: &str,
    lang: Option<&str>,
    spec: &str,
    crate_srcs: &[String],
    go_module: Option<&str>,
    top_dirs: &std::collections::HashSet<&str>,
) -> Option<String> {
    let s = spec.split(['?', '#']).next().unwrap_or(spec).trim().trim_matches('"');
    if s.is_empty() {
        return None;
    }
    let segs: Vec<&str> = s.split('/').collect();
    // 相对导入：./ ../
    if segs[0] == "." || segs[0] == ".." {
        let parent_dir = file_rel.rfind('/').map(|i| &file_rel[..i]).unwrap_or("");
        let parent: Vec<&str> = parent_dir.split('/').filter(|x| !x.is_empty()).collect();
        let mut parts: Vec<&str> = parent.to_vec();
        for seg in &segs {
            match *seg {
                "." => {}
                ".." => {
                    parts.pop();
                }
                other => parts.push(other),
            }
        }
        let target = parts.join("/");
        let first = parts.first().map(|s| s.to_string()).unwrap_or_default();
        if first.is_empty() || !top_dirs.contains(first.as_str()) {
            return None;
        }
        // 目标若是目录直接用，否则去掉最后一段（文件名），得到目录键
        let key_dir = if root.join(&target).is_dir() {
            target.as_str()
        } else {
            target.rsplit('/').next().map(|_| &target[..target.len() - target.rsplit('/').next().unwrap().len()]).unwrap_or(&target)
        };
        return Some(dir_module_key(key_dir));
    }
    // Rust：crate:: / super:: / self::
    if lang == Some("rust") {
        if let Some(cs) = crate_srcs.iter().find(|c| file_rel.starts_with(c.as_str())) {
            if let Some(rest) = s.strip_prefix("crate::") {
                let first_mod = rest.split("::").next().unwrap_or("").trim_end_matches('*');
                return if first_mod.is_empty() {
                    Some(cs.clone())
                } else {
                    Some(format!("{}/{}", cs, first_mod))
                };
            }
            if let Some(rest) = s.strip_prefix("super::") {
                let mut parent = rust_module_path(file_rel, cs);
                parent.pop(); // super = 父模块
                let first_mod = rest.split("::").next().unwrap_or("").trim_end_matches('*');
                let base = if parent.is_empty() { cs.clone() } else { format!("{}/{}", cs, parent.join("/")) };
                return if first_mod.is_empty() {
                    Some(base)
                } else {
                    Some(format!("{}/{}", base, first_mod))
                };
            }
            if s.starts_with("self::") {
                return None; // 同模块自引用，不计
            }
        }
        return None; // std::/外部 crate
    }
    // Go：模块前缀内
    if lang == Some("go") {
        if let Some(m) = go_module {
            if let Some(rest) = s.strip_prefix(m) {
                let first = rest.trim_start_matches('/').split('/').next().unwrap_or("");
                if !first.is_empty() {
                    return Some(first.to_string());
                }
            }
        }
        return None;
    }
    // 别名 @/ ~/ / 根相对
    let first = segs[0];
    if first == "@" || first == "~" || first.is_empty() {
        let inner = segs.get(1).copied().unwrap_or("");
        if inner.is_empty() || !top_dirs.contains(inner) {
            return None;
        }
        return Some(inner.to_string());
    }
    // 裸模块名：首段是项目内目录 → 内部根导入；否则外部依赖
    if top_dirs.contains(first) {
        return Some(first.to_string());
    }
    None
}

/// 分析跨文件 import 耦合，输出模块级依赖（供 AI 判断分层）。
fn analyze_coupling(root: &Path, entries: &[Entry]) -> Option<String> {
    let crate_srcs = crate_src_dirs(entries);
    let go_module = go_module_prefix(root);
    let top_dirs: std::collections::HashSet<&str> = entries
        .iter()
        .filter(|e| !e.is_dir)
        .filter_map(|e| e.rel.split('/').next())
        .collect();

    // 按路径排序后取前 MAX_IMPORT_FILES 个源文件（确定性）
    let mut files: Vec<&Entry> = entries.iter().filter(|e| !e.is_dir).filter(|e| lang_of(&e.rel).is_some()).collect();
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    files.truncate(MAX_IMPORT_FILES);
    if files.is_empty() {
        return None;
    }

    let mut edges: std::collections::BTreeMap<(String, String), usize> = std::collections::BTreeMap::new();
    let mut fan_in: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut total = 0usize;
    let mut cross = 0usize;
    let mut internal = 0usize;
    let mut n_files = 0usize;

    for e in &files {
        let path = root.join(&e.rel);
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if meta.len() > MAX_IMPORT_FILE_BYTES {
            continue;
        }
        let Ok(data) = std::fs::read_to_string(&path) else { continue };
        let lang = lang_of(&e.rel);
        let from_key = file_module_key(&e.rel, lang, &crate_srcs);
        if from_key.is_empty() {
            continue;
        }
        n_files += 1;
        let mut imps: Vec<String> = Vec::new();
        for line in data.lines().take(MAX_IMPORT_FILE_BYTES as usize / 8) {
            extract_line_imports(line, lang.unwrap_or(""), &mut imps);
        }
        for spec in imps {
            total += 1;
            let Some(to_key) = resolve_import_key(root, &e.rel, lang, &spec, &crate_srcs, go_module.as_deref(), &top_dirs) else {
                continue;
            };
            if to_key == from_key {
                internal += 1;
                continue;
            }
            // 目标首段必须是项目内目录（rust crate 路径首段即 crate 目录）
            let to_first = to_key.split('/').next().unwrap_or("").to_string();
            if !top_dirs.contains(to_first.as_str()) {
                continue;
            }
            cross += 1;
            *edges.entry((from_key.clone(), to_key.clone())).or_insert(0) += 1;
            *fan_in.entry(to_key).or_insert(0) += 1;
        }
    }
    if edges.is_empty() && internal == 0 {
        return None;
    }

    let mut lines = vec![format!(
        "扫描 {} 个源文件，共 {} 条 import（跨模块 {} 条，模块内 {} 条）",
        n_files, total, cross, internal
    )];
    let mut edge_list: Vec<((String, String), usize)> = edges.into_iter().collect();
    edge_list.sort_by(|a, b| b.1.cmp(&a.1));
    if !edge_list.is_empty() {
        lines.push("模块依赖（跨模块，按次数降序）：".to_string());
        for ((f, t), c) in edge_list.iter().take(15) {
            lines.push(format!("  {} → {} · {}", f, t, c));
        }
    }
    let mut fan: Vec<(String, usize)> = fan_in.into_iter().collect();
    fan.sort_by(|a, b| b.1.cmp(&a.1));
    if !fan.is_empty() {
        lines.push("被引用最多的模块（fan-in）：".to_string());
        for (m, c) in fan.iter().take(5) {
            lines.push(format!("  {} · {}", m, c));
        }
    }
    Some(lines.join("\n"))
}

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
    let name = project_name(path);
    let entries = collect_entries(root);
    let tree = build_tree(&entries);
    let deps = scan_deps(root);
    let dirs = top_dir_stats(&entries);
    let coupling = analyze_coupling(root, &entries);
    let readme = readme_structure(root);
    let markers = project_markers(root, &entries);
    let files = key_files(root);

    let mut parts = vec![
        format!("项目名称：{name}"),
        format!("项目路径：{path}"),
    ];
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

/// 从 AI 请求 JSON 提取合法相对路径：
/// - paths: 数组字符串（去 ./ 前缀、反斜杠→正斜杠、去重）
/// - dirs: 目录路径 → 展开/标记为「目录请求」（不读内容，只确认存在）
/// 返回 (files, dirs)，均过滤为存在于文件集内的路径。
pub fn parse_ai_file_request(
    json: &serde_json::Value,
    project: &ProjectFiles,
) -> (Vec<String>, Vec<String>) {
    let mut files: Vec<String> = Vec::new();
    let mut dirs: Vec<String> = Vec::new();
    let normalize = |s: &str| -> String {
        s.trim()
            .trim_start_matches("./")
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_string()
    };
    if let Some(arr) = json.get("paths").and_then(|v| v.as_array()) {
        for p in arr.iter().filter_map(|x| x.as_str()) {
            let rel = normalize(p);
            if rel.is_empty() || files.contains(&rel) {
                continue;
            }
            if project.files.contains(&rel) {
                files.push(rel);
            }
        }
    }
    if let Some(arr) = json.get("dirs").and_then(|v| v.as_array()) {
        for d in arr.iter().filter_map(|x| x.as_str()) {
            let rel = normalize(d);
            if rel.is_empty() || dirs.contains(&rel) {
                continue;
            }
            // 目录请求：确认真实存在（有文件以它为前缀）
            if project.files.iter().any(|f| f.starts_with(&format!("{}/", rel))) {
                dirs.push(rel);
            }
        }
    }
    (files, dirs)
}

/// 读取单个文件并压缩为上下文文本（去尾部空白、压缩连续空行、截断）。
/// 返回 (文本, 是否被截断)。不可读（二进制/超大）返回 (None, false)。
pub fn read_file_compressed(
    root: &Path,
    rel: &str,
    max_chars: usize,
) -> (Option<String>, bool) {
    let path = root.join(rel);
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(_) => return (None, false),
    };
    if meta.len() > 256 * 1024 {
        return (None, false); // 二进制/超大：不可读
    }
    let Ok(data) = std::fs::read_to_string(&path) else {
        return (None, false); // 二进制/非 UTF-8
    };
    // 压缩：去行尾空白、折叠 2+ 连续空行为 1、去行首空白
    let mut compressed = String::with_capacity(data.len());
    let mut blank_run = 0usize;
    for line in data.lines() {
        let t = line.trim_end();
        if t.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
            compressed.push('\n');
        } else {
            blank_run = 0;
            compressed.push_str(t);
            compressed.push('\n');
        }
    }
    let truncated = compressed.chars().count() > max_chars;
    let text: String = compressed.chars().take(max_chars).collect();
    (Some(text), truncated)
}

/// 把一批文件读取结果压缩为 AI 上下文文本块。
/// files: (rel, content, truncated)；dirs: 目录确认列表。
pub fn format_file_batch(
    files: &[(String, Option<String>, bool)],
    dirs: &[String],
) -> String {
    let mut out = String::new();
    if !dirs.is_empty() {
        out.push_str(&format!("## 目录确认\n{}\n\n", dirs.join("\n")));
    }
    if files.is_empty() {
        out.push_str("## 文件内容\n（本次请求没有可读文件）\n");
        return out;
    }
    out.push_str("## 文件内容\n");
    for (rel, content, truncated) in files {
        match content {
            Some(text) => {
                out.push_str(&format!(
                    "\n=== {} ==={}\n{}\n",
                    rel,
                    if *truncated { "（已截断）" } else { "" },
                    text
                ));
            }
            None => {
                out.push_str(&format!("\n=== {} ===\n（不可读：二进制或超出大小限制）\n", rel));
            }
        }
    }
    out
}

/// 带用户自定义提示的扫描：`user_hint` 作为附加指令追加到扫描上下文末尾，
/// 例如让 AI 忽略特定目录（node_modules/debug 等）或聚焦某模块。
pub fn scan_project_with_hint(path: &str, user_hint: Option<&str>) -> Result<String, String> {
    let mut context = scan_project(path)?;
    if let Some(hint) = user_hint {
        let hint = hint.trim();
        if !hint.is_empty() {
            context.push_str(&format!("\n\n## 用户附加说明（必须遵守）\n{}", hint));
        }
    }
    Ok(context)
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

    #[test]
    fn scans_self_repo() {
        // 以本仓库为样本做冒烟测试：应产出目录树、目录规模与技术栈摘要
        let ctx = scan_project(".").unwrap_or_else(|e| panic!("scan failed: {e}"));
        assert!(ctx.contains("项目名称"));
        assert!(ctx.contains("## 目录规模"));
        assert!(ctx.contains("## 目录结构"));
    }


    #[test]
    fn top_stats_counts_files() {
        let entries = vec![
            Entry { rel: "src/a.ts".into(), is_dir: false },
            Entry { rel: "src/b.ts".into(), is_dir: false },
            Entry { rel: "src/c/d.ts".into(), is_dir: false },
            Entry { rel: "src".into(), is_dir: true },
            Entry { rel: "docs/readme.md".into(), is_dir: false },
        ];
        let s = top_dir_stats(&entries);
        assert!(s.contains("src/ · 3 files"), "got: {s}");
        assert!(s.contains("docs/ · 1 files"), "got: {s}");
    }

    fn tmp_project(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mm_scan_test_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn coupling_ts_project() {
        let dir = tmp_project("ts");
        for (rel, content) in [
            ("src/main.ts", "import { x } from \"./utils/date\";\nimport y from \"react\";\n"),
            ("src/utils/date.ts", "export const x = 1;\n"),
            ("src/pages/home.tsx", "import d from \"../utils/date\";\n"),
            ("src/pages/about.tsx", "import d from \"../utils/date\";\nimport h from \"./home\";\n"),
        ] {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
        let entries = collect_entries(&dir);
        let out = analyze_coupling(&dir, &entries).unwrap_or_else(|| panic!("no coupling"));
        let _ = std::fs::remove_dir_all(&dir);
        // 相对导入解析到二级模块键；外部依赖 react 被过滤；同模块 ./home 不计跨模块
        assert!(out.contains("src → src/utils"), "got:\n{out}");
        assert!(out.contains("src/pages → src/utils"), "got:\n{out}");
        assert!(!out.contains("react"), "got:\n{out}");
        assert!(out.contains("跨模块 3 条"), "got:\n{out}");
    }

    #[test]
    fn coupling_rust_crate() {
        let dir = tmp_project("rust");
        for (rel, content) in [
            ("mylib/Cargo.toml", "[package]\nname = \"mylib\"\n"),
            ("mylib/src/lib.rs", "pub mod commands;\npub mod utils;\nuse crate::commands::network;\n"),
            ("mylib/src/commands/mod.rs", "pub mod network;\nuse super::utils;\n"),
            ("mylib/src/commands/network.rs", "use crate::utils::fmt;\nuse std::collections::HashMap;\n"),
            ("mylib/src/utils/mod.rs", "pub fn fmt() {}\n"),
        ] {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
        let entries = collect_entries(&dir);
        let out = analyze_coupling(&dir, &entries).unwrap_or_else(|| panic!("no coupling"));
        let _ = std::fs::remove_dir_all(&dir);
        // crate::commands 命中 crate 根模块；super:: 解析到父模块；std:: 外部被过滤
        assert!(out.contains("mylib/src → mylib/src/commands"), "got:\n{out}");
        assert!(out.contains("mylib/src/commands → mylib/src/utils"), "got:\n{out}");
        assert!(!out.contains("std::"), "got:\n{out}");
    }

    #[test]
    fn extract_js_and_rust_imports() {
        let mut v = Vec::new();
        extract_line_imports("import { a } from './x';", "js", &mut v);
        extract_line_imports("const b = require('../y');", "js", &mut v);
        extract_line_imports("export * from '@lib/z';", "js", &mut v);
        assert_eq!(v, vec!["./x", "../y", "@lib/z"]);
        let mut r = Vec::new();
        extract_line_imports("use crate::commands::network;", "rust", &mut r);
        extract_line_imports("use super::*;", "rust", &mut r);
        extract_line_imports("use std::io;", "rust", &mut r);
        assert_eq!(r, vec!["crate::commands::network", "super::*", "std::io"]);
    }

    #[test]
    fn parse_request_filters_and_normalizes_paths() {
        let dir = tmp_project("reqparse");
        std::fs::create_dir_all(dir.join("src/services")).unwrap();
        std::fs::write(dir.join("src/services/order.ts"), "export class Order {}\n").unwrap();
        let pf = collect_project_files(dir.to_str().unwrap()).unwrap();
        let json = serde_json::json!({
            "paths": ["./src/services/order.ts", "src\\services\\order.ts", "src/services/order.ts", "src/missing.ts", "", "src/services/order.ts"],
            "dirs": ["src/services", "src/missing", "src/services/"]
        });
        let (files, dirs) = parse_ai_file_request(&json, &pf);
        let _ = std::fs::remove_dir_all(&dir);
        // 去重 + 归一化（./ 前缀与反斜杠等价）+ 不在文件集内的路径被过滤
        assert_eq!(files, vec!["src/services/order.ts"], "got: {files:?}");
        assert_eq!(dirs, vec!["src/services"], "got: {dirs:?}");
    }

    #[test]
    fn read_file_compressed_and_guarded() {
        let dir = tmp_project("readguard");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("src/a.ts"),
            "line1\n\n\n\nline2   \n  indented\n",
        )
        .unwrap();
        let (text, truncated) = read_file_compressed(&dir, "src/a.ts", 10_000);
        assert_eq!(
            text.as_deref(),
            Some("line1\n\nline2\n  indented\n"),
            "连续空行应折叠，行尾空白应去除"
        );
        assert!(!truncated);
        // 截断标记
        let (text2, truncated2) = read_file_compressed(&dir, "src/a.ts", 5);
        assert!(truncated2);
        assert_eq!(text2.as_deref().map(|t| t.chars().count()), Some(5));
        // 不可读路径
        let (missing, _) = read_file_compressed(&dir, "src/missing.ts", 1000);
        assert!(missing.is_none());
        // 超大文件不可读
        let big = dir.join("src/big.ts");
        std::fs::write(&big, "x".repeat(300 * 1024)).unwrap();
        let (big_text, _) = read_file_compressed(&dir, "src/big.ts", 1000);
        assert!(big_text.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn format_batch_reports_unreadable_and_dirs() {
        let files = vec![
            ("src/a.ts".to_string(), Some("code".to_string()), false),
            ("src/bin.dat".to_string(), None, false),
        ];
        let out = format_file_batch(&files, &["src/services".to_string()]);
        assert!(out.contains("## 目录确认"));
        assert!(out.contains("src/services"));
        assert!(out.contains("=== src/a.ts ==="));
        assert!(out.contains("code"));
        assert!(out.contains("不可读"));
        let empty = format_file_batch(&[], &[]);
        assert!(empty.contains("没有可读文件"));
    }
}
