//! config plan responsibility.
use super::*;

pub(super) fn write_tool_config_generic(
    tool_config: &ToolConfig,
    model_id: Option<&str>,
    claimed_model: Option<&str>,
    base_url: &str,
    api_key: &str,
    upstream_url: &str,
    fallback_model_id: Option<&str>,
    fallback_masquerade_model: Option<&str>,
    one_m_context: bool,
    fallback_one_m_context: bool,
    _proxy_mode: bool,
    custom_params: &[ModelCustomParam],
    custom_param_values: &HashMap<String, String>,
    web_search: bool,
    chosen_protocol: &str,
) -> Result<(), String> {
    let cfg = match &tool_config.config_file {
        Some(c) => c,
        None => return Ok(()),
    };

    let resolved_path = resolve_declared_config_path(cfg);

    // 自定义写入器整份接管：schema 不是「路径 → 值」能表达的（WorkBuddy 的 models.json）。
    // 放在 write_map 之前——这类工具本来就没有 write 映射。
    if let Some(writer) = cfg.custom_writer(&tool_config.id) {
        eprintln!(
            "[config_file] 使用自定义写入器: {} → {}",
            writer,
            resolved_path.display()
        );
        return crate::commands::ai::tool_config_custom::write_config(
            &writer,
            &resolved_path,
            &crate::commands::ai::tool_config_custom::ModelWrite {
                model: model_id.unwrap_or(""),
                claimed: claimed_model.unwrap_or(""),
                base_url,
                api_key,
                upstream_url,
                // 1M 开关与通用写入器同一套门槛（工具支持 + 用户勾选）
                one_m: one_m_context && tool_config.support_one_m_context,
            },
        );
    }

    let write_map = match &cfg.write {
        Some(w) => w,
        None => return Ok(()),
    };

    // 确保父目录存在
    if let Some(parent) = resolved_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // 仅 Anthropic 协议工具才追加 [1m] 后缀（对齐原 env 注入行为）
    let apply_one_m = one_m_context
        && tool_config.support_one_m_context
        && (tool_config.api_protocol == "anthropic" || tool_config.api_protocol == "both");
    // fallback/小模型可独立勾选 1M
    let apply_one_m_fb = fallback_one_m_context
        && tool_config.support_one_m_context
        && (tool_config.api_protocol == "anthropic" || tool_config.api_protocol == "both");

    // 组装待写入的 (目标文件, 路径, 值) 列表；值可以是标量字符串或 JSON 数组（如 pi 的 models）
    // 目标文件默认为 configFile.path；路径带 "文件#子路径" 前缀时可写入同目录的兄弟文件
    // （如 omp 的 config.yml#modelRoles.default → ~/.omp/agent/config.yml）。
    let main_config_path = resolved_path.clone();
    let mut writes: Vec<(String, String, serde_json::Value)> = Vec::new();
    // 声明模型名 C 优先；否则回退到实际模型 B
    let effective_model_id = claimed_model.or(model_id);
    let has_model = effective_model_id.is_some();
    let model = effective_model_id
        .map(|m| format_model_name_with_ctx(m, tool_config, apply_one_m))
        .unwrap_or_default();
    let model_name = model.split('/').next_back().unwrap_or(&model).to_string();
    // 真实模型 B 的 id（去前缀）：**窗口 / 能力判定必须按它来**，不能按上面的声明名 C ——
    // 伪装生效时 C 是官方名（如 `gpt-5.1-codex`），拿它查表必然落空（见 ExtrasCtx 文档）。
    let real_model = model_id
        .map(|m| format_model_name_with_ctx(m, tool_config, apply_one_m))
        .unwrap_or_default();
    let real_model_name = real_model
        .split('/')
        .next_back()
        .unwrap_or(&real_model)
        .to_string();
    // fallback/小模型：声明名（伪装优先，否则实际模型 B）。无 fallback 时为 None。
    let fallback_claimed = fallback_model_id.and_then(|fm| {
        if fm.is_empty() {
            return None;
        }
        match fallback_masquerade_model {
            Some(c) if !c.is_empty() => {
                Some(format_model_name_with_ctx(c, tool_config, apply_one_m_fb))
            }
            _ => Some(format_model_name_with_ctx(fm, tool_config, apply_one_m_fb)),
        }
    });

    // 小模型（fallback）的模型名（不带 provider 前缀），用于注册进 provider 的 models 映射
    let fallback_model_name = fallback_claimed
        .as_deref()
        .map(|m| m.split('/').next_back().unwrap_or(m).to_string())
        .unwrap_or_default();

    for (raw_path, value_template) in write_map {
        // env.* 键只应作为进程环境变量注入（见 build_env_vars），不应写入工具配置文件：
        // - opencode 及其 fork（mimocode / kilocode / zcode）的配置 schema 不识别顶层 env 键，
        //   写入会触发 "Unrecognized key: env" 导致工具启动失败；
        // - 进程 env 注入对 CLI 已经够用。
        if raw_path.starts_with("env.") {
            eprintln!(
                "[config_file] skip {} (env 仅注入进程环境，不写配置文件)",
                raw_path
            );
            continue;
        }
        // fileEnv.* = 写进**配置文件**的 env.<KEY>。Claude Code / Qwen Code 这类工具是从
        // settings.json 的 env 块读端点与凭据的：只注入进程环境的话，「只保存模型」（不启动、
        // 不起代理）在这些工具上等于什么都没写。抄 EchoBird claudecode.rs（它把 env 块落盘）。
        let path: String = match raw_path.strip_prefix("fileEnv.") {
            Some(key) => format!("env.{key}"),
            None => raw_path.clone(),
        };
        // 动态键名替换：{model_name} → 主模型名；{fallback_model_name} → 小模型名
        // 模型名里的 "." 先转义为占位符，避免被 set_json_path 当成路径分隔符误拆
        if path.contains("{fallback_model_name}") && fallback_claimed.is_none() {
            eprintln!("[config_file] skip {} (no fallback model)", path);
            continue;
        }
        // "文件#子路径"：子路径写主文件的兄弟文件；文件部分可带 `~` / `%VAR%`（绝对路径，
        // 如 MiMo 桌面端的 `%APPDATA%\Xiaomi MiMo\preferences.json` 就在别的目录）。
        let target_file = if let Some((file, _)) = path.split_once('#') {
            resolve_write_target_file(&main_config_path, file)
        } else {
            main_config_path.clone()
        };
        let sub_path = path
            .split_once('#')
            .map(|(_, s)| s.to_string())
            .unwrap_or_else(|| path.clone());
        let resolved_path = sub_path
            .replace(
                "{model_name}",
                &model_name.replace('.', MODEL_NAME_DOT_ESCAPE),
            )
            .replace(
                "{fallback_model_name}",
                &fallback_model_name.replace('.', MODEL_NAME_DOT_ESCAPE),
            );
        let value: serde_json::Value = match value_template.as_str() {
            "model" | "modelName" if !has_model => {
                eprintln!("[config_file] skip {} (no model)", resolved_path);
                continue;
            }
            "model" => serde_json::json!(model.clone()),
            "modelName" => serde_json::json!(model_name.clone()),
            "fallbackModel" => match &fallback_claimed {
                Some(v) => serde_json::json!(v.clone()),
                None => {
                    eprintln!("[config_file] skip {} (no fallback model)", resolved_path);
                    continue;
                }
            },
            "fallbackModelName" => match &fallback_claimed {
                Some(_) => serde_json::json!(fallback_model_name.clone()),
                None => {
                    eprintln!("[config_file] skip {} (no fallback model)", resolved_path);
                    continue;
                }
            },
            // pi：把主模型（及可选 fallback）注册进 providers.<p>.models 数组
            // 官方 models.json 要求自定义 provider 在 models 数组里声明可用模型（id/name），
            // 否则 /model 列表为空、--model 也可能找不到模型。
            "piModels" => {
                if !has_model {
                    eprintln!("[config_file] skip {} (no model)", resolved_path);
                    continue;
                }
                let mut arr = vec![serde_json::json!({ "id": model_name, "name": model_name })];
                if let Some(fb) = &fallback_claimed {
                    let fb_name = fb.split('/').next_back().unwrap_or(fb).to_string();
                    arr.push(serde_json::json!({ "id": fb_name, "name": fb_name }));
                }
                serde_json::Value::Array(arr)
            }
            "baseUrl" => serde_json::json!(base_url.to_string()),
            // 布尔字面量（如 pi 的 compat.supportsDeveloperRole=false）
            "boolFalse" => serde_json::json!(false),
            "boolTrue" => serde_json::json!(true),
            // `web_search` 曾经由 `webSearchLive` 模板写，现在归 tool_config_extras 管
            // （要按**上游域名**判断，模板表达不了「按域名决定值」）。
            // omp (Oh My Pi)：providers.<p>.api 协议标识（openai-completions / anthropic-messages）
            "ompApiProtocol" => serde_json::json!(if chosen_protocol == "anthropic" {
                "anthropic-messages"
            } else {
                "openai-completions"
            }),
            // omp：无凭证的供应商写 auth: none（有 key 时不写，交给 apiKey 模板）
            "ompAuthNone" => {
                if !api_key.is_empty() {
                    eprintln!(
                        "[config_file] skip {} (有 apiKey，不需要 auth: none)",
                        resolved_path
                    );
                    continue;
                }
                serde_json::json!("none")
            }
            // `modelRoles` 曾经由 `ompModelRole` 模板写 default，现在整张表归
            // tool_config_extras 管：既要逐个角色改写、又要保留各角色的 `:档位` 后缀，
            // 通用映射先跑会把用户设的后缀冲掉（见 tool_config_extras/omp.rs）。
            "apiKey" => {
                // API Key 为空时不写入配置文件，避免写入空字符串被解析器判定为非法凭证
                if api_key.is_empty() {
                    eprintln!(
                        "[config_file] skip {} (empty apiKey, 不写入)",
                        resolved_path
                    );
                    continue;
                }
                serde_json::json!(api_key.to_string())
            }
            // 包名随出站协议切换（openscience）：写死一个会让另一种协议直接失效。
            "npmForProtocol" => serde_json::json!(if chosen_protocol == "anthropic" {
                "@ai-sdk/anthropic"
            } else {
                "@ai-sdk/openai-compatible"
            }),
            // ZCode 的 provider 判别符（作用等同于 opencode 的 npm 字段）
            "zcodeKind" => serde_json::json!(if chosen_protocol == "anthropic" {
                "anthropic"
            } else {
                "openai-compatible"
            }),
            // `json:<字面量>`：用 JSON 表达数组 / 对象 / 数字——「路径 → 标量」说不清的东西
            // （Qwen Code 的 modelProviders[]、ZCode 的 modalities[]、onboarding 的版本号…）。
            // `{model}` `{modelName}` `{baseUrl}` `{apiKey}` 在字符串内按 JSON 规则转义替换。
            other if other.starts_with("json:") => {
                match render_json_template(&other[5..], &model, &model_name, base_url, api_key) {
                    Ok(v) => v,
                    Err(e) => {
                        return Err(format!(
                            "write 映射 {} 的 json: 模板非法: {e}",
                            resolved_path
                        ))
                    }
                }
            }
            "" => serde_json::json!(""),
            other => serde_json::json!(other.to_string()),
        };
        let log = if value.is_string() {
            mask_secret(value.as_str().unwrap())
        } else {
            "<json>".to_string()
        };
        eprintln!("[config_file] set {} = {}", resolved_path, log);
        writes.push((target_file.display().to_string(), resolved_path, value));
    }

    // 追加模型自定义的「config 目标」启动参数（写入工具配置文件指定 JSON 路径）
    for cp in custom_params {
        if cp.target != "config" {
            continue;
        }
        let path = match &cp.config_path {
            Some(p) if !p.is_empty() => p.clone(),
            _ => continue, // 未给 config_path 则无法落盘，跳过
        };
        let value = custom_param_values
            .get(&cp.key)
            .cloned()
            .or_else(|| cp.default_value.clone())
            .unwrap_or_default();
        eprintln!(
            "[config_file] set (custom) {} = {}",
            path,
            mask_secret(&value)
        );
        writes.push((
            main_config_path.display().to_string(),
            path,
            serde_json::json!(value),
        ));
    }

    // 按目标文件分组（主配置文件 + "文件#子路径" 的兄弟文件），逐文件落盘
    let mut by_file: std::collections::BTreeMap<String, Vec<(String, serde_json::Value)>> =
        std::collections::BTreeMap::new();
    for (file, sub, v) in writes {
        by_file.entry(file).or_default().push((sub, v));
    }

    for (file, ws) in by_file {
        let p = PathBuf::from(&file);
        let existing = if p.exists() {
            fs::read_to_string(&p).unwrap_or_default()
        } else {
            String::new()
        };
        // 格式按**目标文件**判定：兄弟文件常与主文件不同（codex 的 config.toml + auth.json），
        // 一律按主文件的 format 写会把 auth.json 写成 TOML。
        let format = write_format_for(&p, &cfg.format);
        eprintln!(
            "[config_file] 目标路径: {} (format={:?})",
            p.display(),
            format
        );
        // 先清掉声明里点名的遗留键（声明改过后旧键会留在文件里继续生效），再写新值。
        let stale = stale_keys_for_file(cfg, &main_config_path, &p);
        let existing =
            crate::commands::ai::tool_config_restore::strip_stale_keys(&existing, format, &stale)?;
        if !stale.is_empty() {
            eprintln!("[config_file] 已清理遗留键: {:?}", stale);
        }
        // 兄弟文件（如 codex 的 auth.json）里本来就可能有用户自己的
        // 官方凭据，第一次接管前先存一份：勾「使用官方模型」还原时把它放回去，
        // 否则「用 Kira 跑一次」就把用户的官方 Key 冲掉了（EchoBird 同样做法：codex-auth.bak.json）。
        if p != main_config_path {
            if let Err(e) = crate::commands::ai::tool_config_restore::backup_before_overwrite(
                &tool_config.id,
                &p,
            ) {
                eprintln!(
                    "[config_file] ⚠ 备份 {} 失败（继续写入）: {}",
                    p.display(),
                    e
                );
            }
        }
        match format {
            WriteFormat::Toml => write_toml_config(&p, &existing, &ws, &tool_config.id)?,
            WriteFormat::Yaml => write_yaml_config(&p, &existing, &ws)?,
            // $schema 只往主配置文件里补，兄弟文件（auth.json 之类）不该被塞 schema
            WriteFormat::Json if p == main_config_path => {
                write_json_config(&p, &existing, &ws, cfg.schema.as_deref(), tool_config)?
            }
            WriteFormat::Json => write_json_config(&p, &existing, &ws, None, tool_config)?,
        }
        eprintln!("[config_file] ✓ 已写入配置到 {}", p.display());

        // 记进「配置账本」：存下我们在这个文件里写了什么 + 接管前的原值。
        // 这样以后能查出「配置后来被谁改了」（用户手改 / 别的工具反向覆写 /
        // 官方客户端升级重写）。接管前的原值只在该键**第一次**被接管时记。
        let applied_keys: Vec<crate::commands::ai::ledger::ManagedKey> = ws
            .iter()
            .map(|(sub, v)| crate::commands::ai::ledger::ManagedKey {
                path: sub.clone(),
                value: ledger_value_text(v),
            })
            .collect();
        if !applied_keys.is_empty() {
            let mut stash: std::collections::BTreeMap<String, Option<String>> =
                std::collections::BTreeMap::new();
            for (sub, _) in &ws {
                stash.insert(
                    sub.clone(),
                    crate::commands::ai::tool_config_restore::read_key_for_ledger(&p, sub, format),
                );
            }
            crate::commands::ai::ledger::record_write(
                &tool_config.id,
                &p.display().to_string(),
                &applied_keys,
                &stash,
            );
        }
    }

    // 通用映射写完之后，再跑「平铺路径说不清」的附加结构化写入：数组内按 id 查找合并、
    // 按上游域名决定写哪个值、逐角色重定向、顺手生成被引用的附属文件（见 tool_config_extras）。
    // 没选模型时不跑：这些语义全都以「当前选的是哪个模型」为前提，空模型名会写出坏值。
    if has_model {
        // 用户显式声明要写的 config 路径（target=config 的 configPath）：附加写入器对这些
        // 键让位，否则写死值会覆盖用户的选择（「界面上填了、启动时被改回去」= 摆设）。
        let user_configured_paths: std::collections::HashSet<String> = custom_params
            .iter()
            .filter(|cp| cp.target == "config")
            .filter_map(|cp| cp.config_path.as_deref())
            .map(|p| p.to_string())
            .filter(|p| !p.is_empty())
            .collect();
        let extras_ctx = crate::commands::ai::tool_config_extras::ExtrasCtx {
            tool_id: &tool_config.id,
            main_path: &main_config_path,
            base_url,
            // 域名判断必须用真上游：代理模式下 base_url 是 127.0.0.1，拿它做域名匹配必然落空
            upstream_url,
            api_key,
            model: &model,
            model_name: &model_name,
            real_model_name: &real_model_name,
            provider: crate::commands::ai::tool_config_extras::provider_for(&tool_config.id),
            chosen_protocol,
            web_search,
            user_configured_paths: &user_configured_paths,
        };
        for file in crate::commands::ai::tool_config_extras::apply_extras(&extras_ctx)? {
            eprintln!("[config_file] ✓ 附加写入: {file}");
        }
    }

    Ok(())
}

