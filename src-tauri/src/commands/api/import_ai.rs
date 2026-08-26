//! AI 辅助导入：从 Node（Nest/Nuxt）/ Java（Spring）项目导入接口。
//!
//! 流程：扫描项目目录里的接口相关源文件 → 把文件路径+内容拼接成提示词，
//! 调用 AI 模块（AiConfig）中用户选定的供应商+模型，让 AI 按 Postman
//! Collection v2.1 标准输出 JSON → 程序解析并导入（复用 api_import_postman 链路）。
//!
//! 与外部 AI 工具无关，仅通过用户已配置的供应商（含 api_key、openai_url）发起
//! 一次非流式 chat/completions 请求，与 ai/translate.rs 同一套模式。

use serde_json::Value;

use super::models::ApiProject;
use super::commands::{api_import_postman, load_project_template};

/// 参与 AI 分析的文件类型（按扩展名）。
const INTERESTING_EXTS: &[&str] = &["ts", "tsx", "js", "java", "kt"];
/// 单个文件送入 AI 的内容上限（字符）。
const MAX_FILE_CHARS: usize = 6000;
/// 参与 AI 分析的文件总数上限（超出截断并在提示词中说明）。
const MAX_FILES: usize = 60;
/// 全部文件内容总字符上限。
const MAX_TOTAL_CHARS: usize = 180_000;
/// AI 返回 JSON 上限（与 import.rs IMPORT_MAX_BYTES 接近）。
const AI_JSON_MAX_BYTES: usize = 20 * 1024 * 1024;

/// 从 AI 模块配置解析出可用的 (provider, model_id)。
/// 优先级：显式 provider_id+model_id > provider_id 的 active_model > 第一个可用供应商。
fn resolve_ai_target(
    provider_id: &Option<String>,
    model_id: &Option<String>,
) -> Result<(crate::commands::ai::models::AiProvider, String), String> {
    let cfg = crate::commands::ai::config::load_ai_config();
    let provider = match provider_id {
        Some(pid) => cfg
            .providers
            .iter()
            .find(|p| p.id == *pid)
            .cloned()
            .ok_or_else(|| "选中的 AI 供应商不存在（请先在 AI 模块配置）".to_string())?,
        None => cfg
            .providers
            .iter()
            .find(|p| !p.api_key.is_empty() && !p.openai_url.is_empty())
            .cloned()
            .ok_or_else(|| "未配置可用的 AI 供应商（需要 api_key + OpenAI 端点）".to_string())?,
    };
    if provider.api_key.is_empty() || provider.openai_url.is_empty() {
        return Err(format!("供应商「{}」未配置 api_key 或 OpenAI 端点", provider.name));
    }
    let model = model_id
        .clone()
        .or_else(|| provider.active_model_id.clone())
        .or_else(|| provider.models.first().map(|m| m.id.clone()))
        .ok_or_else(|| format!("供应商「{}」下没有可用模型", provider.name))?;
    Ok((provider, model))
}

/// 扫描目录，收集可能包含接口定义的源文件（过滤 node_modules/target/.git/dist 等）。
fn collect_source_files(dir: &str) -> Result<Vec<(String, String)>, String> {
    let root = std::path::Path::new(dir);
    if !root.is_dir() {
        return Err("目录不存在或不可读".to_string());
    }
    let mut out: Vec<(String, String)> = Vec::new();
    let mut total_chars = 0usize;
    let mut stack = vec![root.to_path_buf()];
    let mut seen_dirs: usize = 0;
    while let Some(d) = stack.pop() {
        seen_dirs += 1;
        if seen_dirs > 4000 {
            break;
        }
        let entries = match std::fs::read_dir(&d) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if p.is_dir() {
                if matches!(name.as_str(), "node_modules" | "target" | "dist" | "build" | ".git" | ".idea" | "out" | "coverage" | ".next" | ".nuxt") {
                    continue;
                }
                if name.starts_with('.') {
                    continue;
                }
                stack.push(p);
            } else if out.len() < MAX_FILES {
                let is_interesting = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| INTERESTING_EXTS.contains(&e))
                    .unwrap_or(false);
                // 只关注可能含接口的文件名特征
                let fname = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                let likely_api = fname.contains("controller") || fname.contains("route") || fname.contains("api");
                let in_api_dir = p
                    .components()
                    .any(|c| c.as_os_str().to_string_lossy().to_lowercase().contains("api"));
                if !is_interesting {
                    continue;
                }
                if !likely_api && !in_api_dir {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&p) {
                    let content: String = content.chars().take(MAX_FILE_CHARS).collect();
                    if total_chars + content.len() > MAX_TOTAL_CHARS {
                        continue;
                    }
                    total_chars += content.len();
                    out.push((p.display().to_string(), content));
                }
            }
        }
    }
    if out.is_empty() {
        return Err("目录中没有找到接口相关源文件（controller/route/api 下的 ts/js/java/kt）".to_string());
    }
    Ok(out)
}

