use std::process::Command;

/// 创建一个不会弹出控制台窗口的 Command（仅 Windows 生效）。
///
/// 额外附加 `CREATE_BREAKAWAY_FROM_JOB`：让子进程脱离父进程（AnyVersion）所在的
/// Job Object，从而在 AnyVersion 退出/被强杀时子进程不被连带终止。AnyVersion 启动的
/// 所有常驻进程（mihomo 内核、SDK 服务、RTSP 推流、Node/HTTP 服务等）都不应与
/// AnyVersion 绑定生命周期。
pub fn hidden_cmd<S: AsRef<std::ffi::OsStr>>(program: S) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB
        cmd.creation_flags(0x08000000 | 0x01000000);
    }
    cmd
}
