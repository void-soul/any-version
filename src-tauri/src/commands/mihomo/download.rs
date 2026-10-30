use super::*;

// ---------- 下载/安装 ----------
/// 统一的下载客户端：带 60s 超时，避免无限挂起（F7）
pub(super) fn download_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

pub(super) async fn download_bytes(url: &str) -> Result<Vec<u8>, String> {
    let resp = download_client()
        .get(url)
        .header("User-Agent", "any-version")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    Ok(bytes.to_vec())
}

/// 下载并校验 SHA256（仅允许官方 GitHub 源，F1）
pub(super) async fn download_bytes_verified(
    url: &str,
    expected_sha256: &str,
) -> Result<Vec<u8>, String> {
    let data = download_bytes(url).await?;
    let actual = sha256_hex(&data);
    if !expected_sha256.eq_ignore_ascii_case(&actual) {
        return Err(format!(
            "SHA256 校验失败: 期望 {expected_sha256}, 实际 {actual}"
        ));
    }
    Ok(data)
}

/// 计算二进制内容的 SHA256 十六进制串
pub(super) fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    let d = h.finalize();
    let mut s = String::with_capacity(d.len() * 2);
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
pub(super) fn unzip_to(data: &[u8], dest: &Path) -> Result<(), String> {
    // F3 修复：先解压到临时目录，校验非空后再原子 rename 覆盖；保留旧 exe 的 .bak
    let tmp_dest = dest.with_extension(format!("tmp{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dest);
    std::fs::create_dir_all(&tmp_dest).map_err(|e| e.to_string())?;
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| e.to_string())?;
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = f
            .name()
            .map_err(|e| format!("读取ZIP文件名失败: {e}"))?
            .to_string();
        if name.ends_with('/') {
            continue;
        }
        let fname = name.split('/').last().unwrap_or(&name).to_string();
        let out = tmp_dest.join(&fname);
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut buf).map_err(|e| e.to_string())?;
        total += buf.len() as u64;
        std::fs::write(&out, &buf).map_err(|e| e.to_string())?;
    }
    if total == 0 {
        return Err("解压结果为空，疑似损坏的 zip，已拒绝覆盖".to_string());
    }
    if dest.exists() {
        let _ = std::fs::rename(dest, dest.with_extension("bak"));
    }
    std::fs::rename(&tmp_dest, dest).map_err(|e| e.to_string())
}
pub(super) async fn get_latest_mihomo_release() -> Result<(String, String), String> {
    let api = "https://api.github.com/repos/MetaCubeX/mihomo/releases/latest";
    // F7 修复：加超时，避免上游挂起时永久阻塞
    let resp = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .get(api)
        .header("User-Agent", "any-version")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    // F7 补充：保留状态码并给出可操作提示（401 令牌失效 / 403 限流 / 404 仓库不存在），
    // 避免 error_for_status() 一把带过，用户看不出是限流还是网络问题。
    let status = resp.status();
    if !status.is_success() {
        return Err(format!(
            "GitHub API 返回 {}{}",
            status,
            crate::commands::utils::github_status_hint(status.as_u16())
        ));
    }
    let v: Value = resp.json().await.map_err(|e| e.to_string())?;
    let assets = v
        .get("assets")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    for a in assets {
        let name = a
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string();
        if name.contains("windows") && name.contains("amd64") && name.ends_with(".zip") {
            if let Some(url) = a.get("browser_download_url").and_then(|x| x.as_str()) {
                return Ok((name, url.to_string()));
            }
        }
    }
    Err("未找到适配的 mihomo 发布资源".into())
}
