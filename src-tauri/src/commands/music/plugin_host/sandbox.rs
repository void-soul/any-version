use super::*;

// ─── 沙箱（Node 权限模型）───

/// 给 node 子进程清掉会被上层注入的 Node 环境变量。
///
/// `NODE_OPTIONS` 常被 IDE / 工具链用来注入 `--require` 预加载（编辑器自带的 node shim
/// 就这么干）。那种预加载会去读它自己所在目录，在权限模型下直接 `ERR_ACCESS_DENIED`。
///
/// **探测与正式启动都必须清**：不清的话探测会失败，于是得出
/// 「本机 Node 不支持权限模型」的错误结论，沙箱被静默关掉 —— 真机上正是这么踩到的
/// （单元测试当场抓到：所有危险能力都报 allowed）。
pub(super) fn clean_node_env(cmd: &mut Command) {
    cmd.env_remove("NODE_OPTIONS").env_remove("NODE_PATH");
}

/// Node 是否支持权限模型（`--permission`，Node 20+）。
///
/// 用 `--version` 探测：Node 不认识该参数时会直接以 `bad option` 退出，与脚本内容无关，
/// 因此不必写任何临时文件。结论只取决于 node 二进制本身，全进程缓存一次即可。
pub(super) fn sandbox_supported(node: &Invocation) -> bool {
    static CACHE: OnceLock<bool> = OnceLock::new();
    *CACHE.get_or_init(|| {
        let mut cmd = Command::new(&node.program);
        cmd.args(&node.prefix_args)
            .arg("--permission")
            .arg("--version");
        clean_node_env(&mut cmd);
        hide_console(&mut cmd);
        let supported = cmd
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);
        eprintln!("[plugin] 沙箱探测：node 是否支持权限模型 = {supported}");
        supported
    })
}

/// 沙箱状态（给界面展示）。
pub fn sandbox_status(node: &Invocation) -> SandboxStatus {
    if sandbox_supported(node) {
        SandboxStatus {
            enabled: true,
            reason: None,
        }
    } else {
        SandboxStatus {
            enabled: false,
            reason: Some(
                "当前 Node 不支持权限模型（需 Node 20+），插件将以完整权限运行".to_string(),
            ),
        }
    }
}

/// 沙箱启动参数：启用权限模型，只放行**读插件目录**。
///
/// 刻意**不**给 `--allow-fs-write` / `--allow-child-process` / `--allow-worker`：
/// 插件只需要「读自己的目录 + 联网」，其余一律拒绝（实测这套配置下
/// 读系统文件、写外部文件、起子进程、建 worker 全部 `ERR_ACCESS_DENIED`，
/// 而 `require("axios")` 与 https 请求照常可用）。
///
/// 网络不在权限模型管辖内 —— Node 24 没有 `--allow-net`（那是更高版本的开关），
/// 而插件本来就靠它取数据，所以这一项不需要也无法放行。
///
/// 注意：Node 官方把权限模型定位为「安全带」而非安全边界，恶意代码理论上仍可绕过。
/// 它挡的是**插件无意/恶意的越权文件访问**，不是铁笼。
pub(super) fn sandbox_args() -> Vec<String> {
    vec![
        "--permission".to_string(),
        format!("--allow-fs-read={}", plugins_root().display()),
    ]
}

/// 给 node 命令加上沙箱参数（不支持则记一条日志）。
///
/// ⚠️ **必须在脚本路径之前调用**：Node 的命令行是 `node [options] script [args]`，
/// 选项写到脚本后面会被当成**脚本参数**原样转给脚本 —— 沙箱静默失效，
/// 而插件照常能跑，从表象上完全看不出来（真机上正是如此，靠单元测试才抓出来）。
pub(super) fn apply_sandbox(cmd: &mut Command, node: &Invocation) {
    if sandbox_supported(node) {
        cmd.args(sandbox_args());
    } else {
        eprintln!("[plugin] 当前 Node 不支持权限模型（需 20+），插件将以完整权限运行");
    }
}
