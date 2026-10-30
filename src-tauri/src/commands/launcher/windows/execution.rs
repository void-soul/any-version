use super::*;

/// 执行文件/命令（支持 open / runas 管理员提权 / explore 资源管理器 / 任意文件关联程序打开）
pub fn shell_execute(
    operation: &str,
    file: &str,
    params: &str,
    start_location: Option<&str>,
) -> Result<(), String> {
    let p = Path::new(file);
    let is_url = looks_like_url(file);
    let is_dir = operation == "explore" || p.is_dir();
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let is_exe =
        !is_url && (ext == "exe" || ext == "bat" || ext == "cmd" || ext == "ps1" || ext == "com");

    // 只有可执行程序才支持 runas 管理员提权，其它文档/文件夹降级为 open
    let actual_op = if operation == "runas" && !is_exe {
        "open"
    } else {
        operation
    };

    let work_dir = if let Some(dir) = start_location {
        if !dir.trim().is_empty() {
            dir.to_string()
        } else {
            get_default_work_dir(file)
        }
    } else {
        get_default_work_dir(file)
    };

    // runas + 脚本文件（cmd/bat/ps1）：不能把脚本路径直接作为 lpFile 提权执行——
    // ShellExecuteW 的 runas 只对 lpFile 本身提权，脚本关联宿主（cmd.exe / powershell.exe）
    // 不会被提升，导致「黑屏一闪、内容不执行」。改为提权启动脚本宿主并执行脚本
    //（与「右键 → 以管理员身份运行」的真实语义一致：提权的是脚本宿主）。
    if actual_op == "runas" && (ext == "cmd" || ext == "bat" || ext == "ps1") {
        // 脚本路径含空格时必须整体加引号，否则宿主会把它拆成多个参数；
        // 提权后的工作目录默认为 C:\Windows\System32，必须显式传脚本所在目录。
        let quoted = format!("\"{}\"", file);
        let (host, host_args) = if ext == "ps1" {
            // PowerShell：-ExecutionPolicy Bypass 绕过执行策略限制（管理员主动运行的
            // 脚本不应被策略拦截）；-NoProfile 跳过用户 profile（提权环境更快更干净，
            // 也避免 profile 脚本报错吞掉真实输出）；-File 后接脚本与参数。
            // 注：powershell -File 的参数解析不信任引号内空格，参数统一用引号包裹，
            // 与 Explorer 右键「使用 PowerShell 运行」行为一致。
            (
                "powershell.exe",
                format!(
                    "-NoProfile -ExecutionPolicy Bypass -File {}{}",
                    quoted,
                    if params.trim().is_empty() {
                        String::new()
                    } else {
                        format!(" {}", params)
                    }
                ),
            )
        } else {
            // cmd/bat：/d 忽略 AutoRun 注册表（提权环境下避免 AutoRun 脚本干扰）；
            // bat 用 call 保证批间控制权返回。
            let prefix = if ext == "bat" {
                "/d /c call "
            } else {
                "/d /c "
            };
            (
                "cmd.exe",
                format!(
                    "{}{}{}",
                    prefix,
                    quoted,
                    if params.trim().is_empty() {
                        String::new()
                    } else {
                        format!(" {}", params)
                    }
                ),
            )
        };
        crate::exit_log::exit_log(&format!(
            "shell_execute: runas 脚本代理 {} {}（work_dir={}）",
            host, host_args, work_dir
        ));
        return shell_execute(
            "runas",
            host,
            &host_args,
            Some(if work_dir.is_empty() {
                "C:\\Windows\\System32"
            } else {
                &work_dir
            }),
        );
    }

    // 降权代理：若 AnyVersion 自身以管理员(elevated)运行，非 runas 的 open/explore 操作
    // 改用 explorer.exe(普通权限系统 Shell)代理启动，使子进程不继承管理员令牌，
    // 且与 AnyVersion 生命周期完全独立(由系统托管)。
    if actual_op != "runas" && crate::commands::env::is_admin() {
        return shell_execute_as_normal_user(file, params, &work_dir);
    }

    let w_op = to_wide_chars(if is_dir && actual_op == "explore" {
        "open"
    } else {
        actual_op
    });
    let w_file = to_wide_chars(if is_dir && actual_op == "explore" {
        "explorer.exe"
    } else {
        file
    });
    let dir_param_str = if is_dir && actual_op == "explore" {
        format!("\"{}\"", file)
    } else {
        params.to_string()
    };
    let w_params = to_wide_chars(&dir_param_str);
    let w_dir = to_wide_chars(&work_dir);

    unsafe {
        // 初始化当前线程 COM 接口
        let _ = windows_sys::Win32::System::Com::CoInitializeEx(
            std::ptr::null_mut(),
            windows_sys::Win32::System::Com::COINIT_APARTMENTTHREADED as u32,
        );

        let res = ShellExecuteW(
            std::ptr::null_mut(),
            w_op.as_ptr(),
            w_file.as_ptr(),
            if dir_param_str.is_empty() {
                std::ptr::null()
            } else {
                w_params.as_ptr()
            },
            if work_dir.is_empty() {
                std::ptr::null()
            } else {
                w_dir.as_ptr()
            },
            SW_SHOWNORMAL as i32,
        );

        if (res as isize) <= 32 {
            if is_exe {
                let mut cmd = std::process::Command::new(file);
                if !params.trim().is_empty() {
                    cmd.raw_arg(params);
                }
                if !work_dir.is_empty() {
                    cmd.current_dir(&work_dir);
                }
                // 彻底脱离 AnyVersion 生命周期与控制台：不继承 stdin/stdout/stderr 句柄，
                // 不继承控制台，脱离 Job Object。否则子进程打印到 dev 控制台且随 AnyVersion
                // 关闭而关闭。
                cmd.stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .creation_flags_detached();
                cmd.spawn().map_err(|e| {
                    format!("启动程序失败 (ShellExecute code {}): {}", res as isize, e)
                })?;
            } else {
                // 文档/文本文件/目录通过 cmd /c start 打开系统默认关联程序
                let mut cmd = std::process::Command::new("cmd.exe");
                let mut cmd_args = vec![
                    "/c".to_string(),
                    "start".to_string(),
                    "".to_string(),
                    file.to_string(),
                ];
                if !params.trim().is_empty() {
                    cmd_args.push(params.to_string());
                }
                cmd.args(&cmd_args)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .creation_flags_detached();
                if !work_dir.is_empty() {
                    cmd.current_dir(&work_dir);
                }
                cmd.spawn().map_err(|e| {
                    format!("打开文件失败 (ShellExecute code {}): {}", res as isize, e)
                })?;
            }
        }
    }
    Ok(())
}

