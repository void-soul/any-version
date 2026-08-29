use std::process::Command;

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
