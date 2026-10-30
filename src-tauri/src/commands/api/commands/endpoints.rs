use super::*;

// ─── 接口 CRUD ───

#[tauri::command]
pub fn api_list_endpoints(
    project_id: String,
    module_id: Option<String>,
) -> Result<Vec<ApiEndpoint>, String> {
    db::with_db(|conn| {
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(mid) =
            module_id
        {
            (
                format!("SELECT {} FROM api_endpoints WHERE project_id = ?1 AND module_id = ?2 ORDER BY created_at", EP_COLS),
                vec![Box::new(project_id), Box::new(mid)],
            )
        } else {
            (
                format!(
                    "SELECT {} FROM api_endpoints WHERE project_id = ?1 ORDER BY created_at",
                    EP_COLS
                ),
                vec![Box::new(project_id)],
            )
        };
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                |row| db::endpoint_row(row),
            )
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    })
}

#[tauri::command]
pub fn api_get_endpoint(endpoint_id: String) -> Result<ApiEndpoint, String> {
    db::with_db(|conn| {
        conn.query_row(
            &format!("SELECT {} FROM api_endpoints WHERE id = ?1", EP_COLS),
            rusqlite::params![endpoint_id],
            |row| db::endpoint_row(row),
        )
        .map_err(|e| format!("接口不存在: {}", e))
    })
}

/// 读取项目模板（通用 Headers / Params / Body）。
pub fn load_project_template(project_id: &str) -> Result<ApiProject, String> {
    db::with_db(|conn| {
        conn.query_row(
            "SELECT id, name, description, active_env_id, common_headers, common_params, common_body, created_at, updated_at FROM api_projects WHERE id = ?1",
            rusqlite::params![project_id],
            |row| db::project_row(row),
        )
        .map_err(|e| format!("读取项目模板失败: {}", e))
    })
}

/// 把模板条目合并进接口的指定字段（按 key 去重；新条目打上 from_template 标记）。
pub(super) fn merge_template_items(items: &mut Vec<KeyValueItem>, template: &[KeyValueItem]) {
    for t in template {
        if !t.enabled {
            continue;
        }
        if let Some(existing) = items.iter_mut().find(|x| x.key == t.key) {
            // 已存在同名条目：标记为模板继承（值由模板控制）
            existing.from_template = true;
            existing.value = t.value.clone();
        } else {
            let mut item = t.clone();
            item.from_template = true;
            items.push(item);
        }
    }
}

/// 重同步接口中所有 from_template 条目的值（改模板值 → 接口同步）。
pub(super) fn resync_template_items(items: &mut [KeyValueItem], template: &[KeyValueItem]) {
    for item in items.iter_mut() {
        if item.from_template {
            if let Some(t) = template.iter().find(|t| t.key == item.key) {
                item.value = t.value.clone();
                item.enabled = t.enabled;
            }
        }
    }
}

/// 模板条目改名后，把接口中所有继承自旧名的条目同步改名。
pub(super) fn rename_template_items(items: &mut [KeyValueItem], old_key: &str, new_key: &str) {
    for item in items.iter_mut() {
        if item.from_template && item.key == old_key {
            item.key = new_key.to_string();
        }
    }
}

/// 把通用 Body 模板合并进 form-data 字段（text 类型条目，打 from_template 标记）。
pub(super) fn merge_template_form_items(items: &mut Vec<FormDataItem>, template: &[KeyValueItem]) {
    for t in template {
        if !t.enabled {
            continue;
        }
        if let Some(existing) = items.iter_mut().find(|x| x.key == t.key) {
            existing.from_template = true;
            existing.value = t.value.clone();
        } else {
            items.push(FormDataItem {
                key: t.key.clone(),
                value: t.value.clone(),
                enabled: true,
                kind: "text".to_string(),
                file_path: String::new(),
                description: String::new(),
                from_template: true,
            });
        }
    }
}

/// 重同步 form-data 中 from_template 条目的值。
pub(super) fn resync_template_form_items(items: &mut [FormDataItem], template: &[KeyValueItem]) {
    for item in items.iter_mut() {
        if item.from_template {
            if let Some(t) = template.iter().find(|t| t.key == item.key) {
                item.value = t.value.clone();
                item.enabled = t.enabled;
                item.kind = "text".to_string();
                item.file_path.clear();
            }
        }
    }
}

