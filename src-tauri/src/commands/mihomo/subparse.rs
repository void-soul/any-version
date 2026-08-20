//! 订阅内容嗅探与解析（对齐 clash-party 的 fetchAndValidateSubscription + SubStore 能力）
//!
//! 职责：
//! 1. sniff：检测订阅内容是 age 加密 / base64 节点 / 标准 YAML
//! 2. age 解密：若 age 加密且提供 age_secret_key，则解密
//! 3. base64 解码：非 YAML 内容尝试 base64 解码成节点列表
//! 4. 节点 → clash YAML：把 ss:// vmess:// trojan:// vless:// hysteria2:// 等节点
//!    转换为 clash 的 proxies 列表，供 mihomo 加载

use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;

/// 嗅探出的订阅内容类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubType {
    /// 标准 clash / mihomo YAML
    Yaml,
    /// base64 编码的节点列表（如 ss:// vmess://）
    Base64Nodes,
    /// age 加密内容（需 age_secret_key 解密）
    AgeEncrypted,
    /// 未知 / 无法识别的纯文本
    Unknown,
}

/// 嗅探订阅内容类型。
/// 判断顺序：age 头 -> 是否 YAML（含 proxies/proxy-providers/节点标记）-> 是否 base64 节点。
pub fn sniff_subscription_type(text: &str) -> SubType {
    let trimmed = text.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return SubType::Unknown;
    }

    // 1) age 加密：armor 头特征
    if trimmed.starts_with("-----BEGIN AGE ENCRYPTED FILE-----") {
        return SubType::AgeEncrypted;
    }

    // 2) 标准 YAML：以 proxies / proxy-providers / 其它 clash 键开头，或包含关键节
    if looks_like_clash_yaml(trimmed) {
        return SubType::Yaml;
    }

    // 3) base64 节点列表：内容看起来是 base64（无换行的节点序列）或直接是节点 URL 文本
    if looks_like_base64(trimmed) || contains_node_uris(trimmed) {
        return SubType::Base64Nodes;
    }

    SubType::Unknown
}

/// 判断内容是否像 clash/mihomo YAML。
fn looks_like_clash_yaml(text: &str) -> bool {
    let first_line = text.lines().next().unwrap_or("").trim();
    // 常见 clash 顶级键
    const TOP_KEYS: &[&str] = &[
        "proxies:",
        "proxy-groups:",
        "proxy-providers:",
        "rules:",
        "mixed-port:",
        "port:",
        "socks-port:",
        "allow-lan:",
        "mode:",
        "log-level:",
        "dns:",
        "tun:",
        "proxies: [",
    ];
    for k in TOP_KEYS {
        if first_line.starts_with(k) || text.lines().any(|l| l.trim_start().starts_with(k)) {
            return true;
        }
    }
    false
}

/// 判断文本是否像 base64 编码（且包含常见节点协议标记）。
fn looks_like_base64(text: &str) -> bool {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.len() < 16 || compact.len() > 20_000_000 {
        return false;
    }
    // 允许的 base64 字符集，标准 + URL-safe
    let valid = compact.chars().all(|c| {
        c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' || c == '-' || c == '_'
    });
    if !valid {
        return false;
    }
    // 尝试解码，解码后应含协议标记
    if let Ok(decoded) = try_b64_decode(&compact) {
        let d = String::from_utf8_lossy(&decoded);
        return contains_node_uris(&d) || d.contains("proxies");
    }
    false
}

/// 判断文本是否包含已知的节点 URI 前缀。
fn contains_node_uris(text: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "ss://",
        "vmess://",
        "vless://",
        "trojan://",
        "hysteria2://",
        "hy2://",
        "hysteria://",
        "tuic://",
        "warp://",
    ];
    PREFIXES.iter().any(|p| text.contains(p))
}

/// base64 标准/URL-safe 解码。
fn try_b64_decode(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
    // 自动补齐 padding
    let mut pad = s.to_string();
    while pad.len() % 4 != 0 {
        pad.push('=');
    }
    STANDARD
        .decode(&pad)
        .or_else(|_| URL_SAFE_NO_PAD.decode(pad.replace('=', "")))
}

