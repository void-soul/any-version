use crate::protocol::ReloadOptions;
use std::path::Path;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Platform {
    Windows,
    Linux,
    MacOs,
    OtherUnix,
}
impl Platform {
    pub(crate) fn current() -> Self {
        #[cfg(windows)]
        {
            return Self::Windows;
        }
        #[cfg(target_os = "linux")]
        {
            return Self::Linux;
        }
        #[cfg(target_os = "macos")]
        {
            return Self::MacOs;
        }
        #[allow(unreachable_code)]
        Self::OtherUnix
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Command {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub message: String,
    #[serde(default)]
    pub continue_on_error: bool,
}
fn command(
    program: &str,
    args: Vec<String>,
    cwd: Option<String>,
    message: &'static str,
) -> Command {
    Command {
        program: program.into(),
        args,
        cwd,
        message: message.into(),
        continue_on_error: false,
    }
}

pub(crate) fn plan(
    options: &ReloadOptions<'_>,
    platform: Platform,
) -> Result<Vec<Command>, String> {
    let prefix = options.nginx_prefix.trim();
    let cwd = (!prefix.is_empty()).then(|| prefix.to_string());
    let shell = |script: &str, message| {
        let (program, flag) = match platform {
            Platform::Windows => ("cmd.exe", "/c"),
            _ => ("/bin/sh", "-c"),
        };
        command(
            program,
            vec![flag.into(), script.into()],
            cwd.clone(),
            message,
        )
    };
    if let Some(script) = options.post_script.filter(|s| !s.trim().is_empty()) {
        return Ok(vec![shell(script, "正在执行证书部署后脚本")]);
    }
    match options.mode {
        "none" | "" => Ok(vec![]),
        "command" => {
            if options.command.trim().is_empty() {
                return Err("重载命令不能为空".into());
            }
            Ok(vec![shell(options.command.trim(), "正在执行重载命令")])
        }
        "nginx" => {
            let program = match platform {
                Platform::Windows if !prefix.is_empty() => Path::new(prefix)
                    .join("nginx.exe")
                    .to_string_lossy()
                    .into_owned(),
                Platform::Windows => "nginx.exe".into(),
                _ => "nginx".into(),
            };
            let mut args = vec![];
            if !prefix.is_empty() {
                args.extend(["-p".into(), prefix.into()]);
            }
            if !options.nginx_conf.trim().is_empty() {
                args.extend(["-c".into(), options.nginx_conf.trim().into()]);
            }
            let mut check = args.clone();
            check.push("-t".into());
            args.extend(["-s".into(), "reload".into()]);
            Ok(vec![
                command(&program, check, cwd.clone(), "正在检查Nginx配置"),
                command(&program, args, cwd, "正在重载Nginx"),
            ])
        }
        "service" => {
            let name = options.service_name.trim();
            if name.is_empty() {
                return Err("服务名不能为空".into());
            }
            match platform {
                Platform::Windows => Ok(vec![
                    Command {
                        continue_on_error: true,
                        ..command(
                            "net.exe",
                            vec!["stop".into(), name.into()],
                            None,
                            "正在停止证书服务",
                        )
                    },
                    command(
                        "net.exe",
                        vec!["start".into(), name.into()],
                        None,
                        "正在启动证书服务",
                    ),
                ]),
                Platform::Linux => Ok(vec![command(
                    "systemctl",
                    vec!["restart".into(), "--".into(), name.into()],
                    None,
                    "正在重启证书服务",
                )]),
                Platform::MacOs => Err(
                    "macOS 不支持 service 重载模式，请配置 command 模式及对应 launchctl 命令"
                        .into(),
                ),
                Platform::OtherUnix => {
                    Err("此系统不支持 service 重载模式，请配置 command 模式".into())
                }
            }
        }
        _ => Err("重载方式无效".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(mode: &str) -> ReloadOptions<'_> {
        ReloadOptions {
            mode,
            command: "echo reloaded",
            nginx_prefix: "",
            nginx_conf: "",
            service_name: "nginx service",
            post_script: None,
        }
    }
    #[test]
    fn windows_command_and_service_contract_is_preserved() {
        let commands = plan(&options("command"), Platform::Windows).unwrap();
        assert_eq!(commands[0].program, "cmd.exe");
        assert_eq!(commands[0].args, ["/c", "echo reloaded"]);
        let service = plan(&options("service"), Platform::Windows).unwrap();
        assert_eq!(service.len(), 2);
        assert_eq!(service[0].program, "net.exe");
        assert_eq!(service[0].args, ["stop", "nginx service"]);
        assert_eq!(service[1].args, ["start", "nginx service"]);
    }
    #[test]
    fn unix_command_and_post_script_use_sh() {
        let mut options = options("command");
        for platform in [Platform::Linux, Platform::MacOs] {
            let commands = plan(&options, platform).unwrap();
            assert_eq!(commands[0].program, "/bin/sh");
            assert_eq!(commands[0].args, ["-c", "echo reloaded"]);
        }
        options.post_script = Some("post script");
        let commands = plan(&options, Platform::Linux).unwrap();
        assert_eq!(commands[0].args, ["-c", "post script"]);
    }
    #[test]
    fn nginx_is_checked_before_reload_on_both_platforms() {
        let mut options = options("nginx");
        options.nginx_prefix = "nginx-prefix";
        options.nginx_conf = "custom.conf";
        let windows = plan(&options, Platform::Windows).unwrap();
        assert!(windows[0].program.ends_with("nginx.exe"));
        let linux = plan(&options, Platform::Linux).unwrap();
        assert_eq!(linux[0].program, "nginx");
        for commands in [windows, linux] {
            assert_eq!(
                commands[0].args,
                ["-p", "nginx-prefix", "-c", "custom.conf", "-t"]
            );
            assert_eq!(
                commands[1].args,
                ["-p", "nginx-prefix", "-c", "custom.conf", "-s", "reload"]
            );
        }
    }
    #[test]
    fn linux_service_is_an_argv_and_macos_has_explicit_guidance() {
        let commands = plan(&options("service"), Platform::Linux).unwrap();
        assert_eq!(commands[0].program, "systemctl");
        assert_eq!(commands[0].args, ["restart", "--", "nginx service"]);
        assert!(plan(&options("service"), Platform::MacOs)
            .unwrap_err()
            .contains("command"));
    }
}

pub(crate) fn execute(commands: &[Command], log: &impl Fn(&str)) -> Result<String, String> {
    execute_with(commands, log, |command| {
        let args: Vec<_> = command.args.iter().map(String::as_str).collect();
        crate::reload_command::run(&command.program, &args, command.cwd.as_deref())
    })
}

pub(crate) fn execute_with(
    commands: &[Command],
    log: &impl Fn(&str),
    mut run: impl FnMut(&Command) -> Result<String, String>,
) -> Result<String, String> {
    let mut output = String::new();
    for command in commands {
        log(&command.message);
        match run(command) {
            Ok(result) => output = result,
            Err(error) if command.continue_on_error => {
                log(&format!("停止服务未成功，继续尝试启动以确认恢复: {error}"))
            }
            Err(error) => return Err(error),
        }
    }
    Ok(output)
}

#[cfg(test)]
mod retry_tests {
    use super::*;
    #[test]
    fn stopped_windows_service_can_be_started_but_start_must_succeed() {
        let options = ReloadOptions {
            mode: "service",
            command: "",
            nginx_prefix: "",
            nginx_conf: "",
            service_name: "fake-service",
            post_script: None,
        };
        let commands = plan(&options, Platform::Windows).unwrap();
        assert!(commands[0].continue_on_error);
        assert!(!commands[1].continue_on_error);
        let logs = std::cell::RefCell::new(vec![]);
        let mut calls = 0;
        let result = execute_with(&commands, &|s| logs.borrow_mut().push(s.to_string()), |c| {
            calls += 1;
            if c.args[0] == "stop" {
                Err("already stopped".into())
            } else {
                Ok("started".into())
            }
        });
        assert_eq!(result.unwrap(), "started");
        assert_eq!(calls, 2);
        assert!(logs.borrow().iter().any(|s| s.contains("already stopped")));
        assert!(execute_with(&commands, &|_| {}, |_| Err("fake refusal".into())).is_err());
        let old: Command = serde_json::from_str(
            r#"{"program":"net.exe","args":["start","fake"],"cwd":null,"message":"old"}"#,
        )
        .unwrap();
        assert!(!old.continue_on_error);
    }
}
