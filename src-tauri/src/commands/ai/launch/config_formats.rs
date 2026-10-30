//! config formats responsibility.
use super::*;

/// 渲染 `json:` 模板：把占位符替换成按 JSON 规则转义的值后整体解析。
///
/// 模板写错要**报错**而不是静默写半份配置——写错的文件比不写更难排查。
pub(super) fn render_json_template(
    template: &str,
    model: &str,
    model_name: &str,
    base_url: &str,
    api_key: &str,
) -> Result<serde_json::Value, String> {
    let escape = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let rendered = template
        // 先长后短：`{modelName}` 必须先于 `{model}` 替换，否则会被截成 `Name}`
        .replace("{modelName}", &escape(model_name))
        .replace("{model}", &escape(model))
        .replace("{baseUrl}", &escape(base_url))
        .replace("{apiKey}", &escape(api_key));
    serde_json::from_str(&rendered).map_err(|e| format!("{e}（渲染后: {rendered}）"))
}

/// 掩码打印含密钥的值
pub(crate) fn mask_secret(v: &str) -> String {
    if v.is_empty() {
        String::new()
    } else if v.len() <= 12 {
        "***".to_string()
    } else {
        format!("{}...{}", &v[..8], &v[v.len() - 4..])
    }
}

/// 写入 JSON / JSONC 配置文件（serde_json；jsonc 读取失败时按空文档处理，保留写入内容）
/// 注意：对于 fallback 模型相关的环境变量（如 ANTHROPIC_DEFAULT_HAIKU_MODEL），如果已有值不为空，
/// 则不覆盖，避免影响其他正在运行的工具实例。
pub(super) fn write_json_config(
    path: &PathBuf,
    existing: &str,
    writes: &[(String, serde_json::Value)],
    schema: Option<&str>,
    _tool_config: &ToolConfig,
) -> Result<(), String> {
    let mut doc: serde_json::Value = if existing.trim().is_empty() {
        serde_json::json!({})
    } else {
        // 先当纯 JSON 解析；失败再剥注释当 JSONC 解析。以前只做前者，于是
        // mimocode.jsonc / opencode.jsonc 这类带注释的配置文件解析失败 → 被当成空文档
        // **整份覆盖**，用户自己的配置全没了。
        serde_json::from_str(existing)
            .or_else(|_| serde_json::from_str(&strip_jsonc(existing)))
            .unwrap_or(serde_json::json!({}))
    };
    if let Some(s) = schema {
        doc.as_object_mut()
            .unwrap()
            .entry("$schema")
            .or_insert(serde_json::json!(s));
    }
    for (p, v) in writes {
        // 对于 fallback 模型环境变量，如果已有值不为空，则不覆盖
        // 这样可以避免影响其他正在运行的工具实例
        if p.starts_with("env.ANTHROPIC_DEFAULT_") && p.ends_with("_MODEL") {
            if let Some(existing_val) = doc.get(p).and_then(|v| v.as_str()) {
                if !existing_val.is_empty() {
                    eprintln!(
                        "[config_file] skip {} (已有值={}, 避免影响其他实例)",
                        p, existing_val
                    );
                    continue;
                }
            }
        }
        set_json_path(&mut doc, p, v.clone());
    }

    // 清除受管 provider 的 models 映射中因历史 dot bug 产生的畸形条目
    // （形如 { "LongCat-2": { "0": { "name": ... } } }），避免工具配置解析失败
    cleanup_broken_model_entries(&mut doc, writes);

    // 清除本工具管理的、但本次未写入的残留键。
    // 合并写入不会删除旧键，导致切换供应商/模型后旧模型字段残留在配置里，干扰本次启动。
    cleanup_managed_model_keys(&mut doc, writes);

    let content =
        serde_json::to_string_pretty(&doc).map_err(|e| format!("序列化配置失败: {}", e))?;
    crate::commands::config::atomic_write_file(path, content.as_bytes())
        .map_err(|e| format!("写入 {} 失败: {}", path.display(), e))
}

