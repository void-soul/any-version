use super::*;

// ─── 初始化 ───

#[tauri::command]
pub fn api_init() -> Result<(), String> {
    db::init_db()
}

// ─── 项目 CRUD ───

#[tauri::command]
pub fn api_list_projects() -> Result<Vec<ApiProject>, String> {
    db::with_db(|conn| {
        let mut stmt = conn
            .prepare("SELECT id, name, description, active_env_id, common_headers, common_params, common_body, created_at, updated_at FROM api_projects ORDER BY created_at")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| db::project_row(row))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    })
}

#[tauri::command]
pub fn api_create_project(name: String, description: String) -> Result<ApiProject, String> {
    let id = db::new_id("prj");
    let ts = now_ts();
    db::with_db(|conn| {
        conn.execute(
            "INSERT INTO api_projects (id, name, description, active_env_id, common_headers, common_params, common_body, created_at, updated_at) VALUES (?1, ?2, ?3, NULL, '[]', '[]', '[]', ?4, ?4)",
            rusqlite::params![id, name, description, ts],
        )
        .map_err(|e| e.to_string())?;
        // 默认环境（正式版/测试版）——模块不自动创建，按需添加
        for (i, env_name) in ["正式版", "测试版"].iter().enumerate() {
            conn.execute(
                "INSERT INTO api_environments (id, project_id, name, variables, sort_order) VALUES (?1, ?2, ?3, '{}', ?4)",
                rusqlite::params![db::new_id("env"), id, env_name, i as i64],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })?;
    api_list_projects()?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or("项目创建失败".to_string())
}

#[tauri::command]
pub fn api_update_project(project: ApiProject) -> Result<(), String> {
    let old = load_project_template(&project.id)?;
    // 收集改名映射：优先按 key 相同（值被改）排除，剩下按索引对应识别改名
    let mut renames: Vec<(String, String)> = Vec::new();
    // 按索引对应：新条目在第 i 位，若旧列表第 i 位存在且 key 不同、且旧 key 在新列表中已消失 → 改名
    for (i, n) in project.common_headers.iter().enumerate() {
        if let Some(o) = old.common_headers.get(i) {
            if o.key != n.key && !project.common_headers.iter().any(|x| x.key == o.key) {
                renames.push((o.key.clone(), n.key.clone()));
            }
        }
    }
    for (i, n) in project.common_params.iter().enumerate() {
        if let Some(o) = old.common_params.get(i) {
            if o.key != n.key && !project.common_params.iter().any(|x| x.key == o.key) {
                renames.push((o.key.clone(), n.key.clone()));
            }
        }
    }
    for (i, n) in project.common_body.iter().enumerate() {
        if let Some(o) = old.common_body.get(i) {
            if o.key != n.key && !project.common_body.iter().any(|x| x.key == o.key) {
                renames.push((o.key.clone(), n.key.clone()));
            }
        }
    }
    // 保存项目
    db::with_db(|conn| {
        conn.execute(
            "UPDATE api_projects SET name = ?1, description = ?2, active_env_id = ?3, common_headers = ?4, common_params = ?5, common_body = ?6, updated_at = ?7 WHERE id = ?8",
            rusqlite::params![
                project.name, project.description, project.active_env_id,
                serde_json::to_string(&project.common_headers).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&project.common_params).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&project.common_body).unwrap_or_else(|_| "[]".into()),
                now_ts(), project.id
            ],
        )
        .map_err(|e| e.to_string())?;
        // 同步模板值/改名到所有接口的继承参数（无条件执行：值变化也要跟随）
        {
            let ep_rows: Vec<(String, String, String, String, String)> = {
                let mut stmt = conn
                    .prepare("SELECT id, headers, query_params, body_urlencoded, body_form FROM api_endpoints WHERE project_id = ?1")
                    .map_err(|e| e.to_string())?;
                let rows = stmt
                    .query_map(rusqlite::params![project.id], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, String>(4)?,
                        ))
                    })
                    .map_err(|e| e.to_string())?;
                let mut v = Vec::new();
                for r in rows {
                    v.push(r.map_err(|e| e.to_string())?);
                }
                v
            };
            for (ep_id, headers_raw, params_raw, body_raw, body_form_raw) in ep_rows {
                let mut headers: Vec<KeyValueItem> =
                    serde_json::from_str(&headers_raw).unwrap_or_default();
                let mut params: Vec<KeyValueItem> =
                    serde_json::from_str(&params_raw).unwrap_or_default();
                let mut body: Vec<KeyValueItem> =
                    serde_json::from_str(&body_raw).unwrap_or_default();
                let mut body_form: Vec<FormDataItem> =
                    serde_json::from_str(&body_form_raw).unwrap_or_default();
                // 值同步：模板值变化 → 所有继承项跟随（不覆盖接口自建项）
                let headers_before = headers.clone();
                let params_before = params.clone();
                let body_before = body.clone();
                let body_form_before = body_form.clone();
                resync_template_items(&mut headers, &project.common_headers);
                resync_template_items(&mut params, &project.common_params);
                resync_template_items(&mut body, &project.common_body);
                resync_template_form_items(&mut body_form, &project.common_body);
                // 改名同步：模板项改名 → 接口继承项跟随
                for (old_key, new_key) in &renames {
                    rename_template_items(&mut headers, old_key, new_key);
                    rename_template_items(&mut params, old_key, new_key);
                    rename_template_items(&mut body, old_key, new_key);
                    rename_template_form_items(&mut body_form, old_key, new_key);
                }
                if headers != headers_before
                    || params != params_before
                    || body != body_before
                    || body_form != body_form_before
                {
                    conn.execute(
                        "UPDATE api_endpoints SET headers = ?1, query_params = ?2, body_urlencoded = ?3, body_form = ?4 WHERE id = ?5",
                        rusqlite::params![
                            serde_json::to_string(&headers).unwrap_or_else(|_| "[]".into()),
                            serde_json::to_string(&params).unwrap_or_else(|_| "[]".into()),
                            serde_json::to_string(&body).unwrap_or_else(|_| "[]".into()),
                            serde_json::to_string(&body_form).unwrap_or_else(|_| "[]".into()),
                            ep_id
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(())
    })
}

#[tauri::command]
pub fn api_delete_project(project_id: String) -> Result<(), String> {
    db::with_db(|conn| {
        conn.execute(
            "DELETE FROM api_projects WHERE id = ?1",
            rusqlite::params![project_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}
