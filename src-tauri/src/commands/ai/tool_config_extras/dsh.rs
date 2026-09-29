//! dsh（DeepSeek Harness，`~/.dsh`）的附加写入与还原。
//!
//! ## 为什么需要（A1：写的是**权威**文件）
//!
//! dsh 的桌面端配置有**两份**：
//!
//! - `~/.dsh/profiles/<profile>/cordis.patch.yml` —— **权威**，YAML 序列 `[{id, config}]`；
//! - `~/.dsh/settings.yaml` —— legacy 扁平表，仅作兼容（dsh 下次启动会把残留的它**导入**回来）。
//!
//! 只写 `settings.yaml` 的话，那份权威 patch 会盖过它 —— 于是「设置成功了、文件也变了，
//! 但 dsh 用的还是旧模型」。
//!
//! **`<profile>` 不是固定的 `desktop`**：真机校对时这台机器的 dsh 用的是 `web`
//! （bundles 为 `@deepseek-ai/dsh-base` + `@deepseek-ai/dsh-web-app`）。所以这里扫
//! `profiles/*/cordis.patch.yml` 逐个镜像，而不是写死一个名字 —— 详见
//! [`existing_profile_patches`]。
//!
//! ## A2：凭据在 `refs` 层级
//!
//! `~/.dsh/.credentials.yaml` 是**共享 v1 凭据库**：顶层只允许 `version` / `refs` / `records`
//! 三个键，值放在 `refs.<KEY>` 下。写在顶层会被它自己的校验判为格式错误
//! （老格式的顶层「键 → 非空字符串」会被自动迁移，但新建就该直接写对）。
//!
//! ## 镜像而不是重写
//!
//! patch 里的内容**取自通用写入刚写好的 `settings.yaml`**，而不是在这一侧重新算一遍 ——
//! 这样「声明改了什么」和「写进权威文件的是什么」永远一致，不会出现两套逻辑漂移。
//!
//! 对照 EchoBird `services/tool_config_manager/dsh.rs`。

use std::path::{Path, PathBuf};

use serde_yaml::Value;

use super::{read_or_empty, write_file, ExtrasCtx};
use crate::commands::ai::tool_config_restore::RestoreOutcome;

/// 写进 `apiKeyEnv` / 凭据 `refs` 的键名。**必须与声明里 `apiKeyEnv` 的值一致**，
/// 否则 dsh 按这个名字去凭据库里找不到 key。
const API_KEY_ENV: &str = "ANYVERSION_API_KEY";
/// 我们在 dsh 里用的 provider 名（与声明里的键一致）。
const PROVIDER: &str = "anyversion";
/// 我们拥有的两个 section id。
const OWNED_SECTIONS: [&str; 2] = ["llm-pi-ai", "agent-default-model"];
/// 无凭证的本地端点用的占位 key（dsh 不接受空值）。
const LOCAL_NO_AUTH: &str = "local-no-auth";

fn ykey(name: &str) -> Value {
    Value::String(name.to_string())
}

/// `~/.dsh`：从声明的 `~/.dsh/settings.yaml` 往上取一层。
fn dsh_home(main_path: &Path) -> Result<PathBuf, String> {
    main_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("无法从 {} 推导 ~/.dsh 目录", main_path.display()))
}

/// 某个 profile 的权威 patch 文件路径。
fn profile_path_in(home: &Path, profile: &str) -> PathBuf {
    home.join("profiles").join(profile).join("cordis.patch.yml")
}

