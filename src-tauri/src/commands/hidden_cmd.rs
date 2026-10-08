use std::io::Read;
use std::process::Command;

/// 读取子进程 stdout/stderr 的默认上限（8 MiB）。
///
/// `.output()` 会把子进程输出**全量**缓冲进内存。枚举类命令（`tasklist`、`netstat`、
/// `wmic`、`sc query`、`dir /s`）在进程多、目录深时输出可达几十 MB 且无上界，
/// 全量缓冲既吃内存又拖慢界面。这里给一个明确上限：超出即截断并标记，
/// 调用方按需降级（例如只取前 N 行）。
pub const DEFAULT_OUTPUT_CAP: usize = 8 * 1024 * 1024;

/// 带上限的子进程输出。
pub struct CappedOutput {
    pub stdout: String,
    pub stderr: String,
    pub status: std::process::ExitStatus,
    /// stdout 是否被截断（超出上限）
    pub truncated: bool,
}

/// 执行一个命令并**带上限**地取回 stdout/stderr（替代无界的 `Command::output`）。
///
/// 超限处理：stdout 达到上限后标记 `truncated` 并**终止子进程**——否则子进程继续写
/// 管道、我们已停止读取，会在 `wait()` 处死锁。
///
/// 仅捕获 stdout/stderr 两个管道；子进程若继承了父进程的 stdin 则不受影响。
pub fn output_capped(mut cmd: Command, max_bytes: usize) -> std::io::Result<CappedOutput> {
    use std::process::Stdio;

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdout = child.stdout.take().expect("stdout 已设置为 piped");
    let mut stderr = child.stderr.take().expect("stderr 已设置为 piped");

    // 先按上限读 stdout
    let mut out_buf = Vec::new();
    stdout
        .by_ref()
        .take(max_bytes as u64)
        .read_to_end(&mut out_buf)?;
    // 读到上限只说明「可能还有更多」：再试读 1 字节确认是否真的截断。
    let truncated = if out_buf.len() >= max_bytes {
        let mut probe = [0u8; 1];
        match stdout.read(&mut probe) {
            Ok(0) => false,
            Ok(_) => true,
            Err(_) => true,
        }
    } else {
        false
    };

    if truncated {
        // 已超限：子进程可能还在写 stdout，必须终止，否则 wait() 死锁
        let _ = child.kill();
    }

    // stderr 一般很小，同样加上限兜底
    let mut err_buf = Vec::new();
    stderr
        .by_ref()
        .take(DEFAULT_OUTPUT_CAP.min(max_bytes) as u64)
        .read_to_end(&mut err_buf)?;

    let status = child.wait()?;

    Ok(CappedOutput {
        stdout: String::from_utf8_lossy(&out_buf).into_owned(),
        stderr: String::from_utf8_lossy(&err_buf).into_owned(),
        status,
        truncated,
    })
}

/// 创建一个不会弹出控制台窗口的 Command（仅 Windows 生效）。
///
/// 注意：这里只附加 `CREATE_NO_WINDOW`，**不**附加 `CREATE_BREAKAWAY_FROM_JOB`。
/// 原因：当 vex 自身运行在某个不允许 breakaway 的 Job Object 内（例如由
/// 终端 / `yarn start` / 部分 IDE 拉起时），带 `CREATE_BREAKAWAY_FROM_JOB` 的
/// `CreateProcess` 会直接以 `ERROR_ACCESS_DENIED` (os error 5) 失败——表现为
/// `tasklist`/`netstat`/`sc`/`-v` 等所有探测命令全部失效（服务状态恒为 进程数=0、
/// 启动服务即报“拒绝访问”）。
///
/// 只有真正需要脱离 AnyVersion 生命周期的长驻进程（mihomo 内核、SDK 服务、
/// Node 项目、Sub-Store 等）才使用 `hidden_cmd_breakaway()` +
/// `spawn_breakaway_fallback()`，并在受限环境下自动降级。
pub fn hidden_cmd<S: AsRef<std::ffi::OsStr>>(program: S) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW
        cmd.creation_flags(0x08000000);
    }
    cmd
}

/// 需要脱离父进程（AnyVersion）所在 Job Object 的长驻进程使用：
/// `CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB`。
///
/// 注意：当本进程位于不允许 breakaway 的 Job 内时，`CreateProcess` 会以
/// `ERROR_ACCESS_DENIED` 失败。请通过 [`spawn_breakaway_fallback`] 启动，
/// 而不是直接 `.spawn()`。
pub fn hidden_cmd_breakaway<S: AsRef<std::ffi::OsStr>>(program: S) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB
        cmd.creation_flags(0x08000000 | 0x01000000);
    }
    cmd
}

/// 以“先 breakaway、失败自动降级”的方式启动子进程。
///
/// 优先带 `CREATE_BREAKAWAY_FROM_JOB` 启动（让长驻进程脱离 AnyVersion 的生命周期）；
/// 若因所在 Job 不允许 breakaway 返回 `ERROR_ACCESS_DENIED` (5)，则去掉该标志重试一次，
/// 保证即使在受限环境下长驻进程也能正常启动（只是无法脱离父进程生命周期）。
#[cfg(windows)]
pub fn spawn_breakaway_fallback(mut cmd: Command) -> std::io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x08000000 | 0x01000000);
    match cmd.spawn() {
        Ok(child) => Ok(child),
        Err(e) if e.raw_os_error() == Some(5) => {
            cmd.creation_flags(0x08000000);
            cmd.spawn()
        }
        Err(e) => Err(e),
    }
}

#[cfg(not(windows))]
pub fn spawn_breakaway_fallback(mut cmd: Command) -> std::io::Result<std::process::Child> {
    cmd.spawn()
}
