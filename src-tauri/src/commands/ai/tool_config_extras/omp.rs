//! OMP（Oh My Pi，`~/.omp/agent/`）的附加写入。
//!
//! 通用映射把 provider 写进 `models.yml`，并设了 `config.yml` 的 `modelRoles.default`。
//! 但 OMP 的角色是**一整套**（`default` / `smol` / `slow` / `plan` …），用户之前被我们写过
//! 的那些角色都还指着**上一个模型**。换模型时只改 `default`，其余角色就悬在一个已经不在
//! `models.yml` 里的旧 model 上 —— OMP 启动时会报找不到模型。
//!
//! 这里的做法（对照 EchoBird `services/tool_config_manager/omp.rs`）：
//!
//! - 只重定向**值以我们的 provider 前缀开头**的角色，保留它可选的 thinking 档位后缀
//!   （`:high` / `:max` …）—— 那是用户对「这个角色想多少」的偏好，不该被换模型顺手抹掉；
//! - 用户自己指到别处（`personal/…`、`@smol`）的角色**一个都不动**；
//! - `default` 恒被设成当前选择。

use std::path::Path;

use super::{read_or_empty, sibling, write_file, ExtrasCtx};
use crate::commands::ai::tool_config_restore::RestoreOutcome;

/// OMP 认可的 thinking 档位。只有命中这组值才当后缀保留，否则那个 `:` 之后的东西
/// 是模型名的一部分（比如 `provider:model` 这种写法），不能瞎拼。
const THINKING_LEVELS: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

/// 取 `modelRoles` 这张映射；不存在就建一张空的。
fn roles_mapping<'a>(
    root: &'a mut serde_yaml::Value,
    path: &Path,
) -> Result<&'a mut serde_yaml::Mapping, String> {
    let map = root
        .as_mapping_mut()
        .ok_or_else(|| format!("{} 的顶层不是映射（YAML 对象），已放弃写入", path.display()))?;
    let key = serde_yaml::Value::String("modelRoles".to_string());
    if !map.contains_key(&key) {
        map.insert(
            key.clone(),
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
        );
    }
    map.get_mut(&key)
        .and_then(|value| value.as_mapping_mut())
        .ok_or_else(|| format!("{} 的 modelRoles 不是映射（YAML 对象），已放弃写入", path.display()))
}

pub(super) fn apply(ctx: &ExtrasCtx<'_>) -> Result<Vec<String>, String> {
    let path = sibling(ctx.main_path, "config.yml");
    // 文件不存在也建：`modelRoles` 是整个 config.yml 里唯一由我们管的键，新建一个只含它的
    // 文件正是参考实现的做法（EchoBird `omp.rs` 的 `load_config` + `write_yaml_file`）。
    // 反过来「不存在就跳过」会让首次使用永远选不上模型。
    //
    // 另外**不能**让通用写入映射去设 `modelRoles.default`：它会先跑，把用户给 `default`
    // 设的 thinking 档位后缀（`:high`）直接覆盖掉，本模块再想「保留后缀」就晚了。
    // 所以 `modelRoles` 由本模块独占。
    let existing = read_or_empty(&path);
    let mut root: serde_yaml::Value = if existing.trim().is_empty() {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    } else {
        serde_yaml::from_str(&existing).map_err(|e| {
            format!(
                "{} 不是合法 YAML，已放弃写入以免写坏 OMP 配置: {e}",
                path.display()
            )
        })?
    };

    let prefix = format!("{}/", ctx.provider);
    let selector = format!("{}/{}", ctx.provider, ctx.model_name);
    let roles = roles_mapping(&mut root, &path)?;

    // 先把角色名收一份出来，避免「遍历中改值」的借用冲突
    let names: Vec<serde_yaml::Value> = roles.keys().cloned().collect();
    let mut retargeted = 0usize;
    for name in names {
        let Some(old) = roles
            .get(&name)
            .and_then(|value| value.as_str())
            .map(|s| s.to_string())
        else {
            continue;
        };
        // 只动我们写过的角色；用户自己的角色（personal/… 、@smol）一个都不碰
        if !old.starts_with(&prefix) {
            continue;
        }
        let level = old
            .rsplit_once(':')
            .map(|(_, level)| level)
            .filter(|level| THINKING_LEVELS.contains(level));
        let new_value = match level {
            Some(level) => format!("{selector}:{level}"),
            None => selector.clone(),
        };
        if new_value != old {
            retargeted += 1;
        }
        roles.insert(name, serde_yaml::Value::String(new_value));
    }
    // `default` 必须有值。但**它本来就属于我们、且带档位后缀**时，上面的循环已经把它改好
    // 并保留了后缀 —— 这时不能再用裸选择器盖掉（那是用户的偏好，不是我们的字段）。
    // 只有「缺失」或「指着别家」才无条件设成我们的选择器。
    let default_key = serde_yaml::Value::String("default".to_string());
    let default_already_ours = roles
        .get(&default_key)
        .and_then(|value| value.as_str())
        .map(|value| value.starts_with(&prefix))
        .unwrap_or(false);
    if !default_already_ours {
        roles.insert(
            default_key,
            serde_yaml::Value::String(selector.clone()),
        );
    }
    eprintln!(
        "[extras] OMP: default → {}，另有 {retargeted} 个角色被重新指向（档位后缀已保留）",
        roles
            .get(&serde_yaml::Value::String("default".to_string()))
            .and_then(|value| value.as_str())
            .unwrap_or(&selector)
    );

    let updated = serde_yaml::to_string(&root)
        .map_err(|e| format!("序列化 {} 失败: {e}", path.display()))?;
    if updated == existing {
        return Ok(Vec::new());
    }
    Ok(vec![write_file(&path, &updated)?])
}

