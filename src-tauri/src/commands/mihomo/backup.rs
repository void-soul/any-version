//! 备份与恢复：本地 zip 导出/导入 + WebDAV 备份/恢复/列表/删除
//! 对齐 clash-party src/main/resolve/backup.ts

use crate::commands::mihomo::MihomoState;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri::State;

/// 需要备份的文件/目录（相对 data_dir）
const BACKUP_ENTRIES: [&str; 6] = [
    "app.json",
    "controled.json",
    "profile.json",
    "override.json",
    "profiles",
    "override",
];

fn add_dir_to_zip<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    base: &Path,
    dir: &Path,
) -> Result<(), String> {
    let opts: zip::write::FileOptions<()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };
    for e in rd.flatten() {
        let p = e.path();
        let rel = p
            .strip_prefix(base)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        if p.is_dir() {
            zip.add_directory(format!("{rel}/"), opts)
                .map_err(|e| e.to_string())?;
            add_dir_to_zip(zip, base, &p)?;
        } else {
            let data = std::fs::read(&p).unwrap_or_default();
            zip.start_file(rel, opts).map_err(|e| e.to_string())?;
            zip.write_all(&data).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn build_backup_zip(dir: &Path) -> Result<Vec<u8>, String> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let opts: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for name in BACKUP_ENTRIES {
            let p = dir.join(name);
            if !p.exists() {
                continue;
            }
            if p.is_dir() {
                zip.add_directory(format!("{name}/"), opts)
                    .map_err(|e| e.to_string())?;
                add_dir_to_zip(&mut zip, dir, &p)?;
            } else {
                let data = std::fs::read(&p).unwrap_or_default();
                zip.start_file(name, opts).map_err(|e| e.to_string())?;
                zip.write_all(&data).map_err(|e| e.to_string())?;
            }
        }
        zip.finish().map_err(|e| e.to_string())?;
    }
    Ok(cursor.into_inner())
}

fn restore_backup_zip(dir: &Path, data: &[u8]) -> Result<(), String> {
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("解析备份失败: {e}"))?;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = f.name().replace('\\', "/");
        if name.ends_with('/') || name.contains("..") {
            continue;
        }
        let out = dir.join(&name);
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p).ok();
        }
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut buf).ok();
        std::fs::write(&out, &buf).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn backup_name() -> String {
    let now = chrono::Local::now();
    format!("any-version-mihomo-{}.zip", now.format("%Y%m%d-%H%M%S"))
}

// ---------------- 本地备份 ----------------

#[tauri::command]
pub fn mihomo_export_local_backup(
    state: State<'_, MihomoState>,
    path: String,
) -> Result<String, String> {
    let data = build_backup_zip(&state.data_dir)?;
    let mut target = PathBuf::from(&path);
    if target.is_dir() {
        target = target.join(backup_name());
    }
    if let Some(p) = target.parent() {
        std::fs::create_dir_all(p).ok();
    }
    std::fs::write(&target, &data).map_err(|e| e.to_string())?;
    Ok(target.to_string_lossy().to_string())
}

#[tauri::command]
pub fn mihomo_import_local_backup(
    state: State<'_, MihomoState>,
    path: String,
) -> Result<String, String> {
    let data = std::fs::read(&path).map_err(|e| format!("读取失败: {e}"))?;
    restore_backup_zip(&state.data_dir, &data)?;
    Ok("恢复完成，请重启内核使配置生效".into())
}

// ---------------- WebDAV ----------------

struct Dav {
    url: String,
    user: String,
    pass: String,
}

fn dav_of(state: &State<'_, MihomoState>) -> Result<Dav, String> {
    let app = state.app_config.lock().unwrap();
    let g = |k: &str| {
        app.extra
            .get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let mut url = g("webdavUrl");
    if url.is_empty() {
        return Err("未配置 WebDAV 地址".into());
    }
    let dir = {
        let d = g("webdavDir");
        if d.is_empty() {
            "any-version".to_string()
        } else {
            d
        }
    };
    url = format!("{}/{}", url.trim_end_matches('/'), dir.trim_matches('/'));
    Ok(Dav {
        url,
        user: g("webdavUsername"),
        pass: g("webdavPassword"),
    })
}

fn dav_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap_or_default()
}