/// 移除本工具管理的、但本次未写入的残留键。
///
/// 动态基于写入列表与通用 AI 供应商环境变量前缀池（ANTHROPIC_, OPENAI_, GEMINI_, DEVECO_ 等）
/// 清理属于受管范围但本次未写入的旧模型/Auth键，无硬编码适用于所有 CLI 工具。
pub(super) fn cleanup_managed_model_keys(
    doc: &mut serde_json::Value,
    writes: &[(String, serde_json::Value)],
) {
    // 写进配置文件 env 块的键有两种声明：`env.X`（进程环境变量，此时也计进来，
    // 因为它同样占用同一个键名）与 `fileEnv.X`（映射后落盘为 `env.X`）。
    let current_env_keys: std::collections::HashSet<String> = writes
        .iter()
        .filter_map(|(p, _)| {
            p.strip_prefix("fileEnv.")
                .or_else(|| p.strip_prefix("env."))
                .map(|k| k.to_string())
        })
        .collect();

    let mut managed_prefixes: Vec<String> = vec![
        "ANTHROPIC_".to_string(),
        "OPENAI_".to_string(),
        "GEMINI_".to_string(),
        "DEVECO_".to_string(),
        "KILO_".to_string(),
    ];

    for key in &current_env_keys {
        if let Some(pos) = key.find('_') {
            let prefix = key[..=pos].to_string();
            if !managed_prefixes.contains(&prefix) {
                managed_prefixes.push(prefix);
            }
        }
    }

    if let Some(env_obj) = doc.get_mut("env").and_then(|v| v.as_object_mut()) {
        let stale: Vec<String> = env_obj
            .keys()
            .filter(|k| {
                let is_managed = managed_prefixes.iter().any(|p| k.starts_with(p))
                    && (k.contains("MODEL")
                        || k.contains("AUTH_TOKEN")
                        || k.contains("API_KEY")
                        || k.contains("BASE_URL"));
                is_managed && !current_env_keys.contains(*k)
            })
            .cloned()
            .collect();
        for k in stale {
            env_obj.remove(&k);
            eprintln!("[config_file] cleanup stale env.{}", k);
        }
    }

    let writes_top_model = writes.iter().any(|(p, _)| p == "model");
    if !writes_top_model {
        if let Some(obj) = doc.as_object_mut() {
            if obj.remove("model").is_some() {
                eprintln!("[config_file] cleanup stale top-level model");
            }
        }
    }
}

/// 清理受管 provider 的 `models` 映射中，因历史 bug（模型名含 "." 被误拆成嵌套键）
/// 产生的畸形条目。畸形条目表现为：值不是合法的模型定义对象（缺少 `name` 键，
/// 而是形如 `{ "0": { "name": ... } }` 的嵌套结构）。仅删除畸形条目，保留合法条目
/// （含当前正写入的模型与历史正常注册的模型），避免工具配置整体解析失败。
pub(super) fn cleanup_broken_model_entries(
    doc: &mut serde_json::Value,
    writes: &[(String, serde_json::Value)],
) {
    // 从本次写入收集受管 provider（路径形如 provider.<P>.models.<M>.name）
    let mut managed: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (p, _) in writes {
        if let Some(rest) = p.strip_prefix("provider.") {
            let parts: Vec<&str> = rest.split('.').collect();
            if parts.len() >= 4 && parts[1] == "models" && parts.last() == Some(&"name") {
                let provider = parts[0].to_string();
                let model_key = parts[2].replace(MODEL_NAME_DOT_ESCAPE, ".");
                managed.insert(provider, model_key);
            }
        }
    }
    if managed.is_empty() {
        return;
    }
    if let Some(provider_obj) = doc.get_mut("provider").and_then(|v| v.as_object_mut()) {
        for (provider, current_model) in &managed {
            if let Some(models_obj) = provider_obj
                .get_mut(provider)
                .and_then(|v| v.as_object_mut())
                .and_then(|p| p.get_mut("models"))
                .and_then(|m| m.as_object_mut())
            {
                let stale: Vec<String> = models_obj
                    .iter()
                    .filter(|(k, v)| {
                        // 保留当前模型；其余若值不是合法模型定义（缺 name 键）则视为畸形，删除
                        **k != *current_model && !(v.is_object() && v.get("name").is_some())
                    })
                    .map(|(k, _)| k.clone())
                    .collect();
                for k in stale {
                    models_obj.remove(&k);
                    eprintln!(
                        "[config_file] cleanup broken model entry provider.{}.models.{}",
                        provider, k
                    );
                }
            }
        }
    }
}