/// 构造 AI 提示词：项目结构 + 源文件内容，要求输出 Postman Collection v2.1 JSON。
fn build_prompt(dir: &str, files: &[(String, String)]) -> String {
    let mut parts = String::new();
    parts.push_str("你是接口分析专家。请分析下面的源代码项目，提取其中所有 HTTP 接口，并输出一个完整的 Postman Collection v2.1 格式 JSON。\n\n");
    parts.push_str("## 项目目录\n");
    parts.push_str(dir);
    parts.push_str("\n\n## 输出要求\n");
    parts.push_str("1. 只输出 JSON，不要输出任何解释、markdown 代码块围栏或前后缀文字。\n");
    parts.push_str("2. JSON 必须是合法的 Postman collection v2.1 结构：{\"info\": {\"name\": ..., \"schema\": \"https://schema.getpostman.com/json/collection/v2.1.0/collection.json\"}, \"item\": [...]}。\n");
    parts.push_str("3. 每个接口为一个 request 条目（可以是带 folder 的嵌套 item 结构，folder 对应模块/控制器名）：\n");
    parts.push_str("   {\"name\": \"接口名\", \"request\": {\"method\": \"GET\", \"url\": {\"raw\": \"http://host/api/users/{id}\", \"path\": [...], \"query\": [...]}, \"header\": [...], \"body\": {...}, \"description\": \"接口说明\"}}\n");
    parts.push_str("4. method 用标准大写：GET/POST/PUT/PATCH/DELETE。\n");
    parts.push_str("5. 路径参数用 {id} 花括号形式（不要用 :id）。\n");
    parts.push_str("6. 请求体：JSON 请求用 {\"mode\": \"raw\", \"raw\": \"{...}\", \"options\": {\"raw\": {\"language\": \"json\"}}}；form 用 {\"mode\": \"urlencoded\", \"urlencoded\": [{\"key\": ..., \"value\": ..., \"enabled\": true}]}。\n");
    parts.push_str("7. header 数组格式：[{\"key\": \"Content-Type\", \"value\": \"application/json\"}]。\n");
    parts.push_str("8. 认证：如果在代码中看到 JWT/Bearer 认证，在 header 里给出 {\"key\": \"Authorization\", \"value\": \"Bearer {{token}}\"} 占位形式。\n");
    parts.push_str("9. 尽力覆盖所有接口，宁多勿漏；每个接口都要有可读的中文 name 和 description。\n");
    parts.push_str("10. 忽略配置类、数据库实体类等非接口文件内容。\n\n");
    parts.push_str(&format!(
        "## 源文件（共 {} 个，超出部分已截断）\n",
        files.len()
    ));
    for (i, (path, content)) in files.iter().enumerate() {
        parts.push_str(&format!(
            "\n### 文件 {}：{}\n```\n{}\n```\n",
            i + 1,
            path,
            content
        ));
    }
    parts
}

/// 从 AI 响应文本中提取 JSON（剥掉可能的 markdown 围栏）。
fn extract_json(text: &str) -> Result<String, String> {
    let t = text.trim();
    let t = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .unwrap_or(t)
        .trim();
    let t = t.strip_suffix("```").unwrap_or(t).trim();
    // 定位第一个 { 和最后一个 }
    let start = t.find('{').ok_or("AI 输出中没有找到 JSON 对象")?;
    let end = t.rfind('}').ok_or("AI 输出中没有找到 JSON 对象")?;
    Ok(t[start..=end].to_string())
}