/// 以普通用户身份启动目标（降权代理）。仅当 AnyVersion 自身以管理员(elevated)运行时调用：
/// 不继承 AnyVersion 的管理员令牌，与 AnyVersion 进程完全无关(由系统托管)。
///
/// 分两种方式：
/// 1) 可执行程序（exe/bat/cmd/ps1/com）：通过 `WTSQueryUserToken` 取当前交互会话的用户令牌，
///    再用 `CreateProcessWithTokenW` 降权启动。**不能用 explorer.exe 代理**——explorer 是单实例，
///    已运行的 explorer 收到带引号的 exe 路径参数时无法可靠识别为"运行程序"，会退化为打开
///    默认文件夹（如"我的文档"）。
/// 2) 文档/网址/文件夹/shell: 路径：沿用 explorer.exe（普通权限系统 Shell）代理，由系统默认
///    关联程序打开；explorer 对这些"打开"场景可靠。
pub(super) fn shell_execute_as_normal_user(
    file: &str,
    params: &str,
    work_dir: &str,
) -> Result<(), String> {
    let is_url = looks_like_url(file);
    let ext = Path::new(file)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let is_exe =
        !is_url && (ext == "exe" || ext == "bat" || ext == "cmd" || ext == "ps1" || ext == "com");
    // explorer.exe 是单实例 shell 导航命令（如「打开文件位置」用 /select 定位文件）。
    // 不能把它当可执行程序走 CreateProcessWithTokenW 降权启动——explorer 收到带引号的
    // exe 路径参数时无法可靠识别为"运行程序"，会退化为打开默认文件夹（如"我的文档"）。
    // 因此 explorer.exe 应作为 shell 导航走下方 explorer.exe 代理（ShellExecuteW），
    // /select 等导航参数才能被已运行的 explorer 正确解析。
    let file_name = Path::new(file)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("");
    let is_explorer_nav = !is_url && file_name.eq_ignore_ascii_case("explorer.exe");

    if is_exe && !is_explorer_nav {
        return create_process_as_normal_user(file, params, work_dir);
    }

    // 非可执行程序：explorer.exe 代理打开（文档/网址/文件夹等系统 Shell 场景）
    // 对 explorer 导航命令（如「打开文件位置」/select），目标就是 explorer.exe 本身，
    // 参数只应包含导航参数（/select,"path"），不能再重复拼接 "explorer.exe"——否则
    // 已运行的 explorer 收到带引号的 exe 路径参数会退化为打开默认文件夹。
    let arg = if is_explorer_nav {
        params.trim().to_string()
    } else if params.trim().is_empty() {
        format!("\"{}\"", file)
    } else {
        format!("\"{}\" {}", file, params.trim())
    };
    let w_op = to_wide_chars("open");
    let w_file = to_wide_chars(if is_explorer_nav {
        file
    } else {
        "explorer.exe"
    });
    let w_params = to_wide_chars(&arg);
    let w_dir = to_wide_chars(work_dir);
    unsafe {
        let _ = windows_sys::Win32::System::Com::CoInitializeEx(
            std::ptr::null_mut(),
            windows_sys::Win32::System::Com::COINIT_APARTMENTTHREADED as u32,
        );
        let res = ShellExecuteW(
            std::ptr::null_mut(),
            w_op.as_ptr(),
            w_file.as_ptr(),
            w_params.as_ptr(),
            if work_dir.is_empty() {
                std::ptr::null()
            } else {
                w_dir.as_ptr()
            },
            SW_SHOWNORMAL as i32,
        );
        if (res as isize) <= 32 {
            return Err(format!(
                "降权启动失败 (explorer ShellExecute code {}): {}",
                res as isize, file
            ));
        }
    }
    Ok(())
}