/// JSON 值 → TOML 值（**保留类型**）。
///
/// 布尔 / 数字绝不能降级成字符串：codex 的 `requires_openai_auth` 要布尔，
/// 写成 `requires_openai_auth = "true"` 会让整份 config.toml 加载失败 ——
/// 报 `invalid type: string "true", expected a boolean`，工具直接起不来。
/// 数组 / 对象退回 JSON 文本（codex 等 TOML 工具的写入值均为标量，不会用到后者）。
pub(super) fn toml_value(v: &serde_json::Value) -> toml_edit::Value {
    match v {
        serde_json::Value::String(s) => toml_edit::Value::from(s.clone()),
        serde_json::Value::Bool(b) => toml_edit::Value::from(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml_edit::Value::from(i)
            } else if let Some(f) = n.as_f64() {
                toml_edit::Value::from(f)
            } else {
                toml_edit::Value::from(n.to_string())
            }
        }
        other => toml_edit::Value::from(other.to_string()),
    }
}

/// 写入 YAML 配置文件（如 omp 的 models.yml / config.yml）。
/// 读取失败或空文件按空文档处理；dotted path 语义与 set_json_path 一致。
pub(super) fn write_yaml_config(
    path: &PathBuf,
    existing: &str,
    writes: &[(String, serde_json::Value)],
) -> Result<(), String> {
    let mut doc: serde_yaml::Value = if existing.trim().is_empty() {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    } else {
        serde_yaml::from_str(existing)
            .unwrap_or_else(|_| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
    };
    for (p, v) in writes {
        set_yaml_path(&mut doc, p, json_value_to_yaml(v));
    }
    let content =
        serde_yaml::to_string(&doc).map_err(|e| format!("序列化 YAML 配置失败: {}", e))?;
    crate::commands::config::atomic_write_file(path, content.as_bytes())
        .map_err(|e| format!("写入 {} 失败: {}", path.display(), e))
}

/// serde_json::Value → serde_yaml::Value（配置模板产物都是 JSON 值）
pub(super) fn json_value_to_yaml(v: &serde_json::Value) -> serde_yaml::Value {
    match v {
        serde_json::Value::Null => serde_yaml::Value::Null,
        serde_json::Value::Bool(b) => serde_yaml::Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_yaml::Value::Number(i.into())
            } else if let Some(f) = n.as_f64() {
                serde_yaml::Value::Number(serde_yaml::Number::from(f))
            } else {
                serde_yaml::Value::Null
            }
        }
        serde_json::Value::String(s) => serde_yaml::Value::String(s.clone()),
        serde_json::Value::Array(a) => {
            serde_yaml::Value::Sequence(a.iter().map(json_value_to_yaml).collect())
        }
        serde_json::Value::Object(o) => {
            let mut m = serde_yaml::Mapping::new();
            for (k, val) in o {
                m.insert(
                    serde_yaml::Value::String(k.clone()),
                    json_value_to_yaml(val),
                );
            }
            serde_yaml::Value::Mapping(m)
        }
    }
}

/// 按 dotted path 设置 YAML 值（与 set_json_path 同语义，模型名 "." 转义占位符同理还原）
pub(super) fn set_yaml_path(doc: &mut serde_yaml::Value, path: &str, value: serde_yaml::Value) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return;
    }
    let mut cur = doc;
    for (i, p) in parts.iter().enumerate() {
        let key = serde_yaml::Value::String(p.replace(MODEL_NAME_DOT_ESCAPE, "."));
        if i == parts.len() - 1 {
            if !cur.is_mapping() {
                *cur = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
            }
            cur.as_mapping_mut().unwrap().insert(key, value);
            return;
        }
        if !cur.is_mapping() {
            *cur = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
        }
        let map = cur.as_mapping_mut().unwrap();
        let next = map
            .get(&key)
            .cloned()
            .unwrap_or_else(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
        let next = if next.is_mapping() {
            next
        } else {
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
        };
        map.insert(key.clone(), next.clone());
        cur = map.get_mut(&key).unwrap();
    }
}

