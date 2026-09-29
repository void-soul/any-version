//! 插件登记表（`music/plugins/registry.json`）。
//!
//! 存两类东西：
//! - **用户态**：启用开关、排序、导入来源与时间；
//! - **插件自述快照**：platform / version / 作者 / 能力（导入时探测一次）。
//!
//! 自述快照是**刻意缓存**的：列表页要能秒开，不能每次都起 Node 去 `require` 一遍
//! （没装 node 的用户也得能看见自己的插件）。需要最新值时走「刷新信息」重新探测。
//!
//! 表与磁盘**双向对齐**：磁盘上多出的 `.js`（用户自己丢进去的）自动登记；
//! 表里指向已消失文件的条目自动清掉。否则会出现「列表里一堆点不动的插件」
//! 或「刚放进来的插件看不见」。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::plugin_host;

/// 插件自述（探测自插件本体；字段名做 camelCase → snake_case 映射）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PluginMeta {
    /// 来源名，如 `Audiomack`
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    /// 插件自带的更新地址
    #[serde(default)]
    pub src_url: String,
    /// 是否支持搜索（我们要用的两个能力之一）
    #[serde(default)]
    pub has_search: bool,
    /// 是否支持获取播放地址（另一个）
    #[serde(default)]
    pub has_media_source: bool,
    #[serde(default)]
    pub supported_search_type: Vec<String>,
    /// 插件的自定义输入项（用户变量），原样透传给前端
    #[serde(default)]
    pub user_variables: Vec<Value>,
}

impl PluginMeta {
    /// 从桥 `load` 的返回值解析。字段缺失/类型不对时按空值处理 —— 插件千奇百怪，
    /// 一个元信息字段不符合预期不该让整个插件不可用。
    pub fn from_bridge(value: &Value) -> Self {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        PluginMeta {
            platform: text("platform"),
            version: text("version"),
            author: text("author"),
            description: text("description"),
            src_url: text("srcUrl"),
            has_search: value.get("hasSearch").and_then(Value::as_bool) == Some(true),
            has_media_source: value.get("hasMediaSource").and_then(Value::as_bool) == Some(true),
            supported_search_type: value
                .get("supportedSearchType")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            user_variables: value
                .get("userVariables")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        }
    }

    /// 展示名：优先 platform，其次版本串，都没有就空（前端回退文件名）。
    pub fn display_name(&self) -> String {
        self.platform.trim().to_string()
    }

    /// 是否具备「搜索 + 取流」这对必需能力。
    pub fn is_usable(&self) -> bool {
        self.has_search && self.has_media_source
    }
}

fn default_true() -> bool {
    true
}

/// 登记表里的一条。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginEntry {
    /// `scripts/` 下的文件名（含 `.js`），作为稳定 id
    pub file: String,
    /// 用户可见名（默认取自述的 platform，可被用户改）
    #[serde(default)]
    pub name: String,
    /// 启用开关。**缺省为 true**：手改过 registry.json 或用户自己丢进来的插件，
    /// 不该因为少一个字段就静默不生效。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 排序权重（升序）
    #[serde(default)]
    pub order: i64,
    /// 导入来源：URL 或本地路径；空表示是用户自己放进目录的
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub imported_at: String,
    #[serde(default)]
    pub meta: PluginMeta,
}

impl PluginEntry {
    /// 列表页展示名：用户改过的名字 > 自述 platform > 文件名（去扩展名）。
    pub fn display_name(&self) -> String {
        let custom = self.name.trim();
        if !custom.is_empty() {
            return custom.to_string();
        }
        let declared = self.meta.display_name();
        if !declared.is_empty() {
            return declared;
        }
        self.file
            .rsplit_once('.')
            .map(|(stem, _)| stem)
            .unwrap_or(&self.file)
            .to_string()
    }
}

/// 登记表文件内容。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginRegistry {
    #[serde(default)]
    pub plugins: Vec<PluginEntry>,
}

