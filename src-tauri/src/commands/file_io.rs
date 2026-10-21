use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Serialize;

/// 把文件字节解码为文本：优先按 BOM 识别 UTF-8 / UTF-16，
/// 非 UTF-8 时回退 GBK（兼容中文 Windows 工具导出的文件）。
pub fn decode_text_bytes(bytes: &[u8]) -> String {
    // BOM 识别（UTF-8 EF BB BF / UTF-16 LE FF FE / UTF-16 BE FE FF）
    if let Some((encoding, _)) = encoding_rs::Encoding::for_bom(bytes) {
        let (cow, _, _) = encoding.decode(bytes);
        return cow.into_owned();
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    // 中文 Windows 常见 GBK/GB18030 编码回退
    let (cow, _, _) = encoding_rs::GBK.decode(bytes);
    cow.into_owned()
}

/// 读取文本文件内容（JSON 浏览等辅助工具使用）
/// 兼容 UTF-8 BOM / UTF-16 / GBK 编码，避免「文件有效但读出来乱码/报错」。
#[tauri::command]
pub fn read_text_file(path: String) -> Result<String, String> {
    let bytes = fs::read(&path).map_err(|e| format!("读取文件失败: {}", e))?;
    Ok(decode_text_bytes(&bytes))
}

/// 写入文本文件内容
#[tauri::command]
pub fn write_text_file(path: String, content: String) -> Result<(), String> {
    fs::write(&path, content).map_err(|e| e.to_string())
}

/// 保存文本到下载目录（Buddy 导出账号用），返回实际保存路径。
#[tauri::command]
pub fn file_io_save_text(
    app: tauri::AppHandle,
    file_name: String,
    content: String,
) -> Result<String, String> {
    use tauri::Manager;
    let dir = app.path().download_dir().unwrap_or_default();
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建下载目录失败: {}", e))?;
    }
    let mut path = dir.join(file_name.trim());
    // 同名文件自动追加序号，避免覆盖
    if path.exists() {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "export".to_string());
        let ext = path.extension().map(|s| s.to_string_lossy().to_string());
        let mut counter = 1;
        loop {
            let candidate = match &ext {
                Some(e) => dir.join(format!("{}({}).{}", stem, counter, e)),
                None => dir.join(format!("{}({})", stem, counter)),
            };
            if !candidate.exists() {
                path = candidate;
                break;
            }
            counter += 1;
        }
    }
    fs::write(&path, content).map_err(|e| format!("写入文件失败: {}", e))?;
    Ok(path.to_string_lossy().to_string())
}



/// Windows 文件关联状态：.md / .markdown 是否已指向 any-version 的 AnyMarkdown ProgID。
#[tauri::command]
pub fn markdown_assoc_status() -> Result<MarkdownAssocStatus, String> {
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let hkcr = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Classes");
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut md = false;
        let mut markdown = false;
        if let Ok(k) = hkcr {
            for (ext, val) in [(".md", &mut md), (".markdown", &mut markdown)] {
                if let Ok(default) = k.get_value::<String, _>(ext) {
                    *val = default == "AnyMarkdown.Document";
                }
            }
        }
        Ok(MarkdownAssocStatus { md, markdown, exe_path: exe.to_string_lossy().to_string() })
    }
    #[cfg(not(windows))]
    {
        Ok(MarkdownAssocStatus { md: false, markdown: false, exe_path: String::new() })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownAssocStatus {
    pub md: bool,
    pub markdown: bool,
    pub exe_path: String,
}

/// 注册/解除 .md 与 .markdown 的用户级文件关联（HKCU\Software\Classes，无需管理员）。
///
/// register=true: 写入 AnyMarkdown.Document ProgID（open 命令指向当前 exe），
/// 并把 HKCR\.md / HKCR\.markdown 的用户级默认值指过去；同时在
/// Software\Classes\AnyMarkdown.Document\shell\open\command 下注册。
/// register=false: 若当前默认值仍指向 AnyMarkdown.Document，则删除该默认值。
#[tauri::command]
pub fn set_markdown_assoc(register: bool) -> Result<String, String> {
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
        use winreg::RegKey;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let cmd = format!("\"{}\" \"%1\"", exe.to_string_lossy());
        let classes = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags("Software\\Classes", KEY_SET_VALUE)
            .map_err(|e| format!("打开 HKCU\\Software\\Classes 失败: {}", e))?;
        if register {
            let (progkey, _) = classes.create_subkey("AnyMarkdown.Document")
                .map_err(|e| format!("创建 ProgID 失败: {}", e))?;
            progkey.set_value("", &"Kira Markdown 文档")
                .map_err(|e| e.to_string())?;
            let (openkey, _) = classes.create_subkey("AnyMarkdown.Document\\shell\\open\\command")
                .map_err(|e| format!("创建 open 命令失败: {}", e))?;
            openkey.set_value("", &cmd).map_err(|e| e.to_string())?;
            for ext in [".md", ".markdown"] {
                let (extkey, _) = classes.create_subkey(ext)
                    .map_err(|e| format!("创建 {} 失败: {}", ext, e))?;
                extkey.set_value("", &"AnyMarkdown.Document").map_err(|e| e.to_string())?;
            }
            Ok(format!("已注册 .md/.markdown → {}", cmd))
        } else {
            let mut removed = 0;
            for ext in [".md", ".markdown"] {
                if let Ok(k) = classes.open_subkey_with_flags(ext, KEY_SET_VALUE) {
                    if let Ok(cur) = k.get_value::<String, _>("") {
                        if cur == "AnyMarkdown.Document" {
                            k.delete_value("").map_err(|e| e.to_string())?;
                            removed += 1;
                        }
                    }
                }
            }
            let _ = classes.delete_subkey_all("AnyMarkdown.Document\\shell\\open\\command");
            let _ = classes.delete_subkey_all("AnyMarkdown.Document\\shell\\open");
            let _ = classes.delete_subkey_all("AnyMarkdown.Document\\shell");
            let _ = classes.delete_subkey_all("AnyMarkdown.Document");
            Ok(format!("已解除关联（清理了 {} 项）", removed))
        }
    }
    #[cfg(not(windows))]
    {
        let _ = register;
        Err("文件关联仅支持 Windows".into())
    }
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

// ===================== 预览图片（相对路径 → data URL） =====================

/// 单张图片内联上限：超过就拒绝预览，避免大图把 WebView 内存打满。
const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

fn image_mime(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        _ => return None,
    })
}

