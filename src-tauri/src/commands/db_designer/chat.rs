//! 数据库设计器 · AI 会话持久化。
//!
//! 落盘布局（数据目录，项目自建的 `get_data_dir()`，**不是** Tauri 的 `app_data_dir`）：
//!
//! ```text
//! <data>/db_designer/sessions.json      ← 会话索引（id ↔ 文件名 ↔ 最新路径 ↔ 更新时间）
//! <data>/db_designer/chats/<id>.json    ← 每个会话一份对话正文
//! ```
//!
//! 索引与正文分成两个文件：索引小、弹框要一次拿全；正文大、按需读。
//! 打开会话管理弹框不必把所有对话都读进内存。
//!
//! **为什么对话不进 `.dbdesign.json`**：见
//! `docs/plans/2026-10-15-db-design-agent-memory-design.md` —— 分享设计文件时
//! 把两个 AI 的来回对话也发给别人没意义。所以它只落数据目录，
//! 而 `doc.decisions`（设计决策）继续留在设计文件里，两类记忆分开装。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 索引是 read-modify-write：两个窗口各改各的再整份写回，会互相抹掉条目
/// （丢更新）。与 `config.rs::CONFIG_SAVE_LOCK` 同一套做法。
/// 只锁「读-改-写」这一段，纯读不进来 —— 索引很小，没必要把读也串行化。
static INDEX_LOCK: Mutex<()> = Mutex::new(());

/// 一个会话的对话正文。
///
/// `chat_msgs` / `ai_history` 用 `Vec<serde_json::Value>` 而**不镜像前端的 `ChatMsg`**：
/// 前端那个类型有 14 个可选字段，`role` 是字面量联合、`patch`/`clarify`/`repair`
/// 是内联对象（`repair` 还套着 `types.ts` 的 `RepairReport`）。在 Rust 侧抄一份必然
/// 漂移——前端加字段时 Rust 侧不会报错，字段却被静默丢弃。用 `Value` 让磁盘格式
/// 等于前端格式，`default` + 忽略未知字段天然「多退少补」，与 `models.rs`
/// 那套宽容反序列化是同一个思路。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionChat {
    /// 气泡列表（前端 `ChatMsg[]`，上限 40 条由前端保证）
    pub chat_msgs: Vec<serde_json::Value>,
    /// 送给模型的滚动会话历史（最近 3 轮，前端 `ChatTurn[]`）
    pub ai_history: Vec<serde_json::Value>,
    /// 已追问轮数（上限 3 轮，见前序设计）
    pub ai_clarify_round: u32,
}

impl Default for SessionChat {
    fn default() -> Self {
        Self { chat_msgs: Vec::new(), ai_history: Vec::new(), ai_clarify_round: 0 }
    }
}

/// 索引里的一条：一个 id 绑着哪份设计、用什么名字认它、最近在哪。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionEntry {
    pub id: String,
    /// 文件名（去扩展名），只为让人在列表里认得出
    pub title: String,
    /// 最近一次打开/保存的位置 —— 文件被移动后靠它认回原会话
    pub path: String,
    /// 毫秒时间戳。与 `versions.rs` 的同类字段一致用 `u64`（不是 u128：
    /// 那是本项目的孤例，而 u128 多一个「万一传进来负数直接反序列化失败」的边界）
    pub updated_at: u64,
}

impl Default for SessionEntry {
    fn default() -> Self {
        Self { id: String::new(), title: String::new(), path: String::new(), updated_at: 0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionIndex {
    pub sessions: Vec<SessionEntry>,
}

fn dbd_dir() -> PathBuf {
    crate::commands::config::get_data_dir().join("db_designer")
}

fn chats_dir() -> PathBuf {
    dbd_dir().join("chats")
}

fn index_path() -> PathBuf {
    dbd_dir().join("sessions.json")
}

/// Windows 保留设备名：`NUL`、`CON`、`COM1`… 当文件名会被系统当成设备，
/// 写进去的字节全部丢弃 —— 于是**整轮对话静默丢失且可能返回成功**。
/// （不是目录穿越，写不出 `chats/`，但失败同样没有提示。）
const WIN_RESERVED: [&str; 8] = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "LPT1", "LPT2"];

/// 会话 id 是否可用作文件名。**纯函数，不碰数据目录** —— 校验逻辑单独可测。
///
/// 白名单制（而非黑名单）：只放行 ASCII 字母数字 `_` `-`。`..` 因含 `.` 被自然拒绝，
/// Windows 的尾点/尾空格截断也一并关掉。我们实际生成的两种形态
/// （`dbd_<毫秒>_<序号>`、`s_<毫秒>_<随机>`）全在这个集合里，
/// 放宽字符集换不来任何便利，只会多一个能逃出目录的字符。
fn is_safe_session_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 128 {
        return false;
    }
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return false;
    }
    // 设备名是**精确匹配整个基名**（第一个点之前完全相等），不是前缀匹配：
    // `NUL` 是设备，`NUL-session` 只是普通文件名。大小写不敏感
    //（Windows 文件系统本身也不敏感）。
    let upper = id.to_ascii_uppercase();
    !WIN_RESERVED.iter().any(|r| upper == *r)
}