fn registry_path() -> PathBuf {
    plugin_host::plugins_root().join("registry.json")
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn load() -> PluginRegistry {
    match std::fs::read_to_string(registry_path()) {
        Ok(text) => serde_json::from_str::<PluginRegistry>(&text).unwrap_or_default(),
        Err(_) => PluginRegistry::default(),
    }
}

fn save(registry: &PluginRegistry) -> Result<(), String> {
    let value = serde_json::to_value(registry).map_err(|e| e.to_string())?;
    // 复用曲库同一套原子写（临时文件 + rename）
    super::library::write_json_atomic(&registry_path(), &value)
}

// ─── 文件名安全 ───

/// 把任意来源串变成安全的脚本文件名。
///
/// **必须挡住路径穿越**：来源可能是 `https://host/../../x.js` 或用户手输的路径，
/// 拼路径前不洗净就会写到 `scripts/` 之外（覆盖用户其它文件）。
/// 这里只取最后一段、去掉查询串与片段、过滤非法字符，并强制 `.js`。
pub fn sanitize_script_file_name(raw: &str) -> String {
    // 统一分隔符后只认最后一段：`a/b/../c.js` → `c.js`
    let last = raw
        .rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .unwrap_or("plugin.js");
    let stem = last.split(['?', '#']).next().unwrap_or(last);

    let mut cleaned: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@') {
                c
            } else {
                // 非 ASCII（中文插件名）与其它符号一律换成下划线，避免文件系统与 require 的编码坑
                '_'
            }
        })
        .collect();
    // 前导点会被当成隐藏文件，`..` 已被上面过滤，这里再兜一层
    cleaned = cleaned.trim_start_matches('.').to_string();
    if cleaned.is_empty() {
        cleaned = "plugin".to_string();
    }
    if !cleaned.to_lowercase().ends_with(".js") {
        cleaned.push_str(".js");
    }
    cleaned
}

/// 从来源串（URL 或本地路径）推荐一个插件文件名。
///
/// MusicFree 官方插件仓库的形态是 `.../dist/<插件名>/index.js` —— 直接取最后一段会把
/// 50 个插件全命名成 `index.js`，于是「导入第二个就变成 index-2.js」。
/// 所以最后一段是 `index.js` 时改用**上一层目录名**。
pub fn suggest_script_file_name(source: &str) -> String {
    let without_query = source.split(['?', '#']).next().unwrap_or(source);
    let segments: Vec<&str> = without_query
        .rsplit(['/', '\\'])
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .collect();
    let last = segments.first().copied().unwrap_or("plugin.js");
    let chosen = if last.eq_ignore_ascii_case("index.js") {
        segments
            .get(1)
            .map(|parent| format!("{parent}.js"))
            .unwrap_or_else(|| last.to_string())
    } else {
        last.to_string()
    };
    sanitize_script_file_name(&chosen)
}

/// 在同名冲突时给出唯一文件名（`x.js` → `x-2.js`）。
pub fn unique_file_name(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|name| name == base) {
        return base.to_string();
    }
    let (stem, ext) = match base.rsplit_once('.') {
        Some((stem, ext)) => (stem.to_string(), format!(".{ext}")),
        None => (base.to_string(), String::new()),
    };
    for index in 2..1000 {
        let candidate = format!("{stem}-{index}{ext}");
        if !taken.iter().any(|name| name == &candidate) {
            return candidate;
        }
    }
    format!("{stem}-{}{ext}", std::process::id())
}

/// 由登记名解析出磁盘路径。
///
/// **只允许单层文件名**：登记表是可信度较低的数据（可被手改），若放任
/// `../../x.js` 这种值，删除/写入就会跑到插件目录之外。
pub fn resolve_script_path(file: &str) -> Result<PathBuf, String> {
    let trimmed = file.trim();
    if trimmed.is_empty() {
        return Err("插件文件名为空".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err(format!("非法的插件文件名: {trimmed}"));
    }
    Ok(plugin_host::scripts_dir().join(trimmed))
}

// ─── 与磁盘对齐 ───

/// 扫 `scripts/*.js`，返回文件名列表（已排序，保证行为稳定）。
pub fn scan_script_files() -> Vec<String> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(plugin_host::scripts_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let is_script = path
                .extension()
                .map(|ext| ext.eq_ignore_ascii_case("js"))
                .unwrap_or(false);
            if is_script {
                if let Some(name) = path.file_name() {
                    files.push(name.to_string_lossy().to_string());
                }
            }
        }
    }
    files.sort();
    files
}

