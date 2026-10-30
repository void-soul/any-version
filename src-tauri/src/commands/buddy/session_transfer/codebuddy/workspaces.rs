use super::*;

// ─── 工作区哈希 → 真实项目路径 ───

/// 进程级缓存：工作区哈希 → 解析出的真实路径（None = 解析失败也缓存，避免反复开库）。
pub(super) static WORKSPACE_PATH_CACHE: std::sync::LazyLock<
    std::sync::Mutex<HashMap<String, Option<String>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// 把 history 下的工作区哈希目录名还原成真实项目路径。
///
/// 哈希规则（实测验证）：**MD5(项目路径)**，其中 Windows 路径盘符为小写、
/// 分隔符为反斜杠（如 `md5("e:\pro\my\any-version") = 48399de9…`）。
/// 候选路径取 CodeBuddy CN `state.vscdb` 的最近打开列表
/// （`history.recentlyOpenedPathsList`：folderUri 与 workspace.configPath）。
///
/// 还原失败（最近列表被清理/从未在本机打开过）返回 None ——
/// 显示层回退到原始哈希，不影响按哈希定位文件。
pub(crate) fn resolve_workspace_display(workspace_hash: &str) -> Option<String> {
    if let Some(cached) = WORKSPACE_PATH_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.get(workspace_hash).cloned())
    {
        return cached;
    }
    let resolved = resolve_workspace_display_uncached(workspace_hash);
    if let Ok(mut cache) = WORKSPACE_PATH_CACHE.lock() {
        cache.insert(workspace_hash.to_string(), resolved.clone());
    }
    resolved
}

pub(super) fn resolve_workspace_display_uncached(workspace_hash: &str) -> Option<String> {
    let db = crate::commands::buddy::codebuddy_cn::default_state_db_path()?;
    let conn = rusqlite::Connection::open(&db).ok()?;
    let raw: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = 'history.recentlyOpenedPathsList'",
            [],
            |row| row.get(0),
        )
        .ok()?;
    let list: Value = serde_json::from_str(&raw).ok()?;
    for entry in list.get("entries")?.as_array()? {
        // 两种形态：folderUri（文件夹）与 workspace.configPath（.code-workspace 文件）
        for key in ["folderUri", "configPath"] {
            let Some(uri) = entry.get(key).and_then(Value::as_str) else {
                continue;
            };
            let Some(path) = uri_to_os_path(uri) else {
                continue;
            };
            let digest = format!("{:x}", md5::compute(path.as_bytes()));
            if digest == workspace_hash {
                return Some(path);
            }
        }
    }
    None
}

/// `file:///e%3A/pro/my/any-version` → `e:\pro\my\any-version`（盘符小写，与哈希口径一致）。
/// 非 file 协议（remote 等）返回 None。
pub(super) fn uri_to_os_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let decoded = percent_decode(rest);
    let path = decoded.trim_start_matches('/');
    if path.is_empty() {
        return None;
    }
    if cfg!(target_os = "windows") {
        // `e:/pro/x` → `e:\pro\x`，盘符转小写（实测哈希用小写盘符）
        let mut chars = path.chars();
        let drive = chars.next()?;
        let tail: String = chars.collect();
        let tail = tail.replace('/', "\\");
        if !tail.starts_with(':') {
            return None;
        }
        Some(format!("{}{}", drive.to_ascii_lowercase(), tail))
    } else {
        Some(format!("/{}", path))
    }
}

/// 手写 percent-decode（不引入额外依赖；最近打开列表只含 `%3A` 这类盘符转义）。
pub(super) fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(value) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// 待处理冲突文件的定位上下文：冲突落盘后，解析命令要靠这三个字段找回两侧会话。
#[derive(Debug, Clone)]
pub(crate) struct ConflictScanContext {
    pub ide: String,
    pub source_uid: String,
    pub target_uid: String,
}

/// 「两侧都改过」时的处置策略。
///
/// CodeBuddy CN 走 `Resolvable`：记 conflict、落盘待裁决，用户切换完后可逐条
/// 合并 / 覆盖 / 保留。
///
/// WorkBuddy 走 `Suppressed`：它的合并是共享库重映射语义，**不产生可裁决冲突** ——
/// 但共用的 `merge_workspace_history` 仍会按三分支判定。以前这里把冲突记进了
/// 台账（切换回报「有 N 个冲突」、明细里列出来），而 `buddy_list_session_conflicts`
/// 对 WorkBuddy 恒返回空、裁决命令直接报错，于是用户看到的是：
/// **「说有冲突 → 列表里却是 codebuddy 会话 → 点处理说没有冲突」**。
/// 数据行为不变（仍然保留目标版本、不覆盖），只是不再报一个处理不了的冲突。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConflictPolicy {
    /// 记冲突并落盘，等待用户裁决
    Resolvable,
    /// 不产生冲突：两侧都改过时保留目标版本，记 skipped
    Suppressed,
}
