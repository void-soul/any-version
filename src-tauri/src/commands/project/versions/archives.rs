use super::*;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  内部工具函数（从 sdk.rs 迁移）
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 创建临时目录，返回 (路径, 清理闭包)
pub(super) fn setup_temp_dir(prefix: &str) -> Result<(PathBuf, Box<dyn FnOnce() + Send>), String> {
    let base_dir = get_data_dir();
    let temp_root = base_dir.join(".tmp");
    fs::create_dir_all(&temp_root).map_err(|e| e.to_string())?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_dir = temp_root.join(format!("{}_{}", prefix, timestamp));
    fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;

    let cleanup_path = temp_dir.clone();
    let cleanup = move || {
        let _ = fs::remove_dir_all(cleanup_path);
    };

    Ok((temp_dir, Box::new(cleanup)))
}

/// 解压 7z 文件
pub(super) fn extract_7z(src: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    sevenz_rust2::decompress_file(src, dest).map_err(|e| format!("7z 解压失败: {}", e))?;
    Ok(())
}

/// 解压 zip 文件
pub(crate) fn unzip_file(src: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(src).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let outpath = match file.enclosed_name() {
            Some(path) => dest.join(path.to_owned()),
            None => continue,
        };

        if file
            .name()
            .map_err(|e| format!("读取ZIP文件名失败: {e}"))?
            .ends_with('/')
        {
            fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
        } else {
            if let Some(p) = outpath.parent() {
                if !p.exists() {
                    fs::create_dir_all(&p).map_err(|e| e.to_string())?;
                }
            }
            let mut outfile = fs::File::create(&outpath).map_err(|e| e.to_string())?;
            std::io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// 解压 tar.gz 文件
pub(super) fn extract_tar_gz(src: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(src).map_err(|e| e.to_string())?;
    let tar_gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(tar_gz);
    archive.unpack(dest).map_err(|e| e.to_string())?;
    Ok(())
}

/// 提取 tar.xz 归档（需要 xz2 依赖）
pub(super) fn extract_tar_xz(src: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(src).map_err(|e| e.to_string())?;
    let decoder = xz2::read::XzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(dest).map_err(|e| e.to_string())?;
    Ok(())
}

/// 提取 tar.bz2 归档（需要 bzip2 依赖）
pub(super) fn extract_tar_bz2(src: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(src).map_err(|e| e.to_string())?;
    let decoder = bzip2::read::BzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(dest).map_err(|e| e.to_string())?;
    Ok(())
}

/// 提取 MSI 安装包（使用 msiexec /a 行政安装）
pub(super) fn extract_msi(src: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let dest_str = dest.to_string_lossy().to_string();
    let src_str = src.to_string_lossy().to_string();

    let output = crate::commands::hidden_cmd::hidden_cmd("msiexec")
        .args(["/a", &src_str, "/qn", &format!("TARGETDIR={}", dest_str)])
        .output()
        .map_err(|e| format!("msiexec 执行失败: {}", e))?;

    if !output.status.success() {
        return Err(format!("MSI 提取失败，退出码: {:?}", output.status.code()));
    }
    Ok(())
}

/// 递归合并 src 目录内容到 dest。同名目录递归合并，同名文件覆盖。
/// max_depth: 最大递归深度，防止循环 junction/symlink 导致栈溢出。
pub(super) fn merge_dir_all_impl(src: &Path, dest: &Path, max_depth: u32) -> Result<(), String> {
    if max_depth == 0 {
        return Err("递归深度超过限制，跳过".to_string());
    }
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;

    let entries = fs::read_dir(src).map_err(|e| e.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());

        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            // 跳过 junction/symlink 目录，避免循环
            if fs::symlink_metadata(&src_path)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
            {
                continue;
            }
            merge_dir_all_impl(&src_path, &dest_path, max_depth - 1)?;
        } else {
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::copy(&src_path, &dest_path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// 递归合并 src 目录内容到 dest（公开接口，默认最大深度 32）。
pub(super) fn merge_dir_all(src: &Path, dest: &Path) -> Result<(), String> {
    merge_dir_all_impl(src, dest, 32)
}

/// 将解压后的内容移动到目标目录。
/// 当 merge_subdirs=true 时，所有子目录内容递归合并到 dest_dir 根下，
/// 使 rustc/cargo/rust-std-* 等分散的组件融合为同一个目录结构。
pub(crate) fn move_extract_to_dest(
    extracted_dir: &Path,
    dest_dir: &Path,
    merge_subdirs: bool,
) -> Result<(), String> {
    let entries = fs::read_dir(extracted_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .collect::<Vec<_>>();

    let mut src_dir = extracted_dir.to_path_buf();
    if entries.len() == 1 && entries[0].file_type().map(|t| t.is_dir()).unwrap_or(false) {
        src_dir = entries[0].path();
    }

    if dest_dir.exists() {
        fs::remove_dir_all(dest_dir).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;

    let sub_entries = fs::read_dir(&src_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok());

    if merge_subdirs {
        for entry in sub_entries {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                merge_dir_all(&entry.path(), dest_dir)?;
            } else {
                fs::copy(&entry.path(), &dest_dir.join(entry.file_name()))
                    .map_err(|e| e.to_string())?;
            }
        }
    } else {
        for entry in sub_entries {
            let old_path = entry.path();
            let new_path = dest_dir.join(entry.file_name());

            if fs::rename(&old_path, &new_path).is_err() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    crate::commands::cache::copy_dir_all(&old_path, &new_path)
                        .map_err(|e| e.to_string())?;
                } else {
                    fs::copy(&old_path, &new_path).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    Ok(())
}

/// 带进度回调的下载
pub(crate) async fn download_with_progress<F>(
    url: &str,
    dest: &Path,
    on_progress: F,
) -> Result<(), String>
where
    F: Fn(u64, u64, f64),
{
    use futures_util::StreamExt;
    // 下载大文件（如 Rust 200MB+）时不应有短超时
    // connect_timeout 仅限制建立连接的时间，不限制下载总时长
    let client = reqwest::Client::builder()
        .user_agent("Any-Version-Manager")
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let res = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载请求失败: {}", e))?;
    if !res.status().is_success() {
        return Err(format!("HTTP 请求失败，状态码: {}", res.status()));
    }

    let total = res.content_length().unwrap_or(0);
    let mut file = fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut stream = res.bytes_stream();
    let mut downloaded = 0u64;

    let start_time = std::time::Instant::now();

    while let Some(item) = stream.next().await {
        let chunk = item.map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut file, &chunk).map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;

        let elapsed = start_time.elapsed().as_secs_f64();
        let speed = if elapsed > 0.0 {
            downloaded as f64 / elapsed
        } else {
            0.0
        };

        on_progress(downloaded, total, speed);
    }

    Ok(())
}