/// 解析订阅内容为可用于 mihomo 的 proxies（Value 数组）或完整 YAML 文本。
///
/// 返回处理后的内容文本。若为 base64 节点列表，转换为 clash YAML（含 proxies）。
/// age 加密内容由调用方先解密再传入。
pub fn process_subscription_content(text: &str, age_secret_key: Option<&str>) -> Result<String, String> {
    // 先去掉 BOM
    let text = text.trim_start_matches('\u{feff}').to_string();
    let st = sniff_subscription_type(&text);

    match st {
        SubType::AgeEncrypted => {
            let key = age_secret_key
                .ok_or_else(|| "订阅内容为 age 加密，但未配置 age 解密密钥".to_string())?;
            let decrypted = decrypt_age_content(&text, key)?;
            // 解密后可能是 base64 节点，也可能是 YAML
            process_subscription_content(&decrypted, None)
        }
        SubType::Base64Nodes => {
            // 解码为节点列表，再转 YAML
            let nodes = decode_base64_nodes(&text)?;
            build_proxies_yaml(&nodes)
        }
        SubType::Yaml => Ok(text),
        SubType::Unknown => {
            // 可能是纯文本节点列表（每行一个节点 URL）或其它
            if contains_node_uris(&text) {
                build_proxies_yaml(&collect_node_uris(&text))
            } else {
                Ok(text)
            }
        }
    }
}

/// 从 base64 订阅内容解码出节点 URI 列表。
fn decode_base64_nodes(text: &str) -> Result<Vec<String>, String> {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    // 已是节点 URI 文本（可能直接是 ss:// 开头的文本）
    if contains_node_uris(&compact) {
        return Ok(collect_node_uris(&compact));
    }
    // 否则尝试 base64 解码
    let decoded = try_b64_decode(&compact)
        .map_err(|_| "订阅内容既不是 YAML 也不是有效 base64，无法解析".to_string())?;
    let decoded_str = String::from_utf8_lossy(&decoded).to_string();
    // 解码后可能是 base64 的 base64（二次编码），或节点 URI，或 YAML
    if contains_node_uris(&decoded_str) {
        Ok(collect_node_uris(&decoded_str))
    } else if looks_like_base64(&decoded_str) {
        decode_base64_nodes(&decoded_str)
    } else if looks_like_clash_yaml(&decoded_str) {
        // base64 里解出来是 YAML
        Ok(vec![decoded_str])
    } else {
        Ok(collect_node_uris(&decoded_str))
    }
}

/// 从文本中提取所有节点 URI（按行或按 token 提取）。
fn collect_node_uris(text: &str) -> Vec<String> {
    const PREFIXES: &[&str] = &[
        "ss://",
        "vmess://",
        "vless://",
        "trojan://",
        "hysteria2://",
        "hy2://",
        "hysteria://",
        "tuic://",
        "warp://",
    ];
    let mut out = Vec::new();
    // 按行优先
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if let Some(_p) = PREFIXES.iter().find(|p| l.starts_with(**p)) {
            out.push(l.to_string());
        }
    }
    // 若按行没找到，尝试按空白/分隔切分
    if out.is_empty() {
        for token in text.split_whitespace() {
            if PREFIXES.iter().any(|p| token.starts_with(*p)) {
                out.push(token.to_string());
            }
        }
    }
    out
}

/// 把节点 URI 列表转换为 clash proxies YAML。
fn build_proxies_yaml(nodes: &[String]) -> Result<String, String> {
    if nodes.is_empty() {
        return Err("未从订阅中解析到任何节点".to_string());
    }
    let mut proxies = Vec::new();
    let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for uri in nodes {
        let Some(mut proxy) = parse_node_uri(uri) else {
            continue;
        };
        // 重名去重：mihomo 要求 proxy 名字全局唯一，同名会导致启动校验失败
        // （如多个无 #name 的节点都默认解析成 "server:port"）。自动加 -2/-3... 后缀。
        if let Some(name) = proxy.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()) {
            if used_names.contains(&name) {
                let mut i = 2u32;
                let mut new_name = format!("{name}-{i}");
                while used_names.contains(&new_name) {
                    i += 1;
                    new_name = format!("{name}-{i}");
                }
                if let Some(obj) = proxy.as_object_mut() {
                    obj.insert("name".to_string(), serde_json::Value::String(new_name.clone()));
                }
                used_names.insert(new_name);
            } else {
                used_names.insert(name);
            }
        }
        proxies.push(proxy);
    }
    if proxies.is_empty() {
        return Err("订阅内容无法识别为有效节点".to_string());
    }
    let yaml = serde_yaml::to_string(&json!({ "proxies": proxies }))
        .map_err(|e| format!("生成 clash 配置失败: {}", e))?;
    Ok(yaml)
}

