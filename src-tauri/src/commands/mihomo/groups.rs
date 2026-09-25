//! 策略组编辑 + 简易模式配置编译器
//!
//! 借鉴 clash-party：它在 `main/simple/*` 里用「可视化编译器」把订阅 + 策略组 + 规则
//! 拼成一份配置，在 `components/proxies/group-editor-modal.tsx` 里编辑组成员与
//! URLTest 参数。这里做成**覆写（override）**而不是改订阅文件：
//! 订阅下次更新会整份覆盖 `proxy-groups`，直接改订阅内容等于白改；
//! 覆写是叠加层，订阅更新后依然生效，删掉覆写即回到订阅自带的组。

use serde_json::Value;
use std::path::Path;

/// 自定义策略组覆写的固定 id（不存在时按需创建）
pub const GROUPS_OVERRIDE_ID: &str = "custom_proxy_groups";
/// 简易模式的固定 id（规则/全局模式的 `rules` 部分）
pub const SIMPLE_OVERRIDE_ID: &str = "simple_mode_rules";

use super::config::{OverrideItem, ProfileItem};

/// 从 YAML 文本里取出 `proxy-groups`（缺失返回空数组）。
fn yaml_groups(content: &str) -> Result<Vec<Value>, String> {
    let v: serde_yaml::Value =
        serde_yaml::from_str(content).map_err(|e| format!("配置不是合法 YAML: {e}"))?;
    let Some(arr) = v.get("proxy-groups").and_then(|x| x.as_sequence()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in arr {
        if let Ok(j) = serde_json::to_value(item) {
            out.push(j);
        }
    }
    Ok(out)
}

/// 收集可作为成员的名字：配置文件里的 `proxies`、策略组名、`proxy-providers` 的键，
/// 加上内核固定的 DIRECT / REJECT。
fn collect_node_names(content: &str) -> Vec<String> {
    let mut nodes: Vec<String> = vec!["DIRECT".into(), "REJECT".into()];
    let Ok(v) = serde_yaml::from_str::<serde_yaml::Value>(content) else {
        return nodes;
    };
    if let Some(arr) = v.get("proxies").and_then(|x| x.as_sequence()) {
        for p in arr {
            if let Some(n) = p.get("name").and_then(|x| x.as_str()) {
                nodes.push(n.to_string());
            }
        }
    }
    if let Some(arr) = v.get("proxy-groups").and_then(|x| x.as_sequence()) {
        for g in arr {
            if let Some(n) = g.get("name").and_then(|x| x.as_str()) {
                nodes.push(n.to_string());
            }
        }
    }
    if let Some(map) = v.get("proxy-providers").and_then(|x| x.as_mapping()) {
        for (k, _) in map {
            if let Some(n) = k.as_str() {
                nodes.push(n.to_string());
            }
        }
    }
    nodes.sort();
    nodes.dedup();
    nodes
}

/// 读当前配置的原始 YAML（覆写优先：生效的就是它）。
pub(crate) fn editor_payload(
    data_dir: &Path,
    item: &ProfileItem,
    overrides: &[OverrideItem],
) -> Result<Value, String> {
    let base = super::config::read_profile_content(data_dir, item)
        .unwrap_or_default();
    let mut groups = yaml_groups(&base)?;
    // 已存在自定义组覆写 → 以覆写为准（它才是生效的那份）
    if let Some(ov) = overrides.iter().find(|o| o.id == GROUPS_OVERRIDE_ID) {
        if let Some(text) = super::config::read_override_content(data_dir, ov) {
            let from_ov = yaml_groups(&text)?;
            if !from_ov.is_empty() {
                groups = from_ov;
            }
        }
    }
    let nodes = collect_node_names(&base);
    Ok(serde_json::json!({
        "groups": groups,
        "nodes": nodes,
        "overrideId": GROUPS_OVERRIDE_ID,
        "hasOverride": overrides.iter().any(|o| o.id == GROUPS_OVERRIDE_ID),
    }))
}

/// 校验策略组数组：名字非空且唯一、type 合法、成员是数组。
fn validate_groups(groups: &[Value]) -> Result<(), String> {
    // 归一化后比较：内核写法是 `url-test` / `URLTest` / `load-balance` 都见过，
    // 全按「小写去连字符」判定，免得用户照抄内核写法被判非法。
    let allowed = ["select", "urltest", "fallback", "loadbalance", "relay"];
    let mut seen: Vec<&str> = Vec::new();
    for g in groups {
        let name = g.get("name").and_then(|x| x.as_str()).unwrap_or("").trim();
        if name.is_empty() {
            return Err("策略组名不能为空".into());
        }
        if seen.contains(&name) {
            return Err(format!("策略组名重复: {name}"));
        }
        seen.push(name);
        let t = g
            .get("type")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_lowercase()
            .replace(['-', '_'], "");
        if !allowed.contains(&t.as_str()) {
            return Err(format!("不支持的策略组类型: {}（可选 {allowed:?}）", g.get("type").and_then(|x| x.as_str()).unwrap_or("")));
        }
        match g.get("proxies") {
            Some(Value::Array(_)) => {}
            _ => return Err(format!("策略组「{name}」缺少成员列表 proxies")),
        }
    }
    Ok(())
}

/// 把策略组数组渲染成覆写 YAML。
pub fn groups_to_yaml(groups: &[Value]) -> Result<String, String> {
    validate_groups(groups)?;
    let root = serde_json::json!({ "proxy-groups": groups });
    let yaml: serde_yaml::Value = serde_yaml::to_value(&root).map_err(|e| e.to_string())?;
    serde_yaml::to_string(&yaml).map_err(|e| format!("生成 YAML 失败: {e}"))
}

/// 简易模式编译器：按「出站模式」生成规则覆写。
///
/// 只生成 `rules` 一段（策略组另存于策略组覆写），两者互不干扰：
/// - `rule`：常规分流（大陆直连 + 其余走自动选择）
/// - `bypassCN`：绕过大陆（大陆站点直连）
/// - `global`：全局走手动切换（GLOBAL 语义由 MATCH 兜底实现）
pub fn simple_rules_yaml(mode: &str, final_group: &str) -> Result<String, String> {
    let rules: Vec<String> = match mode {
        "global" => vec![format!("MATCH,{}", final_group)],
        "bypassCN" => vec![
            "GEOSITE,cn,DIRECT".into(),
            "GEOIP,CN,DIRECT".into(),
            format!("MATCH,{}", final_group),
        ],
        _ => vec![
            "GEOIP,CN,DIRECT".into(),
            "GEOSITE,category-scholar-!cn,自动选择".into(),
            format!("MATCH,{}", final_group),
        ],
    };
    let root = serde_json::json!({ "rules": rules });
    let yaml: serde_yaml::Value = serde_yaml::to_value(&root).map_err(|e| e.to_string())?;
    serde_yaml::to_string(&yaml).map_err(|e| format!("生成 YAML 失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::{groups_to_yaml, simple_rules_yaml, validate_groups, yaml_groups};
    use serde_json::json;

    #[test]
    fn parses_groups_from_yaml() {
        let yaml = "proxy-groups:\n  - name: 手动切换\n    type: select\n    proxies: [A, B]\n";
        let g = yaml_groups(yaml).unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0]["name"], "手动切换");
        assert_eq!(yaml_groups("proxies: []\n").unwrap().len(), 0);
    }

    #[test]
    fn rejects_invalid_groups() {
        // 空名 / 重名 / 未知类型 / 缺成员都要当场报错，不能写进配置让内核起不来
        let bad = vec![json!({ "name": "", "type": "select", "proxies": [] })];
        assert!(validate_groups(&bad).is_err());
        let dup = vec![
            json!({ "name": "G", "type": "select", "proxies": [] }),
            json!({ "name": "G", "type": "select", "proxies": [] }),
        ];
        assert!(validate_groups(&dup).is_err());
        let unknown = vec![json!({ "name": "G", "type": "magic", "proxies": [] })];
        assert!(validate_groups(&unknown).is_err());
        let no_members = vec![json!({ "name": "G", "type": "select" })];
        assert!(validate_groups(&no_members).is_err());
        let ok = vec![json!({ "name": "G", "type": "url-test", "proxies": ["A"] })];
        assert!(validate_groups(&ok).is_ok());
    }

    #[test]
    fn generates_override_yaml() {
        let yaml = groups_to_yaml(&[json!({
            "name": "自动选择", "type": "url-test",
            "url": "http://www.gstatic.com/generate_204", "interval": 300,
            "proxies": ["A", "B"],
        })])
        .unwrap();
        assert!(yaml.contains("proxy-groups:"), "{yaml}");
        assert!(yaml.contains("自动选择"));
        assert!(yaml.contains("url-test"));
    }

    #[test]
    fn simple_mode_rules_end_with_match() {
        for mode in ["rule", "global", "bypassCN"] {
            let yaml = simple_rules_yaml(mode, "手动切换").unwrap();
            assert!(yaml.contains("rules:"), "{yaml}");
            assert!(yaml.contains("MATCH,手动切换"), "{mode}: {yaml}");
        }
        // 全局模式就是一条 MATCH，不做任何分流
        let g = simple_rules_yaml("global", "手动切换").unwrap();
        assert!(!g.contains("GEOIP"), "{g}");
    }
}