/// 取声明里属于**该文件**的遗留键（`removeKeys`）：`文件#子路径` 与主文件两种写法。
///
/// 与 `write` 的 `file#sub` 写法保持同一套解析，否则「声明要删 env_key」会因为
/// 目标文件对不上而静默失效。
pub(super) fn stale_keys_for_file(
    cfg: &crate::commands::ai_registry::ConfigFileDef,
    main: &std::path::Path,
    target: &std::path::Path,
) -> Vec<String> {
    let mut out = Vec::new();
    for raw in &cfg.remove_keys {
        match raw.split_once('#') {
            Some((file, sub)) => {
                let resolved = resolve_write_target_file(main, file);
                if resolved == target {
                    out.push(sub.to_string());
                }
            }
            None => {
                if main == target {
                    out.push(raw.clone());
                }
            }
        }
    }
    out
}

/// 解析工具配置文件的实际落盘路径：`~` → HOME（Windows 用 USERPROFILE），
/// 再按声明处理「目录环境变量覆盖 / XDG_CONFIG_HOME / 同 stem 扩展名择优」。
/// OpenCode v2 支持 `OPENCODE_CONFIG_DIR`，且只在 `opencode.jsonc` 里读配置（抄自 EchoBird c6f4bc25）。
pub(crate) fn resolve_declared_config_path(
    cfg: &crate::commands::ai_registry::ConfigFileDef,
) -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    let declared_path = if cfg.path.starts_with("~/") {
        home.join(&cfg.path[2..])
    } else {
        PathBuf::from(&cfg.path)
    };
    let env_dirs: Vec<Option<String>> = cfg
        .path_env_dirs
        .iter()
        .map(|name| std::env::var(name).ok())
        .collect();
    crate::commands::ai::tool_config_path::resolve_config_path(
        &declared_path,
        &env_dirs,
        cfg.xdg_subdir.as_deref(),
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        &cfg.prefer_existing_extensions,
    )
}

