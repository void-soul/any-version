use super::*;

// 兼容性迁移：旧 mihomo.json (MihomoSettings) -> 新模型
pub(super) fn migrate_legacy(dir: &Path, app: &mut AppConfig, profiles: &mut ProfileConfig) {
    let old = dir.join("mihomo.json");
    if !old.exists() {
        return;
    }
    if let Ok(s) = std::fs::read_to_string(&old) {
        if let Ok(v) = serde_json::from_str::<Value>(&s) {
            if let Some(cp) = get_u64(&v, "controller_port") {
                app.controller_port = cp as u16;
            }
            if let Some(mp) = get_u64(&v, "mixed_port") {
                app.mixed_port = mp as u16;
            }
            if let Some(secret) = get_str(&v, "secret") {
                app.secret = secret;
            }
            if let Some(tun) = v.get("tun_enabled").and_then(|x| x.as_bool()) {
                app.tun_enabled = tun;
            }
            if let Some(cp) = get_str(&v, "core_path") {
                if !cp.is_empty() {
                    app.core_path = Some(cp);
                }
            }
            if let Some(dp) = get_str(&v, "default_proxy") {
                app.default_proxy = Some(dp);
            }
            let mut item = ProfileItem {
                id: "default".into(),
                name: "默认".into(),
                type_: "subscription".into(),
                file_path: None,
                subscriptions: vec![],
                rule_providers: vec![],
                custom_rules: vec![],
                dns_enabled: true,
                dns_nameservers: vec![],
                age_secret_key: None,
                url: None,
                auth_token: None,
                user_agent: None,
                use_proxy: false,
                auto_update: false,
                skip_verify: false,
                update_interval: 86400,
                update_cron: None,
                update_timeout: 30,
                override_ids: vec![],
                subscription_userinfo: None,
                updated_at: None,
            };
            if let Some(arr) = v.get("subscriptions").and_then(|x| x.as_array()) {
                for s in arr {
                    item.subscriptions.push(Subscription {
                        id: uid(),
                        name: get_str(s, "name").unwrap_or_else(|| "订阅".into()),
                        url: get_str(s, "url").unwrap_or_default(),
                        interval: get_u64(s, "interval").unwrap_or(86400),
                        cron: get_str_opt(s, "cron"),
                        auto_update: s
                            .get("auto_update")
                            .and_then(|x| x.as_bool())
                            .unwrap_or(false),
                        updated_at: get_u64(s, "updated_at"),
                        age_secret: get_str_opt(s, "age_secret"),
                    });
                }
            }
            if let Some(arr) = v.get("rule_providers").and_then(|x| x.as_array()) {
                for s in arr {
                    item.rule_providers.push(RuleProviderEntry {
                        id: uid(),
                        name: get_str(s, "name").unwrap_or_default(),
                        behavior: get_str(s, "behavior").unwrap_or_else(|| "classical".into()),
                        url: get_str(s, "url").unwrap_or_default(),
                        interval: get_u64(s, "interval").unwrap_or(86400),
                        cron: get_str_opt(s, "cron"),
                        auto_update: s
                            .get("auto_update")
                            .and_then(|x| x.as_bool())
                            .unwrap_or(false),
                        updated_at: get_u64(s, "updated_at"),
                        age_secret: get_str_opt(s, "age_secret"),
                    });
                }
            }
            if let Some(arr) = v
                .get("overrides")
                .and_then(|x| x.get("custom_rules"))
                .and_then(|x| x.as_array())
            {
                for r in arr {
                    if let Some(s) = r.as_str() {
                        item.custom_rules.push(s.to_string());
                    }
                }
            }
            item.age_secret_key = get_str_opt(&v, "age_secret_key");
            item.url = get_str_opt(&v, "url");
            item.auth_token = get_str_opt(&v, "auth_token");
            item.user_agent = get_str_opt(&v, "user_agent");
            if let Some(b) = v.get("use_proxy").and_then(|x| x.as_bool()) {
                item.use_proxy = b;
            }
            if let Some(b) = v.get("auto_update").and_then(|x| x.as_bool()) {
                item.auto_update = b;
            }
            if let Some(n) = get_u64(&v, "update_interval") {
                item.update_interval = n;
            }
            if let Some(n) = get_u64(&v, "update_timeout") {
                item.update_timeout = n;
            }
            if let Some(arr) = v.get("override_ids").and_then(|x| x.as_array()) {
                for id in arr {
                    if let Some(s) = id.as_str() {
                        item.override_ids.push(s.to_string());
                    }
                }
            }
            if let Some(dns) = v
                .get("overrides")
                .and_then(|x| x.get("dns_enabled"))
                .and_then(|x| x.as_bool())
            {
                item.dns_enabled = dns;
            }
            if let Some(arr) = v
                .get("overrides")
                .and_then(|x| x.get("dns_nameservers"))
                .and_then(|x| x.as_array())
            {
                for n in arr {
                    if let Some(s) = n.as_str() {
                        item.dns_nameservers.push(s.to_string());
                    }
                }
            }
            profiles.items.push(item);
            profiles.current = "default".into();
            app.current_profile = "default".into();
        }
    }
    let _ = std::fs::rename(&old, dir.join("mihomo.json.bak"));
}
