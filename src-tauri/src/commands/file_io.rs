use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// 读取文本文件内容（JSON 浏览等辅助工具使用）
#[tauri::command]
pub fn read_text_file(path: String) -> Result<String, String> {
    fs::read_to_string(&path).map_err(|e| e.to_string())
}

/// 写入文本文件内容
#[tauri::command]
pub fn write_text_file(path: String, content: String) -> Result<(), String> {
    fs::write(&path, content).map_err(|e| e.to_string())
}

/// Markdown 阅读器：同目录（含子目录）中发现的一个 md 文件。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownEntry {
    /// 绝对路径
    pub path: String,
    /// 文件名（含扩展名）
    pub name: String,
    /// 相对于扫描根目录的路径，用 / 分隔，便于展示与匹配
    pub rel: String,
    /// 字节数
    pub size: u64,
}

const MD_EXTS: [&str; 4] = ["md", "markdown", "mdx", "mdown"];

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| MD_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// 扫描时跳过的目录，避免在 node_modules 这类目录里空转。
fn is_ignored_dir(name: &str) -> bool {
    matches!(
        name,
        "node_modules" | ".git" | "target" | "dist" | "build" | ".next" | ".venv" | "__pycache__"
    )
}

fn walk_markdown(root: &Path, dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<MarkdownEntry>) {
    if out.len() >= 2000 {
        return;
    }
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            if depth < max_depth && !name.starts_with('.') && !is_ignored_dir(&name) {
                subdirs.push(path);
            }
            continue;
        }
        if !is_markdown(&path) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        out.push(MarkdownEntry {
            path: path.to_string_lossy().to_string(),
            name,
            rel,
            size,
        });
    }
    for sub in subdirs {
        walk_markdown(root, &sub, depth + 1, max_depth, out);
    }
}

/// 列出某个 markdown 文件所在目录下的全部 markdown 文件（默认向下递归 3 层）。
///
/// 传入的是「文件路径」或「目录路径」皆可：若为文件则取其父目录作为扫描根。
#[tauri::command]
pub fn list_sibling_markdown(path: String, max_depth: Option<usize>) -> Result<Vec<MarkdownEntry>, String> {
    let p = PathBuf::from(&path);
    let root = if p.is_dir() {
        p
    } else {
        p.parent()
            .map(|x| x.to_path_buf())
            .ok_or_else(|| format!("无法定位目录: {}", path))?
    };
    if !root.is_dir() {
        return Err(format!("目录不存在: {}", root.to_string_lossy()));
    }
    let mut out = Vec::new();
    walk_markdown(&root, &root, 0, max_depth.unwrap_or(3), &mut out);
    out.sort_by(|a, b| a.rel.to_lowercase().cmp(&b.rel.to_lowercase()));
    Ok(out)
}

/// 把 markdown 中的相对链接解析为绝对路径。
///
/// `from` 是当前文档的绝对路径，`href` 是文档里写的链接（可能带 `#锚点` 或 URL 编码）。
/// 找不到时会依次尝试补 `.md` / `README.md`，仍找不到则返回 None。
#[tauri::command]
pub fn resolve_markdown_link(from: String, href: String) -> Result<Option<String>, String> {
    // 去掉锚点与查询串
    let raw = href.split('#').next().unwrap_or("").split('?').next().unwrap_or("");
    if raw.is_empty() {
        return Ok(None);
    }
    // 还原 %20 之类的转义
    let decoded = percent_decode(raw);

    let base = PathBuf::from(&from);
    let dir = base.parent().unwrap_or_else(|| Path::new("."));
    let candidate = if Path::new(&decoded).is_absolute() {
        PathBuf::from(&decoded)
    } else {
        dir.join(&decoded)
    };

    // 依次尝试：原路径 → 补 .md → 目录下的 README.md
    let tries = [
        candidate.clone(),
        PathBuf::from(format!("{}.md", candidate.to_string_lossy())),
        candidate.join("README.md"),
    ];
    for t in tries.iter() {
        if t.is_file() {
            let abs = t.canonicalize().unwrap_or_else(|_| t.clone());
            return Ok(Some(strip_unc(&abs)));
        }
    }
    Ok(None)
}

/// Windows 的 canonicalize 会返回 `\\?\C:\...` 前缀，去掉以免展示与比较出问题。
fn strip_unc(p: &Path) -> String {
    let s = p.to_string_lossy().to_string();
    s.strip_prefix(r"\\?\").map(|x| x.to_string()).unwrap_or(s)
}

/// 最小化的 percent-decode，只处理 %XX，无效序列原样保留。
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(h) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(h);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}