/// 目标文件的写入格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteFormat {
    Json,
    Toml,
    Yaml,
}

/// 按目标文件扩展名挑写入格式；认不出来时用工具声明的 `format`。
pub(crate) fn write_format_for(path: &std::path::Path, declared: &str) -> WriteFormat {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("json") | Some("jsonc") => WriteFormat::Json,
        Some("toml") => WriteFormat::Toml,
        Some("yaml") | Some("yml") => WriteFormat::Yaml,
        _ => match declared {
            "jsonc" | "json" => WriteFormat::Json,
            "toml" => WriteFormat::Toml,
            "yaml" | "yml" => WriteFormat::Yaml,
            _ => WriteFormat::Json,
        },
    }
}

/// 解析 `write` 映射里「文件#子路径」的文件部分。
///
/// - 空 → 主配置文件自身；
/// - 展开后是绝对路径（写 `~` 或 `%APPDATA%` 这类模板）→ 直接用；
/// - 其余相对路径 → 主配置文件的同目录兄弟文件（历史行为，如 `auth.json`、`settings.json`）。
pub(crate) fn resolve_write_target_file(main_config_path: &std::path::Path, file: &str) -> PathBuf {
    let file = file.trim();
    if file.is_empty() {
        return main_config_path.to_path_buf();
    }
    let expanded = PathBuf::from(crate::commands::ai::tool_paths::expand_tool_path(file));
    if expanded.is_absolute() {
        return expanded;
    }
    main_config_path
        .parent()
        .map(|p| p.join(&expanded))
        .unwrap_or_else(|| main_config_path.to_path_buf())
}
