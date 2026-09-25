//! 「还原官方配置」：把 Kira 写进工具配置里的自定义模型清掉，让工具回到自己的初始状态。
//!
//! 参考 EchoBird `services/tool_config_manager.rs::restore_tool_to_official`：它给每个工具
//! 写一段定向删除逻辑（硬编码自家的 provider 名 `echobird` / 值前缀 `echobird/` / 一组 env 键），
//! 认不出来的工具走兜底 —— 直接删掉整个配置文件，让工具下次启动重建默认。
//!
//! 我们沿用同一套语义，但删什么不用再手写一遍：**要删的键直接取自声明里的 `configFile.write`**
//! （写了什么就删什么），所以以后加工具不用再补一份还原清单。差异只在于：
//! - 自定义写入器（WorkBuddy / Claude Desktop）整份接管，还原也整份处理；
//! - 接管像 `~/.codex/auth.json` 这种**本来就有用户凭据**的兄弟文件前先备份，
//!   还原时先把备份放回去（EchoBird 同样做法：`codex-auth.bak.json`），
//!   否则「用 Kira 的代理跑一次」会把用户自己的官方 API Key 冲掉。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::commands::ai_registry::{ConfigFileDef, ToolConfig};

/// 还原结果：动了哪些文件 + 给用户看的说明。
#[derive(Debug, Default, serde::Serialize)]
pub struct RestoreOutcome {
    pub files: Vec<String>,
    pub notes: Vec<String>,
}

/// 把某个工具还原成「不带 Kira 自定义模型」的状态。
///
/// 幂等：没有写入过时不会有任何副作用（只返回说明），因此可以放心在
/// 「勾选使用官方模型 → 启动」时每次都调。
pub fn restore_tool_config(tool_config: &ToolConfig) -> Result<RestoreOutcome, String> {
    let Some(cfg) = tool_config.config_file.as_ref() else {
        return Ok(RestoreOutcome {
            files: Vec::new(),
            notes: vec!["该工具没有声明配置文件，无需还原".to_string()],
        });
    };

    // 自定义写入器整份接管，还原也整份处理
    if let Some(writer) = cfg.custom_writer(&tool_config.id) {
        return restore_custom_writer(&writer, cfg, &tool_config.id);
    }

    let main = super::launch::resolve_declared_config_path(cfg);
    let mut by_file: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for raw in declared_write_paths(cfg) {
        let (target, sub) = match raw.split_once('#') {
            Some((file, sub)) => (
                super::launch::resolve_write_target_file(&main, file),
                sub.to_string(),
            ),
            None => (main.clone(), raw.clone()),
        };
        by_file
            .entry(target.display().to_string())
            .or_default()
            .push(sub);
    }

    let mut outcome = RestoreOutcome::default();
    for (file, sub_paths) in by_file {
        let path = PathBuf::from(&file);
        // 先恢复接管前的备份：这一步会把用户自己的凭据放回去
        if let Some(restored) = restore_backup(&tool_config.id, &path) {
            outcome
                .notes
                .push(format!("已从备份恢复 {}（接管前的内容）", restored.display()));
        }
        if !path.exists() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let format = super::launch::write_format_for(&path, &cfg.format);
        // 返回 None = 该文件里根本没有我们要删的键，什么都不做（避免把没动过的文件
        // 重新序列化一遍 —— 那会把用户的 JSONC 注释、排版一起冲掉）。
        let updated = match format {
            super::launch::WriteFormat::Toml => remove_toml_keys(&text, &sub_paths),
            super::launch::WriteFormat::Yaml => remove_yaml_keys(&text, &sub_paths)?,
            super::launch::WriteFormat::Json => remove_json_keys(&text, &sub_paths)?,
        };
        let Some(updated) = updated else {
            continue;
        };
        crate::commands::config::atomic_write_file(&path, updated.as_bytes())
            .map_err(|e| format!("还原 {} 失败: {e}", path.display()))?;
        eprintln!("[restore] 已还原 {}（删除受管键）", path.display());
        outcome.files.push(file);
    }

    if outcome.files.is_empty() {
        outcome
            .notes
            .push("没有发现 Kira 写进去的配置（可能本来就没设置过）".to_string());
    }
    Ok(outcome)
}

/// 声明里会**落盘**的键（跳过只注入进程环境的 `env.`，并把 `fileEnv.X` 折成落盘的 `env.X`）。
fn declared_write_paths(cfg: &ConfigFileDef) -> Vec<String> {
    let Some(write) = cfg.write.as_ref() else {
        return Vec::new();
    };
    write
        .keys()
        .filter(|k| !k.starts_with("env."))
        .map(|k| match k.strip_prefix("fileEnv.") {
            Some(rest) => format!("env.{rest}"),
            None => k.clone(),
        })
        .collect()
}

