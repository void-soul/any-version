//! 项目目录根表 — SDK 模块「机器级目录」的唯一真源。
//!
//! 背景：`projects/<id>/` 下的 24 个项目配置里，大量目录字段重复写死了同一批
//! 机器级路径（`C:\Program Files`、`C:\ProgramData`、`C:\msys64`、
//! `%USERPROFILE%\AppData\Local` …）。这些路径一旦分多处维护：
//!   - 改一处漏一处 → 同一批「同特征目录」在不同项目里表现不一致；
//!   - 系统盘/Program Files 被重定向（ARM、精简系统、企业镜像）时全部失效；
//!   - 无法统一审计「到底引用了哪些机器目录」。
//!
//! 因此把具备相同特征的目录收敛为**同一组根配置**（本模块的 `ROOTS`），
//! 项目配置只用 `{根名}` 占位符引用，例如：
//!   `C:\Program Files\MySQL\MySQL Server 8.4\bin` → `{program_files}\MySQL\MySQL Server 8.4\bin`
//!
//! 解析规则：**环境变量优先、字面默认值兜底**。环境变量缺失/为空（或指向不存在的
//! 目录）时回退默认值，因此在本机（默认布局）行为与改造前完全一致。
//!
//! 展开入口：`crate::commands::utils::expand_home` → `expand()`。
//! 即所有走 `expand_home` 的路径模板（数据目录、缓存目录、配置文件候选、
//! 启动命令参数）以及 `sdk_resolver` 的固定路径规则都会自动获得根展开。

use std::collections::HashMap;
use std::sync::OnceLock;

/// 单个目录根的定义。
struct RootDef {
    /// 候选环境变量（按顺序取第一个非空值）。
    env: &'static [&'static str],
    /// 环境变量都不可用时的字面默认值。
    default: &'static str,
}

/// 机器级目录根表：**同一组根配置**，所有项目共享。
///
/// 只收录「被两个及以上项目引用」或「属于系统固定位置」的目录；项目自有的
/// 相对目录（如 `{install_root}\data`）不属于根，由项目配置自己声明。
const ROOTS: &[(&str, RootDef)] = &[
    (
        "program_files",
        RootDef {
            env: &["ProgramFiles", "ProgramW6432"],
            default: "C:\\Program Files",
        },
    ),
    (
        "program_data",
        RootDef {
            env: &["ProgramData"],
            default: "C:\\ProgramData",
        },
    ),
    (
        "local_appdata",
        RootDef {
            env: &["LOCALAPPDATA"],
            default: "{home}\\AppData\\Local",
        },
    ),
    (
        "roaming_appdata",
        RootDef {
            env: &["APPDATA"],
            default: "{home}\\AppData\\Roaming",
        },
    ),
    (
        "msys2",
        RootDef {
            env: &["MSYS2_ROOT", "MSYS2_HOME"],
            default: "C:\\msys64",
        },
    ),
];

/// 按名字长度降序排列的已解析根表（缓存）。
///
/// 降序是为了让 `program_files_x86` 这类「前缀包含另一个根名」的情况先被替换，
/// 避免 `{program_files}` 抢先吃掉 `{program_files_x86}` 的前缀。当前根表没有
/// 这种前缀关系，但保持该不变量可防止后续新增根时踩坑。
fn resolved_roots() -> &'static Vec<(String, String)> {
    static CACHE: OnceLock<Vec<(String, String)>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut list: Vec<(String, String)> = ROOTS
            .iter()
            .map(|(name, def)| {
                let value = def
                    .env
                    .iter()
                    .find_map(|var| {
                        let v = std::env::var(var).ok()?;
                        let v = v.trim().to_string();
                        if v.is_empty() { None } else { Some(v) }
                    })
                    .unwrap_or_else(|| def.default.to_string());
                // 默认值里可以再引用 {home}（如 local_appdata）
                let value = value.replace("{home}", &crate::commands::utils::get_home_dir().to_string_lossy());
                (name.to_string(), value)
            })
            .collect();
        list.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        list
    })
}

/// 展开路径模板中的目录根占位符（`{program_files}` 等）。
///
/// - 不含 `{` 的字符串原样返回（快路径，绝大多数调用走这里）。
/// - 未登记的占位符（如 `{install_root}`、`{version}`）原样保留，由各自的
///   展开方（`service::expand_path_template` / 下载模板渲染）处理。
pub fn expand(text: &str) -> String {
    if !text.contains('{') {
        return text.to_string();
    }
    let mut out = text.to_string();
    for (name, value) in resolved_roots() {
        let token = format!("{{{}}}", name);
        if out.contains(&token) {
            out = out.replace(&token, value);
        }
    }
    out
}

/// `{根名}` → 解析值快照（供诊断/测试使用）。
pub fn roots_snapshot() -> HashMap<String, String> {
    resolved_roots().iter().cloned().collect()
}

/// 本模块登记的根名清单。
pub fn root_names() -> Vec<String> {
    ROOTS.iter().map(|(n, _)| n.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_keeps_unknown_placeholders() {
        // 未登记的占位符必须原样保留，交给后续展开方
        let out = expand("{install_root}\\{version}\\data");
        assert_eq!(out, "{install_root}\\{version}\\data");
    }

    #[test]
    fn expand_replaces_known_roots() {
        let pf = roots_snapshot().get("program_files").unwrap().clone();
        let out = expand("{program_files}\\MySQL\\bin");
        assert_eq!(out, format!("{}\\MySQL\\bin", pf));
    }

    #[test]
    fn expand_is_noop_without_placeholder() {
        assert_eq!(expand("C:\\plain\\path"), "C:\\plain\\path");
        assert_eq!(expand(""), "");
    }

    #[test]
    fn all_roots_resolve_to_non_empty_absolute() {
        for (name, value) in resolved_roots() {
            assert!(!value.trim().is_empty(), "根 {name} 解析为空");
            assert!(
                value.len() >= 3 && value.as_bytes()[1] == b':',
                "根 {name} 解析结果不是盘符绝对路径: {value}"
            );
        }
    }
}
