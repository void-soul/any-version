use super::*;

/// 源文件扩展名 → 语言（决定 import 解析方式）。
pub(super) fn lang_of(rel: &str) -> Option<&'static str> {
    let ext = rel.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" | "vue" | "svelte" => {
            Some("js")
        }
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
pub(super) fn last_quoted(line: &str) -> Option<String> {
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
pub(super) fn extract_line_imports(line: &str, lang: &str, out: &mut Vec<String>) {
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
                let rest = t["import ".len()..]
                    .trim_end_matches(';')
                    .trim()
                    .to_string();
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
pub(super) fn go_module_prefix(root: &Path) -> Option<String> {
    let data = std::fs::read_to_string(root.join("go.mod")).ok()?;
    data.lines()
        .find(|l| l.trim_start().starts_with("module "))
        .map(|l| l.split_whitespace().nth(1).unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 各 crate 的 src 根（Cargo.toml 所在目录的 src/，存在才计）。
pub(super) fn crate_src_dirs(entries: &[Entry]) -> Vec<String> {
    let mut out = Vec::new();
    for e in entries {
        if e.is_dir {
            continue;
        }
        if e.rel.ends_with("Cargo.toml") {
            let dir = e.rel.trim_end_matches("Cargo.toml").trim_end_matches('/');
            // 根级 Cargo.toml（rel 为空）的 crate src 就是 "src"
            let src = if dir.is_empty() {
                "src".to_string()
            } else {
                format!("{}/src", dir)
            };
            if !out.contains(&src) {
                out.push(src);
            }
        }
    }
    out
}

/// Rust 文件在 crate 内的模块路径（commands/network.rs → [commands, network]；mod/lib/main.rs 表示所在目录模块）。
pub(super) fn rust_module_path(file_rel: &str, crate_src: &str) -> Vec<String> {
    let rel = file_rel
        .strip_prefix(crate_src)
        .unwrap_or(file_rel)
        .trim_start_matches('/');
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
pub(super) fn dir_module_key(dir_rel: &str) -> String {
    let segs: Vec<&str> = dir_rel.split('/').filter(|s| !s.is_empty()).collect();
    match segs.len() {
        0 => String::new(),
        1 => segs[0].to_string(),
        _ => format!("{}/{}", segs[0], segs[1]),
    }
}

/// 文件 → 模块键（非 Rust：所在目录的前 2 段；Rust：crate src + 模块路径）。
pub(super) fn file_module_key(file_rel: &str, lang: Option<&str>, crate_srcs: &[String]) -> String {
    if lang == Some("rust") {
        if let Some(cs) = crate_srcs.iter().find(|c| file_rel.starts_with(c.as_str())) {
            let mp = rust_module_path(file_rel, cs);
            return if mp.is_empty() {
                cs.clone()
            } else {
                format!("{}/{}", cs, mp.join("/"))
            };
        }
    }
    let idx = file_rel.rfind('/').map(|i| i + 1).unwrap_or(0);
    dir_module_key(&file_rel[..idx])
}

/// 解析 import 目标为模块键（内部依赖才返回 Some）。
pub(super) fn resolve_import_key(
    root: &Path,
    file_rel: &str,
    lang: Option<&str>,
    spec: &str,
    crate_srcs: &[String],
    go_module: Option<&str>,
    top_dirs: &std::collections::HashSet<&str>,
) -> Option<String> {
    let s = spec
        .split(['?', '#'])
        .next()
        .unwrap_or(spec)
        .trim()
        .trim_matches('"');
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
            target
                .rsplit('/')
                .next()
                .map(|_| &target[..target.len() - target.rsplit('/').next().unwrap().len()])
                .unwrap_or(&target)
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
                let base = if parent.is_empty() {
                    cs.clone()
                } else {
                    format!("{}/{}", cs, parent.join("/"))
                };
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
pub(super) fn analyze_coupling(root: &Path, entries: &[Entry]) -> Option<String> {
    let crate_srcs = crate_src_dirs(entries);
    let go_module = go_module_prefix(root);
    let top_dirs: std::collections::HashSet<&str> = entries
        .iter()
        .filter(|e| !e.is_dir)
        .filter_map(|e| e.rel.split('/').next())
        .collect();

    // 按路径排序后取前 MAX_IMPORT_FILES 个源文件（确定性）
    let mut files: Vec<&Entry> = entries
        .iter()
        .filter(|e| !e.is_dir)
        .filter(|e| lang_of(&e.rel).is_some())
        .collect();
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    files.truncate(MAX_IMPORT_FILES);
    if files.is_empty() {
        return None;
    }

    let mut edges: std::collections::BTreeMap<(String, String), usize> =
        std::collections::BTreeMap::new();
    let mut fan_in: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut total = 0usize;
    let mut cross = 0usize;
    let mut internal = 0usize;
    let mut n_files = 0usize;

    for e in &files {
        let path = root.join(&e.rel);
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.len() > MAX_IMPORT_FILE_BYTES {
            continue;
        }
        let Ok(data) = std::fs::read_to_string(&path) else {
            continue;
        };
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
            let Some(to_key) = resolve_import_key(
                root,
                &e.rel,
                lang,
                &spec,
                &crate_srcs,
                go_module.as_deref(),
                &top_dirs,
            ) else {
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
