//! process responsibility.
use super::*;

/// reasonix 会话文件路径：`--resume` 需要的是会话文件（路径，不是 id）。
/// reasonix 把会话持久化在 `<REASONIX_HOME>/projects/<encoded-cwd>/sessions/<session_id>.jsonl`，
/// 其中 `<encoded-cwd>` 是 reasonix 对 cwd 的编码（/ 和 . 替换为 -；Windows 路径的 \ 与 : 一并替换），
/// REASONIX_HOME 默认 `~/.reasonix`。计算路径不存在时按 session_id 兜底搜索（绑定挂载/路径拼写差异）。
pub(super) fn reasonix_session_file(session_id: &str, cwd: &str) -> String {
    let home = std::env::var("REASONIX_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| crate::commands::utils::get_home_dir().join(".reasonix"));
    let encoded: String = cwd
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c == '.' || c == ':' {
                '-'
            } else {
                c
            }
        })
        .collect();
    let computed = home
        .join("projects")
        .join(&encoded)
        .join("sessions")
        .join(format!("{}.jsonl", session_id));
    if computed.exists() {
        return computed.to_string_lossy().replace('\\', "/");
    }
    // 兜底：按 session_id 在 projects/*/sessions/ 下搜索（编码差异时计算路径 miss）
    let projects_dir = home.join("projects");
    if let Ok(rd) = std::fs::read_dir(&projects_dir) {
        for entry in rd.flatten() {
            let f = entry
                .path()
                .join("sessions")
                .join(format!("{}.jsonl", session_id));
            if f.exists() {
                return f.to_string_lossy().replace('\\', "/");
            }
        }
    }
    String::new()
}

/// 转义用于 cmd /c 字符串拼接的参数：用双引号整体包裹，内部 " 转义为 ""。
/// 双引号内的 & | < > ^ 以及 % 均为字面量，可防止工具返回的 session_id 逃逸执行命令（RCE）。
pub(super) fn escape_cmd_arg(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' {
            out.push_str("\"\"");
        } else {
            out.push(c);
        }
    }
    out.push('"');
    out
}

/// 对齐 cross-spawn 的 Windows 参数引号规则（用于 cmd /d /s /c "<full>" 的逐元素转义）：
/// - 空串 → ""
/// - 含空格 / 制表符 / 引号 → 用双引号包裹，内部 " 转义为 \"
/// - 否则原样返回
/// 与 escape_cmd_arg（无条件加引号、内部 "" 双写）不同：此函数仅在必要时加引号，
/// 配合 /s 标志使 cmd 正确解析含空格的程序路径/参数（cross-spawn 的成熟做法）。
#[cfg(windows)]
pub(super) fn windows_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".to_string();
    }
    if arg.contains(' ') || arg.contains('\t') || arg.contains('"') {
        // 内部 " 必须转义为 ""（双写），这是 Windows cmd 的转义约定；
        // 不能用 \"（那是 CreateProcess argv 的约定），否则 /s 剥掉外层引号后，
        // 内部 \" 退化为字面引号，导致 prompt 中的 < > & | 暴露成重定向/管道符，命令被拆坏。
        let escaped: String = arg.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        arg.to_string()
    }
}

