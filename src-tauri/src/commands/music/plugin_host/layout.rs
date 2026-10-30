use super::*;

/// 宿主桥脚本（编译期内联，避免依赖打包资源的目录布局）。
pub(super) const BRIDGE_JS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/musicfree-bridge/bridge.js"
));
/// 桥的依赖清单。
pub(super) const BRIDGE_MANIFEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/musicfree-bridge/package.json"
));

/// 桥保证可用的依赖，与 `package.json` 一一对应。
/// 用来判断「依赖是否就绪」：要求每个包在 `node_modules` 里都有实体。
pub(super) const REQUIRED_PACKAGES: &[&str] = &[
    "axios",
    "cheerio",
    "crypto-js",
    "dayjs",
    "big-integer",
    "qs",
    "he",
];

// ─── 目录布局 ───

/// 测试注入的数据根。
///
/// **必须有这个钩子**：本模块会往磁盘写 bridge.js / package.json。
/// 若测试直接走 `crate::commands::music::library::music_dir()`，一次 `cargo test` 就会往真实用户的
/// `data_dir/music/plugins` 里写东西 —— 本项目已经因为「测试绕过声明目录」踩过一次
/// 真实事故（见 `codex_catalog` 的注释），所以这里照 `buddy::store::TEST_DATA_ROOT`
/// 的既有做法，一开始就留出注入点。
#[cfg(test)]
pub(super) static TEST_ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);

#[cfg(test)]
pub(super) fn set_test_root(root: PathBuf) {
    *TEST_ROOT.lock().unwrap() = Some(root);
}

/// 写入型测试的全局串行锁。
///
/// `plugin_host` 与 `plugin_registry` 共用同一个测试根（同一份 `registry.json`、
/// 同一个 `scripts/`），而 `cargo test` 默认并行 —— 不串行的话，
/// 「A 测试刚设好的启用状态被 B 测试的 save 覆盖」这类随机失败必然出现
/// （同 `buddy::store` 的处理方式）。
#[cfg(test)]
pub(in crate::commands::music) fn serialize_test() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // 单个测试失败会毒化锁，恢复它以免连锁拖垮后续用例
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 本模块与 `plugin_registry` 共用的测试根。
///
/// **必须只有一份**：两个测试模块各自设一次根的话，后写的会盖掉先写的，
/// 于是「另一个模块的测试把文件写进前一个模块的目录」这种乱象会出现，且是随机失败。
#[cfg(test)]
pub(in crate::commands::music) fn shared_test_root() -> PathBuf {
    static INIT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    INIT.get_or_init(|| {
        let dir =
            std::env::temp_dir().join(format!("kira_music_plugin_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建测试根失败");
        set_test_root(dir.clone());
        dir
    })
    .clone()
}

/// 插件根目录：`<data_dir>/music/plugins`。
pub fn plugins_root() -> PathBuf {
    #[cfg(test)]
    {
        if let Some(root) = TEST_ROOT.lock().unwrap().clone() {
            return root.join("plugins");
        }
    }
    crate::commands::music::library::music_dir().join("plugins")
}

/// 插件本体目录（`scripts/*.js`）。
pub fn scripts_dir() -> PathBuf {
    plugins_root().join("scripts")
}

/// 宿主桥脚本路径。
pub fn bridge_script_path() -> PathBuf {
    plugins_root().join("bridge.js")
}

/// 依赖清单路径。
pub fn manifest_path() -> PathBuf {
    plugins_root().join("package.json")
}

/// 依赖安装目录。
pub fn node_modules_dir() -> PathBuf {
    plugins_root().join("node_modules")
}

/// 确保目录布局与桥资产就位。返回**本次是否写了东西**（幂等：内容一致就不动）。
///
/// 用内容比对而非「存在即跳过」：app 升级后桥脚本会变，不更新的话就是在跑旧协议。
pub fn ensure_layout() -> Result<bool, String> {
    let root = plugins_root();
    std::fs::create_dir_all(scripts_dir())
        .map_err(|e| format!("创建插件目录 {} 失败: {e}", root.display()))?;

    let mut wrote = false;
    for (path, content) in [
        (bridge_script_path(), BRIDGE_JS),
        (manifest_path(), BRIDGE_MANIFEST),
    ] {
        let same = std::fs::read_to_string(&path)
            .map(|existing| existing == content)
            .unwrap_or(false);
        if !same {
            std::fs::write(&path, content.as_bytes())
                .map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;
            wrote = true;
        }
    }
    Ok(wrote)
}
