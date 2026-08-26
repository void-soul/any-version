//! API 模块的 Tauri 命令层：CRUD + 请求执行 + 单测 + 压测 + 导入导出。

use serde_json::Value;

use super::db::{self, now_ts};
use super::models::*;

/// 接口表的完整列清单（顺序与 endpoint_row 一致）。
const EP_COLS: &str = "id, project_id, module_id, name, method, url, headers, query_params, path_params, body, body_type, description, docs_md, timeout_ms, created_at, updated_at, body_form, body_urlencoded, body_graphql_query, body_graphql_variables, authorization, cookies, settings, response_comment, is_favorite";

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
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?))
                    })
                    .map_err(|e| e.to_string())?;
                let mut v = Vec::new();
                for r in rows {
                    v.push(r.map_err(|e| e.to_string())?);
                }
                v
            };
            for (ep_id, headers_raw, params_raw, body_raw, body_form_raw) in ep_rows {
                let mut headers: Vec<KeyValueItem> = serde_json::from_str(&headers_raw).unwrap_or_default();
                let mut params: Vec<KeyValueItem> = serde_json::from_str(&params_raw).unwrap_or_default();
                let mut body: Vec<KeyValueItem> = serde_json::from_str(&body_raw).unwrap_or_default();
                let mut body_form: Vec<FormDataItem> = serde_json::from_str(&body_form_raw).unwrap_or_default();
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
                if headers != headers_before || params != params_before || body != body_before || body_form != body_form_before {
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
        conn.execute("DELETE FROM api_projects WHERE id = ?1", rusqlite::params![project_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

// ─── 环境（变量集合） CRUD ───

#[tauri::command]
pub fn api_list_environments(project_id: String) -> Result<Vec<ApiEnvironment>, String> {
    db::with_db(|conn| {
        let mut stmt = conn
            .prepare("SELECT id, project_id, name, variables, sort_order FROM api_environments WHERE project_id = ?1 ORDER BY sort_order")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![project_id], |row| db::env_row(row))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    })
}

#[tauri::command]
pub fn api_create_environment(project_id: String, name: String, variables: serde_json::Map<String, Value>) -> Result<ApiEnvironment, String> {
    let id = db::new_id("env");
    let sort_order = db::with_db(|conn| {
        conn.query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM api_environments WHERE project_id = ?1",
            rusqlite::params![project_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|e| e.to_string())
    })?;
    db::with_db(|conn| {
        conn.execute(
            "INSERT INTO api_environments (id, project_id, name, variables, sort_order) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, project_id, name, serde_json::to_string(&variables).unwrap_or_else(|_| "{}".into()), sort_order],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    Ok(ApiEnvironment { id, project_id, name, variables, sort_order })
}

#[tauri::command]
pub fn api_update_environment(env: ApiEnvironment) -> Result<(), String> {
    db::with_db(|conn| {
        conn.execute(
            "UPDATE api_environments SET name = ?1, variables = ?2, sort_order = ?3 WHERE id = ?4",
            rusqlite::params![env.name, serde_json::to_string(&env.variables).unwrap_or_else(|_| "{}".into()), env.sort_order, env.id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub fn api_delete_environment(env_id: String) -> Result<(), String> {
    db::with_db(|conn| {
        conn.execute("DELETE FROM api_environments WHERE id = ?1", rusqlite::params![env_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub fn api_set_active_env(project_id: String, env_id: Option<String>) -> Result<(), String> {
    db::with_db(|conn| {
        conn.execute(
            "UPDATE api_projects SET active_env_id = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![env_id, now_ts(), project_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

// ─── 模块 CRUD ───

#[tauri::command]
pub fn api_list_modules(project_id: String) -> Result<Vec<ApiModule>, String> {
    db::with_db(|conn| {
        let mut stmt = conn
            .prepare("SELECT id, project_id, name, description, sort_order FROM api_modules WHERE project_id = ?1 ORDER BY sort_order, name")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![project_id], |row| db::module_row(row))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    })
}

#[tauri::command]
pub fn api_create_module(project_id: String, name: String, description: String) -> Result<ApiModule, String> {
    let id = db::new_id("mod");
    let sort_order = db::with_db(|conn| {
        conn.query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM api_modules WHERE project_id = ?1",
            rusqlite::params![project_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|e| e.to_string())
    })?;
    db::with_db(|conn| {
        conn.execute(
            "INSERT INTO api_modules (id, project_id, name, description, sort_order) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, project_id, name, description, sort_order],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    Ok(ApiModule { id, project_id, name, description, sort_order })
}

#[tauri::command]
pub fn api_update_module(module: ApiModule) -> Result<(), String> {
    db::with_db(|conn| {
        conn.execute(
            "UPDATE api_modules SET name = ?1, description = ?2, sort_order = ?3 WHERE id = ?4",
            rusqlite::params![module.name, module.description, module.sort_order, module.id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
/// 删除模块：其下所有接口一并删除（接口的单测/压测经外键级联删除）。
pub fn api_delete_module(module_id: String) -> Result<(), String> {
    db::with_db(|conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        // 先取该模块下接口 id，逐个删除（触发 unit_tests / load_runs 级联）
        let ep_ids: Vec<String> = {
            let mut stmt = tx
                .prepare("SELECT id FROM api_endpoints WHERE module_id = ?1")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(rusqlite::params![module_id], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            let mut v = Vec::new();
            for r in rows {
                v.push(r.map_err(|e| e.to_string())?);
            }
            v
        };
        for ep_id in ep_ids {
            tx.execute("DELETE FROM api_endpoints WHERE id = ?1", rusqlite::params![ep_id])
                .map_err(|e| e.to_string())?;
        }
        tx.execute("DELETE FROM api_modules WHERE id = ?1", rusqlite::params![module_id])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
}

/// 把源模块下的所有接口转移到目标模块（目标模块不存在则报错）。
#[tauri::command]
pub fn api_move_module_endpoints(
    source_module_id: String,
    target_module_id: String,
) -> Result<usize, String> {
    if source_module_id == target_module_id {
        return Err("源模块与目标模块相同".to_string());
    }
    db::with_db(|conn| {
        // 校验目标模块存在
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM api_modules WHERE id = ?1)",
                rusqlite::params![target_module_id],
                |r| r.get::<_, i64>(0).map(|n| n > 0),
            )
            .map_err(|e| format!("校验目标模块失败: {}", e))?;
        if !exists {
            return Err("目标模块不存在".to_string());
        }
        let n = conn
            .execute(
                "UPDATE api_endpoints SET module_id = ?1 WHERE module_id = ?2",
                rusqlite::params![target_module_id, source_module_id],
            )
            .map_err(|e| e.to_string())?;
        Ok(n)
    })
}

// ─── 请求历史 ───

#[tauri::command]
pub fn api_list_history(project_id: String) -> Result<Vec<ApiHistoryEntry>, String> {
    db::with_db(|conn| {
        let mut stmt = conn
            .prepare("SELECT id, project_id, endpoint_id, name, method, url, input, created_at FROM api_history WHERE project_id = ?1 ORDER BY created_at DESC LIMIT 200")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![project_id], |row| {
                let input_raw: String = row.get(6)?;
                Ok(ApiHistoryEntry {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    endpoint_id: row.get(2)?,
                    name: row.get(3)?,
                    method: row.get(4)?,
                    url: row.get(5)?,
                    input: serde_json::from_str(&input_raw).unwrap_or_default(),
                    created_at: row.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    })
}

#[tauri::command]
pub fn api_add_history(
    project_id: String,
    endpoint_id: Option<String>,
    name: String,
    input: SendRequestInput,
) -> Result<(), String> {
    let id = db::new_id("hist");
    let ts = now_ts();
    db::with_db(|conn| {
        // 同 method+url 的旧记录移除（保最新一条）
        conn.execute(
            "DELETE FROM api_history WHERE project_id = ?1 AND method = ?2 AND url = ?3",
            rusqlite::params![project_id, input.method, input.url],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO api_history (id, project_id, endpoint_id, name, method, url, input, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![id, project_id, endpoint_id, name, input.method, input.url, serde_json::to_string(&input).unwrap_or_else(|_| "{}".into()), ts],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub fn api_clear_history(project_id: String) -> Result<(), String> {
    db::with_db(|conn| {
        conn.execute("DELETE FROM api_history WHERE project_id = ?1", rusqlite::params![project_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub fn api_set_favorite(endpoint_id: String, favorite: bool) -> Result<(), String> {
    db::with_db(|conn| {
        conn.execute(
            "UPDATE api_endpoints SET is_favorite = ?1 WHERE id = ?2",
            rusqlite::params![favorite as i64, endpoint_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

// ─── 接口 CRUD ───

#[tauri::command]
pub fn api_list_endpoints(project_id: String, module_id: Option<String>) -> Result<Vec<ApiEndpoint>, String> {
    db::with_db(|conn| {
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(mid) = module_id {
            (
                format!("SELECT {} FROM api_endpoints WHERE project_id = ?1 AND module_id = ?2 ORDER BY created_at", EP_COLS),
                vec![Box::new(project_id), Box::new(mid)],
            )
        } else {
            (
                format!("SELECT {} FROM api_endpoints WHERE project_id = ?1 ORDER BY created_at", EP_COLS),
                vec![Box::new(project_id)],
            )
        };
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())), |row| db::endpoint_row(row))
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
fn merge_template_items(items: &mut Vec<KeyValueItem>, template: &[KeyValueItem]) {
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
fn resync_template_items(items: &mut [KeyValueItem], template: &[KeyValueItem]) {
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
fn rename_template_items(items: &mut [KeyValueItem], old_key: &str, new_key: &str) {
    for item in items.iter_mut() {
        if item.from_template && item.key == old_key {
            item.key = new_key.to_string();
        }
    }
}

/// 把通用 Body 模板合并进 form-data 字段（text 类型条目，打 from_template 标记）。
fn merge_template_form_items(items: &mut Vec<FormDataItem>, template: &[KeyValueItem]) {
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
fn resync_template_form_items(items: &mut [FormDataItem], template: &[KeyValueItem]) {
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
fn rename_template_form_items(items: &mut [FormDataItem], old_key: &str, new_key: &str) {
    for item in items.iter_mut() {
        if item.from_template && item.key == old_key {
            item.key = new_key.to_string();
        }
    }
}

#[tauri::command]
pub fn api_create_endpoint(ep: ApiEndpoint) -> Result<ApiEndpoint, String> {
    let id = if ep.id.is_empty() { db::new_id("ep") } else { ep.id };
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
            &format!("INSERT INTO api_endpoints ({}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25)", EP_COLS),
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
                ep.response_comment, ep.is_favorite
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
            &format!("UPDATE api_endpoints SET module_id=?1, name=?2, method=?3, url=?4, headers=?5, query_params=?6, path_params=?7, body=?8, body_type=?9, description=?10, docs_md=?11, timeout_ms=?12, updated_at=?13, body_form=?14, body_urlencoded=?15, body_graphql_query=?16, body_graphql_variables=?17, authorization=?18, cookies=?19, settings=?20, response_comment=?21, is_favorite=?22 WHERE id=?23"),
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
                ep.response_comment, ep.is_favorite, ep.id
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub fn api_delete_endpoint(endpoint_id: String) -> Result<(), String> {
    db::with_db(|conn| {
        conn.execute("DELETE FROM api_endpoints WHERE id = ?1", rusqlite::params![endpoint_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

// ─── 请求执行 ───

#[tauri::command]
pub async fn api_send_request(input: SendRequestInput) -> Result<SendRequestOutput, String> {
    super::exec::execute_request(&input).await
}

// ─── 单元测试 ───

#[tauri::command]
pub fn api_list_unit_tests(endpoint_id: String) -> Result<Vec<UnitTest>, String> {
    db::with_db(|conn| {
        let mut stmt = conn
            .prepare("SELECT id, endpoint_id, name, assertions, created_at FROM api_unit_tests WHERE endpoint_id = ?1 ORDER BY created_at")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![endpoint_id], |row| {
                let assertions_raw: String = row.get(3)?;
                Ok(UnitTest {
                    id: row.get(0)?,
                    endpoint_id: row.get(1)?,
                    name: row.get(2)?,
                    assertions: serde_json::from_str(&assertions_raw).unwrap_or_default(),
                    created_at: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    })
}

#[tauri::command]
pub fn api_save_unit_test(test: UnitTest) -> Result<UnitTest, String> {
    let id = if test.id.is_empty() { db::new_id("tst") } else { test.id };
    db::with_db(|conn| {
        conn.execute(
            "INSERT INTO api_unit_tests (id, endpoint_id, name, assertions, created_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET name = ?3, assertions = ?4",
            rusqlite::params![id, test.endpoint_id, test.name, serde_json::to_string(&test.assertions).unwrap_or_else(|_| "[]".into()), now_ts()],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    Ok(UnitTest { id, ..test })
}

#[tauri::command]
pub fn api_delete_unit_test(test_id: String) -> Result<(), String> {
    db::with_db(|conn| {
        conn.execute("DELETE FROM api_unit_tests WHERE id = ?1", rusqlite::params![test_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub async fn api_run_unit_test(
    endpoint_id: String,
    variables: serde_json::Map<String, Value>,
    // 可选：直接使用当前（可能未保存的）请求配置执行断言
    input_override: Option<SendRequestInput>,
) -> Result<Vec<UnitTestRunOutput>, String> {
    let tests = api_list_unit_tests(endpoint_id.clone())?;
    if tests.is_empty() {
        return Err("该接口还没有单元测试，请先添加断言".to_string());
    }
    let input = if let Some(ov) = input_override {
        ov
    } else {
        let ep = api_get_endpoint(endpoint_id.clone())?;
        super::exec::endpoint_to_input(&ep, &variables)
    };
    let mut outputs = Vec::new();
    for test in tests {
        let output = super::exec::execute_request(&input).await?;
        let results = super::exec::evaluate_assertions(&test.assertions, &output);
        let pass = results.iter().all(|r| r.pass);
        outputs.push(UnitTestRunOutput {
            pass,
            time_ms: output.time_ms,
            status: output.status,
            results,
        });
    }
    Ok(outputs)
}

// ─── 压力测试 ───

#[tauri::command]
pub fn api_start_load_test(
    endpoint_id: String,
    name: String,
    config: LoadTestConfig,
    variables: serde_json::Map<String, Value>,
    // 可选：直接使用当前（可能未保存的）请求配置压测
    input_override: Option<SendRequestInput>,
) -> Result<String, String> {
    let input = if let Some(ov) = input_override {
        ov
    } else {
        let ep = api_get_endpoint(endpoint_id.clone())?;
        super::exec::endpoint_to_input(&ep, &variables)
    };
    let run_id = db::new_id("load");
    let created_at = now_ts();
    // 落库占位（报告完成后更新）
    db::with_db(|conn| {
        conn.execute(
            "INSERT INTO api_load_runs (id, endpoint_id, name, config, report, created_at) VALUES (?1, ?2, ?3, ?4, '', ?5)",
            rusqlite::params![run_id, endpoint_id, name, serde_json::to_string(&config).unwrap_or_default(), created_at],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    Ok(super::loadtest::start_load_run(run_id, endpoint_id, name, config, created_at, input))
}

#[tauri::command]
pub fn api_load_run_status(run_id: String) -> Result<LoadRunStatus, String> {
    super::loadtest::load_run_status(&run_id)
}

#[tauri::command]
pub fn api_list_load_runs(endpoint_id: String) -> Result<Vec<LoadTestRun>, String> {
    db::with_db(|conn| {
        let mut stmt = conn
            .prepare("SELECT id, endpoint_id, name, config, report, created_at FROM api_load_runs WHERE endpoint_id = ?1 ORDER BY created_at DESC LIMIT 50")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![endpoint_id], |row| {
                let config_raw: String = row.get(3)?;
                let report_raw: String = row.get(4)?;
                Ok(LoadTestRun {
                    id: row.get(0)?,
                    endpoint_id: row.get(1)?,
                    name: row.get(2)?,
                    config: serde_json::from_str(&config_raw).unwrap_or_default(),
                    report: if report_raw.is_empty() {
                        None
                    } else {
                        serde_json::from_str(&report_raw).ok()
                    },
                    created_at: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    })
}

#[tauri::command]
pub fn api_delete_load_run(run_id: String) -> Result<(), String> {
    db::with_db(|conn| {
        conn.execute("DELETE FROM api_load_runs WHERE id = ?1", rusqlite::params![run_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

// ─── 导入导出 ───

/// Postman 导入上限：50 MB（超大集合拒绝解析，避免 OOM）。
const IMPORT_MAX_BYTES: usize = 50 * 1024 * 1024;

#[tauri::command]
pub fn api_import_postman(json: String, project_id: String, module_id: Option<String>) -> Result<usize, String> {
    if json.len() > IMPORT_MAX_BYTES {
        return Err(format!("Postman 集合文件过大（{:.1} MB），上限 {:.1} MB", json.len() as f64 / 1_048_576.0, IMPORT_MAX_BYTES as f64 / 1_048_576.0));
    }
    let imp = super::import::parse_postman_collection(&json)?;
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
fn collection_name(json: &str) -> Option<String> {
    let root: serde_json::Value = serde_json::from_str(json).ok()?;
    root.get("info")?.get("name")?.as_str().map(|s| s.to_string())
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
    super::import::export_postman_collection(&project.name, &modules, &endpoints, &variables)
}

#[tauri::command]
pub async fn api_import_swagger(source: String, project_id: String, module_id: Option<String>) -> Result<usize, String> {
    let json = super::import::fetch_swagger_source(&source).await?;
    let (_module, _base, drafts) = super::import::parse_swagger(&json)?;
    let count = insert_drafts(project_id, module_id, drafts)?;
    Ok(count)
}

#[tauri::command]
pub fn api_scan_framework(dir: String, framework: String, project_id: String, module_id: Option<String>) -> Result<usize, String> {
    let drafts = super::import::scan_framework(&dir, &framework)?;
    let count = insert_drafts(project_id, module_id, drafts)?;
    Ok(count)
}

/// 把导入候选写入数据库：按模块名查找/创建模块，再插入接口。
fn insert_drafts(
    project_id: String,
    module_id: Option<String>,
    drafts: Vec<super::import::EndpointDraft>,
) -> Result<usize, String> {
    let mut count = 0;
    for draft in drafts {
        let target_module: Option<String> = if let Some(mid) = &module_id {
            if mid.is_empty() {
                find_or_create_module(&project_id, &draft.module)?
            } else {
                Some(mid.clone())
            }
        } else {
            find_or_create_module(&project_id, &draft.module)?
        };
        let ep = ApiEndpoint {
            id: db::new_id("ep"),
            project_id: project_id.clone(),
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
            settings: Default::default(),
            response_comment: String::new(),
            is_favorite: false,
            description: String::new(),
            docs_md: draft.docs_md,
            timeout_ms: 15000,
            created_at: now_ts(),
            updated_at: now_ts(),
        };
        let _ = api_create_endpoint(ep)?;
        count += 1;
    }
    Ok(count)
}

// ─── 预设 Headers（项目级） ───

#[tauri::command]
pub fn api_list_preset_headers(project_id: String) -> Result<Vec<PresetHeaderSet>, String> {
    db::with_db(|conn| {
        let mut stmt = conn
            .prepare("SELECT id, project_id, name, headers, created_at FROM api_preset_headers WHERE project_id = ?1 ORDER BY created_at")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![project_id], |row| {
                let headers_raw: String = row.get(3)?;
                Ok(PresetHeaderSet {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    name: row.get(2)?,
                    headers: serde_json::from_str(&headers_raw).unwrap_or_default(),
                    created_at: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    })
}

#[tauri::command]
pub fn api_save_preset_headers(set: PresetHeaderSet) -> Result<PresetHeaderSet, String> {
    let id = if set.id.is_empty() { db::new_id("ph") } else { set.id };
    db::with_db(|conn| {
        conn.execute(
            "INSERT INTO api_preset_headers (id, project_id, name, headers, created_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET name = ?3, headers = ?4",
            rusqlite::params![id, set.project_id, set.name, serde_json::to_string(&set.headers).unwrap_or_else(|_| "[]".into()), now_ts()],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    Ok(PresetHeaderSet { id, ..set })
}

#[tauri::command]
pub fn api_delete_preset_headers(set_id: String) -> Result<(), String> {
    db::with_db(|conn| {
        conn.execute("DELETE FROM api_preset_headers WHERE id = ?1", rusqlite::params![set_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

fn find_or_create_module(project_id: &str, name: &str) -> Result<Option<String>, String> {
    if name.trim().is_empty() {
        return Ok(None);
    }
    let existing = api_list_modules(project_id.to_string())?;
    if let Some(m) = existing.iter().find(|m| m.name == name.trim()) {
        return Ok(Some(m.id.clone()));
    }
    let created = api_create_module(project_id.to_string(), name.trim().to_string(), String::new())?;
    Ok(Some(created.id))
}