/// 解析单个节点 URI 为 clash proxy 对象（Value）。
/// 支持：ss:// vmess:// trojan:// vless:// hysteria2:// hy2:// hysteria:// tuic://
fn parse_node_uri(uri: &str) -> Option<Value> {
    if let Some(rest) = uri.strip_prefix("ss://") {
        parse_ss(rest)
    } else if let Some(rest) = uri.strip_prefix("vmess://") {
        parse_vmess(rest)
    } else if let Some(rest) = uri.strip_prefix("trojan://") {
        parse_trojan(rest)
    } else if let Some(rest) = uri.strip_prefix("vless://") {
        parse_vless(rest)
    } else if let Some(rest) = uri.strip_prefix("hysteria2://").or_else(|| uri.strip_prefix("hy2://")) {
        parse_hysteria2(rest)
    } else if let Some(rest) = uri.strip_prefix("hysteria://") {
        parse_hysteria(rest)
    } else if let Some(rest) = uri.strip_prefix("tuic://") {
        parse_tuic(rest)
    } else {
        None
    }
}

/// 解析 ss:// 节点。支持两种格式：
/// - 旧式：ss://base64(method:password)@server:port#name
/// - 新式：ss://method:password@server:port#name 或带参数
fn parse_ss(rest: &str) -> Option<Value> {
    // 分离 fragment(name) 与 query
    let (main, _name) = split_fragment(rest);
    let (auth_host, query) = split_query(&main);
    // 旧式 ss:// 把整个 method:password@server:port base64 成一个字符串（无字面 @）。
    // 若 auth_host 不是 base64 但整体是 base64（解码后含 @），先整体解码。
    let mut body = auth_host.clone();
    if !body.contains('@') {
        if let Ok(decoded) = try_b64_decode(body.trim_end_matches('=')) {
            let ds = String::from_utf8_lossy(&decoded).to_string();
            if ds.contains('@') {
                body = ds;
            }
        }
    }
    let (auth, host) = split_at_last(&body, '@')?;
    let (server, port) = split_host_port(&host)?;
    // auth 可能是 method:password 明文 或 base64
    let (method, password) = if auth.contains(':') {
        let idx = auth.find(':')?;
        (auth[..idx].to_string(), auth[idx + 1..].to_string())
    } else {
        // base64 编码的 method:password
        let decoded = try_b64_decode(auth.trim_end_matches('=')).ok()?;
        let s = String::from_utf8_lossy(&decoded).to_string();
        let idx = s.find(':')?;
        (s[..idx].to_string(), s[idx + 1..].to_string())
    };
    // 解析参数（plugin 等可忽略，或支持 obfs）
    let mut obj = json!({
        "name": format!("{}:{}", server, port),
        "type": "ss",
        "server": server,
        "port": port,
        "cipher": method,
        "password": password,
    });
    if !query.is_empty() {
        let params = parse_query(&query);
        if let Some(plugin) = params.get("plugin") {
            obj["plugin"] = json!(plugin);
        }
    }
    Some(obj)
}