// ─── 自定义写入器的还原 ───

fn restore_custom_writer(
    writer: &str,
    cfg: &ConfigFileDef,
    tool_id: &str,
) -> Result<RestoreOutcome, String> {
    match writer {
        // WorkBuddy 的 models.json 整份是我们生成的，直接删掉让工具重建默认
        // （EchoBird 对没有专属 restore 的工具就是这个兜底语义）。
        "workbuddy" => {
            let path = super::launch::resolve_declared_config_path(cfg);
            let mut outcome = RestoreOutcome::default();
            if path.exists() {
                std::fs::remove_file(&path)
                    .map_err(|e| format!("删除 {} 失败: {e}", path.display()))?;
                eprintln!("[restore] 已删除 {}", path.display());
                outcome.files.push(path.display().to_string());
                outcome
                    .notes
                    .push("已删除模型的 models.json（工具下次启动会重建默认配置）".to_string());
            } else {
                outcome
                    .notes
                    .push("没有发现 models.json，无需还原".to_string());
            }
            Ok(outcome)
        }
        // Claude Desktop 是「模式开关 + profile 文件」，还原 = 切回官方模式并删掉 profile
        "claudedesktop" => super::tool_config_custom::restore_claudedesktop(),
        other => Err(format!(
            "工具 {tool_id} 声明的自定义写入器 {other} 还没有还原实现"
        )),
    }
}

// ─── 备份（接管别人的文件前先存一份）───

fn backup_dir(tool_id: &str) -> PathBuf {
    crate::commands::utils::get_home_dir()
        .join(".any-version")
        .join("config-backups")
        .join(tool_id)
}

/// 写入前备份目标文件（**只在还没有备份时**拷一份，保留最原始的那份）。
///
/// 只用于「本来就可能有用户自己内容」的文件（如 `~/.codex/auth.json`）：
/// 我们往里面写 Kira 代理的临时 token，还原时要把用户原来的凭据放回去。
pub fn backup_before_overwrite(tool_id: &str, file: &Path) -> Result<(), String> {
    if !file.is_file() {
        return Ok(());
    }
    let Some(name) = file.file_name() else {
        return Ok(());
    };
    let dir = backup_dir(tool_id);
    let dest = dir.join(name);
    if dest.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建备份目录失败: {e}"))?;
    std::fs::copy(file, &dest)
        .map_err(|e| format!("备份 {} 失败: {e}", file.display()))?;
    eprintln!("[restore] 已备份 {} → {}", file.display(), dest.display());
    Ok(())
}

/// 恢复备份（有就放回去并删掉备份），返回被恢复的文件路径。
pub fn restore_backup(tool_id: &str, file: &Path) -> Option<PathBuf> {
    let name = file.file_name()?;
    let backup = backup_dir(tool_id).join(name);
    if !backup.is_file() {
        return None;
    }
    std::fs::copy(&backup, file).ok()?;
    let _ = std::fs::remove_file(&backup);
    Some(file.to_path_buf())
}

// ─── 按键删除（JSON / YAML / TOML）───

/// 删除点号路径对应的 JSON 键；父对象因此变空时一并删掉，
/// 避免留下 `"provider": {}` 这种半截配置（有些工具会因此判定配置非法）。
/// 返回 `None` = 没有任何键被删（调用方应跳过写回，避免重排用户文件）。
fn remove_json_keys(text: &str, paths: &[String]) -> Result<Option<String>, String> {
    let mut doc: serde_json::Value =
        serde_json::from_str(&super::launch::strip_jsonc(text)).map_err(|e| {
            format!("解析配置文件失败（是合法 JSON/JSONC 吗）: {e}")
        })?;
    if !paths.iter().any(|p| json_path_exists(&doc, p)) {
        return Ok(None);
    }
    for path in paths {
        remove_json_path(&mut doc, path);
    }
    serde_json::to_string_pretty(&doc)
        .map(Some)
        .map_err(|e| format!("序列化配置失败: {e}"))
}

fn json_path_exists(doc: &serde_json::Value, path: &str) -> bool {
    let mut cur = doc;
    for key in path.split('.') {
        match cur.get(key) {
            Some(next) => cur = next,
            None => return false,
        }
    }
    true
}

fn remove_json_path(doc: &mut serde_json::Value, path: &str) -> bool {
    let parts: Vec<String> = path.split('.').map(str::to_string).collect();
    remove_json_parts(doc, &parts)
}

