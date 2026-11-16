//! 端口转发的纯逻辑（规则校验 + SOCKS5 目标解析）。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForwardKind {
    /// 本地监听 → 远端目标（direct-tcpip）
    Local,
    /// 远端监听 → 本地目标（tcpip-forward）
    Remote,
    /// 本地 SOCKS5 代理 → 每连接一个 direct-tcpip
    Dynamic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardRule {
    pub id: String,
    pub kind: ForwardKind,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
}

pub fn validate_rule(r: &ForwardRule) -> Result<(), String> {
    if r.bind_port == 0 {
        return Err("监听端口不能为 0".to_string());
    }
    if r.bind_host.trim().is_empty() {
        return Err("监听地址不能为空".to_string());
    }
    if r.kind == ForwardKind::Dynamic {
        // SOCKS5 由客户端在每条连接里给出目标，规则上不需要预填。
        return Ok(());
    }
    if r.target_host.trim().is_empty() {
        return Err("目标主机不能为空".to_string());
    }
    if r.target_port == 0 {
        return Err("目标端口不能为 0".to_string());
    }
    Ok(())
}

/// 解析 `tcpip-forward` 的生效端口。
/// RFC 4254 §7.1：服务端同意固定端口请求时返回 0，表示「沿用请求的端口」；
/// 只有请求端口为 0（由服务端分配）时，返回值才是真正端口。
pub fn resolve_assigned_port(requested: u16, returned: u32) -> u16 {
    if returned == 0 {
        requested
    } else {
        u16::try_from(returned).unwrap_or(requested)
    }
}

/// 解析 SOCKS5 的 ATYP + DST.ADDR + DST.PORT（不含 VER/CMD/RSV）。
/// ATYP: 1=IPv4, 3=域名, 4=IPv6。长度不足或类型未知一律拒绝，避免读到越界数据。
pub fn parse_socks5_target(data: &[u8]) -> Option<(String, u16)> {
    if data.len() < 2 {
        return None;
    }
    let atyp = data[0];
    let rest = &data[1..];
    let (host, tail) = match atyp {
        1 => {
            if rest.len() < 6 {
                return None;
            }
            let h = std::net::Ipv4Addr::new(rest[0], rest[1], rest[2], rest[3]).to_string();
            (h, &rest[4..])
        }
        3 => {
            let len = *rest.first()? as usize;
            if len == 0 || rest.len() < len + 3 {
                return None;
            }
            let h = std::str::from_utf8(&rest[1..1 + len]).ok()?.to_string();
            (h, &rest[1 + len..])
        }
        4 => {
            if rest.len() < 18 {
                return None;
            }
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&rest[..16]);
            let h = std::net::Ipv6Addr::from(octets).to_string();
            (h, &rest[16..])
        }
        _ => return None,
    };
    if tail.len() < 2 {
        return None;
    }
    let port = u16::from_be_bytes([tail[0], tail[1]]);
    if port == 0 {
        return None;
    }
    Some((host, port))
}