/// 获取当前交互会话用户的"主令牌"(TokenPrimary)，供 CreateProcessWithTokenW 降权使用。
/// 先尝试 WTSQueryUserToken（服务/会话场景）；失败时回退到在活动控制台会话中
/// 枚举 explorer.exe 进程令牌 —— explorer 始终以当前用户的普通（非提权）身份运行，
/// 用它降权启动绝对可靠。两者都失败才返回 Err。
/// 返回的令牌需调用方用 CloseHandle 释放。
#[cfg(target_os = "windows")]
unsafe fn get_active_user_primary_token() -> Result<windows_sys::Win32::Foundation::HANDLE, String>
{
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::DuplicateTokenEx;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::RemoteDesktop::{
        ProcessIdToSessionId, WTSGetActiveConsoleSessionId, WTSQueryUserToken, WTS_CURRENT_SESSION,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, OpenProcessToken};
    // 降权复制令牌所需的常量
    const SECURITY_IMPERSONATION_LEVEL: i32 = 2; // SecurityImpersonation
    const TOKEN_TYPE_PRIMARY: i32 = 1; // TokenPrimary
    const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const TOKEN_QUERY: u32 = 0x0008;
    const TOKEN_DUPLICATE: u32 = 0x0002;

    // 1) 尝试 WTSQueryUserToken（需要 SE_TCB_NAME，普通提权进程通常没有）
    let mut user_token: HANDLE = std::ptr::null_mut();
    if WTSQueryUserToken(WTS_CURRENT_SESSION, &mut user_token) != 0 {
        let mut primary: HANDLE = std::ptr::null_mut();
        let ok = DuplicateTokenEx(
            user_token,
            MAXIMUM_ALLOWED,
            std::ptr::null(),
            SECURITY_IMPERSONATION_LEVEL,
            TOKEN_TYPE_PRIMARY,
            &mut primary,
        );
        let _ = CloseHandle(user_token);
        if ok != 0 && !primary.is_null() {
            return Ok(primary);
        }
        crate::exit_log!("[启动] WTSQueryUserToken 可用但复制令牌失败，改用 explorer 令牌");
    } else {
        crate::exit_log!(
            "[启动] WTSQueryUserToken 失败 ({}), 改用 explorer.exe 进程令牌降权",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
        );
    }

    // 2) 回退：枚举 explorer.exe 的进程令牌。优先活动控制台会话（正常桌面），
    //    找不到再放宽到任意会话（RDP / 多会话场景），最后才报错。
    let active_session = WTSGetActiveConsoleSessionId();
    let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
    if snapshot.is_null() {
        return Err("无法枚举进程列表（CreateToolhelp32Snapshot 失败）".to_string());
    }
    let mut found_explorers = 0usize;
    // 优先精确匹配活动会话；失败后用 fallback 再扫一遍放宽会话
    let mut take_token_from = |entry: &PROCESSENTRY32W,
                               allowed_sessions: &[u32]|
     -> Result<Option<HANDLE>, String> {
        let mut sess: u32 = 0;
        if ProcessIdToSessionId(entry.th32ProcessID, &mut sess) == 0
            || !allowed_sessions.contains(&sess)
        {
            return Ok(None);
        }
        found_explorers += 1;
        let proc_handle =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID) };
        if proc_handle.is_null() {
            return Ok(None);
        }
        let mut raw_token: HANDLE = std::ptr::null_mut();
        let token_ok =
            unsafe { OpenProcessToken(proc_handle, TOKEN_QUERY | TOKEN_DUPLICATE, &mut raw_token) };
        if token_ok == 0 {
            let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            crate::exit_log!(
                "[启动] explorer pid={} OpenProcessToken 失败 ({})",
                entry.th32ProcessID,
                err
            );
            unsafe {
                let _ = CloseHandle(proc_handle);
            }
            return Ok(None);
        }
        let mut primary: HANDLE = std::ptr::null_mut();
        let ok = unsafe {
            DuplicateTokenEx(
                raw_token,
                MAXIMUM_ALLOWED,
                std::ptr::null(),
                SECURITY_IMPERSONATION_LEVEL,
                TOKEN_TYPE_PRIMARY,
                &mut primary,
            )
        };
        unsafe {
            let _ = CloseHandle(raw_token);
            let _ = CloseHandle(proc_handle);
        }
        if ok == 0 || primary.is_null() {
            let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            crate::exit_log!(
                "[启动] explorer pid={} DuplicateTokenEx 失败 ({})",
                entry.th32ProcessID,
                err
            );
            return Ok(None);
        }
        Ok(Some(primary))
    };

    let mut collect = |snapshot: HANDLE, allowed_sessions: &[u32]| -> Option<HANDLE> {
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if unsafe { Process32FirstW(snapshot, &mut entry) } == 0 {
            return None;
        }
        loop {
            // szExeFile 是固定 260 的 u16 数组：必须截断到第一个 NUL 再比较，
            // 否则字符串是 "explorer.exe\0\0..." 永远匹配不上（explorer 回退必败）。
            let exe_len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let exe = String::from_utf16_lossy(&entry.szExeFile[..exe_len]);
            if exe.eq_ignore_ascii_case("explorer.exe") {
                if let Ok(Some(token)) = take_token_from(&entry, allowed_sessions) {
                    return Some(token);
                }
            }
            if unsafe { Process32NextW(snapshot, &mut entry) } == 0 {
                return None;
            }
        }
    };

    // 第一遍：仅活动控制台会话
    if let Some(token) = collect(snapshot, &[active_session]) {
        let _ = unsafe { CloseHandle(snapshot) };
        return Ok(token);
    }
    // 第二遍：任意会话（RDP 等场景 explorer 可能不在控制台会话）
    if let Some(token) = collect(snapshot, &[u32::MAX]) {
        let _ = unsafe { CloseHandle(snapshot) };
        return Ok(token);
    }
    let _ = unsafe { CloseHandle(snapshot) };
    crate::exit_log!(
        "[启动] explorer 回退失败：找到 {} 个 explorer.exe，均无法复制令牌",
        found_explorers
    );
    Err("无法获取普通用户令牌（WTS 与 explorer 回退均失败）".to_string())
}