/// 写入 TOML 配置文件（支持顶层 key 和 dotted keys 如 `model_providers.x.base_url`）。
///
/// **语义合并**（抄 CodexPlusPlus `relay_config.rs`，用 `toml_edit::DocumentMut`）：
/// 把现有文件解析成文档树，只改写受管的键，其余内容（注释、未托管的 `mcp_servers`、
/// `[[hooks]]` 数组表、用户自己的 `[profiles.*]`）原样保留。
///
/// 为什么不用文本级替换：文本替换要自己重建 `[table]` 上下文，遇到「嵌套表 + 数组表头
/// （`[[hooks.X]]`）+ 跨文件重复表头」这三类输入必然错位 —— 轻则把键写进别的表作用域
/// （`[model_providers.custom]` 下多出 `model_providers.anyversion.*`，codex 报
/// "provider anyversion not found"），重则写出非法 TOML。
pub(super) fn write_toml_config(
    path: &PathBuf,
    existing: &str,
    writes: &[(String, serde_json::Value)],
    tool_id: &str,
) -> Result<(), String> {
    // 仅处理非 env.* 的键（env.* 走环境变量注入）；布尔 / 数字保留原类型
    //（数组/对象退回 JSON 文本，codex 等工具的写入值均为标量，不会传入）
    let toml_writes: Vec<(String, toml_edit::Value)> = writes
        .iter()
        .filter(|(p, _)| !p.starts_with("env."))
        .map(|(p, v)| (p.clone(), toml_value(v)))
        .collect();
    if toml_writes.is_empty() {
        return Ok(());
    }

    let mut doc = parse_toml_lenient(existing);
    for (key_path, value) in &toml_writes {
        set_toml_path(&mut doc, key_path, value);
    }

    let content = doc.to_string();
    // 写回前先自校验：解析不通过说明我们改坏了，宁可报错也不能把用户的配置文件写成废纸
    content.parse::<toml_edit::DocumentMut>().map_err(|e| {
        format!(
            "合并后的 TOML 无法通过解析，已放弃写入 {}: {}",
            path.display(),
            e
        )
    })?;

    // Codex 的 config.toml 里常有用户自己的 mcp_servers / [[hooks]] / profiles，
    // 第一次接管前先存一份（只存一次，保留最原始那份）
    if let Err(e) = crate::commands::ai::tool_config_restore::backup_before_overwrite(tool_id, path)
    {
        eprintln!(
            "[config_file] ⚠ 备份 {} 失败（继续写入）: {}",
            path.display(),
            e
        );
    }
    crate::commands::config::atomic_write_file(path, content.as_bytes())
        .map_err(|e| format!("写入 {} 失败: {}", path.display(), e))
}

/// 解析 TOML，容忍「重复表头」这类非法输入。
///
/// 抄 CodexPlusPlus `relay_config.rs:1716`：真实用户的 config.toml 里确实存在重复的
/// `[mcp_servers]` 表头（手工编辑或旧版本工具留下的），整体解析会直接失败。
/// 这里退化为「按根表头切块 → 逐块解析 → 再合并」：标量后写覆盖，数组表（`[[x]]`）累加，
/// 从而在不丢内容的前提下得到一个合法的文档树。
pub(crate) fn parse_toml_lenient(existing: &str) -> toml_edit::DocumentMut {
    if let Ok(doc) = existing.parse::<toml_edit::DocumentMut>() {
        return doc;
    }
    let mut merged = toml_edit::DocumentMut::new();
    for block in split_toml_blocks(existing) {
        let Ok(block_doc) = block.parse::<toml_edit::DocumentMut>() else {
            continue;
        };
        merge_toml_table(merged.as_table_mut(), block_doc.as_table());
    }
    merged
}