/// form-data 中模板项改名。
pub(super) fn rename_template_form_items(items: &mut [FormDataItem], old_key: &str, new_key: &str) {
    for item in items.iter_mut() {
        if item.from_template && item.key == old_key {
            item.key = new_key.to_string();
        }
    }
}

#[tauri::command]
pub fn api_create_endpoint(ep: ApiEndpoint) -> Result<ApiEndpoint, String> {
    let id = if ep.id.is_empty() {
        db::new_id("ep")
    } else {
        ep.id
    };
    let ts = now_ts();
    let project = load_project_template(&ep.project_id)?;
    // 接口模板：合并项目级通用 Headers / Params / Body（urlencoded）进新接口
    let mut headers = ep.headers.clone();
    merge_template_items(&mut headers, &project.common_headers);
    let mut query_params = ep.query_params.clone();
    merge_template_items(&mut query_params, &project.common_params);
    let mut body_urlencoded = ep.body_urlencoded.clone();
    merge_template_items(&mut body_urlencoded, &project.common_body);
    let mut body_form = ep.body_form.clone();
    merge_template_form_items(&mut body_form, &project.common_body);
    db::with_db(|conn| {
        conn.execute(
            &format!("INSERT INTO api_endpoints ({}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26)", EP_COLS),
            rusqlite::params![
                id, ep.project_id, ep.module_id, ep.name, ep.method, ep.url,
                serde_json::to_string(&headers).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&query_params).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&ep.path_params).unwrap_or_else(|_| "[]".into()),
                ep.body, ep.body_type, ep.description, ep.docs_md, ep.timeout_ms, ts, ts,
                serde_json::to_string(&body_form).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&body_urlencoded).unwrap_or_else(|_| "[]".into()),
                ep.body_graphql_query, ep.body_graphql_variables,
                serde_json::to_string(&ep.authorization).unwrap_or_else(|_| "{}".into()),
                serde_json::to_string(&ep.cookies).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&ep.settings).unwrap_or_else(|_| "{}".into()),
                ep.response_comment, ep.is_favorite,
                serde_json::to_string(&ep.extract_rules).unwrap_or_else(|_| "[]".into())
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    api_get_endpoint(id)
}

#[tauri::command]
pub fn api_update_endpoint(ep: ApiEndpoint) -> Result<(), String> {
    // 重同步模板值：接口中 from_template 的条目不允许修改（名称/值均由项目模板控制）
    let project = load_project_template(&ep.project_id)?;
    let mut headers = ep.headers.clone();
    resync_template_items(&mut headers, &project.common_headers);
    let mut query_params = ep.query_params.clone();
    resync_template_items(&mut query_params, &project.common_params);
    let mut body_urlencoded = ep.body_urlencoded.clone();
    resync_template_items(&mut body_urlencoded, &project.common_body);
    let mut body_form = ep.body_form.clone();
    resync_template_form_items(&mut body_form, &project.common_body);
    db::with_db(|conn| {
        conn.execute(
            &format!("UPDATE api_endpoints SET module_id=?1, name=?2, method=?3, url=?4, headers=?5, query_params=?6, path_params=?7, body=?8, body_type=?9, description=?10, docs_md=?11, timeout_ms=?12, updated_at=?13, body_form=?14, body_urlencoded=?15, body_graphql_query=?16, body_graphql_variables=?17, authorization=?18, cookies=?19, settings=?20, response_comment=?21, is_favorite=?22, extract_rules=?23 WHERE id=?24"),
            rusqlite::params![
                ep.module_id, ep.name, ep.method, ep.url,
                serde_json::to_string(&headers).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&query_params).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&ep.path_params).unwrap_or_else(|_| "[]".into()),
                ep.body, ep.body_type, ep.description, ep.docs_md, ep.timeout_ms, now_ts(),
                serde_json::to_string(&body_form).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&body_urlencoded).unwrap_or_else(|_| "[]".into()),
                ep.body_graphql_query, ep.body_graphql_variables,
                serde_json::to_string(&ep.authorization).unwrap_or_else(|_| "{}".into()),
                serde_json::to_string(&ep.cookies).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&ep.settings).unwrap_or_else(|_| "{}".into()),
                ep.response_comment, ep.is_favorite,
                serde_json::to_string(&ep.extract_rules).unwrap_or_else(|_| "[]".into()),
                ep.id
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub fn api_delete_endpoint(endpoint_id: String) -> Result<(), String> {
    db::with_db(|conn| {
        conn.execute(
            "DELETE FROM api_endpoints WHERE id = ?1",
            rusqlite::params![endpoint_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}