/// 找出该 dsh 实例实际存在的所有 profile patch 文件。
///
/// **不能写死 `profiles/desktop/`**。真机校对结果：这台机器的 dsh profile 是 `web`
/// （`profiles/web/package.json` 的 `dsh.profile.bundles` = `@deepseek-ai/dsh-base` +
/// `@deepseek-ai/dsh-web-app`），且 `profiles/web/cordis.yml` 的注释明确写着
/// 「Edit **cordis.patch.yml**, not this file」—— 权威文件确实是 patch，只是**目录名随 profile
/// 而变**。写死 `desktop` 会写到一个 dsh 根本不读的文件里，正好是我们要修的那类 bug。
///
/// 因此扫 `profiles/*/cordis.patch.yml` 并**逐个镜像**（多 profile 共存时让它们保持一致）。
/// 一个都没有时返回空 —— 此时无法确定 profile 名，凭空造一个目录更可能落在错位置；
/// `settings.yaml` 那一份照常写，dsh 的 legacy 导入会读它。
fn existing_profile_patches(home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(home.join("profiles")) else {
        return out;
    };
    for entry in entries.flatten() {
        let candidate = entry.path().join("cordis.patch.yml");
        if candidate.is_file() {
            out.push(candidate);
        }
    }
    out.sort();
    out
}

fn credentials_path(home: &Path) -> PathBuf {
    home.join(".credentials.yaml")
}

/// 读权威 profile。**结构不符就报错中止**：这是 dsh 自己维护的文件，
/// 猜着改等于制造一个 dsh 读不了的配置。
fn read_profile(path: &Path) -> Result<Vec<Value>, String> {
    let text = read_or_empty(path);
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let value: Value = serde_yaml::from_str(&text)
        .map_err(|e| format!("{} 不是合法 YAML，已放弃写入以免写坏 dsh 配置: {e}", path.display()))?;
    let entries = value
        .as_sequence()
        .ok_or_else(|| format!("{} 的顶层不是序列（期望 [{{id, config}}]），已放弃写入", path.display()))?;
    let mut seen = std::collections::HashSet::new();
    for entry in entries {
        let id = entry
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{} 里有条目缺少 id，已放弃写入", path.display()))?;
        let config_ok = entry.get("config").map(Value::is_mapping).unwrap_or(true);
        if !seen.insert(id.to_string()) || !config_ok {
            return Err(format!(
                "{} 里有重复 id 或 config 不是映射，已放弃写入",
                path.display()
            ));
        }
    }
    Ok(entries.clone())
}

/// 读凭据库并保证是 v1 结构（老格式自动迁移）。
///
/// 顶层多出任何键都判为「不是我们认识的结构」而中止 —— 宁可什么都不写，
/// 也不要把用户的凭据文件改成 dsh 读不了的样子。
fn read_credentials(path: &Path) -> Result<serde_yaml::Mapping, String> {
    let text = read_or_empty(path);
    if text.trim().is_empty() {
        return Ok(fresh_credentials());
    }
    let mut value: Value = serde_yaml::from_str(&text).map_err(|e| {
        format!(
            "{} 不是合法 YAML，已放弃写入以免写坏 dsh 凭据: {e}",
            path.display()
        )
    })?;
    let Some(map) = value.as_mapping_mut() else {
        return Err(format!("{} 的顶层不是映射，已放弃写入", path.display()));
    };

    if !map.contains_key(&ykey("version")) {
        // 老格式：顶层直接是「键 → 字符串」。只有全部是非空字符串才敢当 refs 迁移。
        if !map
            .values()
            .all(|v| v.as_str().is_some_and(|s| !s.is_empty()))
        {
            return Err(format!(
                "{} 既不是 v1 结构（version/refs/records）、也不像老格式（顶层键 → 非空字符串），已放弃写入",
                path.display()
            ));
        }
        let refs = std::mem::take(map);
        let mut migrated = serde_yaml::Mapping::new();
        migrated.insert(ykey("version"), Value::Number(1u64.into()));
        migrated.insert(ykey("refs"), Value::Mapping(refs));
        migrated.insert(ykey("records"), Value::Mapping(serde_yaml::Mapping::new()));
        *map = migrated;
    }

    let version_ok = map.get(&ykey("version")).and_then(Value::as_u64) == Some(1);
    let keys_ok = map
        .keys()
        .all(|k| matches!(k.as_str(), Some("version" | "refs" | "records")));
    if !version_ok || !keys_ok {
        return Err(format!(
            "{} 不是 v1 凭据结构（顶层只允许 version/refs/records 且 version=1），已放弃写入",
            path.display()
        ));
    }
    for name in ["refs", "records"] {
        let key = ykey(name);
        if !map.contains_key(&key) {
            map.insert(key.clone(), Value::Mapping(serde_yaml::Mapping::new()));
        }
        let ok = map.get(&key).map(Value::is_mapping).unwrap_or(false);
        if !ok {
            return Err(format!(
                "{} 的 {name} 不是映射，已放弃写入",
                path.display()
            ));
        }
    }
    Ok(std::mem::take(map))
}