/// 会话 id → 正文文件路径。校验不过就报错，**绝不落到 `chats/` 之外**。
fn chat_path(id: &str) -> Result<PathBuf, String> {
    if !is_safe_session_id(id) {
        return Err(format!("会话 id 非法：{:?}", id));
    }
    Ok(chats_dir().join(format!("{}.json", id)))
}

/// 解析一份 JSON 文本；**坏 JSON / 缺字段 / 类型不对一律回退默认值**。
///
/// 独立成纯函数才能被单测真正覆盖（写在 `read_json` 里的话，测试只能把
/// `serde_json` 的行为抄一遍 —— 改坏 `read_json` 它照样绿，等于没测）。
///
/// 刻意剥 UTF-8 BOM（`file.rs:23` 有同样的处理，理由也在那儿）：
/// Windows 上的编辑器 / Agent 脚本写出的文件常带 BOM，而 serde_json 不认。
/// **不剥的后果特别恶劣**：解析失败 → 回退空会话 → 前端把 40 条气泡全清空 →
/// 用户接着聊两轮 → 下一轮原子覆盖把原对话**永久销毁，全程零提示**。
fn parse_json<T: serde::de::DeserializeOwned + Default>(raw: &str) -> T {
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    serde_json::from_str(raw).unwrap_or_default()
}

