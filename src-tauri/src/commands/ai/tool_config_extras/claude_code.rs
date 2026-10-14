//! Claude Code（`~/.claude/settings.json`）的附加写入：`CLAUDE_CODE_AUTO_MODE_SERVER`。
//!
//! ## 为什么需要这个键
//!
//! Claude Code 有个「auto 模式服务端复核」：把请求交给 Anthropic 的服务端分类器判定。
//! **经我们代理路由时这条路走不通** —— 网关不返回服务端分类判定，于是 Claude Code 会
//! 一直等这个判定，用户看到的就是「auto 模式不工作 / 每次都退化成本地分类」。
//! 官方给的逃生口就是 `CLAUDE_CODE_AUTO_MODE_SERVER=0`（关闭该复核）。
//!
//! EchoBird（`f542afda`）与 magpie（`internal/gateway`，写 `"0"`）**独立地都做了这一条**，
//! 两家交叉验证，不是我们臆想的需要。
//!
//! ## 归属语义（为什么不能无脑写、也不能无脑删）
//!
//! 这个键是**用户也可能自己设**的环境变量。我们必须做到：
//!
//! - 写入：**用户没设过**才补 `"0"`。用户显式设了（`"0"` 或 `"1"`）就一个字都不动 ——
//!   否则「界面上设一次、启动时被我们改回去」，等于摆设。
//! - 还原：**只删「值仍是 `"0"`」的**。我们补进去的值必然是 `"0"`，所以按值判定即可。
//!
//! 取舍说明：EchoBird 靠它自己的 relay 配置里记的 `autoModeServerSettingsPath` 判断
//! 「这份 settings 是我写的」，比按值判定更严。**在单配置文件场景下两者等价** ——
//! Claude Code 只有一份 `settings.json`，不存在「多份文件里哪份是我的」这个问题，
//! 所以这里不引入额外账本（那正是 magpie `applied.json` 要解决的问题，见双账本那条候选）。
//! 边界：用户自己恰好设了 `= "0"` 时，还原后这个键会消失（需用户自行设回）——
//! 比「把用户的 `1` 覆盖成 `0`」或「留着 `0` 不让官方复核恢复」安全得多。

use std::path::Path;

use super::{read_or_empty, write_file, ExtrasCtx};
use crate::commands::ai::tool_config_restore::RestoreOutcome;

/// Claude Code 读取 auto 模式复核端点的环境变量名。
const AUTO_MODE_SERVER: &str = "CLAUDE_CODE_AUTO_MODE_SERVER";

/// 我们补进去的值。只认这一个值 —— 还原时的删除条件也用它。
const OUR_VALUE: &str = "0";

/// 读 `~/.claude/settings.json`；不存在 / 非法 JSONC 时返回一个空文档。
///
/// **解析失败时返回空文档而不是报错**：这个键是纯增量优化，缺失只会让 auto 模式复核
/// 继续尝试（退化成本地分类，仍可用）。若在这里报错，整个「设置模型」流程会失败 ——
/// 为了一个可选键拖垮主流程不划算。对比 `omp.rs` 的 YAML 分支选择报错，是因为那里
/// 整份 `config.yml` 是我们要改的主体，解析失败意味着改动无法落地。
fn load_settings(path: &Path) -> (serde_json::Value, String) {
    let existing = read_or_empty(path);
    let doc = serde_json::from_str::<serde_json::Value>(
        &crate::commands::ai::launch::strip_jsonc(&existing),
    )
    .unwrap_or_else(|_| serde_json::json!({}));
    (doc, existing)
}

/// 取 `env` 对象（不存在就建）。
fn env_mut<'a>(
    doc: &'a mut serde_json::Value,
    path: &Path,
) -> Result<&'a mut serde_json::Map<String, serde_json::Value>, String> {
    let root = doc
        .as_object_mut()
        .ok_or_else(|| format!("{} 的顶层不是对象，已放弃写入", path.display()))?;
    let key = "env".to_string();
    let entry = root.entry(key).or_insert_with(|| serde_json::json!({}));
    entry.as_object_mut().ok_or_else(|| {
        format!("{} 的 env 不是对象，已放弃写入该键", path.display())
    })
}