/// 最后兜底：拿不到普通用户令牌时用 ShellExecuteExW 启动，保证项目至少能跑起来。
/// 1) 先尝试 rundll32 shell32.dll,ShellExec_RunDLL 方式——由系统 Shell 组件解析启动，
///    在部分环境（如 UAC 关闭、会话含普通用户 shell 上下文）下可获得非提权上下文；
/// 2) 再失败则直接 ShellExecuteExW（子进程继承当前管理员令牌），日志明确警告。
#[cfg(target_os = "windows")]
pub(super) fn shell_execute_fallback(
    file: &str,
    params: &str,
    work_dir: &str,
) -> Result<(), String> {
    use std::mem::size_of;
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let w_dir = to_wide_chars(work_dir);
    let w_dir_ptr = if work_dir.is_empty() {
        std::ptr::null()
    } else {
        w_dir.as_ptr()
    };

    unsafe {
        // 1) rundll32 ShellExec_RunDLL 方式（尝试普通用户上下文）
        let rundll_cmd = if params.trim().is_empty() {
            format!("shell32.dll,ShellExec_RunDLL \"{}\"", file)
        } else {
            format!("shell32.dll,ShellExec_RunDLL \"{}\" {}", file, params)
        };
        let w_rundll = to_wide_chars("rundll32.exe");
        let w_rundll_cmd = to_wide_chars(&rundll_cmd);
        let mut sei: SHELLEXECUTEINFOW = std::mem::zeroed();
        sei.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        sei.fMask = 0;
        sei.nShow = SW_SHOWNORMAL as i32;
        sei.lpFile = w_rundll.as_ptr();
        sei.lpParameters = w_rundll_cmd.as_ptr();
        sei.lpDirectory = w_dir_ptr;
        if ShellExecuteExW(&mut sei) != 0 {
            crate::exit_log!(
                "[启动] 降权令牌获取失败，已用 rundll32 ShellExec_RunDLL 兜底启动: {}",
                file
            );
            return Ok(());
        }
        let err1 = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);

        // 2) 直接 ShellExecuteExW（继承当前令牌，日志警告）
        let w_file = to_wide_chars(file);
        let w_params = to_wide_chars(params);
        let mut sei2: SHELLEXECUTEINFOW = std::mem::zeroed();
        sei2.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        sei2.fMask = SEE_MASK_NOCLOSEPROCESS;
        sei2.nShow = SW_SHOWNORMAL as i32;
        sei2.lpFile = w_file.as_ptr();
        sei2.lpParameters = if params.is_empty() {
            std::ptr::null()
        } else {
            w_params.as_ptr()
        };
        sei2.lpDirectory = w_dir_ptr;
        if ShellExecuteExW(&mut sei2) != 0 {
            crate::exit_log!(
                "[启动] 降权令牌获取失败，ShellExecuteExW 兜底启动（子进程继承管理员令牌）: {}",
                file
            );
            if !sei2.hProcess.is_null() {
                windows_sys::Win32::Foundation::CloseHandle(sei2.hProcess);
            }
            return Ok(());
        }
        let err2 = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        Err(format!(
            "启动失败：普通用户令牌不可用，且 ShellExecuteEx 兜底也失败（rundll32={} ShellExecute={}）",
            err1, err2
        ))
    }
}