/// AI 辅助导入：扫描项目目录 → AI 生成 Postman collection JSON → 导入。
#[tauri::command]
pub async fn api_import_with_ai(
    dir: String,
    project_id: String,
    module_id: Option<String>,
    provider_id: Option<String>,
    model_id: Option<String>,
) -> Result<(usize, String), String> {
    // 1. 收集源文件
    let files = collect_source_files(&dir)?;
    crate::exit_log!(
        "[api-ai-import] 收集到 {} 个源文件，目录={}",
        files.len(),
        dir
    );

    // 2. 解析 AI 目标（供应商+模型）
    let (provider, model) = resolve_ai_target(&provider_id, &model_id)?;

    // 3. 读取项目信息（用于提示词中的集合名）
    let project: ApiProject = load_project_template(&project_id)?;

    // 4. 构造提示词并请求
    let mut prompt = build_prompt(&dir, &files);
    prompt.push_str(&format!(
        "\n## 集合名称\n请把 collection 的 name 设为「{}」。\n",
        project.name
    ));

    let base_url = provider.openai_url.trim_end_matches('/').to_string();
    let url = if base_url.ends_with("/v1") {
        format!("{}/chat/completions", base_url)
    } else {
        format!("{}/v1/chat/completions", base_url)
    };
    let body = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": "你是接口分析专家，只输出 Postman Collection v2.1 格式的合法 JSON。" },
            { "role": "user", "content": prompt }
        ],
        "stream": false,
        "temperature": 0.2,
    });

    crate::exit_log!(
        "[api-ai-import] provider={} model={} url={} prompt_chars={}",
        provider.name,
        model,
        url,
        prompt.len()
    );

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", provider.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(600))
        .send()
        .await
        .map_err(|e| format!("AI 请求失败: {}", e))?;

    let status = resp.status();
    let value: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 AI 响应失败: {}", e))?;
    if !status.is_success() {
        let msg = value
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("未知错误");
        return Err(format!("AI 供应商返回错误 ({}): {}", status, msg));
    }
    let content = value
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .ok_or("AI 响应中没有内容字段")?
        .to_string();

    // 5. 提取 JSON 并导入
    let json = extract_json(&content)?;
    if json.len() > AI_JSON_MAX_BYTES {
        return Err(format!(
            "AI 生成的集合过大（{:.1} MB），上限 {:.1} MB",
            json.len() as f64 / 1_048_576.0,
            AI_JSON_MAX_BYTES as f64 / 1_048_576.0
        ));
    }
    let count = api_import_postman(json, project_id, module_id)?;
    crate::exit_log!("[api-ai-import] 导入完成，接口数={}", count);
    Ok((count, format!("{} · {}", provider.name, model)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_strips_markdown_fence() {
        let s = "```json\n{\"info\":{\"name\":\"x\"},\"item\":[]}\n```";
        let out = extract_json(s).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["info"]["name"], "x");
    }

    #[test]
    fn extract_json_takes_first_to_last_brace() {
        let s = "好的，这是结果：{\"info\":{\"name\":\"y\"}} 完毕";
        let out = extract_json(s).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["info"]["name"], "y");
    }

    #[test]
    fn collect_source_files_finds_controllers() {
        let dir = std::env::temp_dir().join(format!("ai-import-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/users")).unwrap();
        std::fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
        std::fs::write(dir.join("src/users/users.controller.ts"), "import { Controller } from '@nestjs/common';\n@Controller('users')\nexport class UsersController {\n  @Get(':id') get(@Param('id') id: string) {}\n}\n").unwrap();
        std::fs::write(dir.join("src/app.module.ts"), "module config, not api").unwrap();
        std::fs::write(dir.join("node_modules/pkg/x.ts"), "ignored").unwrap();
        std::fs::write(dir.join("src/users/user.entity.ts"), "entity, not api").unwrap();

        let files = collect_source_files(dir.to_str().unwrap()).unwrap();
        let names: Vec<String> = files.iter().map(|(p, _)| p.clone()).collect();
        assert!(names.iter().any(|p| p.contains("users.controller.ts")), "应收集 controller 文件");
        assert!(!names.iter().any(|p| p.contains("node_modules")), "应跳过 node_modules");
        assert!(!names.iter().any(|p| p.contains("app.module.ts")), "模块文件不应被收集");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn collect_source_files_missing_dir_errors() {
        assert!(collect_source_files("Z:/no/such/dir-xyz").is_err());
    }
}
