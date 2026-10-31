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

#[derive(Debug)]
pub(crate) struct Command {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub message: &'static str,
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
        message,
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
                    command(
                        "net.exe",
                        vec!["stop".into(), name.into()],
                        None,
                        "正在停止证书服务",
                    ),
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
