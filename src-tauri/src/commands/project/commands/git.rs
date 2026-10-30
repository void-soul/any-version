use super::*;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct GitRepoStatus {
    pub is_git: bool,
    pub has_exe: bool,
    pub has_update: bool,
    pub current_commit: String,
    pub latest_commit: String,
}

pub(super) fn run_git_cmd(dir: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = std::process::Command::new("git");
    cmd.current_dir(dir);
    cmd.args(crate::commands::utils::with_git_long_paths(args));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let output = cmd.output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[tauri::command]
pub fn check_git_repo_status(
    path: String,
    exe_name: String,
    bootstrap_cmd: Option<String>,
) -> Result<GitRepoStatus, String> {
    let path_buf = std::path::Path::new(&path);
    if !path_buf.exists() {
        return Ok(GitRepoStatus {
            is_git: false,
            has_exe: false,
            has_update: false,
            current_commit: String::new(),
            latest_commit: String::new(),
        });
    }

    let has_bootstrap = if let Some(ref b_cmd) = bootstrap_cmd {
        path_buf.join(b_cmd).exists() || {
            if b_cmd.ends_with(".bat") || b_cmd.ends_with(".cmd") {
                let sh_version = b_cmd.replace(".bat", ".sh").replace(".cmd", ".sh");
                path_buf.join(sh_version).exists()
            } else {
                false
            }
        }
    } else {
        false
    };

    let is_git = path_buf.join(".git").exists() && (has_bootstrap || bootstrap_cmd.is_none());

    let full_exe_name = if cfg!(windows) {
        let lower = exe_name.to_lowercase();
        if !lower.ends_with(".exe") && !lower.ends_with(".cmd") && !lower.ends_with(".bat") {
            format!("{}.exe", exe_name)
        } else {
            exe_name.clone()
        }
    } else {
        exe_name.clone()
    };
    let has_exe = path_buf.join(&full_exe_name).exists()
        || path_buf.join("bin").join(&full_exe_name).exists();

    let mut has_update = false;
    let mut current_commit = String::new();
    let mut latest_commit = String::new();

    if is_git {
        if let Ok(curr) = run_git_cmd(path_buf, &["rev-parse", "--short", "HEAD"]) {
            current_commit = curr;
        }

        let _ = run_git_cmd(path_buf, &["fetch"]);

        let upstream = run_git_cmd(path_buf, &["rev-parse", "--short", "@{u}"])
            .or_else(|_| run_git_cmd(path_buf, &["rev-parse", "--short", "origin/master"]))
            .or_else(|_| run_git_cmd(path_buf, &["rev-parse", "--short", "origin/main"]));

        if let Ok(up) = upstream {
            latest_commit = up.clone();
            if !current_commit.is_empty() && current_commit != latest_commit {
                if let Ok(merge_base) =
                    run_git_cmd(path_buf, &["merge-base", "HEAD", &latest_commit])
                {
                    if merge_base == current_commit {
                        has_update = true;
                    }
                }
            }
        }
    }

    Ok(GitRepoStatus {
        is_git,
        has_exe,
        has_update,
        current_commit,
        latest_commit,
    })
}

#[tauri::command]
pub fn bootstrap_git_repo(path: String, cmd: String) -> Result<(), String> {
    let dir = std::path::Path::new(&path);
    if !dir.exists() {
        return Err("目录不存在".to_string());
    }

    let cmd_to_run = if cfg!(windows) {
        cmd.clone()
    } else {
        if cmd.ends_with(".bat") || cmd.ends_with(".cmd") {
            let sh_version = cmd.replace(".bat", ".sh").replace(".cmd", ".sh");
            if dir.join(&sh_version).exists() {
                format!("./{}", sh_version)
            } else {
                cmd.clone()
            }
        } else {
            cmd.clone()
        }
    };

    // 防范命令行注入：禁止在初始化脚本参数中包含危险 Shell 多命令连接符
    if cmd_to_run.contains('&')
        || cmd_to_run.contains('|')
        || cmd_to_run.contains(';')
        || cmd_to_run.contains('>')
        || cmd_to_run.contains('<')
    {
        return Err("非法初始化脚本命令，禁止包含 Shell 管道或多命令分隔符".to_string());
    }

    let mut process_cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.current_dir(dir);
        c.args(&["/c", &cmd_to_run]);
        c
    } else {
        let mut c = std::process::Command::new("sh");
        c.current_dir(dir);
        if cmd_to_run.starts_with("./") {
            c.args(&[&cmd_to_run[2..]]);
        } else {
            c.args(&[&cmd_to_run]);
        }
        c
    };

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        process_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let output = process_cmd
        .output()
        .map_err(|e| format!("执行初始化脚本失败: {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Err(format!(
            "初始化脚本执行失败!\nStderr: {}\nStdout: {}",
            stderr, stdout
        ))
    }
}

#[tauri::command]
pub fn update_git_repo(path: String, bootstrap_cmd: Option<String>) -> Result<(), String> {
    let dir = std::path::Path::new(&path);
    if !dir.exists() {
        return Err("目录不存在".to_string());
    }

    let mut pull_cmd = std::process::Command::new("git");
    pull_cmd.current_dir(dir);
    pull_cmd.args(crate::commands::utils::with_git_long_paths(&["pull"]));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        pull_cmd.creation_flags(0x08000000);
    }
    let pull_output = pull_cmd
        .output()
        .map_err(|e| format!("git pull 失败: {}", e))?;
    if !pull_output.status.success() {
        let err = String::from_utf8_lossy(&pull_output.stderr)
            .trim()
            .to_string();
        return Err(format!("Git pull 失败: {}", err));
    }

    if let Some(ref b_cmd) = bootstrap_cmd {
        bootstrap_git_repo(path, b_cmd.clone())
    } else {
        Ok(())
    }
}