/// 让登记表与磁盘一致，返回是否有改动。
///
/// - 磁盘上多出的 `.js` → 登记（未探测自述，前端会显示「未识别」，可点「刷新信息」）
/// - 表里指向已消失文件的条目 → 清掉
pub fn reconcile(registry: &mut PluginRegistry) -> bool {
    let on_disk = scan_script_files();
    let mut changed = false;

    let before = registry.plugins.len();
    registry
        .plugins
        .retain(|entry| on_disk.iter().any(|name| name == &entry.file));
    if registry.plugins.len() != before {
        changed = true;
    }

    let mut next_order = registry
        .plugins
        .iter()
        .map(|entry| entry.order)
        .max()
        .map(|max| max + 1)
        .unwrap_or(0);
    for file in on_disk {
        if registry.plugins.iter().any(|entry| entry.file == file) {
            continue;
        }
        registry.plugins.push(PluginEntry {
            file,
            name: String::new(),
            enabled: true,
            order: next_order,
            source: String::new(),
            imported_at: now_iso(),
            meta: PluginMeta::default(),
        });
        next_order += 1;
        changed = true;
    }
    changed
}

/// 读取并对齐磁盘后的列表（按 order，再按文件名，保证稳定顺序）。
pub fn list_reconciled() -> Result<Vec<PluginEntry>, String> {
    let mut registry = load();
    if reconcile(&mut registry) {
        save(&registry)?;
    }
    let mut plugins = registry.plugins;
    plugins.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.file.cmp(&b.file)));
    Ok(plugins)
}

// ─── 增删改 ───

/// 新增或覆盖一条（按 `file` 匹配）。
pub fn upsert(registry: &mut PluginRegistry, entry: PluginEntry) {
    match registry
        .plugins
        .iter_mut()
        .find(|existing| existing.file == entry.file)
    {
        Some(slot) => *slot = entry,
        None => registry.plugins.push(entry),
    }
}

/// 导入后登记（写入磁盘 + 更新登记表）。`meta` 为探测结果。
///
/// 已有同名条目时是**更新**：只换插件内容与自述，**保留用户改过的名字、启用状态与排序**。
/// 否则「更新一下插件」会顺手把用户的偏好也重置掉。
///
/// `name_hint` 只在**新条目**上生效（订阅清单里给的名字，如「网易」比插件自述的
/// `wy` 更贴用户）。已有条目一律保留用户的名字 —— 用户改过名就该一直是那个名字。
pub fn register_import(
    file: &str,
    source: &str,
    meta: PluginMeta,
    name_hint: Option<&str>,
) -> Result<PluginEntry, String> {
    let mut registry = load();
    // **先判「是否已登记过」，再与磁盘对齐。**
    // `install_plugin` 是先写文件、后登记的，所以对齐时那个文件已经在磁盘上 ——
    // 若先对齐，reconcile 会把它收编成条目，于是「新导入」与「更新」分不出来，
    // `name_hint` 就永远用不上（订阅名形同虚设）。
    let registered_before = registry.plugins.iter().any(|entry| entry.file == file);
    reconcile(&mut registry);
    let existing = registry
        .plugins
        .iter()
        .find(|entry| entry.file == file)
        .cloned();
    let order = existing.as_ref().map(|entry| entry.order).unwrap_or_else(|| {
        registry
            .plugins
            .iter()
            .map(|entry| entry.order)
            .max()
            .map(|max| max + 1)
            .unwrap_or(0)
    });
    let entry = PluginEntry {
        file: file.to_string(),
        name: if registered_before {
            existing
                .as_ref()
                .map(|entry| entry.name.clone())
                .unwrap_or_default()
        } else {
            name_hint
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or("")
                .to_string()
        },
        enabled: existing.as_ref().map(|entry| entry.enabled).unwrap_or(true),
        order,
        source: source.to_string(),
        imported_at: now_iso(),
        meta,
    };
    upsert(&mut registry, entry.clone());
    save(&registry)?;
    Ok(entry)
}

/// 更新某条的自述快照（「刷新信息」用）。
pub fn update_meta(file: &str, meta: PluginMeta) -> Result<PluginEntry, String> {
    let mut registry = load();
    let entry = registry
        .plugins
        .iter_mut()
        .find(|entry| entry.file == file)
        .ok_or_else(|| format!("未找到插件 {file}"))?;
    entry.meta = meta;
    let updated = entry.clone();
    save(&registry)?;
    Ok(updated)
}

/// 启停。
pub fn set_enabled(file: &str, enabled: bool) -> Result<Vec<PluginEntry>, String> {
    let mut registry = load();
    let entry = registry
        .plugins
        .iter_mut()
        .find(|entry| entry.file == file)
        .ok_or_else(|| format!("未找到插件 {file}"))?;
    entry.enabled = enabled;
    save(&registry)?;
    list_reconciled()
}

