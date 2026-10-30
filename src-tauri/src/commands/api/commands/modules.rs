use super::*;

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
pub fn api_create_module(
    project_id: String,
    name: String,
    description: String,
) -> Result<ApiModule, String> {
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
    Ok(ApiModule {
        id,
        project_id,
        name,
        description,
        sort_order,
    })
}

#[tauri::command]
pub fn api_update_module(module: ApiModule) -> Result<(), String> {
    db::with_db(|conn| {
        conn.execute(
            "UPDATE api_modules SET name = ?1, description = ?2, sort_order = ?3 WHERE id = ?4",
            rusqlite::params![
                module.name,
                module.description,
                module.sort_order,
                module.id
            ],
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
            tx.execute(
                "DELETE FROM api_endpoints WHERE id = ?1",
                rusqlite::params![ep_id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.execute(
            "DELETE FROM api_modules WHERE id = ?1",
            rusqlite::params![module_id],
        )
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
