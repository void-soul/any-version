//! 退出诊断日志。
//!
//! 退出路径（托盘「退出」→ app.exit → ExitRequested → 清理/强杀）在长时运行
//! 后偶发「无法退出」。由于 tracing 未挂载 subscriber（调用为空操作），此前
//! 没有任何可见日志。这里用一个独立的同步文件 appender，把退出关键检查点
//! 逐行追加到 exe 同级的 `exit.log`，每行即时 flush，确保即便进程随后被
//! std::process::exit 强杀，日志也已落盘，便于定位到底卡在哪一步。

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;

use std::time::SystemTime;

/// 返回可用的日志文件路径。
///
/// 安装到 `C:\Program Files\...` 时 exe 同级目录通常不可写，而
/// `windows_subsystem = "windows"` 下 stderr 不可见 —— 两个通道同时失效，
/// 最需要日志的「无法退出」故障会零诊断（AUDIT-017）。
/// 因此优先尝试 exe 同级，失败则回退到应用数据目录（%APPDATA%/kira）。
fn log_path() -> Option<std::path::PathBuf> {
    if let Some(mut path) = std::env::current_exe()
        .ok()?
        .parent()
        .map(|p| p.join("exit.log"))
    {
        if is_writable(&path) {
            return Some(path);
        }
        path = crate::commands::foundation::get_data_dir().join("exit.log");
        if is_writable(&path) {
            return Some(path);
        }
    }
    // 最后兜底：系统临时目录也写不进去时，至少 stderr 已由调用方输出。
    Some(std::env::temp_dir().join("kira-exit.log"))
}

fn is_writable(path: &std::path::Path) -> bool {
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return false;
        }
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .is_ok()
}

/// 追加一行带时间戳的退出诊断日志。同步写入并立即 flush，绝不阻塞调用方。
pub fn exit_log(msg: &str) {
    // 单线程串行化写入，避免交错；Mutex<()> 仅用于互斥，不持有文件句柄。
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // 同时打印到 stderr，便于在终端/调试器中也看到（windows_subsystem=windows
    // 下 stderr 仍可被父进程/调试器捕获）。
    let line = format!("[{now}] {msg}\n");
    eprint!("{}", line);

    if let Some(path) = log_path() {
        match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(mut f) => {
                if let Err(e) = f.write_all(line.as_bytes()).and_then(|_| f.flush()) {
                    eprintln!("[exit-log] 写入 {} 失败: {e}", path.display());
                }
            }
            Err(e) => eprintln!("[exit-log] 打开 {} 失败: {e}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_path_is_writable_or_falls_back_to_temp() {
        let path = log_path().expect("应始终返回可尝试的路径");
        assert!(path.file_name().is_some());
        assert!(is_writable(&path) || path.starts_with(std::env::temp_dir()));
    }

    #[test]
    fn is_writable_rejects_impossible_paths() {
        let impossible = std::path::Path::new("/dev/null/impossible/exit.log");
        assert!(!is_writable(impossible) || cfg!(windows));
    }
}

/// 便捷宏式包装：exit_log(&format!(...))。
#[macro_export]
macro_rules! exit_log {
    ($($arg:tt)*) => {
        $crate::exit_log::exit_log(&format!($($arg)*))
    };
}