/// 把文档里的图片 src 解析成绝对路径：支持相对路径、绝对路径与 file:// 前缀。
fn resolve_image_path(from: &Path, src: &str) -> Option<PathBuf> {
    let raw = src.split('#').next().unwrap_or("").split('?').next().unwrap_or("");
    let raw = raw
        .strip_prefix("file:///")
        .or_else(|| raw.strip_prefix("file://"))
        .unwrap_or(raw);
    if raw.trim().is_empty() {
        return None;
    }
    let decoded = percent_decode(raw.trim());
    let p = PathBuf::from(&decoded);
    let candidate = if p.is_absolute() {
        p
    } else {
        from.parent()?.join(p)
    };
    candidate.is_file().then_some(candidate)
}

/// 读取 Markdown 引用的本地图片，返回可直接放进 `img.src` 的 data URL。
///
/// 找不到文件返回 None（前端显示占位），文件存在但格式不支持/过大则返回错误。
/// 这里不用 asset 协议，是为了避开全局 CSP 与 scope 配置，只对本次预览按需读取。
#[tauri::command]
pub fn read_local_image(from: String, src: String) -> Result<Option<String>, String> {
    let base = PathBuf::from(&from);
    let path = match resolve_image_path(&base, &src) {
        Some(p) => p,
        None => return Ok(None),
    };
    let meta = fs::metadata(&path).map_err(|e| format!("读取图片失败: {}", e))?;
    if !meta.is_file() {
        return Ok(None);
    }
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(format!("图片超过 {} MB，已跳过预览", MAX_IMAGE_BYTES / 1024 / 1024));
    }
    let mime = image_mime(&path).unwrap_or("application/octet-stream");
    let bytes = fs::read(&path).map_err(|e| format!("读取图片失败: {}", e))?;
    Ok(Some(format!("data:{};base64,{}", mime, STANDARD.encode(bytes))))
}

// ===================== 从系统「打开方式」/ 命令行打开 .md =====================

/// 前端尚未就绪时先攒着，等 Markdown 模块挂载后一次性取走。
static PENDING_MARKDOWN: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

fn pending_markdown() -> &'static Mutex<Vec<String>> {
    PENDING_MARKDOWN.get_or_init(|| Mutex::new(Vec::new()))
}

/// 从进程参数里筛出真实存在的 markdown 文件（跳过 `--flag` 与程序自身路径）。
pub fn collect_markdown_args<I: IntoIterator<Item = String>>(args: I) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in args {
        if a.starts_with('-') {
            continue;
        }
        let p = PathBuf::from(&a);
        if !is_markdown(&p) || !p.is_file() {
            continue;
        }
        let abs = strip_unc(&p.canonicalize().unwrap_or(p));
        if !out.iter().any(|x| x == &abs) {
            out.push(abs);
        }
    }
    out
}

/// 记录待打开的 markdown 并通知前端（前端已挂载时立即响应，未挂载时靠队列兜底）。
pub fn queue_markdown_args(app: &tauri::AppHandle, args: &[String]) -> Vec<String> {
    use tauri::Emitter;
    let found = collect_markdown_args(args.iter().cloned());
    if found.is_empty() {
        return found;
    }
    if let Ok(mut q) = pending_markdown().lock() {
        for p in &found {
            if !q.iter().any(|x| x == p) {
                q.push(p.clone());
            }
        }
    }
    let _ = app.emit("markdown-open-file", found.clone());
    found
}

/// 前端 Markdown 模块取走待打开文件（取走后清空，避免重复打开）。
#[tauri::command]
pub fn take_pending_markdown_files() -> Vec<String> {
    pending_markdown()
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
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
