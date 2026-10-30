use super::*;

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
        conn.execute(
            "DELETE FROM api_history WHERE project_id = ?1",
            rusqlite::params![project_id],
        )
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
