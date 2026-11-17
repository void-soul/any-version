//! 「能否安装在线版本」的统一判定。
//!
//! 为什么要单独一份：这个语义在前后端各写过一遍，容易分叉。
//! - 前端 `hasVersionSupportOf` 决定「版本」页签是否出现、托管时该选项默认是否勾选；
//! - 后端若只认 `download_url_template`（`tool_version.rs` 的旧写法），
//!   则「按版本前缀映射」的 redis/mysql/postgresql/zig 与 github_asset 的 ghidra
//!   都会被判成「不支持」——界面上有下载入口，自动升级却永远跳过它们。
//!
//! 判定 = 有版本清单来源 + 会拼下载地址，缺一不可。

use super::types::ProjectDef;

/// 有「版本清单来源」才能列出可安装版本（static 列表或远程接口）。
pub fn has_version_source(def: &ProjectDef) -> bool {
    def.remote_versions_config.is_some()
}

/// 会拼下载地址。五种方式任一即可：
/// 通用模板、按版本前缀映射、GitHub release 资产、npm 包、git 仓库。
pub fn has_download_url_builder(def: &ProjectDef) -> bool {
    def.download_url_template.is_some()
        || def.version_url_prefix_map.is_some()
        || def.github_asset.is_some()
        || def.npm_pkg_name.is_some()
        || def.is_git_repo
}

/// 能否安装在线版本。与前端 `hasVersionSupportOf` 同一语义。
pub fn supports_online_install(def: &ProjectDef) -> bool {
    has_version_source(def) && has_download_url_builder(def)
}