/// 解析 vmess:// 节点（payload 是 base64 的 JSON）。
fn parse_vmess(rest: &str) -> Option<Value> {
    let decoded = try_b64_decode(rest.trim_end_matches('=')).ok()?;
    let json_str = String::from_utf8_lossy(&decoded);
    let v: Value = serde_json::from_str(&json_str).ok()?;
    let server = v.get("add")?.as_str()?;
    // port/aid 可能是字符串或数字（部分订阅生成器用字符串）
    let port = v
        .get("port")
        .and_then(|p| p.as_u64().or_else(|| p.as_str().and_then(|s| s.parse().ok())))?;
    let uuid = v.get("id")?.as_str()?;
    let aid = v
        .get("aid")
        .and_then(|a| a.as_u64().or_else(|| a.as_str().and_then(|s| s.parse().ok())))
        .unwrap_or(0);
    let default_name = format!("{}:{}", server, port);
    let name = v.get("ps").and_then(|x| x.as_str()).unwrap_or(&default_name);
    let network = v.get("net").and_then(|x| x.as_str()).unwrap_or("tcp");
    let tls = v.get("tls").and_then(|x| x.as_str()).unwrap_or("");
    let mut obj = json!({
        "name": name,
        "type": "vmess",
        "server": server,
        "port": port,
        "uuid": uuid,
        "alterId": aid,
        "cipher": "auto",
        "network": network,
    });
    // tls
    if !tls.is_empty() && tls != "none" {
        obj["tls"] = json!(true);
        if let Some(host) = v.get("host").and_then(|x| x.as_str()) {
            if !host.is_empty() {
                obj["servername"] = json!(host);
            }
        }
    }
    // ws 路径
    if network == "ws" || network == "h2" {
        if let Some(path) = v.get("path").and_then(|x| x.as_str()) {
            obj["ws-opts"] = json!({ "path": path });
        }
        if let Some(host) = v.get("host").and_then(|x| x.as_str()) {
            if !host.is_empty() {
                if obj.get("ws-opts").is_none() {
                    obj["ws-opts"] = json!({});
                }
                obj["ws-opts"]["headers"] = json!({ "Host": host });
            }
        }
    }
    // grpc
    if network == "grpc" {
        if let Some(spath) = v.get("path").and_then(|x| x.as_str()) {
            obj["grpc-opts"] = json!({ "grpc-service-name": spath });
        }
    }
    Some(obj)
}

/// 解析 trojan:// 节点。
fn parse_trojan(rest: &str) -> Option<Value> {
    let (main, name) = split_fragment(rest);
    let (auth_host, query) = split_query(&main);
    let (password, host) = split_at_last(&auth_host, '@')?;
    let (server, port) = split_host_port(&host)?;
    let mut obj = json!({
        "name": if name.is_empty() { format!("{}:{}", server, port) } else { name },
        "type": "trojan",
        "server": server,
        "port": port,
        "password": password,
    });
    if !query.is_empty() {
        let params = parse_query(&query);
        if let Some(sni) = params.get("sni") {
            obj["sni"] = json!(sni);
            obj["servername"] = json!(sni);
        }
        if let Some(allow) = params.get("allowInsecure").and_then(|x| x.parse::<bool>().ok()) {
            obj["skip-cert-verify"] = json!(allow);
        }
        if let Some(ws) = params.get("ws") {
            if ws == "1" {
                obj["network"] = json!("ws");
                if let Some(wspath) = params.get("wspath") {
                    obj["ws-opts"] = json!({ "path": wspath });
                }
            }
        }
    }
    Some(obj)
}

/// 解析 vless:// 节点。
fn parse_vless(rest: &str) -> Option<Value> {
    let (main, name) = split_fragment(rest);
    let (auth_host, query) = split_query(&main);
    let (uuid, host) = split_at_last(&auth_host, '@')?;
    let (server, port) = split_host_port(&host)?;
    let mut obj = json!({
        "name": if name.is_empty() { format!("{}:{}", server, port) } else { name },
        "type": "vless",
        "server": server,
        "port": port,
        "uuid": uuid,
        "network": "tcp",
        "tls": false,
    });
    if !query.is_empty() {
        let params = parse_query(&query);
        if let Some(security) = params.get("security") {
            if security == "tls" || security == "reality" {
                obj["tls"] = json!(true);
            }
            if let Some(sni) = params.get("sni") {
                obj["servername"] = json!(sni);
            }
            if let Some(pb) = params.get("pbk") {
                obj["reality-opts"] = json!({ "public-key": pb });
            }
            if let Some(spid) = params.get("sid") {
                if obj.get("reality-opts").is_none() {
                    obj["reality-opts"] = json!({});
                }
                obj["reality-opts"]["short-id"] = json!(spid);
            }
        }
        if let Some(net) = params.get("type") {
            obj["network"] = json!(net);
            if net == "ws" {
                let mut wsopts = json!({});
                if let Some(path) = params.get("path") {
                    wsopts["path"] = json!(path);
                }
                if let Some(host) = params.get("host") {
                    wsopts["headers"] = json!({ "Host": host });
                }
                obj["ws-opts"] = wsopts;
            } else if net == "grpc" {
                let mut gopts = json!({});
                if let Some(spath) = params.get("serviceName").or(params.get("path")) {
                    gopts["grpc-service-name"] = json!(spath);
                }
                obj["grpc-opts"] = gopts;
            }
        }
    }
    Some(obj)
}

