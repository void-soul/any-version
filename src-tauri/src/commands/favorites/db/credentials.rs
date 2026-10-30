use super::*;

/// 读取某平台的 Cookie（没配过返回 None）。
pub fn get_credential(conn: &Connection, source: &str) -> Result<Option<String>, String> {
    let mut statement = conn
        .prepare("SELECT cookie FROM favorite_credential WHERE source = ?1")
        .map_err(|e| format!("读取凭证失败: {}", e))?;
    match statement.query_row([source], |row| row.get::<_, String>(0)) {
        Ok(cookie) => Ok(Some(cookie)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("读取凭证失败: {}", e)),
    }
}

/// 写入 / 清除某平台的 Cookie（传空串即清除）。
///
/// 写入时顺手清掉旧的健康状态：新 Cookie 还没验证过，不该继承上一次的 `expired`。
pub fn set_credential(conn: &Connection, source: &str, cookie: &str) -> Result<(), String> {
    let trimmed = cookie.trim();
    if trimmed.is_empty() {
        conn.execute(
            "DELETE FROM favorite_credential WHERE source = ?1",
            [source],
        )
        .map_err(|e| format!("清除凭证失败: {}", e))?;
        return Ok(());
    }
    conn.execute(
        "INSERT INTO favorite_credential (source, cookie, updated_at, status, checked_at, expires_at) \
         VALUES (?1, ?2, ?3, 'unknown', NULL, ?4) \
         ON CONFLICT(source) DO UPDATE SET cookie = ?2, updated_at = ?3, \
         status = 'unknown', checked_at = NULL, expires_at = ?4",
        rusqlite::params![
            source,
            trimmed,
            now_str(),
            crate::commands::favorites::cookie_expiry::parse_cookie_expiry(source, trimmed)
        ],
    )
    .map_err(|e| format!("保存凭证失败: {}", e))?;
    Ok(())
}

/// 记一次使用结果（`ok` / `expired`），并刷新 `checked_at`。
pub fn mark_credential_status(conn: &Connection, source: &str, status: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE favorite_credential SET status = ?1, checked_at = ?2 WHERE source = ?3",
        rusqlite::params![status, now_str(), source],
    )
    .map_err(|e| format!("更新凭证状态失败: {}", e))?;
    Ok(())
}

/// 读某平台的凭证健康状态（没配过时 `configured = false`）。
pub fn credential_status(conn: &Connection, source: &str) -> Result<CredentialStatus, String> {
    let mut statement = conn
        .prepare(
            "SELECT status, checked_at, expires_at, updated_at FROM favorite_credential \
             WHERE source = ?1",
        )
        .map_err(|e| format!("读取凭证状态失败: {}", e))?;
    match statement.query_row([source], |row| {
        Ok(CredentialStatus {
            source: source.to_string(),
            configured: true,
            status: row.get(0)?,
            checked_at: row.get(1)?,
            expires_at: row.get(2)?,
            updated_at: row.get(3)?,
        })
    }) {
        Ok(status) => Ok(status),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(CredentialStatus {
            source: source.to_string(),
            configured: false,
            status: "unknown".to_string(),
            ..CredentialStatus::default()
        }),
        Err(e) => Err(format!("读取凭证状态失败: {}", e)),
    }
}
