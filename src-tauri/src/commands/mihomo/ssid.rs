//! SSID 感知：按当前 Wi-Fi 名称自动切换订阅配置（抄 clash-party `sys/ssid.ts`）。
//!
//! 场景：家里的 Wi-Fi 走家庭宽带节点，公司的 Wi-Fi 走另一份订阅。
//! 这里只做「读当前 SSID + 按规则表匹配」，切换动作交给调用方（避免本模块反向
//! 依赖 profile/核心重载那一坨）。

/// 解析 Windows `netsh wlan show interfaces` 的输出，取出当前连接的 SSID。
///
/// 输出形如（注意还有 `BSSID` / `Signal` 等行，不能直接找包含 "SSID" 的行）：
/// ```text
///     Name                   : WLAN
///     State                  : connected
///     SSID                   : MyHomeWifi
///     BSSID                  : aa:bb:cc:dd:ee:ff
/// ```
pub fn parse_windows_ssid(output: &str) -> Option<String> {
    for line in output.lines() {
        let line = line.trim();
        // 只认以「SSID」开头的行：BSSID 以 B 开头，Signal 等不含该键。
        // 注意这里必须用 continue 而不是 `?` —— `?` 会在第一行不匹配时直接结束整个函数。
        let Some(rest) = line.strip_prefix("SSID") else {
            continue;
        };
        let Some(value) = rest.trim().strip_prefix(':') else {
            continue;
        };
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// 解析 Linux `nmcli -t -f active,ssid dev wifi` 的输出（`yes:MyHomeWifi`）。
pub fn parse_nmcli_ssid(output: &str) -> Option<String> {
    for line in output.lines() {
        let mut parts = line.splitn(2, ':');
        let active = parts.next()?.trim();
        let ssid = parts.next()?.trim();
        if active == "yes" && !ssid.is_empty() {
            return Some(ssid.to_string());
        }
    }
    None
}

/// 解析 macOS `networksetup -getairportnetwork en0` 的输出（`Current Wi-Fi Network: MyHomeWifi`）。
pub fn parse_macos_ssid(output: &str) -> Option<String> {
    let rest = output.split_once(':')?.1.trim();
    // 未连 Wi-Fi 时 macOS 会回 "You are not associated with an AirPort network."
    if rest.is_empty() || rest.starts_with("You are not") {
        return None;
    }
    Some(rest.to_string())
}

/// 当前所连 Wi-Fi 的 SSID（未连接 / 取不到返回 None）。
///
/// 全程同步命令 + 短超时：它会被调度器每分钟调用一次，绝不能卡住调度循环。
pub fn current_ssid() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW (0x08000000)：每分钟跑一次，不能弹黑框
        let out = std::process::Command::new("netsh")
            .args(["wlan", "show", "interfaces"])
            .creation_flags(0x0800_0000)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        // netsh 中文系统输出 GBK，先按 UTF-8 试，失败退回 lossy（SSID 多为 ASCII）
        let text = String::from_utf8(out.stdout.clone())
            .unwrap_or_else(|_| String::from_utf8_lossy(&out.stdout).to_string());
        return parse_windows_ssid(&text);
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("networksetup")
            .args(["-getairportnetwork", "en0"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        return parse_macos_ssid(&text);
    }
    #[cfg(target_os = "linux")]
    {
        let out = std::process::Command::new("nmcli")
            .args(["-t", "-f", "active,ssid", "dev", "wifi"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        return parse_nmcli_ssid(&text);
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_macos_ssid, parse_nmcli_ssid, parse_windows_ssid};

    #[test]
    fn parses_windows_netsh_output() {
        let out = "\r\n接口名称: WLAN\r\n    State                  : connected\r\n    SSID                   : MyHomeWifi\r\n    BSSID                  : aa:bb:cc:dd:ee:ff\r\n";
        assert_eq!(parse_windows_ssid(out).as_deref(), Some("MyHomeWifi"));
        // 关键：不能被 BSSID 那行带偏（都含 "SSID" 子串）
        let only_bssid = "    BSSID                  : aa:bb:cc:dd:ee:ff\r\n";
        assert_eq!(parse_windows_ssid(only_bssid), None);
        assert_eq!(parse_windows_ssid(""), None);
    }

    #[test]
    fn parses_nmcli_and_macos_output() {
        assert_eq!(parse_nmcli_ssid("no:Neighbour\nyes:Office5G\n").as_deref(), Some("Office5G"));
        assert_eq!(parse_nmcli_ssid("no:Neighbour\n"), None);
        assert_eq!(
            parse_macos_ssid("Current Wi-Fi Network: HomeNet").as_deref(),
            Some("HomeNet")
        );
        // 未连接时的提示语不能被当成 SSID
        assert_eq!(parse_macos_ssid("You are not associated with an AirPort network."), None);
    }
}
