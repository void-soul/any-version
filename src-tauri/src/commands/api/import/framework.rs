use super::*;

// ─── 框架扫描：Nest / Nuxt / Spring ───

pub fn scan_framework(dir: &str, framework: &str) -> Result<Vec<EndpointDraft>, String> {
    let root = Path::new(dir);
    if !root.is_dir() {
        return Err("目录不存在或不可读".to_string());
    }
    let mut files = Vec::new();
    let mut max_files = 8000usize;
    walk_files(root, 0, 14, &mut max_files, &mut files);
    if files.is_empty() {
        return Err("目录中没有找到可扫描的文件".to_string());
    }
    let mut drafts: Vec<EndpointDraft> = Vec::new();
    match framework {
        "nest" => {
            for f in files
                .iter()
                .filter(|f| f.extension().map(|e| e == "ts").unwrap_or(false))
            {
                if f.file_name()
                    .map(|n| n.to_string_lossy().contains("controller"))
                    .unwrap_or(false)
                {
                    if let Ok(content) = std::fs::read_to_string(f) {
                        drafts.extend(scan_nest_controller(&content, root, f));
                    }
                }
            }
        }
        "nuxt" => {
            for f in files
                .iter()
                .filter(|f| f.extension().map(|e| e == "ts").unwrap_or(false))
            {
                if let Ok(content) = std::fs::read_to_string(f) {
                    drafts.extend(scan_nuxt_api(&content, root, f));
                }
            }
        }
        "spring" => {
            for f in files
                .iter()
                .filter(|f| f.extension().map(|e| e == "java").unwrap_or(false))
            {
                if f.file_name()
                    .map(|n| n.to_string_lossy().contains("Controller"))
                    .unwrap_or(false)
                {
                    if let Ok(content) = std::fs::read_to_string(f) {
                        drafts.extend(scan_spring_controller(&content));
                    }
                }
            }
        }
        other => {
            return Err(format!(
                "未知框架类型: {}（支持 nest / nuxt / spring）",
                other
            ))
        }
    }
    if drafts.is_empty() {
        return Err(format!("在 {} 中未解析到接口定义", framework));
    }
    // 去重（method + url）
    let mut seen = std::collections::HashSet::new();
    drafts.retain(|d| seen.insert(format!("{} {}", d.method, d.url)));
    Ok(drafts)
}

pub(super) fn rel_path(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}

pub(super) fn extract_decorator<'a>(content: &'a str, name: &str) -> Option<&'a str> {
    let marker = format!("@{}(", name);
    if let Some(idx) = content.find(&marker) {
        let after = &content[idx + marker.len()..];
        let trimmed = after.trim_start();
        if trimmed.starts_with('\'') || trimmed.starts_with('"') {
            let quote = trimmed.chars().next().unwrap();
            let rest = &trimmed[1..];
            if let Some(end) = rest.find(quote) {
                let val = &rest[..end];
                if !val.contains('(') {
                    return Some(val);
                }
            }
        }
    }
    None
}