/// 按根表头（`[a]` / `[[a]]`）把文本切成块，文件头部（表外键）作为第一块。
pub(super) fn split_toml_blocks(existing: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in existing.lines() {
        let trimmed = line.trim_start();
        // 只按**根级**表头切（行首就是 `[`），缩进的表头属于当前块
        if line.starts_with('[') && (trimmed.starts_with('[') || trimmed.starts_with("[[")) {
            if !current.trim().is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        blocks.push(current);
    }
    blocks
}

/// 递归合并 TOML 表：`target` 被 `source` 覆盖（标量直接换，表递归，数组表累加）。
pub(super) fn merge_toml_table(target: &mut toml_edit::Table, source: &toml_edit::Table) {
    // 数组表（`[[hooks.x]]`）在 DocumentMut 里表现为 Table 内 Item::ArrayOfTables
    for (key, value) in source.iter() {
        match value {
            toml_edit::Item::Table(src_table) => match target.get_mut(key) {
                Some(toml_edit::Item::Table(dst_table)) => {
                    merge_toml_table(dst_table, src_table);
                }
                _ => {
                    target.insert(key, toml_edit::Item::Table(src_table.clone()));
                }
            },
            toml_edit::Item::ArrayOfTables(src_arr) => match target.get_mut(key) {
                Some(toml_edit::Item::ArrayOfTables(dst_arr)) => {
                    for t in src_arr.iter() {
                        dst_arr.push(t.clone());
                    }
                }
                _ => {
                    target.insert(key, toml_edit::Item::ArrayOfTables(src_arr.clone()));
                }
            },
            other => {
                target.insert(key, other.clone());
            }
        }
    }
}

/// 按点分路径写入标量，自动创建中间表（已存在则原地覆盖）。
pub(super) fn set_toml_path(
    doc: &mut toml_edit::DocumentMut,
    path: &str,
    value: &toml_edit::Value,
) {
    // 还原模型名中被转义的 "."（如 LongCat-2__DOT__0 → LongCat-2.0）
    let parts: Vec<String> = path
        .split('.')
        .map(|p| p.replace(MODEL_NAME_DOT_ESCAPE, "."))
        .collect();
    let Some((leaf, parents)) = parts.split_last() else {
        return;
    };
    let mut table = doc.as_table_mut();
    for parent in parents {
        let entry = table.entry(parent.as_str());
        let item = entry.or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
        // 已存在但不是表（比如同名标量）→ 换成空表，保证后续路径能建起来
        if !item.is_table() {
            *item = toml_edit::Item::Table(toml_edit::Table::new());
        }
        table = item.as_table_mut().expect("just ensured it is a table");
    }
    // 已存在的键只换值、保留 decor：TOML 里「键上方的注释」是挂在该键值对的
    // prefix decor 上的，`insert` 会把整对（含注释）一起换掉 —— 用户写的注释就没了。
    if let Some(toml_edit::Item::Value(existing)) = table.get_mut(leaf.as_str()) {
        let decor = existing.decor().clone();
        *existing = value.clone();
        *existing.decor_mut() = decor;
        return;
    }
    table.insert(leaf.as_str(), toml_edit::value(value.clone()));
}

/// 根据点分路径设置 JSON 文档中的值（自动创建中间对象）
pub(super) fn set_json_path(doc: &mut serde_json::Value, path: &str, value: serde_json::Value) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return;
    }
    let mut cur = doc;
    for (i, p) in parts.iter().enumerate() {
        // 还原模型名中被转义的 "."（如 LongCat-2__DOT__0 → LongCat-2.0）
        let key = p.replace(MODEL_NAME_DOT_ESCAPE, ".");
        if i == parts.len() - 1 {
            if !cur.is_object() {
                *cur = serde_json::json!({});
            }
            cur.as_object_mut().unwrap().insert(key, value);
            return;
        }
        if !cur.is_object() {
            *cur = serde_json::json!({});
        }
        if cur.get(&key).is_none() || !cur[&key].is_object() {
            cur.as_object_mut()
                .unwrap()
                .insert(key.clone(), serde_json::json!({}));
        }
        cur = cur.as_object_mut().unwrap().get_mut(&key).unwrap();
    }
}