/// 改名（空串 = 回退自述名）。
pub fn set_name(file: &str, name: &str) -> Result<Vec<PluginEntry>, String> {
    let mut registry = load();
    let entry = registry
        .plugins
        .iter_mut()
        .find(|entry| entry.file == file)
        .ok_or_else(|| format!("未找到插件 {file}"))?;
    entry.name = name.trim().to_string();
    save(&registry)?;
    list_reconciled()
}

/// 按给定顺序重排（只认表里存在的文件，其余保持相对次序排在后面）。
pub fn reorder(files: &[String]) -> Result<Vec<PluginEntry>, String> {
    let mut registry = load();
    for (index, file) in files.iter().enumerate() {
        if let Some(entry) = registry.plugins.iter_mut().find(|entry| &entry.file == file) {
            entry.order = index as i64;
        }
    }
    save(&registry)?;
    list_reconciled()
}

/// 删除插件：文件（含 `.js` 同名附属）与登记条目一起清掉。
///
/// 文件删除失败**不阻断**登记表清理 —— 否则会留下「列表里删不掉、点着又报错」的死条目。
pub fn remove(file: &str) -> Result<Vec<PluginEntry>, String> {
    let path = resolve_script_path(file)?;
    let mut warnings = Vec::new();
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            warnings.push(format!("删除文件失败（{}）: {e}", path.display()));
        }
    }
    let mut registry = load();
    registry.plugins.retain(|entry| entry.file != file);
    save(&registry)?;
    for warning in warnings {
        eprintln!("[plugin] {warning}");
    }
    list_reconciled()
}