fn fresh_credentials() -> serde_yaml::Mapping {
    let mut out = serde_yaml::Mapping::new();
    out.insert(ykey("version"), Value::Number(1u64.into()));
    out.insert(
        ykey("refs"),
        Value::Mapping(serde_yaml::Mapping::new()),
    );
    out.insert(
        ykey("records"),
        Value::Mapping(serde_yaml::Mapping::new()),
    );
    out
}

/// 端点是不是本机（本机不需要真 key）。
fn is_local_endpoint(url: &str) -> bool {
    url.contains("127.0.0.1") || url.contains("localhost") || url.contains("[::1]")
}

pub(super) fn apply(ctx: &ExtrasCtx<'_>) -> Result<Vec<String>, String> {
    let home = dsh_home(ctx.main_path)?;

    // 凭据：真 key 优先；本机端点用占位值；两者都不是 → 不写（写空值会被 dsh 判为非法）
    let api_key = if !ctx.api_key.trim().is_empty() {
        ctx.api_key.trim().to_string()
    } else if is_local_endpoint(ctx.base_url) {
        LOCAL_NO_AUTH.to_string()
    } else {
        eprintln!(
            "[extras] dsh: apiKey 为空且端点不是本机 → 不写凭据（dsh 不接受空凭证）"
        );
        String::new()
    };

    let mut touched: Vec<String> = Vec::new();

    // ① 权威 profile：把通用写入落在 settings.yaml 里的那两个 section 镜像过去
    let settings_text = read_or_empty(ctx.main_path);
    if settings_text.trim().is_empty() {
        // 通用写入什么都没落下（多半没选模型）→ 不该由这里凭空空造
        return Ok(Vec::new());
    }
    let settings: Value = serde_yaml::from_str(&settings_text).map_err(|e| {
        format!(
            "{} 不是合法 YAML（通用写入刚写过，不该是这样）: {e}",
            ctx.main_path.display()
        )
    })?;

    let profiles = existing_profile_patches(&home);
    if profiles.is_empty() {
        eprintln!(
            "[extras] dsh: 没找到 profiles/*/cordis.patch.yml（dsh 还没建过 profile）→ 只写 settings.yaml 与凭据"
        );
    }
    for profile in &profiles {
        let profile_existing = read_or_empty(profile);
        let mut entries = read_profile(profile)?;
        let mut mirrored = 0usize;
        for id in OWNED_SECTIONS {
            let Some(config) = settings.get(id) else {
                // 这一项没被声明写出来（例如 apiKey 模板被跳过）→ 不镜像，免得写半截
                continue;
            };
            match entries
                .iter_mut()
                .find(|entry| entry.get("id").and_then(Value::as_str) == Some(id))
            {
                // 已有同 id 的条目 → **就地更新** config，其余键与顺序原样保留
                Some(entry) => {
                    if let Some(map) = entry.as_mapping_mut() {
                        map.insert(ykey("config"), config.clone());
                        mirrored += 1;
                    }
                }
                // 没有才 append
                None => {
                    let mut entry = serde_yaml::Mapping::new();
                    entry.insert(ykey("id"), Value::String(id.to_string()));
                    entry.insert(ykey("config"), config.clone());
                    entries.push(Value::Mapping(entry));
                    mirrored += 1;
                }
            }
        }
        if mirrored == 0 {
            continue;
        }
        let profile_updated = serde_yaml::to_string(&Value::Sequence(entries))
            .map_err(|e| format!("序列化 dsh profile 失败: {e}"))?;
        if profile_updated != profile_existing {
            write_file(profile, &profile_updated)?;
            eprintln!(
                "[extras] dsh: 已同步权威 profile {}（{mirrored} 个 section）",
                profile.display()
            );
            touched.push(profile.display().to_string());
        }
    }

    // ② 凭据：写在 refs 层级（顶层会被 dsh 的校验拒掉）
    if !api_key.is_empty() {
        let creds_path = credentials_path(&home);
        let creds_existing = read_or_empty(&creds_path);
        let mut creds = read_credentials(&creds_path)?;
        let refs = creds
            .get_mut(&ykey("refs"))
            .and_then(Value::as_mapping_mut)
            .ok_or_else(|| format!("{} 的 refs 不是映射", creds_path.display()))?;
        refs.insert(ykey(API_KEY_ENV), Value::String(api_key));
        let creds_updated = serde_yaml::to_string(&Value::Mapping(creds))
            .map_err(|e| format!("序列化 dsh 凭据失败: {e}"))?;
        if creds_updated != creds_existing {
            touched.push(write_file(&creds_path, &creds_updated)?);
        }
    }

    Ok(touched)
}