async fn ensure_collection(d: &Dav) -> Result<(), String> {
    let req = dav_client()
        .request(
            reqwest::Method::from_bytes(b"MKCOL").unwrap(),
            format!("{}/", d.url),
        )
        .basic_auth(&d.user, Some(&d.pass));
    let _ = req.send().await;
    Ok(())
}

#[tauri::command]
pub async fn mihomo_webdav_backup(state: State<'_, MihomoState>) -> Result<String, String> {
    let d = dav_of(&state)?;
    let data = build_backup_zip(&state.data_dir)?;
    ensure_collection(&d).await?;
    let name = backup_name();
    let resp = dav_client()
        .put(format!("{}/{}", d.url, name))
        .basic_auth(&d.user, Some(&d.pass))
        .header("Content-Type", "application/zip")
        .body(data)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("WebDAV 上传失败: HTTP {}", resp.status()));
    }
    Ok(name)
}

#[tauri::command]
pub async fn mihomo_webdav_list(state: State<'_, MihomoState>) -> Result<Value, String> {
    let d = dav_of(&state)?;
    let resp = dav_client()
        .request(
            reqwest::Method::from_bytes(b"PROPFIND").unwrap(),
            format!("{}/", d.url),
        )
        .basic_auth(&d.user, Some(&d.pass))
        .header("Depth", "1")
        .header("Content-Type", "application/xml")
        .body(r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:displayname/><d:getcontentlength/><d:getlastmodified/></d:prop></d:propfind>"#)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() && status.as_u16() != 207 {
        return Err(format!("WebDAV 列表失败: HTTP {status}"));
    }
    // 极简 XML 解析：抽取 <d:href>...</d:href> 与 getlastmodified
    let mut items = Vec::new();
    for seg in text.split("<").collect::<Vec<_>>().windows(1) {
        let _ = seg;
        break;
    }
    let lower = text.replace("D:", "d:").replace("<d:response", "\u{1}<d:response");
    for block in lower.split('\u{1}').skip(1) {
        let href = extract_tag(block, "d:href").unwrap_or_default();
        let name = href
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_string();
        if name.is_empty() || !name.to_lowercase().ends_with(".zip") {
            continue;
        }
        items.push(json!({
            "name": urldecode(&name),
            "href": href,
            "size": extract_tag(block, "d:getcontentlength").and_then(|s| s.parse::<u64>().ok()),
            "lastModified": extract_tag(block, "d:getlastmodified"),
        }));
    }
    items.reverse();
    Ok(json!({ "items": items }))
}

fn extract_tag(s: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let a = s.find(&open)? + open.len();
    let b = s[a..].find(&close)? + a;
    Some(s[a..b].trim().to_string())
}

fn urldecode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[tauri::command]
pub async fn mihomo_webdav_restore(
    state: State<'_, MihomoState>,
    name: String,
) -> Result<String, String> {
    let d = dav_of(&state)?;
    let resp = dav_client()
        .get(format!("{}/{}", d.url, name))
        .basic_auth(&d.user, Some(&d.pass))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("下载失败: HTTP {}", resp.status()));
    }
    let data = resp.bytes().await.map_err(|e| e.to_string())?;
    restore_backup_zip(&state.data_dir, &data)?;
    Ok("恢复完成，请重启内核使配置生效".into())
}

#[tauri::command]
pub async fn mihomo_webdav_delete(
    state: State<'_, MihomoState>,
    name: String,
) -> Result<(), String> {
    let d = dav_of(&state)?;
    let resp = dav_client()
        .delete(format!("{}/{}", d.url, name))
        .basic_auth(&d.user, Some(&d.pass))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("删除失败: HTTP {}", resp.status()));
    }
    Ok(())
}
