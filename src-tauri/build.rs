use std::fs;
use std::path::Path;

fn main() {
    // ai-tools/（仓库根）是工具注册表的**唯一源头**；运行时读的是 src-tauri/_up_/ai-tools，
    // 由 npm 侧的 scripts/bundle-resources.mjs 复制过去。
    // 但直接跑 cargo build / cargo test 时不会经过 npm，于是「定义加了却读不到」。
    // 这里在构建脚本里也同步一次：声明依赖 + 覆盖复制，两条路都保证最新。
    println!("cargo:rerun-if-changed=../ai-tools");
    sync_dir("../ai-tools", "_up_/ai-tools");

    #[cfg(target_os = "windows")]
    {
        // 仅正式(release)构建让应用以管理员身份运行：通过 UAC manifest 请求 requireAdministrator。
        // 这样打包部署后无论从注册表 Run 键 / 托盘 / 双击启动，都会静默提权为管理员。
        // 注意：开发(debug)构建不做提权，否则 `tauri dev` / `cargo run` 直接运行
        // 非提权 exe 会报「os error 740 请求的操作需要提升」，导致 `yarn start` 无法启动。
        let is_release = std::env::var("PROFILE").map(|p| p == "release").unwrap_or(false);
        let windows = tauri_build::WindowsAttributes::new();
        if is_release {
            // 自定义清单须额外声明 Common Controls v6 依赖，否则 tauri-plugin-dialog 的对话框会异常。
            let windows = windows.app_manifest(r#"
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity
        type="win32"
        name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0"
        processorArchitecture="*"
        publicKeyToken="6595b64144ccf1df"
        language="*"
      />
    </dependentAssembly>
  </dependency>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="requireAdministrator" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#);
            tauri_build::try_build(
                tauri_build::Attributes::new().windows_attributes(windows),
            )
            .expect("failed to run build script");
        } else {
            tauri_build::build();
        }
        return;
    }
    #[cfg(not(target_os = "windows"))]
    tauri_build::build()
}

/// 把源目录（相对 src-tauri）同步到目标目录：目录建齐、文件覆盖、**多余项删除**。
///
/// 早先只加不删，于是「从 ai-tools 删掉一个工具」在 cargo 构建/dev 下不生效 ——
/// 运行时读到的还是 _up_ 里的旧副本，工具列表里那个不该存在的工具一直在
/// （npm 侧的 bundle-resources.mjs 是整目录重建的，没这个问题；这里补上删除）。
///
/// 失败仅告警：构建脚本抛错会让整个项目编译不过，为资源同步失败中断编译不值当。
fn sync_dir(src: &str, dst: &str) {
    let src = Path::new(src);
    let dst = Path::new(dst);
    if !src.is_dir() {
        println!("cargo:warning=资源源目录不存在，跳过同步: {}", src.display());
        return;
    }
    if let Err(e) = fs::create_dir_all(dst) {
        println!("cargo:warning=创建资源目录失败 {}：{}", dst.display(), e);
        return;
    }
    if let Err(e) = copy_dir_all(src, dst) {
        println!("cargo:warning=同步 {} → {} 失败：{}", src.display(), dst.display(), e);
    }
    if let Err(e) = prune_dir(src, dst) {
        println!("cargo:warning=清理残留资源失败 {}：{}", dst.display(), e);
    }
}

/// 递归删掉目标目录里源目录已经没有的条目（只双向比对文件名）。
fn prune_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(dst)? {
        let entry = entry?;
        let name = entry.file_name();
        let counterpart = src.join(&name);
        if !counterpart.exists() {
            let path = entry.path();
            if path.is_dir() {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_file(&path)?;
            }
            continue;
        }
        if entry.path().is_dir() && counterpart.is_dir() {
            prune_dir(&counterpart, &entry.path())?;
        }
    }
    Ok(())
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            fs::create_dir_all(&target)?;
            copy_dir_all(&path, &target)?;
        } else {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}
