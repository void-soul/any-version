use super::finalshell;
use super::models::{ImportReport, RdpConnection};
use super::store;
use std::path::{Path, PathBuf};

fn validate(conn: &RdpConnection) -> Result<(), String> {
    if conn.name.trim().is_empty() {
        return Err("连接名称不能为空".to_string());
    }
    if conn.host.trim().is_empty() {
        return Err("主机地址不能为空".to_string());
    }
    if conn.port == 0 {
        return Err("端口不能为 0".to_string());
    }
    Ok(())
}

fn merge_by_id(list: &mut Vec<RdpConnection>, incoming: RdpConnection) {
    if let Some(slot) = list.iter_mut().find(|c| c.id == incoming.id) {
        *slot = incoming;
    } else {
        list.push(incoming);
    }
}

fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|ch| if r#"\/:*?"<>|"#.contains(ch) { '_' } else { ch })
        .collect();
    cleaned.trim().trim_end_matches('.').to_string()
}

#[tauri::command]
pub fn rdp_list() -> Result<Vec<RdpConnection>, String> {
    store::load()
}

#[tauri::command]
pub fn rdp_save(mut conn: RdpConnection) -> Result<Vec<RdpConnection>, String> {
    validate(&conn)?;
    // 前端传来明文密码时才加密；已带 ENC_V2: 前缀说明是回显的既有密文，不重复加密。
    if let Some(plain) = conn.password.clone() {
        if !plain.is_empty() && !plain.starts_with("ENC_V2:") {
            conn.password = Some(crate::commands::secrets::encrypt_secret(&plain)?);
        }
    }
    // 私钥口令同样只在内存里出现一次；ENC_V2: 前缀说明是回显的既有密文。
    if !conn.key_passphrase.is_empty() && !conn.key_passphrase.starts_with("ENC_V2:") {
        conn.key_passphrase = crate::commands::secrets::encrypt_secret(&conn.key_passphrase)?;
    }
    let now = chrono::Utc::now().timestamp_millis();
    conn.updated_at = now;
    if conn.id.is_empty() {
        conn.id = format!("rdp_{now}");
        conn.created_at = now;
        if conn.source.is_empty() {
            conn.source = "local".to_string();
        }
    }
    let mut list = store::load()?;
    merge_by_id(&mut list, conn);
    store::save(&list)?;
    Ok(list)
}

#[tauri::command]
pub fn rdp_delete(id: String) -> Result<Vec<RdpConnection>, String> {
    let mut list = store::load()?;
    let before = list.len();
    list.retain(|c| c.id != id);
    if list.len() == before {
        return Err(format!("连接不存在: {id}"));
    }
    store::save(&list)?;
    Ok(list)
}

/// 导入 FinalShell 导出的单连接 JSON，或包含多个 `*_connect_config.json` 的目录。
/// 只接收 RDP(`conection_type=101`)，其余类型跳过并计数；密文原样保留。
#[tauri::command]
pub fn rdp_import_finalshell(paths: Vec<String>) -> Result<ImportReport, String> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut report = ImportReport::default();
    for raw in paths {
        let path = PathBuf::from(&raw);
        if path.is_dir() {
            let entries = std::fs::read_dir(&path)
                .map_err(|e| format!("读取目录失败 {}: {e}", path.display()))?;
            for entry in entries.flatten() {
                let file = entry.path();
                if file.extension().and_then(|e| e.to_str()) == Some("json") {
                    files.push(file);
                }
            }
        } else if path.is_file() {
            files.push(path);
        } else {
            report.failed += 1;
            report.messages.push(format!("路径不存在: {raw}"));
        }
    }
    let mut list = store::load()?;
    for file in &files {
        let result = std::fs::read_to_string(file)
            .map_err(|e| e.to_string())
            .and_then(|text| finalshell::parse_finalshell(&text));
        match result {
            Ok(Some((conn, _))) => {
                merge_by_id(&mut list, conn);
                report.imported += 1;
            }
            Ok(None) => report.skipped += 1,
            Err(e) => {
                report.failed += 1;
                report.messages.push(format!("{}: {e}", file.display()));
            }
        }
    }
    if report.imported > 0 {
        store::save(&list)?;
    }
    Ok(report)
}

/// 导出 FinalShell 兼容文件到用户指定目录（**绝不写回 FinalShell 安装目录**）。
/// 仅 `source=finalshell` 的连接可导出：本地连接没有它那份原始配置。
#[tauri::command]
pub fn rdp_export_finalshell(dir: String, ids: Vec<String>) -> Result<usize, String> {
    let target = PathBuf::from(&dir);
    let list = store::load()?;
    let mut count = 0usize;
    for conn in list
        .iter()
        .filter(|c| c.source == "finalshell" && (ids.is_empty() || ids.contains(&c.id)))
    {
        let text = finalshell::export_finalshell(conn)?;
        let name = safe_file_name(&conn.name);
        let path = target.join(format!("{}_connect_config.json", name));
        crate::commands::foundation::atomic_write_file(&path, text.as_bytes())?;
        count += 1;
    }
    Ok(count)
}

/// 本模块完整备份（顶层数组）。自有密码是机器绑定的 `ENC_V2:` 密文。
#[tauri::command]
pub fn rdp_export_backup(path: String) -> Result<usize, String> {
    let list = store::load()?;
    let bytes = serde_json::to_vec_pretty(&list).map_err(|e| e.to_string())?;
    crate::commands::foundation::atomic_write_file(Path::new(&path), &bytes)?;
    Ok(list.len())
}