/// 以当前交互会话用户的普通令牌降权启动可执行程序（CreateProcessWithTokenW）。
/// 相比 explorer.exe 代理可靠得多，且子进程不继承 AnyVersion 的管理员令牌；
/// 脚本类（.bat/.cmd/.ps1）会经解释器（cmd.exe / powershell.exe）执行，
/// 因为 CreateProcess 系列无法直接运行脚本文件。
/// 若获取用户令牌失败，最后兜底用 ShellExecuteExW 启动（rundll32 方式优先）。
pub(super) fn create_process_as_normal_user(
    file: &str,
    params: &str,
    work_dir: &str,
) -> Result<(), String> {
    use windows_sys::Win32::System::Threading::{
        CreateProcessWithTokenW, PROCESS_INFORMATION, STARTUPINFOW,
    };

    // 0. 获取当前交互会话用户的普通主令牌（WTS 或 explorer 回退）；
    //    两者都失败时不再直接报错，改用 ShellExecuteExW 最后兜底（保证项目能启动）。
    let primary_token = match unsafe { get_active_user_primary_token() } {
        Ok(token) => token,
        Err(e) => {
            crate::exit_log!(
                "[启动] 无法获取普通用户令牌: {}，进入 ShellExecuteExW 最后兜底",
                e
            );
            return shell_execute_fallback(file, params, work_dir);
        }
    };

    unsafe {
        // 1. 组装可执行程序与命令行（CreateProcessWithTokenW 需要可变缓冲）
        //    .bat/.cmd/.ps1 脚本必须经解释器执行；lpApplicationName 指向解释器完整路径。
        let ext = Path::new(file)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let (app_exe, command_text): (String, String) = match ext.as_str() {
            "bat" | "cmd" => {
                let comspec = std::env::var("COMSPEC")
                    .unwrap_or_else(|_| "C:\\Windows\\System32\\cmd.exe".to_string());
                let args = if params.trim().is_empty() {
                    format!("/d /c \"\"{}\"\"", file)
                } else {
                    format!("/d /c \"\"{}\" {}\"", file, params.trim())
                };
                (comspec.clone(), format!("\"{}\" {}", comspec, args))
            }
            "ps1" => {
                let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
                let ps = format!(
                    "{}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
                    windir
                );
                let args = if params.trim().is_empty() {
                    format!("-NoProfile -ExecutionPolicy Bypass -File \"{}\"", file)
                } else {
                    format!(
                        "-NoProfile -ExecutionPolicy Bypass -File \"{}\" {}",
                        file,
                        params.trim()
                    )
                };
                (ps.clone(), format!("\"{}\" {}", ps, args))
            }
            _ => {
                let cmd = if params.trim().is_empty() {
                    format!("\"{}\"", file)
                } else {
                    format!("\"{}\" {}", file, params.trim())
                };
                (file.to_string(), cmd)
            }
        };
        let mut command_line = command_text
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>();
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();

        let w_file = to_wide_chars(&app_exe);
        // 工作目录宽字符必须先绑定变量再取指针，避免临时 Vec 被提前释放导致悬垂指针
        let w_dir_owned = if work_dir.is_empty() {
            Vec::new()
        } else {
            to_wide_chars(work_dir)
        };
        let w_dir = if w_dir_owned.is_empty() {
            std::ptr::null()
        } else {
            w_dir_owned.as_ptr()
        };

        crate::exit_log!(
            "[启动] 降权启动 {} (解释器={}, 参数={}, 工作目录={})",
            file,
            app_exe,
            params,
            work_dir
        );

        let ok = CreateProcessWithTokenW(
            primary_token,
            0, // LOGON_WITH_PROFILE=1，普通程序无需加载用户配置文件
            w_file.as_ptr(),
            command_line.as_mut_ptr(),
            0, // CREATE_UNICODE_ENVIRONMENT 需配合 environment；普通启动用 0
            std::ptr::null_mut(),
            w_dir,
            &si,
            &mut pi,
        );

        let _ = windows_sys::Win32::Foundation::CloseHandle(primary_token);

        if ok == 0 {
            return Err(format!(
                "降权启动失败 (CreateProcessWithTokenW error {}): {}",
                std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
                file
            ));
        }
        let _ = windows_sys::Win32::Foundation::CloseHandle(pi.hThread);
        let _ = windows_sys::Win32::Foundation::CloseHandle(pi.hProcess);
    }
    Ok(())
}

pub(super) fn get_default_work_dir(file: &str) -> String {
    let p = Path::new(file);
    if p.is_dir() {
        file.to_string()
    } else if let Some(parent) = p.parent() {
        parent.to_string_lossy().to_string()
    } else {
        String::new()
    }
}