/// 还原：把**指向我们的**角色整个删掉。
///
/// 不能只删 `default` —— 那些被本模块重新指向过的角色（`smol` / `slow` / `plan` …）
/// 会悬在一个已经不在 `models.yml` 里的 model 上，OMP 启动时会报找不到模型。
/// 删掉即回到 OMP 的内置默认角色。
pub(super) fn restore(main_path: &Path) -> Result<RestoreOutcome, String> {
    let mut outcome = RestoreOutcome::default();
    let path = sibling(main_path, "config.yml");
    if !path.exists() {
        return Ok(outcome);
    }
    let existing = read_or_empty(&path);
    let Ok(mut root) = serde_yaml::from_str::<serde_yaml::Value>(&existing) else {
        outcome
            .notes
            .push(format!("{} 不是合法 YAML，未做还原", path.display()));
        return Ok(outcome);
    };
    let prefix = format!("{}/", super::provider_for("omp"));
    let Some(roles) = root
        .get_mut("modelRoles")
        .and_then(|value| value.as_mapping_mut())
    else {
        return Ok(outcome);
    };
    let names: Vec<serde_yaml::Value> = roles.keys().cloned().collect();
    let mut removed = 0usize;
    for name in names {
        let ours = roles
            .get(&name)
            .and_then(|value| value.as_str())
            .map(|value| value.starts_with(&prefix))
            .unwrap_or(false);
        if ours {
            roles.remove(&name);
            removed += 1;
        }
    }
    if removed > 0 {
        let updated = serde_yaml::to_string(&root)
            .map_err(|e| format!("序列化 {} 失败: {e}", path.display()))?;
        write_file(&path, &updated)?;
        eprintln!("[extras] omp: 已删掉 {removed} 个指向我们的角色");
        outcome.files.push(path.display().to_string());
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("anyver-omp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ctx<'a>(main: &'a Path, model: &'a str) -> ExtrasCtx<'a> {
        ExtrasCtx {
            tool_id: "omp",
            main_path: main,
            base_url: "https://api.example.com/v1",
            upstream_url: "https://api.example.com/v1",
            api_key: "sk-test",
            model,
            model_name: model,
            provider: "echobird",
            chosen_protocol: "openai",
            web_search: false,
        }
    }

    /// 首次使用（`config.yml` 还不存在）也要建出来并设上 `modelRoles`，
    /// 否则「设置成功了但 OMP 选不上模型」。
    #[test]
    fn creates_config_yml_when_missing() {
        let dir = temp_dir("missing");
        let main = dir.join("models.yml");
        let out = apply(&ctx(&main, "glm-5.2")).unwrap();
        assert_eq!(out.len(), 1, "应新建 config.yml: {out:?}");
        let doc: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(dir.join("config.yml")).unwrap()).unwrap();
        assert_eq!(doc["modelRoles"]["default"], "echobird/glm-5.2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retargets_our_roles_and_preserves_thinking_suffix() {
        let dir = temp_dir("roles");
        let main = dir.join("models.yml");
        std::fs::write(
            dir.join("config.yml"),
            "modelRoles:\n  default: echobird/old-model:high\n  smol: echobird/old-smol\n  plan: echobird/old-plan:xhigh\n  custom: personal/some-model:max\n",
        )
        .unwrap();

        apply(&ctx(&main, "glm-5.2")).unwrap();

        let doc: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(dir.join("config.yml")).unwrap()).unwrap();
        let roles = doc.get("modelRoles").unwrap();
        assert_eq!(roles["default"], "echobird/glm-5.2:high");
        assert_eq!(roles["smol"], "echobird/glm-5.2");
        assert_eq!(roles["plan"], "echobird/glm-5.2:xhigh");
        // 用户自己指到别处的角色，一个字都不能改
        assert_eq!(roles["custom"], "personal/some-model:max");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_is_created_when_absent_and_colon_not_a_level_is_dropped() {
        let dir = temp_dir("nolevel");
        let main = dir.join("models.yml");
        // `echobird/a:b` 里的 b 不是合法档位 → 当成模型名的一部分，不保留后缀
        std::fs::write(
            dir.join("config.yml"),
            "modelRoles:\n  slow: echobird/a:b\n",
        )
        .unwrap();

        apply(&ctx(&main, "glm-5.2")).unwrap();

        let doc: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(dir.join("config.yml")).unwrap()).unwrap();
        let roles = doc.get("modelRoles").unwrap();
        assert_eq!(roles["slow"], "echobird/glm-5.2");
        assert_eq!(roles["default"], "echobird/glm-5.2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_pointing_elsewhere_is_taken_over() {
        let dir = temp_dir("defaultelse");
        let main = dir.join("models.yml");
        std::fs::write(
            dir.join("config.yml"),
            "modelRoles:\n  default: personal/x:max\n  plan: personal/y\n",
        )
        .unwrap();

        apply(&ctx(&main, "glm-5.2")).unwrap();

        let doc: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(dir.join("config.yml")).unwrap()).unwrap();
        let roles = doc.get("modelRoles").unwrap();
        // `default` 不是我们的 → 必须接管，否则「设了模型但默认模型没变」
        assert_eq!(roles["default"], "echobird/glm-5.2");
        // 其它指向别家的角色仍然不碰
        assert_eq!(roles["plan"], "personal/y");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preserves_unrelated_top_level_keys() {
        let dir = temp_dir("keep");
        let main = dir.join("models.yml");
        std::fs::write(
            dir.join("config.yml"),
            "telemetry: false\nmodelRoles:\n  default: echobird/old\n",
        )
        .unwrap();

        apply(&ctx(&main, "glm-5.2")).unwrap();

        let doc: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(dir.join("config.yml")).unwrap()).unwrap();
        assert_eq!(doc["telemetry"], serde_yaml::Value::Bool(false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_yaml_errors_and_leaves_file_untouched() {
        let dir = temp_dir("badyaml");
        let main = dir.join("models.yml");
        let broken = "modelRoles: [unclosed\n";
        std::fs::write(dir.join("config.yml"), broken).unwrap();
        let err = apply(&ctx(&main, "glm-5.2")).unwrap_err();
        assert!(err.contains("不是合法 YAML"), "{err}");
        assert_eq!(
            std::fs::read_to_string(dir.join("config.yml")).unwrap(),
            broken
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