/// 按落盘顺序生成「可参与搜索」的插件列表（启用 + 具备必需能力）。
pub fn searchable_plugins() -> Result<Vec<PluginEntry>, String> {
    Ok(list_reconciled()?
        .into_iter()
        // 未探测过自述的（用户刚丢进来的）也放行：能力由桥在调用时报错，
        // 比在列表页就拦掉更好 —— 否则用户得先「刷新信息」才能试。
        .filter(|entry| entry.enabled && (entry.meta.is_usable() || entry.meta.platform.is_empty()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 取全局串行锁并确保测试根就绪。
    ///
    /// **每个碰磁盘的用例都必须先拿它**：`registry.json` 与 `scripts/` 是全模块共享的，
    /// `cargo test` 默认并行，不串行就会出现「刚设好的启用状态被另一个用例 save 覆盖」
    /// 这类随机失败。
    fn guard() -> std::sync::MutexGuard<'static, ()> {
        let guard = plugin_host::serialize_test();
        plugin_host::shared_test_root();
        guard
    }

    fn write_script(name: &str) {
        std::fs::create_dir_all(plugin_host::scripts_dir()).unwrap();
        std::fs::write(plugin_host::scripts_dir().join(name), b"// plugin").unwrap();
    }

    fn sample_meta() -> PluginMeta {
        PluginMeta {
            platform: "Demo".to_string(),
            version: "1.0.0".to_string(),
            has_search: true,
            has_media_source: true,
            ..PluginMeta::default()
        }
    }

    /// 路径穿越必须被挡住 —— 登记表可被手改，来源串也可能带 `../`。
    #[test]
    fn script_file_names_cannot_escape_the_directory() {
        let _g = guard();
        assert_eq!(sanitize_script_file_name("../../evil.js"), "evil.js");
        assert_eq!(sanitize_script_file_name("a/b/../c.js"), "c.js");
        assert_eq!(sanitize_script_file_name(r"..\..\windows\x.js"), "x.js");
        assert_eq!(
            sanitize_script_file_name("https://x.com/p/index.js?v=2#frag"),
            "index.js"
        );
        // 非 ASCII（中文插件名）逐字换成下划线，ASCII 部分保留，且一定带 .js
        assert_eq!(sanitize_script_file_name("猫耳FM.js"), "__FM.js");
        assert_eq!(sanitize_script_file_name("noext"), "noext.js");
        assert_eq!(sanitize_script_file_name(".hidden.js"), "hidden.js");
        assert_eq!(sanitize_script_file_name(""), "plugin.js");

        // 解析阶段再挡一次：即使登记表被手改成穿越值也不许通过
        assert!(resolve_script_path("../x.js").is_err());
        assert!(resolve_script_path(r"a\b.js").is_err());
        assert!(resolve_script_path("ok.js").is_ok());
    }

    /// 官方插件仓库是 `dist/<名字>/index.js`，全取最后一段会得到一堆 `index.js`。
    #[test]
    fn suggested_names_use_the_plugin_directory_not_the_index_file() {
        assert_eq!(
            suggest_script_file_name("https://gitee.com/x/MusicFreePlugins/raw/v0.1/dist/audiomack/index.js"),
            "audiomack.js"
        );
        assert_eq!(
            suggest_script_file_name("https://x.com/dist/猫耳fm/index.js?token=1"),
            "__fm.js"
        );
        // 不是 index.js 时就用它自己
        assert_eq!(suggest_script_file_name("https://x.com/a/my-plugin.js"), "my-plugin.js");
        assert_eq!(suggest_script_file_name(r"D:\plugins\我的音源.js"), "____.js");
        // 退化输入（地址只有主机名）也不能产出非法/空文件名
        assert_eq!(suggest_script_file_name("https://x.com/"), "x.com.js");
        assert_eq!(suggest_script_file_name(""), "plugin.js");
    }

    /// 更新插件不能顺手把用户的名字 / 启用状态 / 排序重置掉。
    #[test]
    fn reimport_keeps_user_preferences() {
        let _g = guard();
        write_script("keep.js");
        list_reconciled().unwrap();
        set_name("keep.js", "我的音源").unwrap();
        set_enabled("keep.js", false).unwrap();

        let entry = register_import(
            "keep.js",
            "https://x.com/keep.js",
            sample_meta(),
            Some("订阅里的名字"),
        )
        .unwrap();

        assert_eq!(entry.name, "我的音源", "更新不该丢掉用户改的名字（也不该改成订阅名）");
        assert!(!entry.enabled, "更新不该把用户关掉的插件悄悄打开");
        assert_eq!(entry.meta.platform, "Demo", "自述应刷新为新探测结果");
        assert_eq!(entry.source, "https://x.com/keep.js");
        let _ = std::fs::remove_file(plugin_host::scripts_dir().join("keep.js"));
    }

    /// 订阅清单里的名字（如「网易」）比插件自述的 `wy` 更贴用户，新导入时采用它；
    /// 但**只在首次**生效，不能覆盖用户后来改的名字。
    #[test]
    fn subscription_name_is_used_only_for_new_entries() {
        let _g = guard();
        write_script("named.js");

        let first = register_import(
            "named.js",
            "https://x.com/named.js",
            sample_meta(),
            Some("网易"),
        )
        .unwrap();
        assert_eq!(first.display_name(), "网易");

        set_name("named.js", "我的网易").unwrap();
        let again = register_import(
            "named.js",
            "https://x.com/named.js",
            sample_meta(),
            Some("网易"),
        )
        .unwrap();
        assert_eq!(again.display_name(), "我的网易", "用户改过的名字不能被订阅名盖掉");

        // 没有订阅名、自述也没有 platform 时，回退文件名
        write_script("bare.js");
        let bare = register_import("bare.js", "x", PluginMeta::default(), None).unwrap();
        assert_eq!(bare.display_name(), "bare");

        let _ = std::fs::remove_file(plugin_host::scripts_dir().join("named.js"));
        let _ = std::fs::remove_file(plugin_host::scripts_dir().join("bare.js"));
    }

    #[test]
    fn duplicate_file_names_get_a_suffix() {
        let taken = vec!["a.js".to_string(), "a-2.js".to_string()];
        assert_eq!(unique_file_name("b.js", &taken), "b.js");
        assert_eq!(unique_file_name("a.js", &taken), "a-3.js");
    }

    /// 磁盘上多出来的 `.js`（用户自己丢进去的）必须被自动登记，
    /// 否则「放进去了却看不见」。
    #[test]
    fn files_dropped_on_disk_are_adopted_and_missing_ones_dropped() {
        let _g = guard();
        write_script("adopted.js");
        write_script("vanishing.js");
        let mut registry = PluginRegistry::default();
        assert!(reconcile(&mut registry));
        assert!(registry.plugins.iter().any(|e| e.file == "adopted.js"));
        // 重复对齐不应再产生改动（幂等）
        assert!(!reconcile(&mut registry));

        std::fs::remove_file(plugin_host::scripts_dir().join("vanishing.js")).unwrap();
        assert!(reconcile(&mut registry));
        assert!(!registry.plugins.iter().any(|e| e.file == "vanishing.js"));
        let _ = std::fs::remove_file(plugin_host::scripts_dir().join("adopted.js"));
    }

    /// 缺 `enabled` 字段的条目按「启用」处理：手改过配置的用户不该踩这个坑。
    #[test]
    fn missing_enabled_field_defaults_to_enabled() {
        let entry: PluginEntry =
            serde_json::from_str(r#"{"file":"x.js"}"#).expect("应能反序列化");
        assert!(entry.enabled);
        assert_eq!(entry.display_name(), "x");
    }

    #[test]
    fn display_name_prefers_custom_then_platform_then_file() {
        let mut entry = PluginEntry {
            file: "some.js".to_string(),
            name: String::new(),
            enabled: true,
            order: 0,
            source: String::new(),
            imported_at: String::new(),
            meta: PluginMeta::default(),
        };
        assert_eq!(entry.display_name(), "some", "无名无自述时回退文件名（去扩展名）");
        entry.meta = sample_meta();
        assert_eq!(entry.display_name(), "Demo", "其次用插件自述的 platform");
        entry.name = "我的音源".to_string();
        assert_eq!(entry.display_name(), "我的音源", "用户改名优先");
    }

    #[test]
    fn bridge_meta_is_parsed_with_camel_case_keys() {
        let value = serde_json::json!({
            "platform": "Audiomack",
            "version": "0.0.2",
            "author": "猫头猫",
            "srcUrl": "https://example.com/index.js",
            "hasSearch": true,
            "hasMediaSource": true,
            "supportedSearchType": ["music", "album"],
            "userVariables": [{"key": "token"}]
        });
        let meta = PluginMeta::from_bridge(&value);
        assert_eq!(meta.platform, "Audiomack");
        assert_eq!(meta.src_url, "https://example.com/index.js");
        assert!(meta.is_usable());
        assert_eq!(meta.supported_search_type, vec!["music", "album"]);
        assert_eq!(meta.user_variables.len(), 1);
    }

    /// 只支持搜索、不会取流的插件不算可用 —— 拿它播不了歌。
    #[test]
    fn plugin_without_media_source_is_not_usable() {
        let meta = PluginMeta {
            has_search: true,
            has_media_source: false,
            ..PluginMeta::default()
        };
        assert!(!meta.is_usable());
    }

    /// 启停 / 重排要能落盘并回读。
    #[test]
    fn enabled_and_order_round_trip() {
        let _g = guard();
        write_script("order_a.js");
        write_script("order_b.js");
        let listed = list_reconciled().unwrap();
        // 只查本次关心的这条：共享目录里可能还留着其它用例的文件
        let a = listed.iter().find(|e| e.file == "order_a.js").unwrap();
        assert!(a.enabled, "新登记的插件应默认启用: {a:?}");

        set_enabled("order_a.js", false).unwrap();
        let listed = list_reconciled().unwrap();
        let a = listed.iter().find(|e| e.file == "order_a.js").unwrap();
        assert!(!a.enabled);

        reorder(&["order_b.js".to_string(), "order_a.js".to_string()]).unwrap();
        let listed = list_reconciled().unwrap();
        let pos_b = listed.iter().position(|e| e.file == "order_b.js").unwrap();
        let pos_a = listed.iter().position(|e| e.file == "order_a.js").unwrap();
        assert!(pos_b < pos_a, "重排后 b 应排在 a 前面: {listed:?}");

        let _ = std::fs::remove_file(plugin_host::scripts_dir().join("order_a.js"));
        let _ = std::fs::remove_file(plugin_host::scripts_dir().join("order_b.js"));
    }

    /// 关掉的插件不参与搜索。
    #[test]
    fn disabled_plugins_are_not_searchable() {
        let _g = guard();
        write_script("searchable.js");
        // 先与磁盘对齐：`update_meta` 改的是登记表里**已存在**的条目
        list_reconciled().unwrap();
        update_meta("searchable.js", sample_meta()).unwrap();
        set_enabled("searchable.js", false).unwrap();
        let names: Vec<String> = searchable_plugins()
            .unwrap()
            .into_iter()
            .map(|entry| entry.file)
            .collect();
        assert!(!names.contains(&"searchable.js".to_string()), "{names:?}");
        let _ = std::fs::remove_file(plugin_host::scripts_dir().join("searchable.js"));
    }

    /// 文件已不存在时删除条目仍要成功（否则列表里会留下删不掉的死条目）。
    #[test]
    fn removal_succeeds_even_if_file_is_already_gone() {
        let _g = guard();
        write_script("gone.js");
        list_reconciled().unwrap();
        std::fs::remove_file(plugin_host::scripts_dir().join("gone.js")).unwrap();
        let listed = remove("gone.js").unwrap();
        assert!(!listed.iter().any(|entry| entry.file == "gone.js"));
    }
}
