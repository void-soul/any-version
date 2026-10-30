use super::*;

// ─── 导入导出 ───

/// Postman 导入上限：50 MB（超大集合拒绝解析，避免 OOM）。
const IMPORT_MAX_BYTES: usize = 50 * 1024 * 1024;

#[tauri::command]
pub fn api_import_postman(
    json: String,
    project_id: String,
    module_id: Option<String>,
) -> Result<usize, String> {
    if json.len() > IMPORT_MAX_BYTES {
        return Err(format!(
            "Postman 集合文件过大（{:.1} MB），上限 {:.1} MB",
            json.len() as f64 / 1_048_576.0,
            IMPORT_MAX_BYTES as f64 / 1_048_576.0
        ));
    }
    let imp = crate::commands::api::import::parse_postman_collection(&json)?;
    // 集合级变量导入为新的变量集合（若项目已有同名则复用）
    if !imp.variables.is_empty() {
        let mut vars = serde_json::Map::new();
        for v in imp.variables.iter().filter(|v| !v.key.is_empty()) {
            vars.insert(v.key.clone(), serde_json::Value::String(v.value.clone()));
        }
        let name = collection_name(&json).unwrap_or_else(|| "导入变量".to_string());
        let existing = api_list_environments(project_id.clone())?;
        if let Some(env) = existing.iter().find(|e| e.name == name) {
            let mut merged = env.variables.clone();
            for (k, v) in vars {
                merged.insert(k, v);
            }
            let _ = api_update_environment(ApiEnvironment {
                id: env.id.clone(),
                project_id: project_id.clone(),
                name: env.name.clone(),
                variables: merged,
                sort_order: env.sort_order,
            });
        } else {
            let _ = api_create_environment(project_id.clone(), name, vars);
        }
    }
    let count = insert_drafts(project_id, module_id, imp.drafts)?;
    Ok(count)
}

/// 从 Postman JSON 里取集合名。
pub(super) fn collection_name(json: &str) -> Option<String> {
    let root: serde_json::Value = serde_json::from_str(json).ok()?;
    root.get("info")?
        .get("name")?
        .as_str()
        .map(|s| s.to_string())
}

#[tauri::command]
pub fn api_export_postman(project_id: String) -> Result<String, String> {
    let project = api_list_projects()?
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or("项目不存在")?;
    let modules = api_list_modules(project_id.clone())?
        .into_iter()
        .map(|m| (m.id, m.name))
        .collect::<Vec<_>>();
    let endpoints = api_list_endpoints(project_id.clone(), None)?;
    // 当前激活变量集合导出为集合变量
    let variables = api_list_environments(project_id)?
        .into_iter()
        .find(|e| Some(e.id.clone()) == project.active_env_id)
        .map(|e| {
            e.variables
                .iter()
                .map(|(k, v)| KeyValueItem {
                    key: k.clone(),
                    value: match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    },
                    enabled: true,
                    description: String::new(),
                    from_template: false,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    crate::commands::api::import::export_postman_collection(
        &project.name,
        &modules,
        &endpoints,
        &variables,
    )
}

/// 导入单条 cURL 命令（支持 -X/-H/-d/-F/-u/-b 等常用参数）。
#[tauri::command]
pub fn api_import_curl(
    curl: String,
    project_id: String,
    module_id: Option<String>,
) -> Result<ApiEndpoint, String> {
    let draft = crate::commands::api::import::parse_curl_command(&curl)?;
    insert_draft(&project_id, module_id, draft)
}

/// 导出 OpenAPI 3.0 文档（JSON）。
#[tauri::command]
pub fn api_export_openapi(project_id: String) -> Result<String, String> {
    let project = api_list_projects()?
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or("项目不存在")?;
    let modules = api_list_modules(project_id.clone())?
        .into_iter()
        .map(|m| (m.id, m.name))
        .collect::<Vec<_>>();
    let endpoints = api_list_endpoints(project_id.clone(), None)?;
    let env_variables = api_list_environments(project_id)?
        .into_iter()
        .find(|e| Some(e.id.clone()) == project.active_env_id)
        .map(|e| e.variables)
        .unwrap_or_default();
    crate::commands::api::import::export_openapi(
        &project.name,
        &project.description,
        &modules,
        &endpoints,
        &env_variables,
    )
}

#[tauri::command]
pub async fn api_import_swagger(
    source: String,
    project_id: String,
    module_id: Option<String>,
) -> Result<usize, String> {
    let json = crate::commands::api::import::fetch_swagger_source(&source).await?;
    let (_module, _base, drafts) = crate::commands::api::import::parse_swagger(&json)?;
    let count = insert_drafts(project_id, module_id, drafts)?;
    Ok(count)
}

#[tauri::command]
pub fn api_scan_framework(
    dir: String,
    framework: String,
    project_id: String,
    module_id: Option<String>,
) -> Result<usize, String> {
    let drafts = crate::commands::api::import::scan_framework(&dir, &framework)?;
    let count = insert_drafts(project_id, module_id, drafts)?;
    Ok(count)
}

/// 把导入候选写入数据库：按模块名查找/创建模块，再插入接口。
pub(super) fn insert_drafts(
    project_id: String,
    module_id: Option<String>,
    drafts: Vec<crate::commands::api::import::EndpointDraft>,
) -> Result<usize, String> {
    let mut count = 0;
    for draft in drafts {
        insert_draft(&project_id, module_id.clone(), draft)?;
        count += 1;
    }
    Ok(count)
}

/// 写入单条导入候选，返回落库后的接口（cURL 导入后前端可直接选中）。
pub(super) fn insert_draft(
    project_id: &str,
    module_id: Option<String>,
    draft: crate::commands::api::import::EndpointDraft,
) -> Result<ApiEndpoint, String> {
    let target_module: Option<String> = if let Some(mid) = &module_id {
        if mid.is_empty() {
            find_or_create_module(project_id, &draft.module)?
        } else {
            Some(mid.clone())
        }
    } else {
        find_or_create_module(project_id, &draft.module)?
    };
    let ep = ApiEndpoint {
        id: db::new_id("ep"),
        project_id: project_id.to_string(),
        module_id: target_module,
        name: draft.name,
        method: draft.method,
        url: draft.url,
        headers: draft.headers,
        query_params: draft.query_params,
        path_params: Vec::new(),
        body: draft.body,
        body_type: draft.body_type,
        body_form: draft.body_form,
        body_urlencoded: draft.body_urlencoded,
        body_graphql_query: draft.body_graphql_query,
        body_graphql_variables: draft.body_graphql_variables,
        authorization: draft.authorization,
        cookies: Vec::new(),
        settings: draft.settings,
        response_comment: String::new(),
        is_favorite: false,
        extract_rules: Vec::new(),
        description: String::new(),
        docs_md: draft.docs_md,
        timeout_ms: 15000,
        created_at: now_ts(),
        updated_at: now_ts(),
    };
    api_create_endpoint(ep)
}