/// 返回「本级对象是否已空」（供父级决定是否把我也删掉）。
fn remove_json_parts(cur: &mut serde_json::Value, parts: &[String]) -> bool {
    let Some(obj) = cur.as_object_mut() else {
        return false;
    };
    let key = &parts[0];
    if parts.len() == 1 {
        let existed = obj.remove(key).is_some();
        return existed && obj.is_empty();
    }
    let Some(child) = obj.get_mut(key) else {
        return false;
    };
    if remove_json_parts(child, &parts[1..]) {
        obj.remove(key);
        return obj.is_empty();
    }
    false
}

fn remove_yaml_keys(text: &str, paths: &[String]) -> Result<Option<String>, String> {
    let mut doc: serde_yaml::Value =
        serde_yaml::from_str(text).map_err(|e| format!("解析 YAML 配置失败: {e}"))?;
    if !paths.iter().any(|p| yaml_path_exists(&doc, p)) {
        return Ok(None);
    }
    for path in paths {
        let parts: Vec<serde_yaml::Value> = path
            .split('.')
            .map(|p| serde_yaml::Value::String(p.to_string()))
            .collect();
        remove_yaml_parts(&mut doc, &parts);
    }
    serde_yaml::to_string(&doc)
        .map(Some)
        .map_err(|e| format!("序列化 YAML 配置失败: {e}"))
}

fn yaml_path_exists(doc: &serde_yaml::Value, path: &str) -> bool {
    let mut cur = doc;
    for key in path.split('.') {
        match cur.get(key) {
            Some(next) => cur = next,
            None => return false,
        }
    }
    true
}

fn remove_yaml_parts(cur: &mut serde_yaml::Value, parts: &[serde_yaml::Value]) -> bool {
    let Some(map) = cur.as_mapping_mut() else {
        return false;
    };
    let key = &parts[0];
    if parts.len() == 1 {
        let existed = map.remove(key).is_some();
        return existed && map.is_empty();
    }
    let Some(child) = map.get_mut(key) else {
        return false;
    };
    if remove_yaml_parts(child, &parts[1..]) {
        map.remove(key);
        return map.is_empty();
    }
    false
}

