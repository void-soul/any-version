use std::fs;
use std::path::{Path, PathBuf};
use serde::{Serialize, Deserialize};
use tauri::{AppHandle, Emitter};
use crate::commands::config::{load_config, get_base_dir};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SdkInfo {
    pub name: String,
    pub category: String,
    pub active_version: String,
    pub installed_versions: Vec<String>,
}

#[derive(Serialize, Clone)]
pub struct DownloadProgress {
    pub sdk: String,
    pub downloaded: u64,
    pub total: u64,
    pub pct: u8,
}

// Minimal JSON decoding structures
#[derive(Deserialize)]
struct GoRelease {
    version: String,
    stable: bool,
}

#[derive(Deserialize)]
struct NodeRelease {
    version: String,
    lts: serde_json::Value,
}

#[derive(Deserialize)]
struct NugetVersions {
    versions: Vec<String>,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
}

#[derive(Deserialize)]
struct AdoptiumReleases {
    releases: Vec<String>,
}

#[derive(Deserialize)]
struct ZuluPackage {
    download_url: String,
    java_version: Vec<i32>,
    name: String,
}

#[derive(Deserialize)]
struct FlutterReleaseJSON {
    releases: Vec<FlutterRelease>,
}

#[derive(Deserialize)]
struct FlutterRelease {
    version: String,
    channel: String,
    archive: String,
}

