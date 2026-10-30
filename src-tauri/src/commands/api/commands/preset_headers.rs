use super::*;

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
    let id = if set.id.is_empty() {
        db::new_id("ph")
    } else {
        set.id
    };
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
        conn.execute(
            "DELETE FROM api_preset_headers WHERE id = ?1",
            rusqlite::params![set_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

pub(super) fn find_or_create_module(
    project_id: &str,
    name: &str,
) -> Result<Option<String>, String> {
    if name.trim().is_empty() {
        return Ok(None);
    }
    let existing = api_list_modules(project_id.to_string())?;
    if let Some(m) = existing.iter().find(|m| m.name == name.trim()) {
        return Ok(Some(m.id.clone()));
    }
    let created = api_create_module(
        project_id.to_string(),
        name.trim().to_string(),
        String::new(),
    )?;
    Ok(Some(created.id))
}
