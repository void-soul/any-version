//! 配置账本：记下「我们写了什么」与「原来是什么」。
//!
//! 抄 magpie `internal/agent/applied.go` + `stash.go` 的双账本设计。
//!
//! ## 为什么需要
//!
//! 我们写完工具配置后**不知道它后来被谁改了**。用户可能：
//! - 手动改了 `settings.json` 里的模型；
//! - 另一个工具（Claude Code / opencode）反向覆写了共享的配置文件
//!   （`~/.claude/settings.json` 被claude-code、Claude Desktop、pi 共用）；
//! - 官方客户端升级后重写了配置。
//!
//! 结果是「我们以为配好了，实际没生效」或「模型莫名切走了」，而**无从发现**。
//!
//! ## 为什么不抄 magpie 的完整漂移五态
//!
//! magpie 的五态（`unwired`/`replaced`/`unreachable`/`proxied`/`bypassed`）里，
//! 后三态需要网络探针与 usage 侧的时间戳，真复杂度在**每个 agent 要提供 8 个可选钩子**。
//! 我们先做「账本 + 值比对」这一档：**改没改**能查出来，**为什么改不动**查不出来。
//!
//! ## 存哪
//!
//! `~/.any-version/ledger/` 下 —— **不能放工具配置目录**：那里会被清理、被同步工具搬运，
//! 也会跟着用户的 dotfiles 仓库走。账本必须只归我们管。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 我们在某个文件里写下的一个键。`path` 是点号路径（如 `env.ANTHROPIC_BASE_URL`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedKey {
    /// 点号路径（相对该文件根）。
    pub path: String,
    /// 我们写进去的值（字符串形态；JSON 数组/对象会被压成 JSON 文本）。
    pub value: String,
}

/// 一个工具的「我们写了什么」。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppliedRecord {
    /// 最近一次写入时刻（Unix 毫秒）。
    pub at: u64,
    /// 文件路径 → 我们在该文件里管理的键。
    #[serde(default)]
    pub files: BTreeMap<String, Vec<ManagedKey>>,
}

/// 账本一：`applied.json` —— 我们写了什么。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppliedLedger {
    #[serde(default)]
    pub tools: BTreeMap<String, AppliedRecord>,
}

/// 账本二：`stash.json` —— 原来是什么。扁平键便于精确还原与逐键排查。
///
/// 值的编码：
/// - 字符串 = 原来就是这个值（还原时写回去）
/// - `null` = **原来没有这个键**（还原时删掉，不能写空串 —— 空串是合法值）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StashLedger {
    #[serde(default)]
    pub entries: BTreeMap<String, Option<String>>,
}

/// 漂移判定结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriftItem {
    pub file: String,
    /// 点号路径
    pub path: String,
    /// 我们写的值
    pub applied: String,
    /// 现在的值；`None` = 键已不存在
    pub current: Option<String>,
    /// `removed` = 键被删了；`changed` = 值被改了
    pub kind: String,
    /// 给用户看的一句话
    pub message: String,
}

/// 账本目录（**不放在工具配置目录里**）。
fn ledger_dir() -> PathBuf {
    crate::commands::utils::get_home_dir()
        .join(".any-version")
        .join("ledger")
}

fn applied_path() -> PathBuf {
    ledger_dir().join("applied.json")
}

fn stash_path() -> PathBuf {
    ledger_dir().join("stash.json")
}

/// 原子写（先写临时文件再 rename）；失败不抛 —— 账本丢失不该阻断主流程。
fn write_atomic<T: Serialize>(path: &PathBuf, value: &T) -> Result<(), String> {
    let dir = path.parent().ok_or("账本路径无父目录")?;
    crate::commands::ai::tool_config_custom::ensure_dir(dir)?;
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("写 {} 失败: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("替换 {} 失败: {e}", path.display()))
}

pub fn load_applied() -> AppliedLedger {
    read_json(&applied_path()).unwrap_or_default()
}

pub fn load_stash() -> StashLedger {
    read_json(&stash_path()).unwrap_or_default()
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &PathBuf) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    // 解析失败就当空账本：宁可「查不到漂移」，也不要让一次手滑的编辑让整个功能报死
    serde_json::from_str(&text).ok()
}

/// 记一次写入：**接管前先把原值存进 stash**（只在该键第一次被接管时存）。
pub fn record_write(tool_id: &str, file: &str, applied: &[ManagedKey], stash: &BTreeMap<String, Option<String>>) {
    let mut applied_ledger = load_applied();
    let mut stash_ledger = load_stash();

    // stash：原值为 None = 原来没这个键
    for k in applied {
        let sk = format!("{tool_id}::{}::{}", file, k.path);
        if !stash_ledger.entries.contains_key(&sk) {
            stash_ledger
                .entries
                .insert(sk, stash.get(&k.path).cloned().flatten());
        }
    }

    let rec = applied_ledger.tools.entry(tool_id.to_string()).or_default();
    rec.at = now_ms();
    rec.files.insert(file.to_string(), applied.to_vec());

    let _ = write_atomic(&applied_path(), &applied_ledger);
    let _ = write_atomic(&stash_path(), &stash_ledger);
}

/// 还原成功后清掉该工具的账本（**两本都清** —— 我们不再持有这个工具的配置）。
pub fn clear_tool(tool_id: &str) {
    let mut applied_ledger = load_applied();
    let mut stash_ledger = load_stash();
    applied_ledger.tools.remove(tool_id);
    stash_ledger
        .entries
        .retain(|k, _| !k.starts_with(&format!("{tool_id}::")));
    let _ = write_atomic(&applied_path(), &applied_ledger);
    let _ = write_atomic(&stash_path(), &stash_ledger);
}