pub(super) fn apply(ctx: &ExtrasCtx<'_>) -> Result<Vec<String>, String> {
    let path = ctx.main_path;
    let (mut doc, existing) = load_settings(path);

    // 用户已设过这个键 → 绝不覆盖（含用户自己设成 "0" 的情况：那是他的选择，不是我们的）
    let already_set = doc
        .get("env")
        .and_then(|e| e.as_object())
        .map(|m| m.contains_key(AUTO_MODE_SERVER))
        .unwrap_or(false);
    if already_set {
        return Ok(Vec::new());
    }

    let env = env_mut(&mut doc, path)?;
    env.insert(
        AUTO_MODE_SERVER.to_string(),
        serde_json::Value::String(OUR_VALUE.to_string()),
    );

    let updated = serde_json::to_string_pretty(&doc)
        .map_err(|e| format!("序列化 {} 失败: {e}", path.display()))?;
    if updated == existing {
        return Ok(Vec::new());
    }
    eprintln!(
        "[extras] claude-code: 已补 {AUTO_MODE_SERVER}={OUR_VALUE}（网关不返回服务端复核判定，\
         关掉它才能让 auto 模式退回本地分类）"
    );
    Ok(vec![write_file(path, &updated)?])
}

/// 还原：只删「值仍是我们补的那个 `0`」的键。用户设成 `"1"` 的一个都不碰。
pub(super) fn restore(main_path: &Path) -> Result<RestoreOutcome, String> {
    let mut outcome = RestoreOutcome::default();
    if !main_path.exists() {
        return Ok(outcome);
    }
    let (mut doc, _existing) = load_settings(main_path);
    let Some(env) = doc.get_mut("env").and_then(|e| e.as_object_mut()) else {
        return Ok(outcome);
    };
    let ours = env
        .get(AUTO_MODE_SERVER)
        .and_then(|v| v.as_str())
        .map(|v| v == OUR_VALUE)
        .unwrap_or(false);
    if !ours {
        // 用户显式设了 "1"（或不认识的值）→ 属于他的配置，留着
        return Ok(outcome);
    }
    env.remove(AUTO_MODE_SERVER);

    let updated = serde_json::to_string_pretty(&doc)
        .map_err(|e| format!("序列化 {} 失败: {e}", main_path.display()))?;
    write_file(main_path, &updated)?;
    eprintln!("[extras] claude-code: 已删掉 {AUTO_MODE_SERVER}（还原为官方默认）");
    outcome.files.push(main_path.display().to_string());
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("anyver-cc-automode-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ctx<'a>(main: &'a Path, model: &'a str) -> ExtrasCtx<'a> {
        static EMPTY: std::sync::OnceLock<std::collections::HashSet<String>> =
            std::sync::OnceLock::new();
        ExtrasCtx {
            tool_id: "claude-code",
            main_path: main,
            base_url: "http://127.0.0.1:3456",
            upstream_url: "https://api.deepseek.com",
            api_key: "sk-test",
            model,
            model_name: model,
            real_model_name: model,
            provider: "anyversion",
            chosen_protocol: "anthropic",
            web_search: false,
            user_configured_paths: EMPTY.get_or_init(std::collections::HashSet::new),
        }
    }

    fn read_env(path: &Path) -> serde_json::Value {
        let text = std::fs::read_to_string(path).unwrap();
        serde_json::from_str::<serde_json::Value>(&crate::commands::ai::launch::strip_jsonc(&text)).unwrap()
    }

    /// 首次使用：补上 `= "0"`
    #[test]
    fn inserts_default_when_absent() {
        let dir = temp_dir("insert");
        let main = dir.join("settings.json");
        std::fs::write(&main, "{\"env\":{\"KEEP\":\"yes\"}}\n").unwrap();

        apply(&ctx(&main, "deepseek-v4-pro")).unwrap();

        let doc = read_env(&main);
        assert_eq!(doc["env"][AUTO_MODE_SERVER], "0");
        assert_eq!(doc["env"]["KEEP"], "yes", "用户的键必须留着");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// settings.json 还不存在 → 也要建（否则「设置成功但没关掉复核」）
    #[test]
    fn creates_settings_when_missing() {
        let dir = temp_dir("missing");
        let main = dir.join("settings.json");

        apply(&ctx(&main, "deepseek-v4-pro")).unwrap();

        assert_eq!(read_env(&main)["env"][AUTO_MODE_SERVER], "0");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 用户自己设过 → 一个字都不动（无论 "0" 还是 "1"）
    #[test]
    fn preserves_user_value() {
        for user_value in ["0", "1"] {
            let dir = temp_dir(&format!("user{user_value}"));
            let main = dir.join("settings.json");
            let original = format!("{{\"env\":{{\"{AUTO_MODE_SERVER}\":\"{user_value}\"}}}}");
            std::fs::write(&main, &original).unwrap();

            let changed = apply(&ctx(&main, "m")).unwrap();
            assert!(changed.is_empty(), "不应写文件：{changed:?}");
            assert_eq!(std::fs::read_to_string(&main).unwrap(), original);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// 反复切换模型：值应稳定留在 "0"，不被清掉也不被改动
    #[test]
    fn survives_repeated_model_switches() {
        let dir = temp_dir("repeat");
        let main = dir.join("settings.json");
        std::fs::write(&main, "{\"env\":{\"KEEP\":\"yes\"}}\n").unwrap();

        for m in ["a", "b", "a", "c"] {
            apply(&ctx(&main, m)).unwrap();
            assert_eq!(read_env(&main)["env"][AUTO_MODE_SERVER], "0", "切到 {m} 后丢了默认值");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 还原：删掉我们补的 "0"，其余键不动
    #[test]
    fn restore_removes_our_default() {
        let dir = temp_dir("restore");
        let main = dir.join("settings.json");
        std::fs::write(&main, "{\"env\":{\"KEEP\":\"yes\"}}\n").unwrap();
        apply(&ctx(&main, "m")).unwrap();

        let outcome = restore(&main).unwrap();
        assert_eq!(outcome.files.len(), 1);

        let doc = read_env(&main);
        assert!(
            doc["env"].get(AUTO_MODE_SERVER).is_none(),
            "我们补的默认值应被删掉"
        );
        assert_eq!(doc["env"]["KEEP"], "yes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 还原：用户手改成 "1" 之后**保留**（不能把用户的设置抹掉）
    #[test]
    fn restore_keeps_user_edited_one() {
        let dir = temp_dir("restore1");
        let main = dir.join("settings.json");
        std::fs::write(&main, "{\"env\":{\"KEEP\":\"yes\"}}\n").unwrap();
        apply(&ctx(&main, "m")).unwrap();
        // 模拟用户手动改回官方默认行为
        std::fs::write(
            &main,
            "{\"env\":{\"KEEP\":\"yes\",\"CLAUDE_CODE_AUTO_MODE_SERVER\":\"1\"}}\n",
        )
        .unwrap();

        let outcome = restore(&main).unwrap();
        assert!(outcome.files.is_empty(), "不该写文件：{:?}", outcome.files);
        assert_eq!(read_env(&main)["env"][AUTO_MODE_SERVER], "1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 非法 JSONC：不能因此让「设置模型」整条流程失败，退化成不写这个键
    #[test]
    fn malformed_json_does_not_fail_the_whole_write() {
        let dir = temp_dir("badjson");
        let main = dir.join("settings.json");
        let broken = "{ this is not json";
        std::fs::write(&main, broken).unwrap();

        // 不 panic、不报错即可；退化行为按「当空文档」处理
        let _ = apply(&ctx(&main, "m"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 无关的顶层键（permissions 等）必须原样保留
    #[test]
    fn preserves_unrelated_top_level_keys() {
        let dir = temp_dir("keep");
        let main = dir.join("settings.json");
        std::fs::write(
            &main,
            "{\"permissions\":{\"allow\":[\"Read\"]},\"env\":{}}",
        )
        .unwrap();

        apply(&ctx(&main, "m")).unwrap();

        let doc = read_env(&main);
        assert_eq!(doc["permissions"]["allow"][0], "Read");
        assert_eq!(doc["env"][AUTO_MODE_SERVER], "0");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// env 不是对象（用户写坏了）→ 不 panic，且不破坏原文件
    #[test]
    fn env_not_object_is_rejected_safely() {
        let dir = temp_dir("badenv");
        let main = dir.join("settings.json");
        let original = "{\"env\":\"not-an-object\"}";
        std::fs::write(&main, original).unwrap();

        assert!(apply(&ctx(&main, "m")).is_err());
        assert_eq!(std::fs::read_to_string(&main).unwrap(), original);
        let _ = std::fs::remove_dir_all(&dir);
    }
}