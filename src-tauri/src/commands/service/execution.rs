use super::*;

pub(super) fn render_command(template: &str, runtime: &ServiceRuntime) -> String {
    let install_root = runtime
        .install_root
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    // 将路径参数（如配置文件、数据目录、日志目录）自动转换为相对于 install_root 的相对路径，
    // 以完美解决 MSYS2 编译环境（如 Redis 7.x）下无法正确解析 Windows 反斜杠绝对路径的 Fatal Bug，
    // 同时也让命令行表现得更加精炼规范。
    let to_relative = |abs_path: &str| -> String {
        if abs_path.is_empty() {
            return String::new();
        }
        if let Some(ref root) = runtime.install_root {
            let path = Path::new(abs_path);
            if let Ok(rel) = path.strip_prefix(root) {
                let rel_str = rel.to_string_lossy().to_string();
                if rel_str.is_empty() {
                    ".".to_string()
                } else {
                    rel_str
                }
            } else {
                abs_path.to_string()
            }
        } else {
            abs_path.to_string()
        }
    };

    let config_file_abs = runtime
        .config_file
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let config_file = to_relative(&config_file_abs);
    let port = runtime.port.map(|p| p.to_string()).unwrap_or_default();
    let data_dir = to_relative(&runtime.data_dir);
    let log_dir = to_relative(&runtime.log_dir);

    let mut result = template
        .replace("{dir}", &install_root)
        .replace("{install_root}", &install_root)
        .replace("{port}", &port)
        .replace("{data_dir}", &data_dir)
        .replace("{log_dir}", &log_dir);

    // 如果 config_file 为空，双引号包裹的 "{config_file}" 占位符也应整体剔除（不能留下 ""，否则会导致程序闪退报错）
    if config_file.is_empty() {
        result = result.replace("\"{config_file}\"", "");
        result = result.replace("{config_file}", "");
    } else {
        result = result.replace("{config_file}", &config_file);
    }

    result
}

pub(super) fn run_service_command(
    cmd_str: &str,
    current_dir: Option<&Path>,
    detached: bool,
) -> Result<(), String> {
    let mut command = crate::commands::hidden_cmd::hidden_cmd("cmd");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.raw_arg(format!("/c \"{}\"", cmd_str));
    }
    #[cfg(not(windows))]
    {
        command.args(&["/c", cmd_str]);
    }
    if let Some(dir) = current_dir {
        command.current_dir(dir);
    }

    if detached {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::commands::hidden_cmd::spawn_breakaway_fallback(command)
            .map_err(|e| format!("启动服务失败: {}", e))?;
        std::thread::sleep(Duration::from_millis(800));
        Ok(())
    } else {
        let output = command
            .output()
            .map_err(|e| format!("执行服务命令失败: {}", e))?;
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !stderr.is_empty() {
                Err(stderr)
            } else if !stdout.is_empty() {
                Err(stdout)
            } else {
                Err(format!(
                    "服务命令执行失败 (exit code: {})",
                    output.status.code().unwrap_or(-1)
                ))
            }
        }
    }
}