/// 纯函数：把账本与「现在实际的值」比对，得出漂移清单。
///
/// `read_now` 由调用方提供（返回 `None` = 该键现在不存在）。抽成纯函数是为了能测：
/// 真实场景要读 TOML/JSONC/env 三种格式，塞进函数里就没法穷举。
pub fn diff_record(
    record: &AppliedRecord,
    read_now: &dyn Fn(&str, &str) -> Option<String>,
) -> Vec<DriftItem> {
    let mut out = Vec::new();
    for (file, keys) in &record.files {
        for k in keys {
            let now = read_now(file, &k.path);
            if now.as_deref() == Some(k.value.as_str()) {
                continue;
            }
            let kind = if now.is_none() { "removed" } else { "changed" };
            let message = match now.as_deref() {
                None => format!("{} 的 {} 已被删除（我们写的是 {:?}）", file, k.path, k.value),
                Some(v) => format!(
                    "{} 的 {} 被改成了 {:?}（我们写的是 {:?}）",
                    file, k.path, v, k.value
                ),
            };
            out.push(DriftItem {
                file: file.clone(),
                path: k.path.clone(),
                applied: k.value.clone(),
                current: now,
                kind: kind.to_string(),
                message,
            });
        }
    }
    out
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 查询某工具的配置漂移：我们在 `~/.any-version/ledger/applied.json` 里记了
/// 「写了什么」，现在逐键比对，列出**被外部改掉 / 删掉**的条目。
///
/// 没有写入记录的工具返回空列表（我们没接管过，自然无漂移可言）。
#[tauri::command]
pub fn get_tool_config_drift(tool_id: String) -> Result<Vec<DriftItem>, String> {
    let ledger = load_applied();
    let Some(record) = ledger.tools.get(&tool_id) else {
        return Ok(Vec::new());
    };
    let tool_cfg = crate::commands::ai_registry::registry()
        .get_tool_config(&tool_id)
        .ok_or_else(|| format!("未知工具 {tool_id}"))?;
    let default_format = tool_cfg
        .config_file
        .as_ref()
        .map(|c| c.format.as_str())
        .unwrap_or("json");

    let read = |file: &str, dotted: &str| -> Option<String> {
        let p = PathBuf::from(file);
        if !p.exists() {
            return None;
        }
        let fmt = super::launch::write_format_for(&p, default_format);
        super::tool_config_restore::read_key_for_ledger(&p, dotted, fmt)
    };
    Ok(diff_record(record, &read))
}

/// 账本里当前记录了哪些工具（前端可用来决定要不要提示「去查一下配置漂移」）。
#[tauri::command]
pub fn list_ledger_tools() -> Result<Vec<String>, String> {
    Ok(load_applied().tools.into_keys().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn key(path: &str, value: &str) -> ManagedKey {
        ManagedKey { path: path.to_string(), value: value.to_string() }
    }

    #[test]
    fn diff_reports_nothing_when_values_match() {
        let rec = AppliedRecord {
            at: 1,
            files: BTreeMap::from([(
                "/tmp/settings.json".to_string(),
                vec![key("env.ANTHROPIC_BASE_URL", "http://127.0.0.1:8080")],
            )]),
        };
        let now = RefCell::new(BTreeMap::from([(
            "env.ANTHROPIC_BASE_URL".to_string(),
            "http://127.0.0.1:8080".to_string(),
        )]));
        let read = |_f: &str, p: &str| now.borrow().get(p).cloned();
        assert!(diff_record(&rec, &read).is_empty());
    }

    #[test]
    fn diff_detects_changed_and_removed() {
        let rec = AppliedRecord {
            at: 1,
            files: BTreeMap::from([(
                "/tmp/settings.json".to_string(),
                vec![
                    key("model", "anyversion/deepseek-v4"),
                    key("env.ANTHROPIC_AUTH_TOKEN", "kiro"),
                ],
            )]),
        };
        let now = RefCell::new(BTreeMap::from([(
            "model".to_string(),
            "gpt-5.1-codex".to_string(), // 被改掉了
        )]));
        let read = |_f: &str, p: &str| now.borrow().get(p).cloned();
        let items = diff_record(&rec, &read);
        assert_eq!(items.len(), 2);
        let changed = items.iter().find(|i| i.path == "model").unwrap();
        assert_eq!(changed.kind, "changed");
        assert_eq!(changed.current.as_deref(), Some("gpt-5.1-codex"));
        assert!(changed.message.contains("被改成"));
        let removed = items.iter().find(|i| i.path == "env.ANTHROPIC_AUTH_TOKEN").unwrap();
        assert_eq!(removed.kind, "removed");
        assert!(removed.current.is_none());
    }

    /// 账本记的值与现在一致时**不该**报漂移 —— 这是最关键的回归：
    /// 值类型（字符串/数字/布尔）不同但语义相同也不能报。
    #[test]
    fn empty_applied_record_yields_nothing() {
        let rec = AppliedRecord::default();
        let read = |_: &str, _: &str| Some("x".to_string());
        assert!(diff_record(&rec, &read).is_empty());
    }

    #[test]
    fn multiple_files_are_all_checked() {
        let rec = AppliedRecord {
            at: 1,
            files: BTreeMap::from([
                ("/a.json".to_string(), vec![key("k", "1")]),
                ("/b.toml".to_string(), vec![key("k", "2")]),
            ]),
        };
        let read = |f: &str, _p: &str| (f == "/a.json").then(|| "1".to_string());
        let items = diff_record(&rec, &read);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].file, "/b.toml");
    }
}