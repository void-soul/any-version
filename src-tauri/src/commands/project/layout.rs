//! 安装根布局（`root_layout`）的执行层。
//!
//! 少数工具把「自己的安装位置」写死在目录结构里，SDK 的「版本库 + 链接目录」模型
//! 与它的假设不重合，托管后必须把这些结构补出来，否则工具本体能跑、装东西却失败：
//!
//! - **scoop**：`lib/core.ps1` 用 `$PSScriptRoot\..\..\..\..` 反推安装根，
//!   SDK 的平铺布局（`sdk/scoop/lib`）反推成盘符根，`$bucketsdir` 变成 `D:\buckets`；
//!   它的 shim 模板又固定取 `<root>\apps\scoop\current\supporting\shims\kiennq\shim.exe`，
//!   该路径不存在时 `scoop install` 会在「Creating shim」一步失败。
//!
//! 因此声明式地补两件事（见 `projects/scoop/config.json` 的 `root_layout`）：
//! 1. `<root>\apps\<id>\current` → SDK 版本链接目录的 junction（版本切换时链式跟随）；
//! 2. `<root>\shims\<name>.cmd` 包装，让 cmd 环境也能调用 PowerShell 脚本。

use std::fs;
use std::path::{Path, PathBuf};

use super::types::{ProjectDef, RootLayoutDef};

/// 读环境变量里声明的根路径（该变量由 env_vars 托管写入）。
fn resolve_root(layout: &RootLayoutDef) -> Option<PathBuf> {
    let raw = crate::commands::env::get_registry_env(&layout.env_var)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

/// 托管 / 修复 / 安装完成后调用：补出根布局并确保 shims 在 PATH 里。
pub fn apply(def: &ProjectDef, link_dir: &Path) -> Result<(), String> {
    let Some(layout) = def.root_layout.as_ref() else {
        return Ok(());
    };
    // 还没装版本时 `links_dir/<id>` 不存在，布局无从投影；安装完成后再调一次。
    if !link_dir.exists() {
        return Ok(());
    }
    let Some(root) = resolve_root(layout) else {
        return Ok(());
    };

    apply_at(layout, link_dir, &root)?;

    // shims 进用户 PATH：scoop 官方安装的结果状态，`scoop install` 自己也会加，
    // 但「刚托管、还没装过任何包」时没人加，cmd 里就敲不出 scoop。
    let shims = root.join("shims");
    if shims.exists() {
        let _ = crate::commands::env::add_to_user_path(&[shims.to_string_lossy().to_string()]);
    }

    if let Some(bucket) = layout.ensure_bucket.as_deref() {
        ensure_bucket_async(&root, link_dir, bucket);
    }

    Ok(())
}

/// 取消托管时调用：撤掉由我们生成的 junction 与 shim 包装（只动自己造的东西）。
pub fn remove(def: &ProjectDef, link_dir: &Path) {
    let Some(layout) = def.root_layout.as_ref() else {
        return;
    };
    let Some(root) = resolve_root(layout) else {
        return;
    };

    for link in &layout.link_dirs {
        let dst = root.join(&link.rel);
        // 只删重解析点（junction），绝不 remove_dir_all 跟随目标内容。
        if fs::symlink_metadata(&dst)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            let _ = fs::remove_dir(&dst);
        }
    }

    for shim in &layout.shim_wrappers {
        let dst = root.join("shims").join(format!("{}.cmd", shim.name));
        let script = link_dir.join(&shim.script);
        if let Ok(current) = fs::read_to_string(&dst) {
            if current == wrapper_content(&script) {
                let _ = fs::remove_file(&dst);
            }
        }
    }
}

/// 纯文件系统部分：建 junction + 写 shim 包装。幂等，且**不接管真实目录**。
fn apply_at(layout: &RootLayoutDef, link_dir: &Path, root: &Path) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|e| format!("创建安装根 {} 失败: {}", root.display(), e))?;

    for link in &layout.link_dirs {
        let dst = root.join(&link.rel);
        let target = if link.target.trim().is_empty() {
            link_dir.to_path_buf()
        } else {
            link_dir.join(&link.target)
        };
        if !target.exists() {
            continue;
        }

        match fs::symlink_metadata(&dst) {
            // 已是 junction：指向正确就跳过，指向别处则重建（重建只删链接本身）。
            Ok(m) if m.file_type().is_symlink() => {
                let already_ok = fs::read_link(&dst)
                    .map(|p| normalize(&p) == normalize(&target))
                    .unwrap_or(false);
                if already_ok {
                    continue;
                }
                let _ = fs::remove_dir(&dst);
            }
            // 真实目录 / 文件：用户自己造的结构，不接管。
            Ok(_) => continue,
            Err(_) => {}
        }

        crate::commands::cache::create_junction(&dst, &target)?;
    }

    for shim in &layout.shim_wrappers {
        let script = link_dir.join(&shim.script);
        if !script.exists() {
            continue;
        }
        let dst = root.join("shims").join(format!("{}.cmd", shim.name));
        let content = wrapper_content(&script);
        if fs::read_to_string(&dst).map(|c| c == content).unwrap_or(false) {
            continue;
        }
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&dst, content).map_err(|e| format!("写入 {} 失败: {}", dst.display(), e))?;
    }

    Ok(())
}

