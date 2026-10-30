use super::*;

// ---- 规则覆写 / 通用文件读写 / 规则集转换（对齐 clash-party getRuleStr / setRuleStr / convertMrsRuleset）----

/// 读取某订阅的规则覆写文件（rules/<id>.yaml）
#[tauri::command]
pub fn mihomo_get_rule_str(state: State<'_, MihomoState>, id: String) -> Result<String, String> {
    let p = rule_override_path(&state.data_dir, &id);
    if !p.exists() {
        return Ok(String::new());
    }
    std::fs::read_to_string(p).map_err(|e| format!("读取规则覆写失败: {e}"))
}

/// 写入某订阅的规则覆写文件，写入后热重载配置
#[tauri::command]
pub async fn mihomo_set_rule_str(
    app: AppHandle,
    state: State<'_, MihomoState>,
    id: String,
    content: String,
) -> Result<(), String> {
    // 先校验 YAML 合法性，避免写入坏文件导致内核起不来
    if !content.trim().is_empty() {
        serde_yaml::from_str::<serde_yaml::Value>(&content)
            .map_err(|e| format!("规则覆写不是合法 YAML: {e}"))?;
    }
    let p = rule_override_path(&state.data_dir, &id);
    atomic_write(&p, &content).map_err(|e| format!("写入规则覆写失败: {e}"))?;
    let current = state.profile_config.lock().unwrap().current.clone();
    if current == id {
        reload_config(&app, Arc::clone(&*state)).await?;
    }
    Ok(())
}

/// 读取规则覆写（结构化：{ prepend, append, delete }），前端无需自带 YAML 解析
#[tauri::command]
pub fn mihomo_get_rule_override(
    state: State<'_, MihomoState>,
    id: String,
) -> Result<Value, String> {
    let p = rule_override_path(&state.data_dir, &id);
    let empty = serde_json::json!({ "prepend": [], "append": [], "delete": [] });
    if !p.exists() {
        return Ok(empty);
    }
    let content = std::fs::read_to_string(&p).map_err(|e| format!("读取规则覆写失败: {e}"))?;
    if content.trim().is_empty() {
        return Ok(empty);
    }
    let v: serde_yaml::Value =
        serde_yaml::from_str(&content).map_err(|e| format!("规则覆写不是合法 YAML: {e}"))?;
    let json: Value = serde_json::to_value(v).map_err(|e| e.to_string())?;
    let take = |k: &str| -> Value {
        json.get(k)
            .and_then(|x| x.as_array())
            .map(|a| Value::Array(a.clone()))
            .unwrap_or_else(|| Value::Array(vec![]))
    };
    Ok(serde_json::json!({
        "prepend": take("prepend"),
        "append": take("append"),
        "delete": take("delete"),
    }))
}

/// 写入规则覆写（结构化），写入后若为当前订阅则热重载
#[tauri::command]
pub async fn mihomo_set_rule_override(
    app: AppHandle,
    state: State<'_, MihomoState>,
    id: String,
    data: Value,
) -> Result<(), String> {
    let yaml_value: serde_yaml::Value =
        serde_yaml::to_value(&data).map_err(|e| format!("序列化规则覆写失败: {e}"))?;
    let content = serde_yaml::to_string(&yaml_value).map_err(|e| format!("生成 YAML 失败: {e}"))?;
    let p = rule_override_path(&state.data_dir, &id);
    atomic_write(&p, &content).map_err(|e| format!("写入规则覆写失败: {e}"))?;
    let current = state.profile_config.lock().unwrap().current.clone();
    if current == id {
        reload_config(&app, Arc::clone(&*state)).await?;
    }
    Ok(())
}

/// 读取任意文本文件（供前端编辑器使用）
#[tauri::command]
pub fn mihomo_get_file_str(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败: {e}"))
}

/// 写入任意文本文件（原子写）
#[tauri::command]
pub fn mihomo_set_file_str(path: String, content: String) -> Result<(), String> {
    atomic_write(Path::new(&path), &content).map_err(|e| format!("写入文件失败: {e}"))
}

/// 用内核把 yaml/text 规则集转换为 mrs 二进制格式（对齐 convertMrsRuleset）
#[tauri::command]
pub fn mihomo_convert_mrs_ruleset(
    state: State<'_, MihomoState>,
    rule_type: String,
    input_format: String,
    input_path: String,
    output_path: String,
) -> Result<String, String> {
    let core = resolve_core_path(&state.app_config.lock().unwrap().clone());
    if !core.exists() {
        return Err("未找到 mihomo 内核".into());
    }
    let out = hidden_cmd(&core)
        .args([
            "convert-ruleset",
            &rule_type,
            &input_format,
            &input_path,
            &output_path,
        ])
        .output()
        .map_err(|e| format!("执行规则集转换失败: {e}"))?;
    if out.status.success() {
        Ok(output_path)
    } else {
        let mut msg = String::from_utf8_lossy(&out.stdout).to_string();
        msg.push_str(&String::from_utf8_lossy(&out.stderr));
        Err(format!("规则集转换失败: {}", msg.trim()))
    }
}

/// 仅退出界面、保留内核后台运行（对齐 clash-party quitWithoutCore 的反向语义：退出应用但不杀内核）
#[tauri::command]
pub fn mihomo_detach_core(state: State<'_, MihomoState>) -> Result<(), String> {
    state
        .stop_flag
        .store(true, std::sync::atomic::Ordering::SeqCst);
    // 用户显式脱离托管：退出时不再补杀该核心
    state
        .launched_by_us
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let mut g = state.child.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(child) = g.take() {
        // 丢弃句柄但不 kill，内核继续运行
        std::mem::forget(child);
    }
    Ok(())
}

/// 读取 JS 覆写脚本的最近一次执行日志
#[tauri::command]
pub fn mihomo_get_override_exec_log(state: State<'_, MihomoState>, id: String) -> Vec<String> {
    let p = factory::override_log_path(&state.data_dir, &id);
    match std::fs::read_to_string(p) {
        Ok(s) => s
            .lines()
            .map(|l| l.to_string())
            .filter(|l| !l.trim().is_empty())
            .collect(),
        Err(_) => Vec::new(),
    }
}
