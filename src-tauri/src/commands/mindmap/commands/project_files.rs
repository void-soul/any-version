use super::*;

/// 绑定/解绑导图的项目目录（一个导图文档至多绑定一个；重复绑定即替换）。
#[tauri::command]
pub fn mm_bind_document_dir(document_id: String, dir: Option<String>) -> Result<(), String> {
    let dir = dir.map(|d| d.trim().to_string()).filter(|d| !d.is_empty());
    if let Some(d) = &dir {
        if !std::path::Path::new(d).is_dir() {
            return Err(format!("目录不存在: {}", d));
        }
    }
    crate::commands::mindmap::db::bind_document_dir(&document_id, dir.as_deref())
}

/// 列出文档绑定目录下的相对文件路径（@ 引用候选；跳过依赖与构建产物目录）。
#[tauri::command]
pub fn mm_list_project_files(document_id: String) -> Result<Vec<String>, String> {
    let dir = crate::commands::mindmap::db::document_project_dir(&document_id)?
        .ok_or("该导图未绑定项目目录")?;
    let root = std::path::PathBuf::from(&dir);
    if !root.is_dir() {
        return Err(format!("绑定的目录不存在: {}", dir));
    }
    let mut out: Vec<String> = Vec::new();
    walk_project_files(&root, &root, 0, &mut out);
    out.sort();
    Ok(out)
}

pub(super) fn walk_project_files(
    root: &std::path::Path,
    dir: &std::path::Path,
    depth: usize,
    out: &mut Vec<String>,
) {
    if depth > PROJECT_WALK_MAX_DEPTH || out.len() >= PROJECT_FILES_MAX {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<std::fs::DirEntry> = rd.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if out.len() >= PROJECT_FILES_MAX {
            return;
        }
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            // 只按跳过清单排除：隐藏目录不再整体屏蔽（见 PROJECT_SKIP_DIRS 注释）
            if PROJECT_SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            walk_project_files(root, &p, depth + 1, out);
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// 读取 @ 引用的附件内容（相对绑定目录解析；单文件截断、总数封顶），拼进上下文。
/// 把用户 @ 引用的文件内容拼进上下文。
///
/// 路径解析：**绝对路径直接用**（盘符 / UNC / Unix 根），相对路径按导图绑定的项目目录拼。
/// 此前只认绑定目录下的相对路径 —— 用户想分析**任意一个文件**（哪怕不在绑定目录里）时，
/// 内容会被静默丢掉，而「分析这个文件并画张图」恰恰是最常见的用法。
///
/// 截断标记只在真的截断时出现（旧实现只要读成功就标「（截断）」，会误导模型以为没读全）。
pub(super) fn agent_attachments_block(document_dir: Option<&str>, attached: &[String]) -> String {
    let dir = document_dir.map(str::trim).filter(|d| !d.is_empty());
    let cap = if attached.len() <= 1 {
        AGENT_ATTACHMENT_CHARS_SINGLE
    } else {
        AGENT_ATTACHMENT_CHARS
    };
    let mut block = String::new();
    let mut used = 0usize;
    for raw in attached {
        if used >= AGENT_MAX_ATTACHMENTS {
            break;
        }
        let rel = raw.trim();
        if rel.is_empty() {
            continue;
        }
        used += 1;
        let Some(path) = resolve_attachment_path(dir, rel) else {
            block.push_str(&format!(
                "\n### 文件 {}\n（未绑定项目目录，无法解析这个相对路径）\n",
                rel
            ));
            continue;
        };
        let (body, truncated) = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let total = text.chars().count();
                if total > cap {
                    (text.chars().take(cap).collect::<String>(), true)
                } else {
                    (text, false)
                }
            }
            Err(_) => ("（读取失败）".to_string(), false),
        };
        block.push_str(&format!(
            "\n### 文件 {}{}（{} 字符）\n{}\n",
            rel,
            if truncated { "（截断）" } else { "" },
            body.chars().count(),
            body
        ));
    }
    block
}

/// 附件路径解析：绝对径直接用；相对路径需要项目目录，没有则 None（由调用方提示）。
pub(super) fn resolve_attachment_path(dir: Option<&str>, rel: &str) -> Option<std::path::PathBuf> {
    if is_absolute_path(rel) {
        return Some(std::path::PathBuf::from(rel));
    }
    dir.map(|d| std::path::Path::new(d).join(rel))
}

/// 绝对路径判定：Unix 根 / UNC / Windows 盘符。
pub(super) fn is_absolute_path(value: &str) -> bool {
    let v = value.trim();
    if v.starts_with('/') || v.starts_with('\\') {
        return true;
    }
    let bytes = v.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}