/// 解析 hysteria2://（hy2://）节点。
fn parse_hysteria2(rest: &str) -> Option<Value> {
    let (main, name) = split_fragment(rest);
    let (auth_host, query) = split_query(&main);
    let (password, host) = split_at_last(&auth_host, '@')?;
    let (server, port) = split_host_port(&host)?;
    let mut obj = json!({
        "name": if name.is_empty() { format!("{}:{}", server, port) } else { name },
        "type": "hysteria2",
        "server": server,
        "port": port,
        "password": password,
    });
    if !query.is_empty() {
        let params = parse_query(&query);
        if let Some(sni) = params.get("sni") {
            obj["sni"] = json!(sni);
        }
        if let Some(obfs) = params.get("obfs") {
            obj["obfs"] = json!(obfs);
        }
        if let Some(obfs_password) = params.get("obfs-password") {
            obj["obfs-password"] = json!(obfs_password);
        }
        if let Some(insecure) = params.get("insecure").and_then(|x| x.parse::<bool>().ok()) {
            obj["skip-cert-verify"] = json!(insecure);
        }
    }
    Some(obj)
}

/// 解析 hysteria:// 节点。
fn parse_hysteria(rest: &str) -> Option<Value> {
    let (main, name) = split_fragment(rest);
    let (auth_host, query) = split_query(&main);
    let (auth, host) = split_at_last(&auth_host, '@')?;
    let (server, port) = split_host_port(&host)?;
    let mut obj = json!({
        "name": if name.is_empty() { format!("{}:{}", server, port) } else { name },
        "type": "hysteria",
        "server": server,
        "port": port,
        "auth_str": auth,
        "protocol": "udp",
    });
    if !query.is_empty() {
        let params = parse_query(&query);
        if let Some(up) = params.get("upmbps") {
            obj["up"] = json!(up);
        }
        if let Some(down) = params.get("downmbps") {
            obj["down"] = json!(down);
        }
        if let Some(insecure) = params.get("insecure").and_then(|x| x.parse::<bool>().ok()) {
            obj["skip-cert-verify"] = json!(insecure);
        }
        if let Some(sni) = params.get("peer") {
            obj["sni"] = json!(sni);
        }
    }
    Some(obj)
}

/// 解析 tuic:// 节点。
fn parse_tuic(rest: &str) -> Option<Value> {
    let (main, name) = split_fragment(rest);
    let (auth_host, query) = split_query(&main);
    let (password, host) = split_at_last(&auth_host, '@')?;
    let (server, port) = split_host_port(&host)?;
    let mut obj = json!({
        "name": if name.is_empty() { format!("{}:{}", server, port) } else { name },
        "type": "tuic",
        "server": server,
        "port": port,
        "password": password,
        "congestion-control": "bbr",
    });
    if !query.is_empty() {
        let params = parse_query(&query);
        if let Some(uuid) = params.get("uuid") {
            obj["uuid"] = json!(uuid);
        }
        if let Some(algorithm) = params.get("congestion_control") {
            obj["congestion-control"] = json!(algorithm);
        }
        if let Some(insecure) = params.get("allow_insecure").and_then(|x| x.parse::<bool>().ok()) {
            obj["skip-cert-verify"] = json!(insecure);
        }
        if let Some(sni) = params.get("sni") {
            obj["sni"] = json!(sni);
        }
    }
    Some(obj)
}

// ---- 工具函数 ----

/// 分离 fragment（#name）。
fn split_fragment(s: &str) -> (String, String) {
    if let Some(idx) = s.find('#') {
        (s[..idx].to_string(), s[idx + 1..].to_string())
    } else {
        (s.to_string(), String::new())
    }
}

/// 分离 query（?xxx）。
fn split_query(s: &str) -> (String, String) {
    if let Some(idx) = s.find('?') {
        (s[..idx].to_string(), s[idx + 1..].to_string())
    } else {
        (s.to_string(), String::new())
    }
}

/// 从右往左按分隔符切分。
fn split_at_last(s: &str, sep: char) -> Option<(String, String)> {
    let idx = s.rfind(sep)?;
    Some((s[..idx].to_string(), s[idx + 1..].to_string()))
}