/// 导入备份：兼容顶层数组与 `{"connections":[...]}` 包装。
/// 跨机器时 `ENC_V2:` 密文可能解不开（主密钥不同）→ 清空该密码并如实提示，不谎报成功。
#[tauri::command]
pub fn rdp_import_backup(path: String) -> Result<ImportReport, String> {
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取备份失败 {path}: {e}"))?;
    let value: serde_json::Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("备份文件不是有效 JSON: {e}"))?;
    let array = value
        .as_array()
        .or_else(|| value.get("connections").and_then(serde_json::Value::as_array))
        .ok_or_else(|| "备份文件缺少连接数组".to_string())?
        .clone();
    let incoming: Vec<RdpConnection> = serde_json::from_value(serde_json::Value::Array(array))
        .map_err(|e| format!("备份里的连接结构无效: {e}"))?;
    let mut list = store::load()?;
    let mut report = ImportReport::default();
    for mut conn in incoming {
        if let Some(enc) = conn.password.clone() {
            if enc.starts_with("ENC_V2:")
                && crate::commands::secrets::decrypt_secret(&enc).is_err()
            {
                conn.password = None;
                report.messages.push(format!(
                    "「{}」的密码由另一台机器加密，本机无法解开，已留空请重新填写",
                    conn.name
                ));
            }
        }
        merge_by_id(&mut list, conn);
        report.imported += 1;
    }
    store::save(&list)?;
    Ok(report)
}

/// 解密 FinalShell 导入连接的密码，供用户查看/编辑（明文只在本次 IPC 返回，绝不落盘）。
#[tauri::command]
pub fn rdp_reveal_password(id: String) -> Result<String, String> {
    let conn = store::load()?
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("连接不存在: {id}"))?;
    let cipher = conn
        .finalshell_cipher
        .as_deref()
        .ok_or_else(|| "该连接没有 FinalShell 密码密文".to_string())?;
    super::finalshell_crypto::decrypt(cipher)
}

/// 用 FinalShell 自己的算法重新加密明文密码并写回密文字段。
/// 只改 `finalshell_cipher`；`finalshell_raw` 保留原样，导出时由 `export_finalshell` 覆盖 password。
#[tauri::command]
pub fn rdp_set_finalshell_password(id: String, plain: String) -> Result<Vec<RdpConnection>, String> {
    let cipher = super::finalshell_crypto::encrypt(&plain)?;
    let mut list = store::load()?;
    let conn = list
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("连接不存在: {id}"))?;
    if conn.source != "finalshell" {
        return Err("只有从 FinalShell 导入的连接才能写回该格式的密码".to_string());
    }
    conn.finalshell_cipher = Some(cipher);
    conn.updated_at = chrono::Utc::now().timestamp_millis();
    store::save(&list)?;
    Ok(list)
}

/// 生成临时 `.rdp` 并用 mstsc 打开。**不写入任何密码明文**，由 Windows 弹凭据框。
#[cfg(windows)]
#[tauri::command]
pub fn rdp_launch(id: String) -> Result<Option<String>, String> {
    let conn = store::load()?
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("连接不存在: {id}"))?;
    if conn.kind == "ssh" {
        return Err("该连接是 SSH 服务器，请使用终端按钮连接".to_string());
    }
    let mut text = format!(
        "full address:s:{}:{}\r\nusername:s:{}\r\n",
        conn.host, conn.port, conn.username
    );
    if !conn.domain.is_empty() {
        text.push_str(&format!("domain:s:{}\r\n", conn.domain));
    }

    // 优先把密码以 DPAPI 密文写进 .rdp，做到与 FinalShell 一样免输入。
    // 取不到密码或加密失败时回退到系统凭据框，并如实告知原因（不静默降级）。
    let mut warning: Option<String> = None;
    let mut embedded = false;
    match super::credentials::resolve_plain_password(&conn) {
        Ok(Some(plain)) if !plain.is_empty() => match super::credentials::protect(&plain) {
            Ok(blob) => {
                let line = super::credentials::rdp_password_line(&blob);
                if !line.is_empty() {
                    text.push_str(&line);
                    embedded = true;
                }
            }
            Err(e) => warning = Some(format!("无法自动填写密码，将弹出系统凭据框：{e}")),
        },
        Ok(_) => {}
        Err(e) => warning = Some(format!("密码解密失败，将弹出系统凭据框：{e}")),
    }
    if !embedded {
        text.push_str("prompt for credentials:i:1\r\n");
    }

    let key: String = conn
        .id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let path = std::env::temp_dir().join(format!("any-version-rdp-{key}.rdp"));
    crate::commands::foundation::atomic_write_file(&path, text.as_bytes())?;
    std::process::Command::new("mstsc")
        .arg(&path)
        .spawn()
        .map_err(|e| format!("启动 mstsc 失败: {e}"))?;

    // mstsc 启动后会自己读取该文件；延迟删除，避免密码密文长期留在磁盘。
    let cleanup = path.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(5));
        let _ = std::fs::remove_file(&cleanup);
    });
    Ok(warning)
}

#[cfg(not(windows))]
#[tauri::command]
pub fn rdp_launch(id: String) -> Result<Option<String>, String> {
    let _ = id;
    Err("远程桌面仅在 Windows 上可用".to_string())
}