/// 首次托管时后台补 bucket：git clone 耗时不进托管主流程，失败只记日志。
fn ensure_bucket_async(root: &Path, link_dir: &Path, bucket: &str) {
    let bucket_dir = root.join("buckets").join(bucket);
    if bucket_dir.exists() {
        return;
    }
    let script = link_dir.join("bin").join("scoop.ps1");
    if !script.exists() {
        return;
    }

    let root = root.to_path_buf();
    let bucket = bucket.to_string();
    std::thread::spawn(move || {
        let mut cmd = crate::commands::hidden_cmd::hidden_cmd("powershell");
        cmd.arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(&script)
            .arg("bucket")
            .arg("add")
            .arg(&bucket)
            .env("SCOOP", &root);
        match cmd.output() {
            Ok(out) if out.status.success() => {
                eprintln!("[layout] 已添加 {} bucket", bucket);
            }
            Ok(out) => {
                eprintln!(
                    "[layout] 添加 {} bucket 失败（退出码 {:?}），可稍后手动 scoop bucket add {}",
                    bucket,
                    out.status.code(),
                    bucket
                );
            }
            Err(e) => {
                eprintln!("[layout] 添加 {} bucket 失败: {}", bucket, e);
            }
        }
    });
}

/// cmd 包装内容：优先 pwsh，回退 Windows PowerShell。CRLF 是 .cmd 的硬要求。
fn wrapper_content(script: &Path) -> String {
    let s = script.to_string_lossy();
    format!(
        "@rem {s}\r\n\
         @echo off\r\n\
         where /q pwsh.exe\r\n\
         if %errorlevel% equ 0 (\r\n    \
             pwsh -noprofile -ex unrestricted -file \"{s}\"  %*\r\n\
         ) else (\r\n    \
             powershell -noprofile -ex unrestricted -file \"{s}\"  %*\r\n\
         )\r\n"
    )
}

/// junction 目标比较用：去掉 `\\?\` 前缀与尾部分隔符，忽略大小写。
fn normalize(p: &Path) -> String {
    p.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::{RootLayoutLink, RootLayoutShim};

    fn layout() -> RootLayoutDef {
        RootLayoutDef {
            env_var: "SCOOP".to_string(),
            link_dirs: vec![RootLayoutLink {
                rel: "apps\\scoop\\current".to_string(),
                target: String::new(),
            }],
            shim_wrappers: vec![RootLayoutShim {
                name: "scoop".to_string(),
                script: "bin\\scoop.ps1".to_string(),
            }],
            ensure_bucket: None,
        }
    }

    fn temp_root(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("kira_layout_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn creates_junction_and_shim_idempotently() {
        let base = temp_root("idem");
        let link_dir = base.join("sdk").join("scoop");
        fs::create_dir_all(link_dir.join("bin")).unwrap();
        fs::write(link_dir.join("bin").join("scoop.ps1"), "# stub").unwrap();
        let root = base.join("root");

        apply_at(&layout(), &link_dir, &root).unwrap();

        let link = root.join("apps").join("scoop").join("current");
        assert!(link.exists(), "应创建 apps\\scoop\\current junction");
        assert!(
            fs::symlink_metadata(&link).unwrap().file_type().is_symlink(),
            "必须是 junction 而不是拷贝"
        );
        let shim = root.join("shims").join("scoop.cmd");
        let content = fs::read_to_string(&shim).expect("应生成 scoop.cmd");
        assert!(content.contains("bin\\scoop.ps1"), "shim 应指向版本目录里的脚本");
        assert!(content.contains("\r\n"), "cmd 包装必须是 CRLF");

        // 第二次执行不应报错，也不应改变指向。
        apply_at(&layout(), &link_dir, &root).unwrap();
        assert!(link.exists());
    }

    #[test]
    fn never_takes_over_a_real_directory() {
        let base = temp_root("real");
        let link_dir = base.join("sdk").join("scoop");
        fs::create_dir_all(link_dir.join("bin")).unwrap();
        fs::write(link_dir.join("bin").join("scoop.ps1"), "# stub").unwrap();
        let root = base.join("root");

        // 用户自己造了一个真实目录占位。
        let link = root.join("apps").join("scoop").join("current");
        fs::create_dir_all(&link).unwrap();
        fs::write(link.join("keep.txt"), "keep").unwrap();

        apply_at(&layout(), &link_dir, &root).unwrap();

        assert!(link.join("keep.txt").exists(), "真实目录里的内容不能被删掉");
        assert!(
            !fs::symlink_metadata(&link).unwrap().file_type().is_symlink(),
            "真实目录不能被替换成 junction"
        );
    }

    #[test]
    fn skips_when_version_link_dir_is_missing() {
        let base = temp_root("missing");
        let link_dir = base.join("sdk").join("scoop");
        let root = base.join("root");

        apply_at(&layout(), &link_dir, &root).unwrap();

        assert!(!root.join("apps").exists(), "还没装版本时不该投影");
    }
}