/// 某个具体版本是否能命中一条下载地址规则（不联网，只复刻 `get_download_url` 的分支顺序）。
///
/// 这是在防一类真实故障：版本清单里列了 v，但 URL 规则只覆盖了别的版本前缀，
/// 用户点「安装」得到的是「未配置下载地址: xxx」——Redis 的 8.x 就是这样
/// （清单有 8.10.1，映射表只写了 7./6./5./4./3.）。
pub fn resolves_download_url(def: &ProjectDef, version: &str) -> bool {
    // 类型特定分支：有这些配置就不依赖版本号拼 URL
    if def.github_asset.is_some() || def.npm_pkg_name.is_some() || def.is_git_repo {
        return true;
    }

    let label = def
        .remote_versions_config
        .as_ref()
        .and_then(|c| c.get("version_label"))
        .and_then(|v| v.as_str());
    let stripped = match label {
        Some(l) => version.strip_prefix(&format!("{l} ")).unwrap_or(version),
        None => version,
    };
    // 与 downloads.rs 一致：去掉前导 v 再参与前缀匹配
    let clean = stripped.strip_prefix('v').unwrap_or(stripped);

    if let Some(map) = def.version_url_prefix_map.as_ref() {
        // remote_versions_url 分支用的是未去 v 的串，这里两种都试，宁可宽松也不能漏判
        if map.keys().any(|p| clean.starts_with(p)) || map.keys().any(|p| stripped.starts_with(p)) {
            return true;
        }
    }

    def.download_url_template.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def_with(f: impl FnOnce(&mut ProjectDef)) -> ProjectDef {
        let mut def = ProjectDef::default();
        def.id = "probe".to_string();
        f(&mut def);
        def
    }

    /// 只有版本清单、没有下载地址 → 列得出但装不了。
    #[test]
    fn version_source_alone_is_not_enough() {
        let def = def_with(|d| d.remote_versions_config = Some(serde_json::json!({ "type": "static", "versions": ["1.0"] })));
        assert!(has_version_source(&def));
        assert!(!supports_online_install(&def));
    }

    /// 只有下载地址、没有版本清单 → 装得了但列不出来。
    #[test]
    fn url_builder_alone_is_not_enough() {
        let def = def_with(|d| d.download_url_template = Some("https://example.com/{version}.zip".into()));
        assert!(has_download_url_builder(&def));
        assert!(!supports_online_install(&def));
    }

    /// 按版本前缀映射（redis/mysql/postgresql/zig）必须算支持 —— 这正是旧写法漏掉的一类。
    #[test]
    fn prefix_map_counts_as_url_builder() {
        let def = def_with(|d| {
            d.remote_versions_config = Some(serde_json::json!({ "type": "static", "versions": ["7.0.0"] }));
            d.version_url_prefix_map = Some(std::collections::HashMap::from([(
                "7.".to_string(),
                "https://example.com/{version}.zip".to_string(),
            )]));
        });
        assert!(supports_online_install(&def));
    }

    #[test]
    fn github_asset_npm_and_git_also_count() {
        let gh = def_with(|d| {
            d.remote_versions_config = Some(serde_json::json!({}));
            d.github_asset = Some(serde_json::json!({}));
        });
        assert!(supports_online_install(&gh));

        let npm = def_with(|d| {
            d.remote_versions_config = Some(serde_json::json!({}));
            d.npm_pkg_name = Some("pkg".into());
        });
        assert!(supports_online_install(&npm));

        let git = def_with(|d| {
            d.remote_versions_config = Some(serde_json::json!({}));
            d.is_git_repo = true;
        });
        assert!(supports_online_install(&git));
    }

    #[test]
    fn empty_project_cannot_install_online() {
        assert!(!supports_online_install(&def_with(|_| {})));
    }

    /// 服务类项目（mysql / redis / postgresql / nginx / mongodb / frpc / frps）必须能装在线版本。
    ///
    /// 少任何一项配置，用户在托管后就只能自己下压缩包再「手动指定目录」，
    /// 等于这个项目在 Kira 里只是个外壳。新增服务项目漏配会直接红。
    #[test]
    fn every_service_project_supports_online_install() {
        use crate::commands::project::registry;
        use crate::commands::project::types::ProjectCategory;

        // 与 registry 测试一致：读仓库里的 projects/，不受打包产物影响
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("src-tauri 应有上级目录")
            .join("projects");
        let defs = registry::load_from_dir(&dir).expect("projects 目录应能解析");

        let mut missing = Vec::new();
        for def in defs {
            if !(def.is_service || def.category == ProjectCategory::Service) {
                continue;
            }
            if !supports_online_install(&def) {
                missing.push(format!(
                    "{} (版本清单={}, 下载地址={})",
                    def.id,
                    has_version_source(&def),
                    has_download_url_builder(&def)
                ));
            }
        }
        assert!(
            missing.is_empty(),
            "以下服务项目无法安装在线版本，托管后只能手动指定目录: {missing:?}"
        );
    }

    /// 静态版本清单里的**每一个**版本都必须能拼出下载地址。
    ///
    /// 真实故障：Redis 清单里有 8.10.1，但 URL 映射只写了 7./6./5./4./3.，
    /// 于是「安装最新版」直接报「未配置下载地址: redis」。
    /// 这条测试把「清单」与「映射」钉在一起，谁改漏了谁红。
    #[test]
    fn every_static_version_resolves_a_download_url() {
        use crate::commands::project::registry;

        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("src-tauri 应有上级目录")
            .join("projects");
        let defs = registry::load_from_dir(&dir).expect("projects 目录应能解析");

        let mut broken = Vec::new();
        for def in defs {
            let Some(cfg) = def.remote_versions_config.as_ref() else {
                continue;
            };
            // 只有静态清单能离线判断；动态拉取的版本号未知，不在此列
            if cfg.get("type").and_then(|v| v.as_str()) != Some("static") {
                continue;
            }
            let versions = cfg
                .get("versions")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for v in versions {
                let Some(v) = v.as_str() else { continue };
                if !resolves_download_url(&def, v) {
                    broken.push(format!("{}@{v}", def.id));
                }
            }
        }
        assert!(
            broken.is_empty(),
            "以下版本在清单里但没有任何下载地址规则（安装会报「未配置下载地址」）: {broken:?}"
        );
    }

    /// 任何项目都必须至少有一种「拼下载地址」的方式。
    ///
    /// 少了它，`get_download_url` 会直接落到兜底分支报「未配置下载地址: xxx」——
    /// 不管版本清单里列了什么。这条对所有项目生效（不限 static 清单）。
    #[test]
    fn every_project_has_a_download_url_builder() {
        use crate::commands::project::registry;

        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("src-tauri 应有上级目录")
            .join("projects");
        let defs = registry::load_from_dir(&dir).expect("projects 目录应能解析");

        let missing: Vec<&str> = defs
            .iter()
            .filter(|def| !has_download_url_builder(def))
            .map(|def| def.id.as_str())
            .collect();
        assert!(
            missing.is_empty(),
            "以下项目没有任何下载地址规则，安装必然失败: {missing:?}"
        );
    }
}
