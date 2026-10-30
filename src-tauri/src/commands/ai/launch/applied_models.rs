//! applied models responsibility.
use super::*;

// 注：这里原本还有个 `set_ai_tool_model`（「只保存模型、不启动工具」）。
// 已删除 —— 启动时就会写入配置（官方模式则还原），单独保存一次没有必要，
// 留着只会让「配置什么时候生效」出现两条互相打架的路径。

/// 把某工具还原成「官方模型」：清掉 Kira 写进去的自定义模型配置（含接管前备份的凭据恢复）。
///
/// 供「恢复官方配置」按钮与**官方模式启动**调用。幂等，没写入过时是空操作。
#[tauri::command]
pub fn restore_ai_tool_config(tool_id: String) -> Result<String, String> {
    let tool_config = registry()
        .get_tool_config(&tool_id)
        .ok_or("未知工具")?
        .clone();
    let outcome = crate::commands::ai::tool_config_restore::restore_tool_config(&tool_config)?;
    let mut msg = if outcome.files.is_empty() {
        format!(
            "{} 没有 Kira 写进去的自定义模型，已处于官方配置",
            tool_config.display_name
        )
    } else {
        format!(
            "已把 {} 还原为官方配置（清理了 {} 个文件）",
            tool_config.display_name,
            outcome.files.len()
        )
    };
    for note in outcome.notes {
        msg.push_str(&format!("\n· {note}"));
    }
    Ok(msg)
}

/// 按点号路径取值（模型名里的 "." 已被转义为占位符，取值时还原）。
pub(super) fn get_json_path(doc: &serde_json::Value, path: &str) -> Option<String> {
    let mut cur = doc;
    for part in path.split('.') {
        let key = part.replace(MODEL_NAME_DOT_ESCAPE, ".");
        cur = cur.get(&key)?;
    }
    match cur {
        serde_json::Value::String(s) => Some(s.clone()),
        other if !other.is_null() => Some(other.to_string()),
        _ => None,
    }
}

/// 剥掉 JSONC 的注释（行注释与块注释），**跳过字符串字面量内部**的 `//` 与 `/*`。
/// 只在需要解析工具配置回显时用；写入侧保持原样（保留用户注释）。
pub(crate) fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
                i += 1;
            }
            '/' if i + 1 < chars.len() && chars[i + 1] == '/' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2.min(chars.len());
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    out
}

/// 文本扫描：`model = "x"` / `model: x` 形态取一次（toml、yaml 回显兜底）。
pub(super) fn scan_text_key(text: &str, key: &str) -> Option<String> {
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let (left, right) = match line.split_once('=').or_else(|| line.split_once(':')) {
            Some((l, r)) => (l.trim(), r.trim()),
            None => continue,
        };
        if left != key {
            continue;
        }
        let value = right.trim_matches('"').trim_matches('\'').trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// 工具配置文件里**当前写定**的模型（回显用）。
///
/// 为什么必须从**工具配置文件**读，而不是只看我们自己记录的「上次启动配置」：
/// 工具真正读的是它自己的配置文件，用户也可能在别处改过它。只信自己的记录会出现
/// 「界面显示没选模型（看着像在用官方配置），实际工具还在用我们上次写进去的自定义模型」——
/// 用户看到的与生效的完全是两回事。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct AppliedModelsDto {
    /// 主模型（写入侧 `model` / `modelName` 模板对应的键）
    pub model: Option<String>,
    /// 小模型 / fallback（`fallbackModel` 模板对应的键）
    pub fallback_model: Option<String>,
}

/// 按声明回读「当前写定」的模型。
///
/// 只认 `configFile.write` 里值模板为 `model` / `modelName` / `fallbackModel` 的路径，
/// 并跳过三类不是「当前选的是哪个模型」的键：
/// - `env.*`：只注入进程环境，不落盘；
/// - 含 `#` 的兄弟文件路径（`auth.json#…`）；
/// - 含 `{…}` 占位符的路径（如 opencode 的 `provider.X.models.{model_name}.name`）——
///   那是「为每个模型建一条定义」，谁先被遍历到就返回谁，纯属碰运气。
///
/// `model` 模板优先于 `modelName`：前者是顶层「当前模型」，后者常见于模型定义块。
pub(super) fn read_applied_models(
    cfg: &ConfigFileDef,
    resolved: &std::path::Path,
    tool_id: &str,
) -> AppliedModelsDto {
    // 自定义写入器读法自成一套（WorkBuddy 的 models[0].id、Claude Desktop 的 3P profile），
    // 且都只有一个「主模型」概念，没有 fallback。
    if let Some(writer) = cfg.custom_writer(tool_id) {
        return AppliedModelsDto {
            model: crate::commands::ai::tool_config_custom::read_model(&writer, resolved),
            fallback_model: None,
        };
    }
    let Some(write_map) = cfg.write.as_ref() else {
        return AppliedModelsDto::default();
    };
    let Ok(text) = fs::read_to_string(resolved) else {
        return AppliedModelsDto::default();
    };
    let doc = match cfg.format.as_str() {
        "json" | "jsonc" => serde_json::from_str::<serde_json::Value>(&strip_jsonc(&text)).ok(),
        _ => None,
    };

    let mut out = AppliedModelsDto::default();
    // 两趟：先按 `model` 模板定位（权威），没有再看 `modelName`。HashMap 迭代无序，
    // 单趟会让 opencode 的模型定义块抢在顶层 `model` 之前命中。
    for wanted in ["model", "modelName"] {
        if out.model.is_some() {
            break;
        }
        for (path, template) in write_map {
            if template != wanted || is_non_selection_path(path) {
                continue;
            }
            let found = doc
                .as_ref()
                .and_then(|d| get_json_path(d, path))
                .or_else(|| scan_text_key(&text, path.rsplit('.').next().unwrap_or(path)));
            if found.is_some() {
                out.model = found;
                break;
            }
        }
    }
    for (path, template) in write_map {
        if template != "fallbackModel" || is_non_selection_path(path) {
            continue;
        }
        let found = doc
            .as_ref()
            .and_then(|d| get_json_path(d, path))
            .or_else(|| scan_text_key(&text, path.rsplit('.').next().unwrap_or(path)));
        if found.is_some() {
            out.fallback_model = found;
            break;
        }
    }
    out
}

/// 该键是否「不是当前选择的模型」（env 注入 / 兄弟文件 / 模型定义占位符）。
pub(super) fn is_non_selection_path(path: &str) -> bool {
    path.starts_with("env.")
        || path.contains('#')
        || path.contains('{')
        || path.starts_with("fileEnv.")
}

/// 读取工具配置文件里当前写定的模型（主模型 + 小模型）。
///
/// 前端用它把「上次选了哪个模型」回显出来 —— 不回显的话，界面看着像用官方配置，
/// 而工具实际还在用我们写进去的模型。
#[tauri::command]
pub fn get_ai_tool_models(tool_id: String) -> Result<AppliedModelsDto, String> {
    let tool_config = registry()
        .get_tool_config(&tool_id)
        .ok_or("未知工具")?
        .clone();
    let Some(cfg) = tool_config.config_file.as_ref() else {
        return Ok(AppliedModelsDto::default());
    };
    let resolved = resolve_declared_config_path(cfg);
    Ok(read_applied_models(cfg, &resolved, &tool_id))
}