/// 把工具命令模板拆成参数数组（按空白分词，双引号仅作为分组、不被保留）。
/// 模板中的 {prompt}/{prompt_file}/{session_id} 占位符保留为独立 token，
/// 交由调用方替换为实际参数，避免整体经 shell 解释。
pub(super) fn tokenize_template(tmpl: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for ch in tmpl.chars() {
        if ch == '"' {
            in_q = !in_q;
        } else if ch.is_whitespace() && !in_q {
            if !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

/// 像 cmd.exe 一样，按 PATH + PATHEXT 把无扩展名的程序名解析为真实可执行文件路径。
/// Windows 上 Rust 的 Command::new 不会自动补全扩展名（bare "claude" 即使 claude.exe 在 PATH 中
/// 也会报 "program not found"），这里在 PATH 中查找 .exe/.cmd 等候选并返回完整路径；
/// 找不到则原样返回（交给后续 spawn 报错，错误信息更准确）。
/// 通过 `cmd /c where <program>` 让系统 shell 自行解析。
/// 用于 PATH / 额外目录都搜不到的情况（如用户自定义 npm prefix 导致全局 bin 不在 PATH 中，
/// 但 shell 自己能 `where` 到）。
/// 优先返回带 Windows 可执行扩展名（.exe/.cmd/.bat/.com）的匹配；npm 全局在 Windows 下会同时
/// 生成无扩展名的 sh 垫片与 .cmd 垫片，`where` 可能先给出无扩展名那一个，而它无法直接 CreateProcess
/// （os error 193: 不是有效的 Win32 应用程序），因此必须优先选 .cmd/.exe 等可执行的。
#[cfg(windows)]
pub(super) fn resolve_via_where(program: &str) -> Option<String> {
    let mut c = std::process::Command::new("cmd");
    c.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let out = c.args(["/c", "where", program]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let mut first: Option<String> = None;
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if first.is_none() {
            first = Some(line.to_string());
        }
        if let Some(ext) = std::path::Path::new(line)
            .extension()
            .and_then(|e| e.to_str())
        {
            let e = ext.to_ascii_lowercase();
            if e == "exe" || e == "cmd" || e == "bat" || e == "com" || e == "btm" {
                return Some(line.to_string());
            }
        }
    }
    first
}

/// 额外需要纳入搜索的目录（GUI 进程常缺少这些 PATH）：
/// 主要是 npm/pnpm/yarn 全局 bin，以及 nvm/fnm 等 node 版本管理器的安装位置。
#[cfg(windows)]
pub(super) fn extra_program_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    let add = |dirs: &mut Vec<std::path::PathBuf>, base: Option<String>, rel: &[&str]| {
        if let Some(b) = base {
            let base = std::path::Path::new(&b);
            let mut p = base.to_path_buf();
            for r in rel {
                p.push(r);
            }
            dirs.push(p);
        }
    };
    if let Ok(appdata) = std::env::var("APPDATA") {
        add(&mut dirs, Some(appdata.clone()), &["npm"]);
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        add(&mut dirs, Some(local.clone()), &["npm"]);
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        add(&mut dirs, Some(profile.clone()), &[".cargo", "bin"]);
        add(
            &mut dirs,
            Some(profile.clone()),
            &["AppData", "Roaming", "npm"],
        );
        add(
            &mut dirs,
            Some(profile.clone()),
            &["AppData", "Local", "npm"],
        );
        add(
            &mut dirs,
            Some(profile.clone()),
            &["AppData", "Roaming", "nvm"],
        );
        add(&mut dirs, Some(profile.clone()), &["scoop", "shims"]);
    }
    dirs
}

pub(super) fn resolve_program(program: &str) -> String {
    if program.is_empty() {
        return program.to_string();
    }
    let p = std::path::Path::new(program);
    // 带路径分隔符/盘符视为绝对或相对路径，不解析
    if p.is_absolute() || program.contains('\\') || program.contains('/') {
        return program.to_string();
    }
    #[cfg(not(windows))]
    {
        // 非 Windows 下扩展名解析由 shell 处理，原样返回即可
        let _ = p;
        return program.to_string();
    }
    #[cfg(windows)]
    {
        let has_ext = p.extension().is_some();
        let pathext =
            std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        let exts: Vec<String> = if has_ext {
            Vec::new()
        } else {
            pathext
                .split(';')
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        };
        // PATH 目录 + 额外常见全局安装目录一起搜索
        let mut dirs: Vec<String> = std::env::var("PATH")
            .unwrap_or_default()
            .split(';')
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .collect();
        for d in extra_program_dirs() {
            if let Some(s) = d.to_str() {
                dirs.push(s.to_string());
            }
        }
        for dir in &dirs {
            if dir.is_empty() {
                continue;
            }
            let dir = std::path::Path::new(dir);
            if has_ext {
                let cand = dir.join(program);
                if cand.is_file() {
                    return cand.to_string_lossy().to_string();
                }
            } else {
                for e in &exts {
                    let name = format!("{}.{}", program, e);
                    let cand = dir.join(&name);
                    if cand.is_file() {
                        return cand.to_string_lossy().to_string();
                    }
                }
            }
        }
        // 兜底：PATH / 额外目录都搜不到时，交给系统 shell 的 where 解析
        // （应对用户自定义 npm prefix 等导致全局 bin 不在任何已知目录的情况）
        if let Some(p) = resolve_via_where(program) {
            return p;
        }
        program.to_string()
    }
}

/// Windows 上 npm 全局工具通常是 .cmd/.bat 垫片，CreateProcess 无法直接执行，
/// 必须退回 cmd /c。其余（含 Unix 下带 shebang 的符号链接）可直接 spawn。
pub(super) fn is_windows_shell_shim(program: &str) -> bool {
    #[cfg(windows)]
    {
        if program.is_empty() {
            return false;
        }
        if let Some(ext) = std::path::Path::new(program)
            .extension()
            .and_then(|e| e.to_str())
        {
            let e = ext.to_ascii_lowercase();
            if e == "cmd" || e == "bat" || e == "btm" || e == "ps1" {
                return true;
            }
        }
        let pathext =
            std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        let exts: Vec<String> = pathext
            .split(';')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        let path = std::env::var("PATH").unwrap_or_default();
        for dir in path.split(';') {
            if dir.is_empty() {
                continue;
            }
            let base = std::path::Path::new(dir).join(program);
            let candidates = if base.extension().is_some() {
                vec![base.clone()]
            } else {
                exts.iter()
                    .map(|e| {
                        let mut name = program.to_string();
                        if !e.starts_with('.') {
                            name.push('.');
                        }
                        name.push_str(e.trim_start_matches('.'));
                        std::path::Path::new(dir).join(name)
                    })
                    .collect()
            };
            for c in candidates {
                if c.is_file() {
                    if let Some(ext) = c.extension().and_then(|e| e.to_str()) {
                        let e = ext.to_ascii_lowercase();
                        return e == "cmd" || e == "bat" || e == "btm" || e == "ps1";
                    }
                    return false;
                }
            }
        }
        false
    }
    #[cfg(not(windows))]
    {
        let _ = program;
        false
    }
}

/// 杀掉进程树（Windows 用 taskkill /T，否则直接 kill）
pub(super) fn kill_tree(child: &Arc<Mutex<Option<Child>>>) {
    // 先取出子进程句柄并立即释放锁，再执行可能阻塞的 kill / taskkill，
    // 避免持锁期间阻塞（taskkill 是外部进程调用）导致其他写操作饿死。
    let taken = child.lock().take();
    if let Some(c) = taken {
        #[cfg(windows)]
        {
            let pid = c.id();
            let mut tcmd = Command::new("taskkill");
            tcmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
            let _ = tcmd.args(["/F", "/T", "/PID", &pid.to_string()]).output();
        }
        #[cfg(not(windows))]
        {
            let mut c = c;
            let _ = c.kill();
        }
    }
}