/// 读一份 JSON。**「不存在」与「读失败」必须区分开**：
/// 前者返回默认值，后者报错 —— 否则 read-modify-write 的调用方会把一个
/// 「读不到」的索引当成空索引写回去，静默清空所有会话条目。
fn read_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> Result<T, String> {
    match std::fs::read_to_string(path) {
        Ok(raw) => Ok(parse_json(&raw)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(format!("读取失败 {}: {}", path.display(), e)),
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let data = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    crate::commands::config::atomic_write_file(path, data.as_bytes())
}

/// 纯函数：登记 / 更新一个会话条目。**按 id 去重**（而不是让前端整份覆盖索引）——
/// 前端只传它刚动过的那一条，免得它手里那份可能已经过期的索引把别人的条目抹掉。
///
/// 抽成纯函数是为了能单测（见 `index_insert_and_remove_round_trip`）。
fn upsert_entry(index: &mut SessionIndex, id: &str, title: &str, path: &str, now: u64) {
    match index.sessions.iter_mut().find(|s| s.id == id) {
        Some(e) => {
            e.title = title.to_string();
            e.path = path.to_string();
            e.updated_at = now;
        }
        None => index.sessions.push(SessionEntry {
            id: id.to_string(),
            title: title.to_string(),
            path: path.to_string(),
            updated_at: now,
        }),
    }
}

/// 纯函数：摘掉某 id。返回是否真的摘掉了（删不存在的条目不算成功）。
fn remove_entry(index: &mut SessionIndex, id: &str) -> bool {
    let before = index.sessions.len();
    index.sessions.retain(|s| s.id != id);
    index.sessions.len() != before
}

fn load_chat(id: &str) -> SessionChat {
    match chat_path(id).and_then(|p| read_json(&p)) {
        Ok(c) => c,
        // 非法 id / 读不到 / 读坏了：一律当「没有这个会话」，不打断界面
        Err(_) => SessionChat::default(),
    }
}

fn store_chat(id: &str, chat: &SessionChat) -> Result<(), String> {
    write_json(&chat_path(id)?, chat)
}

fn load_index() -> Result<SessionIndex, String> {
    read_json(&index_path())
}

fn store_index(index: &SessionIndex) -> Result<(), String> {
    write_json(&index_path(), index)
}

/// 读一个会话的对话。**不存在 / 坏了 / id 非法一律返回空会话，不报错。**
#[tauri::command]
pub fn dbd_get_chat(id: String) -> SessionChat {
    load_chat(&id)
}

/// 写一个会话的对话。返回落盘后的真值（同 `dbd_save_settings`）。
#[tauri::command]
pub fn dbd_save_chat(id: String, chat: SessionChat) -> Result<SessionChat, String> {
    store_chat(&id, &chat)?;
    Ok(load_chat(&id))
}

/// 删一个会话的对话。**文件不存在也算成功** —— 幂等，重复点删除不报错。
#[tauri::command]
pub fn dbd_delete_chat(id: String) -> Result<(), String> {
    let p = chat_path(&id)?;
    match std::fs::remove_file(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// 读会话索引。坏了就当空索引（对话文件仍在，不会丢数据）。
#[tauri::command]
pub fn dbd_get_session_index() -> SessionIndex {
    load_index().unwrap_or_default()
}

/// 登记 / 更新一个会话条目。**upsert 语义**，不是整份覆盖。
///
/// 锁在「读-改-写」这一段上：两个窗口同时登记会互相抹掉条目。
#[tauri::command]
pub fn dbd_upsert_session_entry(
    id: String,
    title: String,
    path: String,
) -> Result<SessionIndex, String> {
    if !is_safe_session_id(&id) {
        return Err(format!("会话 id 非法：{:?}", id));
    }
    let _guard = INDEX_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut index = load_index()?;
    upsert_entry(&mut index, &id, &title, &path, chrono::Utc::now().timestamp_millis() as u64);
    store_index(&index)?;
    Ok(load_index().unwrap_or_default())
}

/// 删一个会话：正文文件 + 索引条目一起没。
///
/// **顺序：先正文后索引** —— 反过来失败会留下永远看不见、也不会被回收的
/// 孤儿文件；现在这样最坏是「文件没了索引还在」，可见、可自愈（再删一次即可，
/// `dbd_delete_chat` 幂等）。不做事务回滚：这是本机缓存，不值当。
///
/// **删的是当前会话时不报错、不补救**：文档的 `doc.id` 还在，下次
/// `resolveSessionId` 会重新登记它、读出空对话 —— 对话自然变空，文档不脏。
#[tauri::command]
pub fn dbd_delete_session(id: String) -> Result<(), String> {
    dbd_delete_chat(id.clone())?;
    let _guard = INDEX_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut index = load_index()?;
    remove_entry(&mut index, &id);
    store_index(&index)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **id 必须挡住目录穿越**：它会直接被当文件名用，而它来自用户可编辑的
    /// 设计文件。`../` 能写到数据目录之外去。
    #[test]
    fn session_id_is_rejected_when_it_can_escape_the_chats_dir() {
        for bad in ["../evil", "..\\evil", "a/b", "a\\b", "", "  ", ".", ".."] {
            assert!(!is_safe_session_id(bad), "id {:?} 必须被拒绝：它能把文件写到 chats/ 外面", bad);
        }
    }

    /// 放行的必须覆盖我们**实际生成**的两种形态，否则是自断手脚。
    #[test]
    fn session_id_allows_the_shapes_we_actually_generate() {
        for ok in ["dbd_1763234567890_3", "s_1763234567890_a7f2", "abc123"] {
            assert!(is_safe_session_id(ok), "id {:?} 该被接受", ok);
        }
    }

    /// 超长 id 直接拒绝而不是截断 —— 截断会让两个长 id 撞车，
    /// 而撞车意味着两份设计共用一份对话。
    #[test]
    fn overly_long_session_id_is_rejected_instead_of_truncated() {
        assert!(is_safe_session_id(&"a".repeat(128)));
        assert!(!is_safe_session_id(&"a".repeat(129)));
    }

    /// Windows 设备名当文件名会被系统吞掉字节 —— 那一轮对话就静默没了。
    ///
    /// 匹配规则是**精确匹配整个基名**（第一个点之前的部分完全等于设备名），
    /// 不是前缀匹配 —— 所以 `NUL` 是设备，而 `NUL-session` 是合法普通文件名，
    /// 不该被误伤。
    #[test]
    fn windows_reserved_device_names_are_rejected() {
        for bad in ["NUL", "nul", "CON", "con", "PRN", "AUX", "COM1", "LPT1"] {
            assert!(!is_safe_session_id(bad), "设备名 {:?} 不能当会话文件名", bad);
        }
        // 只多带后缀就不是设备名了，不该连坐
        assert!(is_safe_session_id("NUL-session"));
        assert!(is_safe_session_id("nullable_session"));
        assert!(is_safe_session_id("CONSULT"));
    }

    /// **白名单通过了 ≠ 路径真的落在 `chats/` 里** —— 这才是安全属性本身。
    /// 把 `chat_path` 改成不 join 目录的话，这条会立刻抓住。
    #[test]
    fn accepted_id_lands_inside_the_chats_dir() {
        let p = chat_path("abc123").unwrap();
        assert!(p.starts_with(chats_dir()), "必须落在 chats/ 内: {}", p.display());
        assert_eq!(p.file_name().unwrap(), "abc123.json");
    }

    #[test]
    fn illegal_id_cannot_produce_a_path() {
        assert!(chat_path("../evil").is_err());
    }

    /// 旧会话文件缺字段必须能读，不能因新增字段作废。
    #[test]
    fn legacy_chat_json_without_new_fields_is_accepted() {
        let c: SessionChat = parse_json(r#"{"chatMsgs":[]}"#);
        assert!(c.chat_msgs.is_empty());
        assert!(c.ai_history.is_empty());
        assert_eq!(c.ai_clarify_round, 0);
    }

    /// **坏 JSON / 缺字段 / 类型不对要回退默认值，不能抛**（设计文档 §五）。
    /// 测的是 `parse_json` 本身 —— 只测 `serde_json` 的行为的话，
    /// 把 `read_json` 改成 panic 这条照样绿，等于没测。
    #[test]
    fn broken_or_partial_json_degrades_to_default() {
        let idx: SessionIndex = parse_json("{ 这不是 json");
        assert!(idx.sessions.is_empty());
        let idx: SessionIndex = parse_json("{}");
        assert!(idx.sessions.is_empty());
        let chat: SessionChat = parse_json(r#"{"chatMsgs":"不是数组"}"#);
        assert!(chat.chat_msgs.is_empty());
    }

    /// **带 BOM 的文件必须能读**（Windows 编辑器 / Agent 脚本的常态）。
    ///
    /// 不剥 BOM 的后果链：解析失败 → 回退空会话 → 前端清空 40 条气泡 →
    /// 用户接着聊两轮 → 下一轮原子覆盖 → **原对话永久销毁且零提示**。
    /// 所以这条不是可有可无的健壮性，是防数据丢失。
    #[test]
    fn utf8_bom_prefixed_json_is_still_readable() {
        let raw = format!("\u{feff}{}", r#"{"aiClarifyRound":7,"chatMsgs":[{"id":"m1"}]}"#);
        let chat: SessionChat = parse_json(&raw);
        assert_eq!(chat.ai_clarify_round, 7, "BOM 不该让内容读不出来");
        assert_eq!(chat.chat_msgs.len(), 1);
    }

    #[test]
    fn round_trip_preserves_chat_fields() {
        let c = SessionChat {
            chat_msgs: vec![serde_json::json!({"id":"m1","role":"user","text":"加个备注字段"})],
            ai_history: vec![serde_json::json!({"role":"user","text":"加备注"})],
            ai_clarify_round: 2,
        };
        let back: SessionChat = parse_json(&serde_json::to_string(&c).unwrap());
        assert_eq!(back.ai_clarify_round, 2);
        assert_eq!(back.chat_msgs[0]["text"], "加个备注字段");
        assert_eq!(back.ai_history[0]["role"], "user");
    }

    /// 气泡里那些可选字段（`repair` / `clarify` / `patch` …）必须原样往返，
    /// 丢掉任何一项都会让恢复出来的对话缺一块。
    /// `repair` 的字段照抄 `types.ts:118-126` 的 `RepairReport`（四个字段）。
    #[test]
    fn optional_bubble_fields_survive_the_round_trip() {
        let rich = serde_json::json!({
            "id": "m2",
            "role": "assistant",
            "status": "done",
            "thinking": "先想…",
            "chars": 128,
            "summary": "已生成 4 个表",
            "decisions": ["status 用枚举"],
            "patch": {"added": 4, "updated": 0, "removed": 0},
            "repair": {"rounds": 1, "ok": true, "remainingErrors": [], "rolledBack": false},
            "clarify": {"question": "允许哪些取值？", "options": ["A", "B"]},
            "truncated": false
        });
        let c = SessionChat {
            chat_msgs: vec![rich.clone()],
            ai_history: Vec::new(),
            ai_clarify_round: 0,
        };
        let back: SessionChat = parse_json(&serde_json::to_string(&c).unwrap());
        assert_eq!(back.chat_msgs[0], rich);
    }

    /// **磁盘格式必须是 camelCase**：前端 invoke 发的是 `{ aiClarifyRound }`。
    /// 哪天有人删掉 `rename_all`，serde 会因为 `default` 把 camelCase 当 unknown
    /// 字段**静默忽略** —— 于是每次保存都把整份对话清空，且没有任何报错。
    /// （`mod.rs` 的 `export_options_defaults_to_including_foreign_keys`
    /// 已经为同一类坑钉过一条。）
    #[test]
    fn wire_format_is_camel_case() {
        let s = serde_json::to_string(&SessionChat {
            chat_msgs: vec![],
            ai_history: vec![],
            ai_clarify_round: 3,
        })
        .unwrap();
        assert!(s.contains("\"aiClarifyRound\""), "前端发的是 aiClarifyRound: {}", s);
        assert!(!s.contains("ai_clarify_round"), "snake_case 不该出现: {}", s);
        // 反向：camelCase 输入必须被认
        assert_eq!(parse_json::<SessionChat>(r#"{"aiClarifyRound":3}"#).ai_clarify_round, 3);

        let i = serde_json::to_string(&SessionIndex {
            sessions: vec![SessionEntry {
                id: "a".into(),
                title: "t".into(),
                path: "p".into(),
                updated_at: 1,
            }],
        })
        .unwrap();
        assert!(i.contains("\"updatedAt\""), "索引条目的时间字段: {}", i);
        assert!(!i.contains("updated_at"), "{}", i);
        assert_eq!(
            parse_json::<SessionIndex>(r#"{"sessions":[{"updatedAt":9}]}"#).sessions[0].updated_at,
            9
        );
    }

    #[test]
    fn index_round_trip_keeps_id_title_path_and_time() {
        let idx = SessionIndex {
            sessions: vec![SessionEntry {
                id: "dbd_1_1".into(),
                title: "订单库".into(),
                path: "C:\\设计\\订单库.dbdesign.json".into(),
                updated_at: 1763234567890,
            }],
        };
        let back: SessionIndex = parse_json(&serde_json::to_string(&idx).unwrap());
        assert_eq!(back.sessions[0].id, "dbd_1_1");
        assert_eq!(back.sessions[0].title, "订单库");
        assert_eq!(back.sessions[0].path, "C:\\设计\\订单库.dbdesign.json");
        assert_eq!(back.sessions[0].updated_at, 1763234567890);
    }

    /// 索引的增删改（设计文档 §六 明确要求的一条）。
    /// upsert 必须**按 id 去重**并刷新 title/path/时间，而不是追加第二条 ——
    /// 追加的话列表会出现同一个设计好几行。
    #[test]
    fn index_insert_and_remove_round_trip() {
        let mut idx = SessionIndex::default();

        upsert_entry(&mut idx, "dbd_1_0", "订单库", "C:\\a.json", 1);
        assert_eq!(idx.sessions.len(), 1);

        // 同一 id 再登记（文件被移走 / 改了名）→ 更新而不是新增
        upsert_entry(&mut idx, "dbd_1_0", "订单库（改名）", "C:\\b.json", 2);
        assert_eq!(idx.sessions.len(), 1, "同一个 id 不能出现两次");
        assert_eq!(idx.sessions[0].path, "C:\\b.json");
        assert_eq!(idx.sessions[0].title, "订单库（改名）");
        assert_eq!(idx.sessions[0].updated_at, 2);

        // 另一个会话互不影响
        upsert_entry(&mut idx, "dbd_2_0", "用户中心", "C:\\c.json", 3);
        assert_eq!(idx.sessions.len(), 2);

        assert!(remove_entry(&mut idx, "dbd_1_0"), "删存在的条目算成功");
        assert_eq!(idx.sessions.len(), 1);
        assert_eq!(idx.sessions[0].id, "dbd_2_0");
        assert!(!remove_entry(&mut idx, "dbd_1_0"), "删不存在的条目不算成功");
        assert_eq!(idx.sessions.len(), 1);
    }

    /// 非法 id 的读写都要安静：读回空、写入报错（而不是写到别处去）。
    #[test]
    fn illegal_id_reads_empty_and_write_refuses() {
        assert_eq!(load_chat("../evil").chat_msgs.len(), 0);
        assert!(store_chat("../evil", &SessionChat::default()).is_err());
    }
}