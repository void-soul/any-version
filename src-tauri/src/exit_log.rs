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

fn log_path() -> Option<std::path::PathBuf> {
    let mut dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    dir.push("exit.log");
    Some(dir)
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
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
            let _ = f.write_all(line.as_bytes());
            let _ = f.flush();
        }
    }
}

/// 便捷宏式包装：exit_log(&format!(...))。
#[macro_export]
macro_rules! exit_log {
    ($($arg:tt)*) => {
        $crate::exit_log::exit_log(&format!($($arg)*))
    };
}