/// 还原：把 patch 里我们那两个 section 摘掉、凭据里我们的 ref 删掉。
///
/// 这一段通用还原**删不到** —— 它们不在声明的 `write` 映射里（patch 是另一个文件，
/// 凭据在 `refs` 层级）。不还原的话「勾了使用官方模型」之后 dsh 仍然跑在我们的
/// provider 上，因为权威文件里那条还在。
pub(super) fn restore(main_path: &Path) -> Result<RestoreOutcome, String> {
    let mut outcome = RestoreOutcome::default();
    let home = dsh_home(main_path)?;

    // profile：摘掉我们的 section，别人的条目原样保留（逐个 profile 都处理）
    for profile in existing_profile_patches(&home) {
        let existing = read_or_empty(&profile);
        let mut entries = read_profile(&profile)?;
        let before = entries.len();
        entries.retain(|entry| {
            let id = entry.get("id").and_then(Value::as_str).unwrap_or("");
            !OWNED_SECTIONS.contains(&id)
        });
        let removed_count = before - entries.len();
        if removed_count == 0 {
            continue;
        }
        let updated = serde_yaml::to_string(&Value::Sequence(entries))
            .map_err(|e| format!("序列化 dsh profile 失败: {e}"))?;
        if updated != existing {
            write_file(&profile, &updated)?;
            eprintln!(
                "[extras] dsh: 已从权威 profile {} 摘掉 {removed_count} 个 section",
                profile.display()
            );
            outcome.files.push(profile.display().to_string());
        }
    }

    // 凭据：删掉我们的 ref（用户的其它 ref 一个不动）
    let creds_path = credentials_path(&home);
    if creds_path.exists() {
        let existing = read_or_empty(&creds_path);
        if let Ok(mut creds) = read_credentials(&creds_path) {
            let removed = creds
                .get_mut(&ykey("refs"))
                .and_then(Value::as_mapping_mut)
                .map(|refs| refs.remove(&ykey(API_KEY_ENV)).is_some())
                .unwrap_or(false);
            if removed {
                let updated = serde_yaml::to_string(&Value::Mapping(creds))
                    .map_err(|e| format!("序列化 dsh 凭据失败: {e}"))?;
                if updated != existing {
                    write_file(&creds_path, &updated)?;
                    outcome.files.push(creds_path.display().to_string());
                }
            }
        }
    }

    if outcome.files.is_empty() {
        outcome
            .notes
            .push("dsh 的权威 profile 与凭据里都没有我们的配置".to_string());
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用的 profile 名。刻意用真机上的 `web` 而不是 `desktop` ——
    /// 「profile 名会变、不能写死」正是这里要锁住的行为。
    const TEST_PROFILE: &str = "web";

    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("anyver-dsh-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ctx<'a>(main: &'a Path, key: &'a str) -> ExtrasCtx<'a> {
        ExtrasCtx {
            tool_id: "dsh",
            main_path: main,
            base_url: "http://127.0.0.1:8787",
            upstream_url: "https://api.deepseek.com/v1",
            api_key: key,
            model: "deepseek-v4-pro",
            model_name: "deepseek-v4-pro",
            provider: PROVIDER,
            chosen_protocol: "openai",
            web_search: false,
        }
    }

    /// 通用写入会落在 settings.yaml 的那两个 section（本模块的输入），
    /// 外加 dsh 自己会建好的 profile 骨架（权威 patch 的落点）。
    fn write_settings(home: &Path) {
        std::fs::create_dir_all(profile_path_in(home, TEST_PROFILE).parent().unwrap()).unwrap();
        std::fs::write(profile_path_in(home, TEST_PROFILE), "[]\n").unwrap();
        std::fs::write(
            home.join("settings.yaml"),
            "llm-pi-ai:\n  providers:\n    anyversion:\n      displayName: AnyVersion\n      apiKeyEnv: ANYVERSION_API_KEY\n      api: openai-completions\n      baseURL: http://127.0.0.1:8787\n      models:\n        - id: deepseek-v4-pro\n          name: deepseek-v4-pro\nagent-default-model:\n  provider: anyversion\n  model: deepseek-v4-pro\n",
        )
        .unwrap();
    }

    #[test]
    fn writes_authoritative_profile_and_credentials_refs() {
        let home = temp_home("basic");
        write_settings(&home);
        let main = home.join("settings.yaml");

        let touched = apply(&ctx(&main, "sk-real")).unwrap();
        assert_eq!(touched.len(), 2, "profile + credentials 都要写: {touched:?}");

        // 权威 profile：YAML 序列，带 id/config
        let profile: Value =
            serde_yaml::from_str(&std::fs::read_to_string(profile_path_in(&home, TEST_PROFILE)).unwrap()).unwrap();
        let entries = profile.as_sequence().unwrap();
        assert_eq!(entries.len(), 2, "{entries:?}");
        let llm = entries
            .iter()
            .find(|e| e["id"] == Value::String("llm-pi-ai".into()))
            .expect("应有 llm-pi-ai section");
        assert_eq!(
            llm["config"]["providers"]["anyversion"]["baseURL"],
            Value::String("http://127.0.0.1:8787".into())
        );
        let selector = entries
            .iter()
            .find(|e| e["id"] == Value::String("agent-default-model".into()))
            .expect("应有 agent-default-model section");
        assert_eq!(selector["config"]["provider"], Value::String(PROVIDER.into()));
        assert_eq!(
            selector["config"]["model"],
            Value::String("deepseek-v4-pro".into())
        );

        // 凭据：必须在 refs 层级，且顶层只有 version/refs/records
        let creds: Value =
            serde_yaml::from_str(&std::fs::read_to_string(credentials_path(&home)).unwrap()).unwrap();
        assert_eq!(creds["version"].as_u64(), Some(1));
        assert_eq!(
            creds["refs"][API_KEY_ENV],
            Value::String("sk-real".into())
        );
        assert!(
            creds.get(API_KEY_ENV).is_none(),
            "不能把 key 写在顶层（dsh 会判为格式错误）"
        );
        assert!(creds.get("records").unwrap().is_mapping());

        let _ = std::fs::remove_dir_all(&home);
    }

    /// **profile 名不能写死**。真机上 dsh 用的是 `profiles/web/`，而参考实现写的是 `desktop`。
    /// 写死 `desktop` 会落到 dsh 根本不读的文件里 —— 又是「设置成功但没生效」那一类 bug。
    #[test]
    fn mirrors_into_whatever_profile_exists_not_a_hardcoded_name() {
        let home = temp_home("profile-name");
        write_settings(&home);
        let main = home.join("settings.yaml");

        apply(&ctx(&main, "sk-real")).unwrap();

        assert!(
            !profile_path_in(&home, "desktop").exists(),
            "不能凭空造一个 dsh 不读的 desktop profile"
        );
        let profile: Value =
            serde_yaml::from_str(&std::fs::read_to_string(profile_path_in(&home, TEST_PROFILE)).unwrap())
                .unwrap();
        let entries = profile.as_sequence().unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e["id"] == Value::String("agent-default-model".into())),
            "应写进实际存在的 profile: {entries:?}"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    /// 多个 profile 共存时**每个都要镜像**，否则「换到另一个 profile 就跑回旧模型」。
    #[test]
    fn mirrors_into_every_existing_profile() {
        let home = temp_home("multi-profile");
        write_settings(&home);
        let main = home.join("settings.yaml");
        std::fs::create_dir_all(profile_path_in(&home, "desktop").parent().unwrap()).unwrap();
        std::fs::write(profile_path_in(&home, "desktop"), "[]\n").unwrap();

        apply(&ctx(&main, "sk-real")).unwrap();

        for name in [TEST_PROFILE, "desktop"] {
            let profile: Value = serde_yaml::from_str(
                &std::fs::read_to_string(profile_path_in(&home, name)).unwrap(),
            )
            .unwrap();
            assert_eq!(
                profile.as_sequence().unwrap().len(),
                2,
                "{name} 也该被镜像: {profile:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    /// 一个 profile 都没有（dsh 还没跑过）：不凭空造目录，只写 settings.yaml 与凭据
    /// —— dsh 首次启动会把它导入成自己的 profile。
    #[test]
    fn no_profile_directory_writes_only_credentials() {
        let home = temp_home("no-profile");
        std::fs::write(
            home.join("settings.yaml"),
            "llm-pi-ai:\n  providers:\n    anyversion:\n      baseURL: http://127.0.0.1:8787\nagent-default-model:\n  provider: anyversion\n  model: deepseek-v4-pro\n",
        )
        .unwrap();
        let main = home.join("settings.yaml");

        let touched = apply(&ctx(&main, "sk-real")).unwrap();

        assert_eq!(touched.len(), 1, "只该写凭据: {touched:?}");
        assert!(!home.join("profiles").exists(), "不能凭空造 profile 目录");
        assert!(credentials_path(&home).exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn existing_profile_entries_are_updated_in_place_not_duplicated() {
        let home = temp_home("merge");
        write_settings(&home);
        let main = home.join("settings.yaml");
        // 已有我们的 section（旧模型）+ 一个别人的 section
        std::fs::create_dir_all(profile_path_in(&home, TEST_PROFILE).parent().unwrap()).unwrap();
        std::fs::write(
            profile_path_in(&home, TEST_PROFILE),
            "- id: llm-pi-ai\n  config:\n    providers:\n      anyversion:\n        baseURL: http://old\n- id: someone-else\n  config:\n    keep: me\n",
        )
        .unwrap();

        apply(&ctx(&main, "sk-real")).unwrap();

        let profile: Value =
            serde_yaml::from_str(&std::fs::read_to_string(profile_path_in(&home, TEST_PROFILE)).unwrap()).unwrap();
        let entries = profile.as_sequence().unwrap();
        // 就地更新，不新增重复 id
        assert_eq!(entries.len(), 3, "{entries:?}");
        assert_eq!(
            entries.iter().filter(|e| e["id"] == Value::String("llm-pi-ai".into())).count(),
            1
        );
        assert_eq!(
            entries[0]["config"]["providers"]["anyversion"]["baseURL"],
            Value::String("http://127.0.0.1:8787".into())
        );
        // 别人的 section 一个字都不能改
        let other = entries
            .iter()
            .find(|e| e["id"] == Value::String("someone-else".into()))
            .unwrap();
        assert_eq!(other["config"]["keep"], Value::String("me".into()));

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn legacy_flat_credentials_are_migrated_to_v1_refs() {
        let home = temp_home("legacy");
        write_settings(&home);
        let main = home.join("settings.yaml");
        std::fs::write(home.join(".credentials.yaml"), "SOME_OTHER_KEY: sk-other\n").unwrap();

        apply(&ctx(&main, "sk-real")).unwrap();

        let creds: Value =
            serde_yaml::from_str(&std::fs::read_to_string(credentials_path(&home)).unwrap()).unwrap();
        assert_eq!(creds["version"].as_u64(), Some(1));
        // 老格式的键要整体搬进 refs，不能丢
        assert_eq!(creds["refs"]["SOME_OTHER_KEY"], Value::String("sk-other".into()));
        assert_eq!(creds["refs"][API_KEY_ENV], Value::String("sk-real".into()));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn credentials_with_unexpected_top_level_key_is_refused() {
        let home = temp_home("badcreds");
        write_settings(&home);
        let main = home.join("settings.yaml");
        let original = "version: 1\nrefs: {}\nrecords: {}\nsurprise: nope\n";
        std::fs::write(home.join(".credentials.yaml"), original).unwrap();

        let err = apply(&ctx(&main, "sk-real")).unwrap_err();
        assert!(err.contains("v1 凭据结构"), "{err}");
        // 文件原样保留
        assert_eq!(
            std::fs::read_to_string(home.join(".credentials.yaml")).unwrap(),
            original
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn local_endpoint_without_key_gets_placeholder() {
        let home = temp_home("local");
        write_settings(&home);
        let main = home.join("settings.yaml");
        apply(&ctx(&main, "")).unwrap();
        let creds: Value =
            serde_yaml::from_str(&std::fs::read_to_string(credentials_path(&home)).unwrap()).unwrap();
        assert_eq!(creds["refs"][API_KEY_ENV], Value::String(LOCAL_NO_AUTH.into()));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn malformed_profile_is_refused_without_touching_it() {
        let home = temp_home("badprofile");
        write_settings(&home);
        let main = home.join("settings.yaml");
        std::fs::create_dir_all(profile_path_in(&home, TEST_PROFILE).parent().unwrap()).unwrap();
        let original = "id: not-a-sequence\n";
        std::fs::write(profile_path_in(&home, TEST_PROFILE), original).unwrap();

        let err = apply(&ctx(&main, "sk-real")).unwrap_err();
        assert!(err.contains("顶层不是序列"), "{err}");
        assert_eq!(
            std::fs::read_to_string(profile_path_in(&home, TEST_PROFILE)).unwrap(),
            original
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn restore_removes_our_sections_and_credential_ref_only() {
        let home = temp_home("restore");
        write_settings(&home);
        let main = home.join("settings.yaml");
        std::fs::create_dir_all(profile_path_in(&home, TEST_PROFILE).parent().unwrap()).unwrap();
        std::fs::write(
            profile_path_in(&home, TEST_PROFILE),
            "- id: someone-else\n  config:\n    keep: me\n",
        )
        .unwrap();
        apply(&ctx(&main, "sk-real")).unwrap();

        let outcome = restore(&main).unwrap();
        assert_eq!(outcome.files.len(), 2, "{:?}", outcome.files);

        let profile: Value =
            serde_yaml::from_str(&std::fs::read_to_string(profile_path_in(&home, TEST_PROFILE)).unwrap()).unwrap();
        let entries = profile.as_sequence().unwrap();
        assert_eq!(entries.len(), 1, "只该剩下别人的 section: {entries:?}");
        assert_eq!(entries[0]["id"], Value::String("someone-else".into()));

        let creds: Value =
            serde_yaml::from_str(&std::fs::read_to_string(credentials_path(&home)).unwrap()).unwrap();
        assert!(creds["refs"].get(API_KEY_ENV).is_none(), "凭据 ref 必须删掉");
        assert_eq!(creds["version"].as_u64(), Some(1), "结构本身要保留");

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn restore_is_idempotent_and_reports_nothing_when_clean() {
        let home = temp_home("clean");
        write_settings(&home);
        let main = home.join("settings.yaml");
        let outcome = restore(&main).unwrap();
        assert!(outcome.files.is_empty());
        assert!(!outcome.notes.is_empty(), "应给出「没有我们的配置」的说明");
        let _ = std::fs::remove_dir_all(&home);
    }
}
