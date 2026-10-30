use super::*;

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
    let id = if test.id.is_empty() {
        db::new_id("tst")
    } else {
        test.id
    };
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
        conn.execute(
            "DELETE FROM api_unit_tests WHERE id = ?1",
            rusqlite::params![test_id],
        )
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
        crate::commands::api::exec::endpoint_to_input(&ep, &variables)
    };
    let mut outputs = Vec::new();
    for test in tests {
        let output = crate::commands::api::exec::execute_request_cancellable(&input).await?;
        let results = crate::commands::api::exec::evaluate_assertions(&test.assertions, &output);
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
