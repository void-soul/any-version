use super::*;

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
pub fn api_create_environment(
    project_id: String,
    name: String,
    variables: serde_json::Map<String, Value>,
) -> Result<ApiEnvironment, String> {
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
    Ok(ApiEnvironment {
        id,
        project_id,
        name,
        variables,
        sort_order,
    })
}

#[tauri::command]
pub fn api_update_environment(env: ApiEnvironment) -> Result<(), String> {
    db::with_db(|conn| {
        conn.execute(
            "UPDATE api_environments SET name = ?1, variables = ?2, sort_order = ?3 WHERE id = ?4",
            rusqlite::params![
                env.name,
                serde_json::to_string(&env.variables).unwrap_or_else(|_| "{}".into()),
                env.sort_order,
                env.id
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub fn api_delete_environment(env_id: String) -> Result<(), String> {
    db::with_db(|conn| {
        conn.execute(
            "DELETE FROM api_environments WHERE id = ?1",
            rusqlite::params![env_id],
        )
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
