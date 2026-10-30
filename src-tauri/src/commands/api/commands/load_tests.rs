use super::*;

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
    crate::commands::api::loadtest::validate_config(&config)?;
    crate::runtime_tasks::require_enabled("api")?;
    let input = if let Some(ov) = input_override {
        ov
    } else {
        let ep = api_get_endpoint(endpoint_id.clone())?;
        crate::commands::api::exec::endpoint_to_input(&ep, &variables)
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
    match crate::commands::api::loadtest::start_load_run(
        run_id.clone(),
        endpoint_id,
        name,
        config,
        created_at,
        input,
    ) {
        Ok(id) => Ok(id),
        Err(error) => {
            db::with_db(|conn| {
                conn.execute("DELETE FROM api_load_runs WHERE id = ?1", [&run_id])
                    .map_err(|e| e.to_string())?;
                Ok(())
            })?;
            Err(error)
        }
    }
}

#[tauri::command]
pub fn api_load_run_status(run_id: String) -> Result<LoadRunStatus, String> {
    crate::commands::api::loadtest::load_run_status(&run_id)
}

/// 手动停止压测（已产生的统计仍会生成报告）。
#[tauri::command]
pub fn api_stop_load_test(run_id: String) -> Result<(), String> {
    crate::commands::api::loadtest::stop_load_run(&run_id)
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
    crate::commands::api::loadtest::ensure_not_active(&run_id)?;
    db::with_db(|conn| {
        conn.execute(
            "DELETE FROM api_load_runs WHERE id = ?1",
            rusqlite::params![run_id],
        )
        .map_err(|e| e.to_string())?;
        crate::commands::api::loadtest::forget_completed(&run_id);
        Ok(())
    })
}