/// NestJS Controller 扫描。
pub(super) fn scan_nest_controller(content: &str, root: &Path, file: &Path) -> Vec<EndpointDraft> {
    let mut drafts = Vec::new();
    let prefix = extract_decorator(content, "Controller").unwrap_or("");
    let module = extract_decorator(content, "ApiTags")
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            file.file_name()
                .map(|n| n.to_string_lossy().replace(".controller.ts", ""))
                .unwrap_or_default()
        });
    for method in ["Get", "Post", "Put", "Delete", "Patch"] {
        let marker = format!("@{}(", method);
        let mut search_from = 0;
        while let Some(idx) = content[search_from..].find(&marker) {
            let abs = search_from + idx;
            let after = &content[abs + marker.len()..];
            let path = after.trim_start();
            let path_val = if path.starts_with('\'') || path.starts_with('"') {
                let quote = path.chars().next().unwrap();
                if let Some(end) = path[1..].find(quote) {
                    path[1..1 + end].to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            let url = ensure_leading_slash(&join_url(prefix, &path_val));
            let after_decorator = &content[abs + marker.len()..];
            let func_name = extract_func_name(after_decorator);
            drafts.push(EndpointDraft {
                module: module.to_string(),
                name: if func_name.is_empty() {
                    format!("{} {}", method.to_uppercase(), url)
                } else {
                    func_name
                },
                method: method.to_uppercase(),
                url,
                docs_md: format!("来源: {}", rel_path(root, file)),
                ..Default::default()
            });
            search_from = abs + marker.len() + 1;
        }
    }
    drafts
}

/// Nuxt3 server/api 扫描：路由从文件相对路径推导，method 从后缀推导。
pub(super) fn scan_nuxt_api(content: &str, root: &Path, file: &Path) -> Vec<EndpointDraft> {
    let _ = content;
    let rel = rel_path(root, file);
    let mut path = rel.trim_start_matches("server/").to_string();
    if path.starts_with("api/") {
        path = format!("/{}", path);
    } else if path.starts_with("routes/") {
        path = format!("/{}", path);
    } else {
        return Vec::new();
    }
    let mut method = "GET".to_string();
    for (suffix, m) in [
        (".get", "GET"),
        (".post", "POST"),
        (".put", "PUT"),
        (".delete", "DELETE"),
        (".patch", "PATCH"),
    ] {
        if path.ends_with(&format!("{}.ts", suffix)) {
            path = path.trim_end_matches(&format!("{}.ts", suffix)).to_string();
            method = m.to_string();
            break;
        }
    }
    if path.ends_with(".ts") {
        path = path.trim_end_matches(".ts").to_string();
    }
    if path.ends_with("/index") {
        path = path.trim_end_matches("/index").to_string();
    }
    let name = path
        .trim_start_matches('/')
        .split('/')
        .last()
        .unwrap_or("api")
        .to_string();
    Vec::from([EndpointDraft {
        module: "API".to_string(),
        name: format!("{} {}", method, name),
        method,
        url: path,
        docs_md: format!("来源: {}", rel),
        ..Default::default()
    }])
}

/// Spring Controller 扫描。
pub(super) fn scan_spring_controller(content: &str) -> Vec<EndpointDraft> {
    let mut drafts = Vec::new();
    let class_prefix = extract_decorator(content, "RequestMapping").unwrap_or("");
    let module = content
        .lines()
        .find(|l| l.contains("class") && l.contains("Controller"))
        .map(|l| {
            l.split("class")
                .nth(1)
                .map(|s| s.trim().split_whitespace().next().unwrap_or("").to_string())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    for method in [
        "GetMapping",
        "PostMapping",
        "PutMapping",
        "DeleteMapping",
        "PatchMapping",
        "RequestMapping",
    ] {
        let marker = format!("@{}(", method);
        let mut search_from = 0;
        while let Some(idx) = content[search_from..].find(&marker) {
            let abs = search_from + idx;
            let after = &content[abs + marker.len()..];
            if method == "RequestMapping" {
                let head = after.to_uppercase();
                let head = &head[..head.len().min(200)];
                if !head.contains("METHOD") {
                    search_from = abs + marker.len() + 1;
                    continue;
                }
            }
            let trimmed = after.trim_start();
            let path_val = if trimmed.starts_with('\'') || trimmed.starts_with('"') {
                let quote = trimmed.chars().next().unwrap();
                if let Some(end) = trimmed[1..].find(quote) {
                    trimmed[1..1 + end].to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            let mut effective_method = method.replace("Mapping", "").to_uppercase();
            if effective_method == "REQUEST" {
                effective_method = "GET".to_string();
                let upper = content[abs..].to_uppercase();
                for m in ["GET", "POST", "PUT", "DELETE", "PATCH"] {
                    if upper[..upper.len().min(120)].contains(&format!("REQUESTMETHOD.{}", m)) {
                        effective_method = m.to_string();
                        break;
                    }
                }
            }
            if effective_method == "REQUEST" {
                effective_method = "GET".to_string();
            }
            let url = ensure_leading_slash(&join_url(class_prefix, &path_val));
            let func_name = extract_func_name(after);
            drafts.push(EndpointDraft {
                module: module.clone(),
                name: if func_name.is_empty() {
                    format!("{} {}", effective_method, url)
                } else {
                    func_name
                },
                method: effective_method,
                url,
                ..Default::default()
            });
            search_from = abs + marker.len() + 1;
        }
    }
    drafts
}
