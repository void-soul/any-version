// mihomo config generation (对齐 clash-party factory.ts)
// profile + overrides + controled config -> 运行时 config.yaml
use crate::commands::mihomo::config::*;
use crate::commands::hidden_cmd::hidden_cmd;
use serde_json::Value;
use std::path::Path;

/// Smart 内核覆写的固定 id（对齐 clash-party SMART_OVERRIDE_ID）
pub const SMART_OVERRIDE_ID: &str = "smart-core-override";

/// 二级代理（家庭 socks5）节点的命名前缀
pub const SECONDARY_PREFIX: &str = "二级-";
/// 收纳全部二级代理节点的 Selector 策略组名
pub const SECONDARY_GROUP: &str = "二级代理";

/// 根据二级代理配置生成其在 proxies 里的节点名
fn secondary_node_name(s5: &crate::commands::mihomo::config::SecondaryProxy) -> String {
    format!("{SECONDARY_PREFIX}{}", s5.name)
}

/// 去掉 `<key>` 形式的尖括号包裹（对齐 clash-party trimWrap）
fn trim_wrap(s: &str) -> &str {
    if s.len() >= 2 && s.starts_with('<') && s.ends_with('>') {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// 与 clash-party 的 lodash mergeWith 等价：patch 覆盖 base（数组整体替换）
pub fn deep_merge(base: &mut Value, patch: &Value) {
    deep_merge_with(base, patch, false);
}

/// 对齐 clash-party deepMerge(target, other, isOverride)
/// is_override = true 时支持覆写专用语法：
/// - `key!`（对象）整体替换而非递归合并
/// - `+key`（数组）前置拼接
/// - `key+`（数组）后置拼接
/// - `<key>` 去掉尖括号包裹
pub fn deep_merge_with(base: &mut Value, patch: &Value, is_override: bool) {
    let (b, p) = match (base, patch) {
        (Value::Object(b), Value::Object(p)) => (b, p),
        (base, patch) => {
            *base = patch.clone();
            return;
        }
    };
    for (key, val) in p {
        match val {
            Value::Object(_) => {
                if let Some(stripped) = key.strip_suffix('!') {
                    b.insert(trim_wrap(stripped).to_string(), val.clone());
                } else {
                    let k = trim_wrap(key).to_string();
                    let entry = b.entry(k).or_insert_with(|| Value::Object(Default::default()));
                    if !entry.is_object() {
                        *entry = Value::Object(Default::default());
                    }
                    deep_merge_with(entry, val, is_override);
                }
            }
            Value::Array(arr) => {
                if is_override && key.starts_with('+') {
                    let k = trim_wrap(&key[1..]).to_string();
                    let mut merged = arr.clone();
                    if let Some(Value::Array(old)) = b.get(&k) {
                        merged.extend(old.clone());
                    }
                    b.insert(k, Value::Array(merged));
                } else if is_override && key.ends_with('+') {
                    let k = trim_wrap(&key[..key.len() - 1]).to_string();
                    let mut merged = match b.get(&k) {
                        Some(Value::Array(old)) => old.clone(),
                        _ => vec![],
                    };
                    merged.extend(arr.clone());
                    b.insert(k, Value::Array(merged));
                } else {
                    b.insert(trim_wrap(key).to_string(), val.clone());
                }
            }
            _ => {
                b.insert(key.clone(), val.clone());
            }
        }
    }
}

/// 订阅型 profile：从订阅源/规则库/自定义规则/DNS 构造基础配置 Value
pub fn build_subscription_profile(item: &ProfileItem) -> Value {
    let mut root = serde_json::Map::new();

    // proxy-providers
    let mut providers = serde_json::Map::new();
    for sub in &item.subscriptions {
        let mut p = serde_json::Map::new();
        p.insert("type".into(), Value::String("http".into()));
        p.insert("url".into(), Value::String(sub.url.clone()));
        p.insert(
            "interval".into(),
            Value::Number((sub.interval.max(60) as u64).into()),
        );
        p.insert("path".into(), Value::String(format!("{}.yaml", sub.id)));
        let mut hc = serde_json::Map::new();
        hc.insert("enable".into(), Value::Bool(true));
        hc.insert(
            "url".into(),
            Value::String("https://www.gstatic.com/generate_204".into()),
        );
        hc.insert("interval".into(), Value::Number(300.into()));
        p.insert("health-check".into(), Value::Object(hc));
        if let Some(age) = &sub.age_secret {
            p.insert("age-secret".into(), Value::String(age.clone()));
        }
        providers.insert(sub.id.clone(), Value::Object(p));
    }
    for rp in &item.rule_providers {
        let mut p = serde_json::Map::new();
        p.insert("type".into(), Value::String("http".into()));
        p.insert("behavior".into(), Value::String(rp.behavior.clone()));
        p.insert("url".into(), Value::String(rp.url.clone()));
        p.insert(
            "interval".into(),
            Value::Number((rp.interval.max(60) as u64).into()),
        );
        p.insert("path".into(), Value::String(format!("{}.yaml", rp.id)));
        if let Some(age) = &rp.age_secret {
            p.insert("age-secret".into(), Value::String(age.clone()));
        }
        providers.insert(rp.id.clone(), Value::Object(p));
    }
    if !providers.is_empty() {
        root.insert("proxy-providers".into(), Value::Object(providers));
    }

    // rules
    if !item.custom_rules.is_empty() {
        root.insert(
            "rules".into(),
            Value::Array(
                item.custom_rules
                    .iter()
                    .map(|r| Value::String(r.clone()))
                    .collect(),
            ),
        );
    }

    // dns：未配置 nameserver 时不注入，避免生成空 dns 块导致核心报错
    if item.dns_enabled && !item.dns_nameservers.is_empty() {
        let mut dns = serde_json::Map::new();
        dns.insert("enable".into(), Value::Bool(true));
        dns.insert("ipv6".into(), Value::Bool(false));
        dns.insert("enhanced-mode".into(), Value::String("fake-ip".into()));
        dns.insert("fake-ip-range".into(), Value::String("198.18.0.1/16".into()));
        dns.insert(
            "nameserver".into(),
            Value::Array(
                item.dns_nameservers
                    .iter()
                    .map(|n| Value::String(n.clone()))
                    .collect(),
            ),
        );
        root.insert("dns".into(), Value::Object(dns));
    }

    Value::Object(root)
}

fn parse_yaml_to_json(s: &str) -> Result<Value, String> {
    serde_yaml::from_str::<Value>(s).map_err(|e| format!("yaml 解析失败: {e}"))
}

fn yaml_value_to_string(v: &Value) -> Result<String, String> {
    serde_yaml::to_string(v).map_err(|e| format!("yaml 序列化失败: {e}"))
}

/// JS 覆盖脚本的执行日志文件（对齐 clash-party 的 overrideLog）
pub fn override_log_path(data_dir: &Path, id: &str) -> std::path::PathBuf {
    data_dir.join("override").join(format!("{}.log", id))
}

fn write_override_log(data_dir: &Path, id: &str, lines: &str) {
    let p = override_log_path(data_dir, id);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    std::fs::write(p, lines).ok();
}

/// 运行 JS 覆盖脚本（需要系统 node）。main(profile) 返回新配置。
/// console.* 输出会被收集到 stderr 并写入 override/<id>.log 供前端查看执行日志。
fn apply_js_override(
    base: Value,
    script: &str,
    data_dir: &Path,
    id: &str,
) -> Result<Value, String> {
    let harness = format!(
        concat!(
            "const __logs = [];\n",
            "const __fmt = (a) => a.map(x => typeof x === 'string' ? x : (()=>{{ try {{ return JSON.stringify(x); }} catch (e) {{ return String(x); }} }})()).join(' ');\n",
            "for (const k of ['log','info','warn','error','debug']) {{ console[k] = (...a) => __logs.push('[' + k + '] ' + __fmt(a)); }}\n",
            "const __flush = () => {{ try {{ process.stderr.write(__logs.join('\\n')); }} catch (e) {{}} }};\n",
            "const profile = {};\n",
            "try {{\n{}\n",
            "if (typeof main !== 'function') {{ __logs.push('[error] override: no main function'); __flush(); process.exit(2); }}\n",
            "const out = main(profile);\n",
            "__flush();\n",
            "process.stdout.write(JSON.stringify(out));\n",
            "}} catch (e) {{ __logs.push('[error] ' + (e && e.stack ? e.stack : String(e))); __flush(); process.exit(3); }}\n"
        ),
        serde_json::to_string(&base).unwrap_or_default(),
        script
    );
    // F4 修复：净化 id 防路径穿越（../ 或 \\ 等）
    let safe_id = id.replace(['/', '\\', '.'], "_");
    let tmp = std::env::temp_dir().join(format!("mihomo_override_{}_{}.js", std::process::id(), safe_id));
    std::fs::write(&tmp, harness).map_err(|e| format!("写入临时脚本失败: {e}"))?;
    let out = hidden_cmd("node")
        .arg(&tmp)
        .output()
        .map_err(|e| format!("执行 node 失败（需安装 node）: {e}"));
    let _ = std::fs::remove_file(&tmp);
    let out = out?;
    let logs = String::from_utf8_lossy(&out.stderr).to_string();
    write_override_log(data_dir, id, &logs);
    if !out.status.success() {
        return Err(format!("node 执行覆盖脚本失败: {}", logs));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("覆盖脚本输出非 JSON: {e}"))
}

/// 处理带偏移量的规则（对齐 clash-party processRulesWithOffset）
/// 规则形如 `3,DOMAIN,example.com,DIRECT` 时，首段数字表示插入位置偏移
fn process_rules_with_offset(
    rule_strings: &[String],
    current_rules: &[String],
    is_append: bool,
) -> (Vec<String>, Vec<String>) {
    let mut normal_rules: Vec<String> = vec![];
    let mut rules: Vec<String> = current_rules.to_vec();

    for rule_str in rule_strings {
        let parts: Vec<&str> = rule_str.split(',').collect();
        let first_is_number = parts.len() >= 3
            && !parts[0].trim().is_empty()
            && parts[0].trim().parse::<i64>().is_ok();
        if first_is_number {
            let offset = parts[0].trim().parse::<i64>().unwrap_or(0).max(0) as usize;
            let rule = parts[1..].join(",");
            let pos = if is_append {
                rules.len().saturating_sub(offset.min(rules.len()))
            } else {
                offset.min(rules.len())
            };
            rules.insert(pos, rule);
        } else {
            normal_rules.push(rule_str.clone());
        }
    }
    (normal_rules, rules)
}

/// 应用规则覆写文件 rules/<profileId>.yaml（prepend / append / delete）
fn apply_rule_override(data_dir: &Path, current_id: &str, base: &mut Value) {
    let p = rule_override_path(data_dir, current_id);
    if !p.exists() {
        return;
    }
    let content = match std::fs::read_to_string(&p) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[mihomo] 读取规则覆写失败: {e}");
            return;
        }
    };
    let data: Value = match parse_yaml_to_json(&content) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[mihomo] 解析规则覆写失败: {e}");
            return;
        }
    };
    let Some(map) = data.as_object() else { return };

    let as_str_vec = |v: Option<&Value>| -> Vec<String> {
        v.and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .map(|r| match r {
                        Value::String(s) => s.clone(),
                        Value::Array(items) => items
                            .iter()
                            .map(|i| i.as_str().unwrap_or_default().to_string())
                            .collect::<Vec<_>>()
                            .join(","),
                        other => other.to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let obj = match base.as_object_mut() {
        Some(o) => o,
        None => return,
    };
    let mut rules: Vec<String> = obj
        .get("rules")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .map(|r| match r {
                    Value::String(s) => s.clone(),
                    Value::Array(items) => items
                        .iter()
                        .map(|i| i.as_str().unwrap_or_default().to_string())
                        .collect::<Vec<_>>()
                        .join(","),
                    other => other.to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    let prepend = as_str_vec(map.get("prepend"));
    if !prepend.is_empty() {
        let (normal, inserted) = process_rules_with_offset(&prepend, &rules, false);
        rules = normal.into_iter().chain(inserted).collect();
    }
    let append = as_str_vec(map.get("append"));
    if !append.is_empty() {
        let (normal, inserted) = process_rules_with_offset(&append, &rules, true);
        rules = inserted.into_iter().chain(normal).collect();
    }
    let del = as_str_vec(map.get("delete"));
    if !del.is_empty() {
        rules.retain(|r| !del.contains(r));
    }
    obj.insert(
        "rules".into(),
        Value::Array(rules.into_iter().map(Value::String).collect()),
    );
}

/// 计算覆写执行顺序：全局覆写在前，profile 自身覆写在后，去重；Smart 覆写单独最后执行
fn ordered_override_ids(profile: &ProfileItem, overrides: &OverrideConfig) -> (Vec<String>, Vec<String>) {
    let mut ordered: Vec<String> = vec![];
    for it in overrides.items.iter().filter(|i| i.global) {
        if !ordered.contains(&it.id) {
            ordered.push(it.id.clone());
        }
    }
    for id in &profile.override_ids {
        if !ordered.contains(id) {
            ordered.push(id.clone());
        }
    }
    let normal = ordered
        .iter()
        .filter(|id| id.as_str() != SMART_OVERRIDE_ID)
        .cloned()
        .collect();
    let smart = ordered
        .iter()
        .filter(|id| id.as_str() == SMART_OVERRIDE_ID)
        .cloned()
        .collect();
    (normal, smart)
}

/// 依次应用指定 id 的覆写
fn apply_overrides(
    mut base: Value,
    ids: &[String],
    overrides: &OverrideConfig,
    data_dir: &Path,
) -> Value {
    for id in ids {
        let Some(ov) = overrides.items.iter().find(|o| &o.id == id) else {
            continue;
        };
        let content = match read_override_content(data_dir, ov) {
            Some(c) => c,
            None => continue,
        };
        match ov.ext.as_str() {
            "js" => match apply_js_override(base.clone(), &content, data_dir, &ov.id) {
                Ok(v) => base = v,
                Err(e) => eprintln!("[mihomo] JS 覆盖跳过: {e}"),
            },
            _ => match parse_yaml_to_json(&content) {
                Ok(v) if v.is_object() => deep_merge_with(&mut base, &v, true),
                _ => {}
            },
        }
    }
    base
}

/// 启用 Smart 覆写且开启 TUN 时，把代理服务器 IP 加入 route-exclude-address 防止回环
fn ensure_smart_tun_exclude(profile: &mut Value, enabled: bool) -> Vec<String> {
    if !enabled {
        return vec![];
    }
    let tun_enabled = profile
        .get("tun")
        .and_then(|t| t.get("enable"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !tun_enabled {
        return vec![];
    }
    let servers: Vec<String> = profile
        .get("proxies")
        .and_then(|p| p.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|p| p.get("server"))
                .map(|s| match s {
                    Value::String(v) => v.clone(),
                    other => other.to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    if servers.is_empty() {
        return vec![];
    }

    let tun = profile
        .as_object_mut()
        .and_then(|o| o.get_mut("tun"))
        .and_then(|t| t.as_object_mut());
    let Some(tun) = tun else { return vec![] };
    let mut list: Vec<String> = tun
        .get("route-exclude-address")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    let mut existing: std::collections::HashSet<String> =
        list.iter().map(|s| s.trim().to_lowercase()).collect();
    let mut added = vec![];
    for server in servers {
        let host = server
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_lowercase();
        let cidr = match host.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V4(_)) => format!("{host}/32"),
            Ok(std::net::IpAddr::V6(_)) => format!("{host}/128"),
            Err(_) => continue,
        };
        if existing.contains(&host) || existing.contains(&cidr) {
            continue;
        }
        existing.insert(cidr.clone());
        list.push(cidr.clone());
        added.push(cidr);
    }
    tun.insert(
        "route-exclude-address".into(),
        Value::Array(list.into_iter().map(Value::String).collect()),
    );
    added
}

/// 独立工作目录模式：把 geo 数据文件复制到 profile 专属工作目录
pub fn prepare_profile_work_dir(data_dir: &Path, current: &str) -> std::io::Result<()> {
    let target_dir = profile_work_dir(data_dir, current);
    std::fs::create_dir_all(&target_dir)?;
    for file in [
        "country.mmdb",
        "geoip.metadb",
        "geoip.dat",
        "geosite.dat",
        "ASN.mmdb",
    ] {
        let src = data_dir.join(file);
        if !src.exists() {
            continue;
        }
        let dst = target_dir.join(file);
        let should_copy = match (std::fs::metadata(&src), std::fs::metadata(&dst)) {
            (Ok(s), Ok(d)) => match (s.modified(), d.modified()) {
                (Ok(sm), Ok(dm)) => sm > dm,
                _ => true,
            },
            (Ok(_), Err(_)) => true,
            _ => false,
        };
        if should_copy {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// 生成结果：runtime 用于前端预览/备份（保留用户 log-level），core 用于实际写入内核配置
pub struct GeneratedConfig {
    pub runtime: String,
    pub core: String,
}

/// 生成运行时配置字符串
pub fn generate_runtime_config(
    app: &AppConfig,
    controled: &Value,
    profile: &ProfileItem,
    overrides: &OverrideConfig,
    data_dir: &Path,
) -> Result<GeneratedConfig, String> {
    // 1. base
    let mut base = if profile.is_file() {
        match read_profile_content(data_dir, profile) {
            Some(s) => parse_yaml_to_json(&s)?,
            None => Value::Object(Default::default()),
        }
    } else {
        build_subscription_profile(profile)
    };
    if !base.is_object() {
        base = Value::Object(Default::default());
    }

    // 2. overrides：全局覆写优先、profile 覆写其次；Smart 覆写在规则覆写之后单独执行
    let (normal_ids, smart_ids) = ordered_override_ids(profile, overrides);
    base = apply_overrides(base, &normal_ids, overrides, data_dir);
    apply_rule_override(data_dir, &profile.id, &mut base);
    base = apply_overrides(base, &smart_ids, overrides, data_dir);
    if !base.is_object() {
        base = Value::Object(Default::default());
    }

    // 3. controled config (受 control_dns/control_sniff/use_nameserver_policy 开关约束)
    let mut controled = controled.clone();
    if let Value::Object(ref mut m) = controled {
        if !app.control_dns {
            m.remove("dns");
            m.remove("hosts");
        }
        if !app.control_sniff {
            m.remove("sniffer");
        }
        if !app.use_nameserver_policy {
            if let Some(Value::Object(dns)) = m.get_mut("dns") {
                dns.remove("nameserver-policy");
            }
        }
    }
    deep_merge(&mut base, &controled);

    // 3.1 关闭 DNS 覆写且最终未启用 DNS 时，清空 dns-hijack，避免劫持后无法处理
    let dns_enabled = base
        .get("dns")
        .and_then(|d| d.get("enable"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !app.control_dns && base.get("tun").is_some() && !dns_enabled {
        if let Some(tun) = base.get_mut("tun").and_then(|t| t.as_object_mut()) {
            tun.insert("dns-hijack".into(), Value::Array(vec![]));
        }
    }

    // 4. 基础运行参数（app config 强制覆盖）
    let obj = base.as_object_mut().unwrap();
    // mixed-port / 外部控制器由 app config 掌管（与托盘、API 探测保持一致）
    obj.insert(
        "mixed-port".into(),
        Value::Number((app.mixed_port as u64).into()),
    );
    obj.insert(
        "external-controller".into(),
        Value::String(format!("127.0.0.1:{}", app.controller_port)),
    );
    if app.secret.is_empty() {
        obj.remove("secret");
    } else {
        obj.insert("secret".into(), Value::String(app.secret.clone()));
    }
    // 其余运行参数以 controled config 为准，缺省时才补默认值
    for (k, v) in [
        ("allow-lan", Value::Bool(false)),
        ("ipv6", Value::Bool(false)),
        ("mode", Value::String("rule".into())),
        ("log-level", Value::String("info".into())),
    ] {
        obj.entry(k.to_string()).or_insert(v);
    }

    // 4.1 二级代理：把配置的家庭 socks5 列表注入为 proxies 节点，
    //     每个命名为「二级-<name>」，并用一个 Selector 策略组「二级代理」收纳全部，
    //     使其在代理页可见、可被规则选中、受全局/规则/直连模式控制。
    //     启用的那个二级代理加上 dialer-proxy 指向一级代理（代理页选中的 default_proxy），
    //     实现「请求 → 一级代理 → 二级代理 → 目标」的链式。
    {
        let mut node_names: Vec<String> = vec![];
        // 收集配置中已有的节点名 + 策略组名，用于校验 dialer-proxy 引用是否存在，
        // 避免 default_proxy（一级代理）指向已不存在的节点名导致 mihomo 配置校验失败。
        let mut known_proxy_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        if let Some(Value::Array(ps)) = obj.get("proxies") {
            for p in ps {
                if let Some(n) = p.get("name").and_then(|v| v.as_str()) {
                    known_proxy_names.insert(n.to_string());
                }
            }
        }
        if let Some(Value::Array(gs)) = obj.get("proxy-groups") {
            for g in gs {
                if let Some(n) = g.get("name").and_then(|v| v.as_str()) {
                    known_proxy_names.insert(n.to_string());
                }
            }
        }
        for s5 in &app.secondary_proxies {
            if s5.host.trim().is_empty() || s5.port == 0 {
                continue;
            }
            let node_name = secondary_node_name(s5);
            node_names.push(node_name.clone());
            let mut node = serde_json::Map::new();
            node.insert("name".into(), Value::String(node_name.clone()));
            node.insert("type".into(), Value::String("socks5".into()));
            node.insert("server".into(), Value::String(s5.host.trim().to_string()));
            node.insert("port".into(), Value::Number((s5.port as u64).into()));
            if let Some(u) = &s5.username {
                if !u.is_empty() {
                    node.insert("username".into(), Value::String(u.clone()));
                }
            }
            if let Some(p) = &s5.password {
                if !p.is_empty() {
                    node.insert("password".into(), Value::String(p.clone()));
                }
            }
            // 仅启用的二级代理套一级代理（dialer-proxy）
            let is_active = app.secondary_active_id.as_deref() == Some(s5.id.as_str());
            if is_active {
                if let Some(dp) = &app.default_proxy {
                    if !dp.trim().is_empty()
                        && known_proxy_names.contains(dp.trim())
                    {
                        node.insert("dialer-proxy".into(), Value::String(dp.trim().to_string()));
                    }
                }
            }
            let proxies = obj
                .entry("proxies".to_string())
                .or_insert_with(|| Value::Array(vec![]));
            let node_val = Value::Object(node);
            if let Value::Array(arr) = proxies {
                if let Some(pos) = arr.iter().position(|p| {
                    p.get("name").and_then(|n| n.as_str()) == Some(node_name.as_str())
                }) {
                    arr[pos] = node_val;
                } else {
                    arr.insert(0, node_val);
                }
            }
        }

        // 移除已删除的二级代理残留节点（幂等清理）
        if let Some(Value::Array(arr)) = obj.get_mut("proxies") {
            arr.retain(|p| {
                p.get("name")
                    .and_then(|n| n.as_str())
                    .map(|n| {
                        !(n.starts_with(SECONDARY_PREFIX) && !node_names.iter().any(|x| x == n))
                    })
                    .unwrap_or(true)
            });
        }

        // 注入「二级代理」Selector 策略组（幂等更新，含全部二级节点）。
        // 注意：无任何二级代理节点时**不注入**空组（mihomo 会因 proxies 为空而报错），
        // 并移除历史残留的空组，避免配置校验失败。
        let groups = obj
            .entry("proxy-groups".to_string())
            .or_insert_with(|| Value::Array(vec![]));
        if node_names.is_empty() {
            if let Value::Array(garr) = groups {
                garr.retain(|g| g.get("name").and_then(|n| n.as_str()) != Some(SECONDARY_GROUP));
            }
        } else {
            let group_val = serde_json::json!({
                "name": SECONDARY_GROUP,
                "type": "select",
                "proxies": node_names,
            });
            if let Value::Array(garr) = groups {
                if let Some(pos) = garr.iter().position(|g| {
                    g.get("name").and_then(|n| n.as_str()) == Some(SECONDARY_GROUP)
                }) {
                    garr[pos] = group_val;
                } else {
                    garr.push(group_val);
                }
            }
        }
    }

    // 5. TUN：若 base 中已存在启用的 tun 块（来自 controled/config 的 tun.enable=true），
    //    则强制写入网卡名称（wintun 适配器名在设备创建时确定，故每次构建都需覆盖 device）。
    //    注意：此处以 base 实际是否启用 TUN 为准，而非 app.tun_enabled——
    //    UI 的 TUN 开关只 patch controled 的 tun.enable，app.tun_enabled 未必同步，
    //    若以 app.tun_enabled 为守卫会导致自定义的 tun_name 永远不生效。
    let tun_enabled = obj
        .get("tun")
        .and_then(|t| t.get("enable"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if tun_enabled {
        if let Some(Value::Object(tun)) = obj.get_mut("tun") {
            tun.insert("device".into(), Value::String(app.tun_name.clone()));
        }
    }

    // 6. 注入订阅用量信息（subscription-userinfo），使核心 /subscriptions 能反映余额
    if let Some(info) = &profile.subscription_userinfo {
        let s = format!(
            "upload={}; download={}; total={}; expire={}",
            info.upload, info.download, info.total, info.expire
        );
        obj.insert("subscription-userinfo".into(), Value::String(s));
    }

    // 7. Smart 覆写：把代理服务器 IP 排除出 TUN 路由，避免回环
    let added = ensure_smart_tun_exclude(&mut base, !smart_ids.is_empty());
    if !added.is_empty() {
        eprintln!("[mihomo] Smart 覆写已排除代理服务器路由: {added:?}");
    }

    // 8. 清理空字段：空的局域网白名单会导致局域网访问异常
    if let Some(obj) = base.as_object_mut() {
        let lan_empty = obj
            .get("lan-allowed-ips")
            .map(|v| v.as_array().map(|a| a.is_empty()).unwrap_or(true))
            .unwrap_or(false);
        if lan_empty {
            obj.remove("lan-allowed-ips");
        }
        // 外部控制器关闭时，WebUI 相关字段一并移除
        if obj.get("external-controller").and_then(|v| v.as_str()) == Some("") {
            obj.remove("external-controller");
            obj.remove("external-ui");
            obj.remove("external-ui-url");
            obj.remove("external-controller-cors");
        } else if obj.get("external-ui").and_then(|v| v.as_str()) == Some("") {
            obj.remove("external-ui");
            obj.remove("external-ui-url");
        }
    }

    // 9. 预览配置保留用户 log-level；内核配置强制 info/debug 以便启动检测解析日志
    let runtime = yaml_value_to_string(&base)?;
    let mut core_value = base.clone();
    if let Some(obj) = core_value.as_object_mut() {
        let lv = obj.get("log-level").and_then(|v| v.as_str()).unwrap_or("");
        if lv != "info" && lv != "debug" {
            obj.insert("log-level".into(), Value::String("info".into()));
        }
    }
    let core = yaml_value_to_string(&core_value)?;

    // 10. 独立工作目录模式：准备 geo 数据
    if app.diff_work_dir {
        if let Err(e) = prepare_profile_work_dir(data_dir, &profile.id) {
            eprintln!("[mihomo] 准备独立工作目录失败: {e}");
        }
    }

    Ok(GeneratedConfig { runtime, core })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::mihomo::config::SecondaryProxy;

    /// 构造一个含「德国」节点的本地 file profile（模拟真实订阅里的一级代理节点存在）
    fn file_profile_with_de() -> ProfileItem {
        let mut p = ProfileItem::default();
        p.id = "t".into();
        p.type_ = "file".into();
        let yaml = r#"
proxies:
  - name: 德国
    type: ss
    server: de.example.com
    port: 8388
    cipher: aes-128-gcm
    password: test
proxy-groups:
  - name: Proxy
    type: select
    proxies: [德国]
"#;
        let dir = std::env::temp_dir().join("mihomo_factory_test");
        std::fs::create_dir_all(&dir).unwrap();
        let fp = dir.join("de.yaml");
        std::fs::write(&fp, yaml).unwrap();
        p.file_path = Some(fp.to_string_lossy().to_string());
        p
    }

    fn gen_with(profile: ProfileItem, secondary: Vec<SecondaryProxy>, active_id: Option<&str>) -> Value {
        let mut app = AppConfig::default();
        app.secondary_proxies = secondary;
        app.secondary_active_id = active_id.map(|s| s.to_string());
        app.default_proxy = Some("德国".into());
        let overrides = OverrideConfig::default();
        let data_dir = std::env::temp_dir().join("mihomo_factory_test");
        let out = generate_runtime_config(&app, &Value::Object(Default::default()), &profile, &overrides, &data_dir).unwrap();
        serde_yaml::from_str::<Value>(&out.core).unwrap()
    }

    #[test]
    fn empty_secondary_proxies_injects_no_empty_group() {
        let cfg = gen_with(file_profile_with_de(), vec![], None);
        let groups = cfg.get("proxy-groups").and_then(|g| g.as_array());
        assert!(
            groups.map_or(true, |gs| !gs.iter().any(|g| g.get("name").and_then(|n| n.as_str()) == Some(SECONDARY_GROUP))),
            "空列表不应注入空的「二级代理」组"
        );
        let proxies = cfg.get("proxies").and_then(|p| p.as_array()).cloned().unwrap_or_default();
        assert!(!proxies.iter().any(|p| p.get("name").and_then(|n| n.as_str()).map_or(false, |n| n.starts_with(SECONDARY_PREFIX))));
    }

    #[test]
    fn with_secondary_proxy_and_existing_upstream_injects_dialer_proxy() {
        let mut s5 = SecondaryProxy::default();
        s5.id = "s1".into();
        s5.name = "美国1号".into();
        s5.host = "1.2.3.4".into();
        s5.port = 1080;
        let cfg = gen_with(file_profile_with_de(), vec![s5], Some("s1"));
        // 节点已注入
        let node_name = format!("{SECONDARY_PREFIX}美国1号");
        let proxies = cfg.get("proxies").and_then(|p| p.as_array()).cloned().unwrap_or_default();
        let node = proxies.iter().find(|p| p.get("name").and_then(|n| n.as_str()) == Some(node_name.as_str())).expect("二级节点应被注入");
        assert_eq!(node.get("type").and_then(|v| v.as_str()), Some("socks5"));
        // 一级代理「德国」在配置中存在 → 启用节点带 dialer-proxy = 德国
        assert_eq!(node.get("dialer-proxy").and_then(|v| v.as_str()), Some("德国"));
        // 组已注入
        let groups = cfg.get("proxy-groups").and_then(|g| g.as_array()).cloned().unwrap_or_default();
        let group = groups.iter().find(|g| g.get("name").and_then(|n| n.as_str()) == Some(SECONDARY_GROUP)).expect("应注入「二级代理」组");
        let members = group.get("proxies").and_then(|m| m.as_array()).map(|a| a.len()).unwrap_or(0);
        assert_eq!(members, 1);
    }

    #[test]
    fn with_secondary_proxy_but_missing_upstream_degrades_to_direct() {
        // 一级代理「德国」在空订阅里不存在 → 不注入 dialer-proxy（降级直连），避免配置校验失败
        let mut s5 = SecondaryProxy::default();
        s5.id = "s1".into();
        s5.name = "美国1号".into();
        s5.host = "1.2.3.4".into();
        s5.port = 1080;
        let profile = ProfileItem {
            id: "t".into(),
            type_: "subscription".into(),
            ..Default::default()
        };
        let cfg = gen_with(profile, vec![s5], Some("s1"));
        let node_name = format!("{SECONDARY_PREFIX}美国1号");
        let proxies = cfg.get("proxies").and_then(|p| p.as_array()).cloned().unwrap_or_default();
        let node = proxies.iter().find(|p| p.get("name").and_then(|n| n.as_str()) == Some(node_name.as_str())).expect("二级节点应被注入");
        assert!(node.get("dialer-proxy").is_none(), "一级节点不存在时应降级为直连，不注入 dialer-proxy");
    }
}