/// TOML 按键删除：与 `write_toml_config` 同一套「表上下文 + 点分 key」判定，
/// 逐行扫、命中就丢弃整行（不解析 TOML 语法，也就不会因为注释/格式怪而失败）。
/// 返回 `None` = 一行都没删（调用方跳过写回）。
fn remove_toml_keys(text: &str, paths: &[String]) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut table = String::new();
    let mut removed = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            table = trimmed
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim()
                .to_string();
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if let Some((key, _)) = trimmed.split_once('=') {
            let key = key.trim().trim_matches('"').trim();
            let full = if table.is_empty() {
                key.to_string()
            } else {
                format!("{table}.{key}")
            };
            if paths.iter().any(|p| p == &full) {
                eprintln!("[restore] 删除 TOML 键 {}", full);
                removed += 1;
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if removed == 0 {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        backup_before_overwrite, remove_json_keys, remove_toml_keys, remove_yaml_keys,
        restore_backup, restore_tool_config,
    };
    use crate::commands::ai_registry::registry;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("anyver-restore-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn json_removal_keeps_user_keys_and_prunes_empty_parents() {
        let text = r#"{
  "provider": { "anyversion": { "options": { "baseURL": "http://127.0.0.1:1", "apiKey": "sk" } },
                "personal": { "options": { "apiKey": "KEEP" } } },
  "model": "anyversion/gpt-5",
  "theme": "dark"
}"#;
        let out = remove_json_keys(
            text,
            &[
                "provider.anyversion.options.baseURL".to_string(),
                "provider.anyversion.options.apiKey".to_string(),
                "model".to_string(),
            ],
        )
        .unwrap()
        .expect("这些键都在，应产出删除后的文本");
        let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
        // 用户自己的 provider 与无关键必须原样保留
        assert_eq!(doc["provider"]["personal"]["options"]["apiKey"], "KEEP");
        assert_eq!(doc["theme"], "dark");
        // 我们写的整棵子树被删干净（父对象空了就一起删）
        assert!(doc["provider"].get("anyversion").is_none());
        assert!(doc.get("model").is_none());
    }

    #[test]
    fn yaml_and_toml_removal_only_touch_declared_keys() {
        let yaml = "providers:\n  personal:\n    apiKey: KEEP\n  anyversion:\n    baseUrl: http://127.0.0.1:1\nmodelRoles:\n  default: anyversion/gpt-5\n";
        let out = remove_yaml_keys(
            yaml,
            &[
                "providers.anyversion.baseUrl".to_string(),
                "modelRoles.default".to_string(),
            ],
        )
        .unwrap()
        .expect("这些键都在，应产出删除后的文本");
        let doc: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(doc["providers"]["personal"]["apiKey"].as_str(), Some("KEEP"));
        assert!(doc["providers"].get("anyversion").is_none());
        assert!(doc["modelRoles"].get("default").is_none());

        let toml = "model = \"anyversion/gpt-5\"\ntheme = \"dark\"\n\n[model_providers.anyversion]\nname = \"anyversion\"\nbase_url = \"http://127.0.0.1:1\"\n\n[model_providers.other]\nbase_url = \"https://api.x.com\"\n";
        let out = remove_toml_keys(
            toml,
            &[
                "model".to_string(),
                "model_providers.anyversion.name".to_string(),
                "model_providers.anyversion.base_url".to_string(),
            ],
        )
        .expect("这些键都在，应产出删除后的文本");
        assert!(!out.contains("model = "), "顶层 model 该行要删掉: {out}");
        assert!(!out.contains("name = "));
        assert!(!out.contains("http://127.0.0.1:1"));
        // 别人的表与无关键都不能动
        assert!(out.contains("theme = \"dark\""));
        assert!(out.contains("https://api.x.com"));
    }

    /// 接管 `auth.json` 之前备份、还原时放回去：否则用一次 Kira 就把用户官方 Key 冲掉了。
    #[test]
    fn backup_and_restore_of_user_credentials() {
        let dir = temp_dir("backup");
        let file = dir.join("auth.json");
        std::fs::write(&file, r#"{"OPENAI_API_KEY":"user-official-key"}"#).unwrap();

        // 模拟一次写入：先备份，再覆盖
        backup_before_overwrite("unittest-tool", &file).unwrap();
        std::fs::write(&file, r#"{"OPENAI_API_KEY":"kira-proxy-token"}"#).unwrap();
        // 第二次写入不该覆盖已有备份（要保留最原始的那份）
        backup_before_overwrite("unittest-tool", &file).unwrap();
        std::fs::write(&file, r#"{"OPENAI_API_KEY":"kira-proxy-token-2"}"#).unwrap();

        let restored = restore_backup("unittest-tool", &file).expect("应恢复备份");
        assert_eq!(restored, file);
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(doc["OPENAI_API_KEY"], "user-official-key");
        // 备份用过即删，下次写入会重新备份当前内容
        assert!(restore_backup("unittest-tool", &file).is_none());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(super::backup_dir("unittest-tool"));
    }

    /// 用**真实声明**跑一遍还原：只删我们写的键，用户的键留着（claude-code 的 settings.json）。
    #[test]
    fn restore_uses_the_real_declaration() {
        let dir = temp_dir("decl");
        let file = dir.join("settings.json");
        std::fs::write(
            &file,
            r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:9","ANTHROPIC_AUTH_TOKEN":"t","MY_OWN":"keep"},
                 "permissions":{"allow":["Bash"]},"model":"claude-sonnet-4"}"#,
        )
        .unwrap();

        let mut cfg = registry()
            .get_tool_config("claude-code")
            .expect("claude-code 应在注册表里")
            .clone();
        cfg.config_file.as_mut().unwrap().path = file.to_string_lossy().to_string();

        let outcome = restore_tool_config(&cfg).expect("还原应成功");
        assert!(!outcome.files.is_empty(), "应报告改动的文件");

        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert!(doc["env"].get("ANTHROPIC_BASE_URL").is_none());
        assert!(doc["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
        // 用户自己的键与顶层 model 不属于我们的声明范围，不能被删
        assert_eq!(doc["env"]["MY_OWN"], "keep");
        assert_eq!(doc["model"], "claude-sonnet-4");
        assert_eq!(doc["permissions"]["allow"][0], "Bash");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 没有写入过时还原必须是空操作（幂等），不能把文件写成 {}。
    #[test]
    fn restore_is_a_noop_when_nothing_was_written() {
        let dir = temp_dir("noop");
        let file = dir.join("settings.json");
        let original = r#"{"permissions":{"allow":["Bash"]}}"#;
        std::fs::write(&file, original).unwrap();

        let mut cfg = registry().get_tool_config("claude-code").unwrap().clone();
        cfg.config_file.as_mut().unwrap().path = file.to_string_lossy().to_string();
        restore_tool_config(&cfg).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