/// 分离 host:port。
fn split_host_port(s: &str) -> Option<(String, String)> {
    // 处理 IPv6 中括号
    if s.starts_with('[') {
        if let Some(end) = s.find(']') {
            let host = s[1..end].to_string();
            let rest = &s[end + 1..];
            let port = rest.strip_prefix(':').unwrap_or("443");
            return Some((host, port.to_string()));
        }
    }
    if let Some(idx) = s.rfind(':') {
        return Some((s[..idx].to_string(), s[idx + 1..].to_string()));
    }
    Some((s.to_string(), "443".to_string()))
}

/// 解析 query 参数。
fn parse_query(query: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.find('=') {
            Some(idx) => (pair[..idx].to_string(), pair[idx + 1..].to_string()),
            None => (pair.to_string(), String::new()),
        };
        map.insert(k, percent_decode(&v));
    }
    map
}

/// 简单百分号解码。
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            if hi < 16 && lo < 16 {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn hex_val(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 16,
    }
}

/// age 解密：支持 armor 格式（-----BEGIN AGE ENCRYPTED FILE-----）。
/// 由于未引入 age crate，这里通过调用 age 命令行工具实现。
/// 若本机无 age CLI，则返回明确错误提示。
pub fn decrypt_age_content(armored: &str, secret_key: &str) -> Result<String, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // 先判断是否真的是 armor 格式
    if !armored.trim_start().starts_with("-----BEGIN AGE ENCRYPTED FILE-----") {
        return Err("内容不是 age 加密格式".to_string());
    }

    let mut child = Command::new("age")
        .arg("--decrypt")
        .arg("-i")
        .arg("-") // 私钥从 stdin 读
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法调用 age 解密工具（请先安装 age：winget install age），错误: {}", e))?;

    // 私钥 + 密文都写 stdin
    {
        let mut stdin = child.stdin.take().ok_or("无法获取 age 输入")?;
        let input = format!("{}\n{}", secret_key, armored);
        let _ = stdin.write_all(input.as_bytes());
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("age 解密失败: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        Err(format!("age 解密失败: {}", err.trim()))
    }
}

/// 从订阅 URL 解析出 clash 可用的 YAML（供 mihomo proxy-providers 使用）。
/// 由外部调用：先下载订阅文本，再 process_subscription_content。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sniff_yaml() {
        let yaml = "proxies:\n  - name: x\n    type: ss\n";
        assert_eq!(sniff_subscription_type(yaml), SubType::Yaml);
    }

    #[test]
    fn test_sniff_base64() {
        // base64("ss://YmFzZTY0OnBhc3NAMS4yLjMuNDo0NDM=") 这种
        let plain = "ss://YWVzLTI1Ni1nY206cGFzc0AxLjIuMy40OjQ0Mw==";
        let b64 = base64::engine::general_purpose::STANDARD.encode(plain);
        assert_eq!(sniff_subscription_type(&b64), SubType::Base64Nodes);
    }

    #[test]
    fn test_parse_ss() {
        let v = parse_ss("YWVzLTI1Ni1nY206cGFzc0AxLjIuMy40OjQ0Mw==").unwrap();
        assert_eq!(v["type"], "ss");
        assert_eq!(v["server"], "1.2.3.4");
    }

    #[test]
    fn test_parse_ss_plain() {
        let v = parse_ss("aes-256-gcm:pass@1.2.3.4:443").unwrap();
        assert_eq!(v["cipher"], "aes-256-gcm");
        assert_eq!(v["password"], "pass");
    }

    #[test]
    fn test_parse_vmess() {
        let cfg = json!({"add":"example.com","port":"443","id":"uuid-1","aid":"0","ps":"test","net":"ws","tls":"tls","path":"/ws","host":"cdn.example.com"});
        let b64 = base64::engine::general_purpose::STANDARD.encode(cfg.to_string());
        let v = parse_vmess(&b64).unwrap();
        assert_eq!(v["type"], "vmess");
        assert_eq!(v["uuid"], "uuid-1");
    }

    #[test]
    fn test_parse_trojan() {
        let v = parse_trojan("pass123@example.com:443?security=tls#node").unwrap();
        assert_eq!(v["name"], "node");
        assert_eq!(v["server"], "example.com");
        assert_eq!(v["port"], "443");
    }

    #[test]
    fn test_build_yaml() {
        let nodes = vec!["ss://aes-256-gcm:pass@1.2.3.4:443".to_string()];
        let yaml = build_proxies_yaml(&nodes).unwrap();
        assert!(yaml.contains("proxies:"));
        assert!(yaml.contains("type: ss"));
    }
}