pub fn setup_temp_dir(prefix: &str) -> Result<(PathBuf, Box<dyn FnOnce() + Send>), String> {
    let base_dir = get_base_dir();
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

pub fn unzip_file(src: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(src).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let outpath = match file.enclosed_name() {
            Some(path) => dest.join(path.to_owned()),
            None => continue,
        };

        if file.name().ends_with('/') {
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

pub fn extract_tar_gz(src: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(src).map_err(|e| e.to_string())?;
    let tar_gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(tar_gz);
    archive.unpack(dest).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn move_extract_to_dest(extracted_dir: &Path, dest_dir: &Path) -> Result<(), String> {
    let entries = fs::read_dir(extracted_dir).map_err(|e| e.to_string())?
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

    let sub_entries = fs::read_dir(&src_dir).map_err(|e| e.to_string())?
        .filter_map(|e| e.ok());

    for entry in sub_entries {
        let old_path = entry.path();
        let new_path = dest_dir.join(entry.file_name());

        if let Err(_) = fs::rename(&old_path, &new_path) {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                crate::commands::cache::copy_dir_all(&old_path, &new_path).map_err(|e| e.to_string())?;
            } else {
                fs::copy(&old_path, &new_path).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

pub async fn download_with_progress<F>(url: &str, dest: &Path, on_progress: F) -> Result<(), String>
where
    F: Fn(u64, u64),
{
    use futures_util::StreamExt;
    let client = reqwest::Client::builder()
        .user_agent("Any-Version-Manager")
        .build()
        .map_err(|e| e.to_string())?;

    let res = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("HTTP request failed with status: {}", res.status()));
    }

    let total = res.content_length().unwrap_or(0);
    let mut file = fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut stream = res.bytes_stream();
    let mut downloaded = 0u64;

    while let Some(item) = stream.next().await {
        let chunk = item.map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut file, &chunk).map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;
        on_progress(downloaded, total);
    }

    Ok(())
}

#[tauri::command]
pub async fn get_sdks_list() -> Result<Vec<SdkInfo>, String> {
    let config = load_config();
    let sdks = vec![
        ("nodejs", "language"),
        ("java", "language"),
        ("python", "language"),
        ("flutter", "language"),
        ("go", "language"),
        ("rust", "language"),
        ("bun", "language"),
        ("android", "mobile"),
        ("harmony", "mobile"),
        ("nginx", "service"),
        ("redis", "service"),
        ("mysql", "service"),
        ("mongodb", "service"),
        ("postgresql", "service"),
        ("maven", "build_tool"),
        ("gradle", "build_tool"),
        ("pub", "build_tool"),
        ("vcpkg", "build_tool"),
        ("yarn", "build_tool"),
        ("pnpm", "build_tool"),
    ];

    let mut list = Vec::new();
    for (name, cat) in sdks {
        let sdk_dir = Path::new(&config.versions_dir).join(name);
        let junction_path = Path::new(&config.links_dir).join(name);

        let active_dir = fs::canonicalize(&junction_path)
            .map(|p| p.to_string_lossy().to_string().trim_start_matches(r"\\?\").to_string().to_lowercase())
            .unwrap_or_default();

        let mut installed = Vec::new();
        if sdk_dir.exists() {
            if let Ok(entries) = fs::read_dir(&sdk_dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        installed.push(entry.file_name().to_string_lossy().to_string());
                    }
                }
            }
        }

        let mut active_version = String::new();
        for v in &installed {
            let v_path = sdk_dir.join(v).to_string_lossy().to_string().to_lowercase();
            if v_path == active_dir {
                active_version = v.clone();
                break;
            }
        }

        list.push(SdkInfo {
            name: name.to_string(),
            category: cat.to_string(),
            active_version,
            installed_versions: installed,
        });
    }

    Ok(list)
}

#[tauri::command]
pub async fn list_remote_versions(sdk_name: String) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .user_agent("Any-Version-Manager")
        .build()
        .map_err(|e| e.to_string())?;

    match sdk_name.as_str() {
        "go" => {
            let releases: Vec<GoRelease> = client.get("https://go.dev/dl/?mode=json&include=all")
                .send().await.map_err(|e| e.to_string())?
                .json().await.map_err(|e| e.to_string())?;
            let versions = releases.into_iter()
                .filter(|r| r.stable)
                .map(|r| r.version.trim_start_matches("go").to_string())
                .take(100)
                .collect();
            Ok(versions)
        }
        "nodejs" => {
            let releases: Vec<NodeRelease> = client.get("https://nodejs.org/dist/index.json")
                .send().await.map_err(|e| e.to_string())?
                .json().await.map_err(|e| e.to_string())?;
            let versions = releases.into_iter()
                .map(|r| {
                    let v = r.version.trim_start_matches('v').to_string();
                    let lts_label = if r.lts.is_boolean() && r.lts.as_bool().unwrap_or(false) {
                        " (LTS)".to_string()
                    } else if r.lts.is_string() {
                        format!(" (LTS: {})", r.lts.as_str().unwrap_or_default())
                    } else {
                        "".to_string()
                    };
                    format!("{}{}", v, lts_label)
                })
                .take(120)
                .collect();
            Ok(versions)
        }
        "python" => {
            let data: NugetVersions = client.get("https://api.nuget.org/v3-flatcontainer/python/index.json")
                .send().await.map_err(|e| e.to_string())?
                .json().await.map_err(|e| e.to_string())?;
            let mut versions: Vec<String> = data.versions.into_iter()
                .filter(|v| !v.contains('-'))
                .collect();
            versions.reverse();
            versions.truncate(100);
            Ok(versions)
        }
        "bun" => {
            let releases: Vec<GithubRelease> = client.get("https://api.github.com/repos/oven-sh/bun/releases")
                .send().await.map_err(|e| e.to_string())?
                .json().await.map_err(|e| e.to_string())?;
            let versions = releases.into_iter()
                .map(|r| r.tag_name.trim_start_matches("bun-v").trim_start_matches('v').to_string())
                .collect();
            Ok(versions)
        }
        "rust" => {
            let releases: Vec<GithubRelease> = client.get("https://api.github.com/repos/rust-lang/rust/releases")
                .send().await.map_err(|e| e.to_string())?
                .json().await.map_err(|e| e.to_string())?;
            let versions = releases.into_iter()
                .filter(|r| !r.tag_name.contains('-'))
                .map(|r| r.tag_name.clone())
                .collect();
            Ok(versions)
        }
        "flutter" => {
            let data: FlutterReleaseJSON = client.get("https://storage.googleapis.com/flutter_infra_release/releases/releases_windows.json")
                .send().await.map_err(|e| e.to_string())?
                .json().await.map_err(|e| e.to_string())?;
            let versions = data.releases.into_iter()
                .filter(|r| r.channel == "stable")
                .map(|r| r.version)
                .collect();
            Ok(versions)
        }
        "java" => {
            let mut versions = Vec::new();
            // Fetch Adoptium versions
            for major in &["21", "17", "11", "8"] {
                let adopt_url = format!("https://api.adoptium.net/v3/info/release_names?project=jdk&release_type=ga&os=windows&architecture=x64&image_type=jdk&version=[{},{})", major, major.parse::<i32>().unwrap() + 1);
                if let Ok(res) = client.get(&adopt_url).send().await {
                    if let Ok(data) = res.json::<AdoptiumReleases>().await {
                        for r in data.releases.into_iter().take(5) {
                            versions.push(format!("adoptium-{}", r.trim_start_matches("jdk-")));
                        }
                    }
                }
            }
            // Azul Zulu
            let zulu_url = "https://api.azul.com/metadata/v1/zulu/packages/?os=windows&arch=amd64&archive_type=zip&java_package_type=jdk&release_status=ga&latest=true&page_size=20";
            if let Ok(res) = client.get(zulu_url).send().await {
                if let Ok(pkgs) = res.json::<Vec<ZuluPackage>>().await {
                    for pkg in pkgs {
                        if pkg.name.contains("-ca-jdk") {
                            let v = pkg.java_version.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(".");
                            versions.push(format!("zulu-{}", v));
                        }
                    }
                }
            }
            versions.push("microsoft-21".to_string());
            versions.push("microsoft-17".to_string());
            versions.push("oracle-21".to_string());
            versions.push("oracle-17".to_string());
            Ok(versions)
        }
        "android" => {
            // Android 命令行工具 (commandline-tools)，来源: Google 官方 dl.google.com 仓库。
            // 这里列出常见的稳定构建号；用户也可在界面手动输入构建号查询。
            // 远程地址形如: https://dl.google.com/android/repository/commandlinetools-win-<build>_latest.zip
            Ok(vec![
                "13114758".to_string(),
                "11076708".to_string(),
                "10406996".to_string(),
                "9477386".to_string(),
                "8512546".to_string(),
            ])
        }
        "harmony" => {
            // OpenHarmony / 鸿蒙 命令行工具 (ohcommandline-tools)，来源: 华为云公共镜像 contentcenter-vali-drcn。
            // 这里列出常见发行版本号；远程地址在下载时会完整透明展示给用户。
            Ok(vec![
                "5.0.5".to_string(),
                "5.0.3".to_string(),
                "4.1.0".to_string(),
                "4.0.0".to_string(),
            ])
        }
        "nginx" => Ok(vec!["1.26.1".to_string(), "1.26.0".to_string(), "1.24.0".to_string(), "1.22.1".to_string()]),
        "redis" => Ok(vec!["5.0.14.1".to_string(), "3.0.504".to_string()]),
        "mysql" => Ok(vec!["8.0.36".to_string(), "8.4.0".to_string(), "5.7.44".to_string()]),
        "mongodb" => Ok(vec!["7.0.5".to_string(), "6.0.13".to_string(), "5.0.24".to_string()]),
        "postgresql" => Ok(vec!["16.2".to_string(), "15.6".to_string(), "14.11".to_string()]),
        "maven" => Ok(vec!["3.9.6".to_string(), "3.8.8".to_string(), "3.6.3".to_string()]),
        "gradle" => Ok(vec!["8.6".to_string(), "8.5".to_string(), "7.6.4".to_string()]),
        "yarn" => Ok(vec!["1.22.19".to_string(), "3.8.1".to_string()]),
        "pnpm" => Ok(vec!["9.0.5".to_string(), "8.15.4".to_string()]),
        _ => Err("不支持的 SDK 类别".to_string()),
    }
}

pub fn get_download_url(sdk_name: &str, version: &str) -> Result<(String, String), String> {
    let version_clean = version.trim_start_matches('v').to_string();
    let download_url: String;
    let mut file_ext = "zip".to_string();

    match sdk_name {
        "go" => {
            download_url = format!("https://go.dev/dl/go{}.windows-amd64.zip", version_clean);
        }
        "nodejs" => {
            download_url = format!("https://nodejs.org/dist/v{}/node-v{}-win-x64.zip", version_clean, version_clean);
        }
        "python" => {
            download_url = format!("https://www.nuget.org/api/v2/package/python/{}", version_clean);
            file_ext = "nupkg".to_string();
        }
        "bun" => {
            download_url = format!("https://github.com/oven-sh/bun/releases/download/bun-v{}/bun-windows-x64.zip", version_clean);
        }
        "android" => {
            // Android 命令行工具：版本号即为 Google 仓库的构建号(build number)。
            // 官方公开下载地址，无需登录。
            download_url = format!("https://dl.google.com/android/repository/commandlinetools-win-{}_latest.zip", version_clean);
        }
        "harmony" => {
            // OpenHarmony / 鸿蒙 命令行工具：华为官方下载需要登录开发者账号，
            // 没有稳定的免登录公开直链。为保证透明与诚实，这里不伪造下载地址，
            // 而是引导用户前往官网下载后，使用『注册本地 SDK』功能导入。
            return Err("鸿蒙(HarmonyOS)命令行工具需在华为开发者官网登录后下载，暂无免登录直链。\n请前往官网下载：https://developer.huawei.com/consumer/cn/download/ （选择 Command Line Tools）。\n下载解压后，使用本页下方的『注册本地 SDK』功能，填入版本号和解压目录即可纳入版本管理。".to_string());
        }
        "rust" => {
            download_url = format!("https://static.rust-lang.org/dist/rust-{}-x86_64-pc-windows-msvc.tar.gz", version_clean);
            file_ext = "tar.gz".to_string();
        }
        "java" => {
            let mut resolved_ver = version.to_string();
            if !version.starts_with("adoptium-") && !version.starts_with("microsoft-") && !version.starts_with("oracle-") && !version.starts_with("zulu-") {
                resolved_ver = format!("adoptium-{}", version);
            }

            if resolved_ver.starts_with("adoptium-") {
                let v = resolved_ver.trim_start_matches("adoptium-");
                download_url = format!("https://api.adoptium.net/v3/binary/version/jdk-{}/windows/x64/jdk/hotspot/normal/eclipse?project=jdk", v);
            } else if resolved_ver.starts_with("microsoft-") {
                let v = resolved_ver.trim_start_matches("microsoft-");
                download_url = format!("https://aka.ms/download-jdk/microsoft-jdk-{}-windows-x64.zip", v);
            } else if resolved_ver.starts_with("oracle-") {
                let v = resolved_ver.trim_start_matches("oracle-");
                download_url = format!("https://download.oracle.com/java/{}/latest/jdk-{}_windows-x64_bin.zip", v, v);
            } else {
                let v = resolved_ver.trim_start_matches("zulu-");
                download_url = format!("https://api.adoptium.net/v3/binary/latest/{}/ga/windows/x64/jdk/hotspot/normal/eclipse?project=jdk", v);
            }
        }
        "flutter" => {
            download_url = format!("https://storage.googleapis.com/flutter_infra_release/releases/stable/windows/flutter_windows_{}-stable.zip", version_clean);
        }
        "nginx" => {
            download_url = format!("https://nginx.org/download/nginx-{}.zip", version_clean);
        }
        "redis" => {
            if version_clean == "3.0.504" {
                download_url = "https://github.com/microsoftarchive/redis/releases/download/win-3.0.504/Redis-x64-3.0.504.zip".to_string();
            } else {
                download_url = format!("https://github.com/tporadowski/redis/releases/download/v{}/Redis-x64-{}.zip", version_clean, version_clean);
            }
        }
        "mysql" => {
            if version_clean.starts_with("5.7") {
                download_url = format!("https://cdn.mysql.com/Downloads/MySQL-5.7/mysql-{}-winx64.zip", version_clean);
            } else if version_clean.starts_with("8.0") {
                download_url = format!("https://cdn.mysql.com/Downloads/MySQL-8.0/mysql-{}-winx64.zip", version_clean);
            } else if version_clean.starts_with("8.4") {
                download_url = format!("https://cdn.mysql.com/Downloads/MySQL-8.4/mysql-{}-winx64.zip", version_clean);
            } else {
                download_url = format!("https://cdn.mysql.com/Downloads/MySQL-8.0/mysql-{}-winx64.zip", version_clean);
            }
        }
        "mongodb" => {
            download_url = format!("https://fastdl.mongodb.org/windows/mongodb-windows-x86_64-{}.zip", version_clean);
        }
        "postgresql" => {
            download_url = format!("https://get.enterprisedb.com/postgresql/postgresql-{}-1-windows-x64-binaries.zip", version_clean);
        }
        "maven" => {
            download_url = format!("https://archive.apache.org/dist/maven/maven-3/{}/binaries/apache-maven-{}-bin.zip", version_clean, version_clean);
        }
        "gradle" => {
            download_url = format!("https://services.gradle.org/distributions/gradle-{}-bin.zip", version_clean);
        }
        "yarn" => {
            download_url = format!("https://github.com/yarnpkg/yarn/releases/download/v{}/yarn-v{}.tar.gz", version_clean, version_clean);
            file_ext = "tar.gz".to_string();
        }
        "pnpm" => {
            download_url = format!("https://github.com/pnpm/pnpm/releases/download/v{}/pnpm-win-x64.exe", version_clean);
            file_ext = "exe".to_string();
        }
        _ => return Err(format!("不支持自动下载的 SDK/服务: {}", sdk_name)),
    }

    Ok((download_url, file_ext))
}

/// 透明化：在不下载任何内容的前提下，返回某个 SDK 版本将要访问的远程地址与文件类型。
/// 用于界面在用户点击"下载"之前，先清楚展示"将从哪里、下载什么文件"。
#[tauri::command]
pub fn get_sdk_download_info(sdk_name: String, version: String) -> Result<DownloadInfo, String> {
    let version_clean = version.split(' ').next().unwrap_or(&version).to_string();
    let (url, file_ext) = get_download_url(&sdk_name, &version_clean)?;
    let host = url
        .split("//")
        .nth(1)
        .and_then(|s| s.split('/').next())
        .unwrap_or("")
        .to_string();
    Ok(DownloadInfo {
        url,
        file_ext,
        host,
    })
}

#[derive(Serialize, Clone)]
pub struct DownloadInfo {
    pub url: String,
    pub file_ext: String,
    pub host: String,
}

#[tauri::command]
pub async fn query_custom_version(sdk_name: String, version: String) -> Result<String, String> {
    let (url, _) = get_download_url(&sdk_name, &version)?;

    let client = reqwest::Client::builder()
        .user_agent("Any-Version-Manager")
        .build()
        .map_err(|e| e.to_string())?;

    let res = client.get(&url)
        .header("Range", "bytes=0-0")
        .send()
        .await;

    match res {
        Ok(r) => {
            if r.status().is_success() || r.status() == reqwest::StatusCode::PARTIAL_CONTENT {
                Ok(url)
            } else {
                Err(format!("未找到该版本（HTTP 状态码: {}）。请确认版本号是否存在及正确。", r.status()))
            }
        }
        Err(e) => Err(format!("网络请求失败: {}", e)),
    }
}

#[tauri::command]
pub async fn install_sdk_version(app: AppHandle, sdk_name: String, version: String) -> Result<(), String> {
    let config = load_config();
    let (download_url, file_ext) = get_download_url(&sdk_name, &version)?;

    // 2. Download with progress emit
    let (temp_dir, cleanup) = setup_temp_dir(&sdk_name)?;
    let archive_path = temp_dir.join(format!("archive.{}", file_ext));

    let sdk_name_cap = sdk_name.clone();
    let app_handle = app.clone();
    let dl_result = download_with_progress(&download_url, &archive_path, move |downloaded, total| {
        let pct = if total > 0 { (downloaded * 100 / total) as u8 } else { 0 };
        let _ = app_handle.emit("download-progress", DownloadProgress {
            sdk: sdk_name_cap.clone(),
            downloaded,
            total,
            pct,
        });
    }).await;

    if let Err(e) = dl_result {
        cleanup();
        return Err(format!("下载失败: {}", e));
    }

    // 3. Extract
    let extract_dir = temp_dir.join("extracted");
    let ext_result = if file_ext == "tar.gz" {
        extract_tar_gz(&archive_path, &extract_dir)
    } else if file_ext == "exe" {
        fs::create_dir_all(&extract_dir).map_err(|e| e.to_string())?;
        fs::copy(&archive_path, extract_dir.join(format!("{}.exe", sdk_name)))
            .map(|_| ())
            .map_err(|e| e.to_string())
    } else {
        unzip_file(&archive_path, &extract_dir)
    };

    if let Err(e) = ext_result {
        cleanup();
        return Err(format!("解压失败: {}", e));
    }

    // 4. Install
    let dest_dir = Path::new(&config.versions_dir).join(&sdk_name).join(&version);
    
    // special python/rust directory manipulation
    let src_dir = if sdk_name == "python" {
        extract_dir.join("tools")
    } else {
        extract_dir
    };

    if let Err(e) = move_extract_to_dest(&src_dir, &dest_dir) {
        cleanup();
        return Err(format!("安装失败: {}", e));
    }

    cleanup();

    // 5. Post-installation configuration (e.g. mysql initialization)
    if sdk_name == "mysql" {
        let my_ini_path = dest_dir.join("my.ini");
        let data_dir = dest_dir.join("data");
        let clean_base = dest_dir.to_string_lossy().replace("\\", "/");
        let clean_data = data_dir.to_string_lossy().replace("\\", "/");

        let my_ini_content = format!(
            "[mysqld]\nport=3306\nbasedir={}\ndatadir={}\nmax_connections=200\ncharacter-set-server=utf8mb4\ndefault-storage-engine=INNODB\ndefault_authentication_plugin=mysql_native_password\n\n[mysql]\ndefault-character-set=utf8mb4\n\n[client]\nport=3306\ndefault-character-set=utf8mb4\n",
            clean_base, clean_data
        );
        let _ = fs::write(&my_ini_path, my_ini_content);

        // Initialize MySQL
        let mysql_daemon = dest_dir.join("bin").join("mysqld.exe");
        let _ = std::process::Command::new(mysql_daemon)
            .args(&["--defaults-file", &my_ini_path.to_string_lossy(), "--initialize-insecure"])
            .output();
    }

    // 6. Auto-switch if first installed
    let junction_path = Path::new(&config.links_dir).join(&sdk_name);
    if !junction_path.exists() {
        let _ = crate::commands::cache::create_junction(&junction_path, &dest_dir);
    }

    // 7. 移动端 SDK：自动配置环境变量，指向稳定的链接目录（切换版本无需改环境变量）
    if sdk_name == "android" {
        let android_home = junction_path.to_string_lossy().to_string();
        let _ = crate::commands::env::set_registry_env("ANDROID_HOME", &android_home);
        let _ = crate::commands::env::set_registry_env("ANDROID_SDK_ROOT", &android_home);
    }

    Ok(())
}

#[tauri::command]
pub fn uninstall_sdk_version(sdk_name: String, version: String) -> Result<(), String> {
    let config = load_config();
    let dest_dir = Path::new(&config.versions_dir).join(&sdk_name).join(&version);
    if !dest_dir.exists() {
        return Err(format!("版本 {} 的 {} 未安装", version, sdk_name));
    }

    // If active, break the link first
    let junction_path = Path::new(&config.links_dir).join(&sdk_name);
    let active_dir = fs::canonicalize(&junction_path)
        .map(|p| p.to_string_lossy().to_string().trim_start_matches(r"\\?\").to_string().to_lowercase())
        .unwrap_or_default();
    let dest_dir_clean = dest_dir.to_string_lossy().to_string().to_lowercase();

    if active_dir == dest_dir_clean {
        let _ = fs::remove_file(&junction_path);
    }

    fs::remove_dir_all(dest_dir).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn use_sdk_version(sdk_name: String, version: String) -> Result<(), String> {
    let config = load_config();
    let dest_dir = Path::new(&config.versions_dir).join(&sdk_name).join(&version);
    if !dest_dir.exists() {
        return Err(format!("版本 {} 的 {} 未安装", version, sdk_name));
    }

    let junction_path = Path::new(&config.links_dir).join(&sdk_name);
    crate::commands::cache::create_junction(&junction_path, &dest_dir)?;
    Ok(())
}

#[tauri::command]
pub fn add_local_sdk_version(sdk_name: String, version: String, local_path: String) -> Result<(), String> {
    let config = load_config();
    let src = Path::new(&local_path);
    if !src.exists() {
        return Err("本地路径不存在".to_string());
    }

    let dest_dir = Path::new(&config.versions_dir).join(&sdk_name).join(&version);
    if dest_dir.exists() {
        return Err("版本已存在，无需重复添加".to_string());
    }

    crate::commands::cache::copy_dir_all(src, &dest_dir).map_err(|e| e.to_string())?;

    // Auto-switch if first installed
    let junction_path = Path::new(&config.links_dir).join(&sdk_name);
    if !junction_path.exists() {
        let _ = crate::commands::cache::create_junction(&junction_path, &dest_dir);
    }

    // 移动端 SDK：注册本地版本后也自动配置环境变量，指向稳定链接目录
    if sdk_name == "android" {
        let android_home = junction_path.to_string_lossy().to_string();
        let _ = crate::commands::env::set_registry_env("ANDROID_HOME", &android_home);
        let _ = crate::commands::env::set_registry_env("ANDROID_SDK_ROOT", &android_home);
    } else if sdk_name == "harmony" {
        let sdk_home = junction_path.to_string_lossy().to_string();
        let _ = crate::commands::env::set_registry_env("OHOS_SDK_HOME", &sdk_home);
    }

    Ok(())
}
