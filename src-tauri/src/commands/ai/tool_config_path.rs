//! 工具配置文件「实际落盘路径」解析。
//!
//! 抄自 EchoBird c6f4bc25（`feat: update OpenCode v2 support`）：OpenCode v2 起配置目录
//! 支持 `OPENCODE_CONFIG_DIR` / `XDG_CONFIG_HOME` 覆盖，且原生二进制优先读
//! `opencode.jsonc`。我们原先只按 `~/.config/opencode/opencode.json` 写死，
//! 在这两种机器上会出现「写进去了但工具不读」。
//!
//! 解析顺序（与 EchoBird 的 `opencode_config_dir_from` 一致）：
//! 1. `path_env_dirs` 中第一个非空环境变量的值作为**目录**（文件名沿用声明路径的）
//! 2. `xdg_subdir` 且 `XDG_CONFIG_HOME` 非空 → `<XDG_CONFIG_HOME>/<subdir>`
//! 3. 声明路径自身（调用方已完成 `~` / `%VAR%` 展开）
//!
//! 之后再处理文件名：`prefer_existing_extensions` 中任一扩展名的同 stem 文件已存在时，
//! 优先沿用它（OpenCode v2 只在 `opencode.jsonc` 里读配置）。

use std::path::{Path, PathBuf};

/// 解析工具配置文件的实际落盘路径。
///
/// - `declared_abs`：配置里声明的路径，调用方已完成 `~` / `%VAR%` 展开。
/// - `env_dirs`：`path_env_dirs` 依次查到的环境变量值（`None` = 未设置）。
/// - `xdg_subdir` / `xdg_home`：`XDG_CONFIG_HOME` 分支所需的子目录名与变量值。
/// - `prefer_existing_extensions`：已存在时优先沿用的扩展名（可带或不带前导 `.`）。
///
/// 空串 / 纯空白一律视同未设置 —— `set X=` 在 Windows 上很常见。
pub fn resolve_config_path(
    declared_abs: &Path,
    env_dirs: &[Option<String>],
    xdg_subdir: Option<&str>,
    xdg_home: Option<&str>,
    prefer_existing_extensions: &[String],
) -> PathBuf {
    let non_blank = |value: &str| !value.trim().is_empty();

    let dir_override = env_dirs
        .iter()
        .flatten()
        .find(|value| non_blank(value))
        .map(PathBuf::from)
        .or_else(|| match (xdg_subdir, xdg_home) {
            (Some(subdir), Some(home)) if !subdir.trim().is_empty() && non_blank(home) => {
                Some(PathBuf::from(home.trim()).join(subdir.trim()))
            }
            _ => None,
        });

    // 目录被覆盖时文件名沿用声明路径的；未覆盖时仍要在原目录里做扩展名择优
    let (dir, target) = match dir_override {
        Some(dir) => {
            let Some(file_name) = declared_abs.file_name() else {
                return declared_abs.to_path_buf();
            };
            let target = dir.join(file_name);
            (dir, target)
        }
        None => {
            let Some(parent) = declared_abs.parent() else {
                return declared_abs.to_path_buf();
            };
            (parent.to_path_buf(), declared_abs.to_path_buf())
        }
    };

    // 已存在的同名不同扩展名文件优先（OpenCode v2 只在 opencode.jsonc 里读配置）
    let Some(stem) = target.file_stem().and_then(|s| s.to_str()) else {
        return target;
    };
    for extension in prefer_existing_extensions {
        let extension = extension.trim().trim_start_matches('.');
        if extension.is_empty() {
            continue;
        }
        let candidate = dir.join(format!("{}.{}", stem, extension));
        if candidate.is_file() {
            return candidate;
        }
    }
    target
}

#[cfg(test)]
mod tests {
    use super::resolve_config_path;
    use std::fs;
    use std::path::PathBuf;

    fn probe_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("anyver-cfgpath-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn env_dir_override_wins_over_declared_path() {
        let declared = PathBuf::from("/home/u/.config/opencode/opencode.json");
        let out = resolve_config_path(
            &declared,
            &[Some("/custom".to_string()), Some("/ignored".to_string())],
            Some("opencode"),
            Some("/xdg"),
            &[],
        );
        // 第一个非空环境变量生效，且文件名沿用声明路径的
        assert_eq!(out, PathBuf::from("/custom").join("opencode.json"));
    }

    #[test]
    fn xdg_config_home_is_used_when_env_override_absent() {
        let declared = PathBuf::from("/home/u/.config/opencode/opencode.json");
        let out = resolve_config_path(&declared, &[None], Some("opencode"), Some("/xdg"), &[]);
        assert_eq!(
            out,
            PathBuf::from("/xdg").join("opencode").join("opencode.json")
        );
    }

    #[test]
    fn blank_values_fall_back_to_declared_path() {
        let declared = PathBuf::from("/home/u/.config/opencode/opencode.json");
        // 空串/纯空白（`set X=` 的产物）不得被当成有效目录
        let out = resolve_config_path(
            &declared,
            &[Some("   ".to_string()), None],
            Some("opencode"),
            Some(""),
            &[],
        );
        assert_eq!(out, declared);
    }

    #[test]
    fn missing_xdg_subdir_skips_xdg_branch() {
        let declared = PathBuf::from("/home/u/.config/opencode/opencode.json");
        let out = resolve_config_path(&declared, &[], None, Some("/xdg"), &[]);
        assert_eq!(out, declared);
    }

    #[test]
    fn existing_alternate_extension_is_preferred() {
        let dir = probe_dir("ext");
        // 声明的是 .json，但 OpenCode v2 只在 opencode.jsonc 里读配置
        let jsonc = dir.join("opencode.jsonc");
        fs::write(&jsonc, "{}").unwrap();
        let out = resolve_config_path(
            &dir.join("opencode.json"),
            &[],
            None,
            None,
            &["jsonc".to_string()],
        );
        assert_eq!(out, jsonc);

        // 同 stem 的候选文件不存在时，仍写声明的路径（不主动造新文件）
        let out2 = resolve_config_path(
            &dir.join("other.json"),
            &[],
            None,
            None,
            &["jsonc".to_string()],
        );
        assert_eq!(out2, dir.join("other.json"));

        // 扩展名前后的 "." 由调用方提供内容，两种写法都接受
        let out3 = resolve_config_path(
            &dir.join("opencode.json"),
            &[],
            None,
            None,
            &[".jsonc".to_string()],
        );
        assert_eq!(out3, jsonc);
    }
}
