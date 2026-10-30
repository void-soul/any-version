// 数据库设计器的 AI 助手：给一段需求，产出一份 `.dbdesign.json`。
//
// 设计要点：
//   · **只返回文档，不落盘**。应用拿到后走正常的「打开 → 校验 → 用户决定是否保存」链路，
//     避免 AI 直接覆盖用户正在画的设计（应用里替换当前内容是要红色确认的）。
//   · 复用 AI 模块已有的供应商 / 模型解析与流式通道（`ai::channel::stream_chat_with_resume`），
//     不自建请求逻辑 —— 超时、重试、断点续写、宽容 JSON 解析都在那一层。
//   · **流式构图**：流式过程中每写完一张表就推一份部分文档（`dbd-ai-partial`），
//     前端边收边画，与思维导图观感一致。
//   · 不强依赖 tool-call：只有部分网关支持，通道内部会在不支持时降级为纯文本协议。
//   · 产出后**过一遍规范化**（`store::normalize_naming`）：命名改合规 + 外键补索引。
//     这两类都是 error 级（阻断保存 / 导出），而提示词只是概率性约束 —— 只靠提示词，
//     用户会拿到「满屏报错且导不出来」的设计。规范化与 `store::validate` 共用同一套规则，
//     不会两套口径漂移；它只改明显的规范问题，业务语义仍由 AI 决定。

use super::file::new_document;
use super::models::{DbDesignDocument, DbDesignNode, DbDesignRelation, DbField};
use crate::commands::ai;

/// 前端传进来的生成请求
///
/// **必须**带 `rename_all = "camelCase"`：前端 invoke 传的是 `providerId` /
/// `modelId` / `runId` / `currentDoc`，漏了这个属性 serde 会静默全部变 None
/// —— 后果是模型回落全局默认（用户选了 A 模型实际跑了 B 模型）、「停止」失效。
/// 思维导图模块的入参结构体全都有（`mindmap/models.rs`），这里漏了。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiDesignInput {
    /// 需求描述（用户输入的原始文本）
    pub text: String,
    /// 设计名（可空，AI 自己起）
    pub name: Option<String>,
    pub dialect: Option<String>,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    /// 画布上的当前设计（前端原样传入）。非空时提示词带它作基线：
    /// AI 基于它修改并输出完整更新后的设计；是「新建」还是「修改」由模型
    /// 看需求自己判断（系统提示里写明了两种情况的处理方式）。
    #[serde(default)]
    pub current_doc: Option<DbDesignDocument>,
    /// 本次运行的 id（前端生成）。用于「停止」，不填则不能中途停。
    #[serde(default)]
    pub run_id: Option<String>,
    /// 最近的对话轮次（滚动窗口，前端只留最近几轮）。
    ///
    /// **只存文本，不存模型吐过的整份 JSON**：下一轮模型看到的是更新后的当前设计，
    /// 旧 JSON 里九成是重复的表定义，存起来只烧 token。
    #[serde(default)]
    pub history: Vec<ChatTurn>,
    /// 已经追问了几轮。写进提示词是为了让模型遵守「3 轮后必须出结果」——
    /// 光在提示词里写死上限而每次都发同样的话，模型数不清自己问过几轮。
    #[serde(default)]
    pub clarify_round: u32,
}

/// 一轮对话（用户或助手）。
#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ChatTurn {
    /// `"user"` 或 `"assistant"`
    pub role: String,
    pub text: String,
}

/// 模型这一轮的产出：**要么给设计，要么反过来问你**。
///
/// 用 `tag = "kind"` 让前端能直接 `switch (outcome.kind)`：
/// `{"kind":"design","document":{…},"decisions":[…]}` /
/// `{"kind":"clarify","question":"…","options":[…]}`。
///
/// `decisions` 与 `document` **分开放**，而不是让模型把 decisions 写进文档里再回吐：
/// 它每轮都要重写整份文档，很可能把你之前写的决策丢了或改写。真正的所有权在前端 ——
/// 保留原文档的决策，只追加本轮新增的。
#[derive(serde::Serialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AiOutcome {
    /// 产出了一份设计（可能带本轮的决策摘要）
    Design {
        document: DbDesignDocument,
        /// 本轮「为什么这么改」。模型没给就是空数组，**不算错误**。
        decisions: Vec<String>,
        /// 自校验结果：设计落图前回验一遍，不合规就把错误清单回喂模型重修。
        repair: RepairReport,
        /// **产出被截断**（模型中途写崩了）：这里的表是已完成的那部分，但关联与决策记录可能不完整。
        truncated: bool,
    },
    /// 增量修改：模型只说哪几张表改了，没提到的原样保留。
    /// 经 `apply_patch` 合并到基线文档后，对外仍是一份完整文档。
    Patch {
        document: DbDesignDocument,
        decisions: Vec<String>,
        /// 这次实际改了什么（给前端展示）
        applied: crate::commands::db_designer::patch::PatchApplied,
        repair: RepairReport,
    },
    /// 业务语义上有歧义，停下来问用户（此时不落图，文档一个字都不改）
    Clarify {
        question: String,
        /// 候选项，供用户点一下就答；模型没给就是空数组（自由输入）。
        options: Vec<String>,
    },
}

/// 已请求停止的 run_id。用 HashSet 而不是每任务一个 AtomicBool：命令是异步的、
/// 「停止」走的是另一条命令，只能按 run_id 在全局里对上号（同思维导图 mm_ai_cancel 的思路）。
fn cancelled_runs() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    use std::sync::{Mutex, OnceLock};
    static SET: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    SET.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

/// 停止一次生成。重复调用无害；run_id 不存在也不会报错。
#[tauri::command]
pub fn dbd_ai_cancel(run_id: String) -> Result<(), String> {
    cancelled_runs()
        .lock()
        .map_err(|_| "取消状态被占用".to_string())?
        .insert(run_id);
    Ok(())
}

fn is_cancelled(run_id: &str) -> bool {
    cancelled_runs()
        .lock()
        .map(|s| s.contains(run_id))
        .unwrap_or(false)
}

/// 把进度推给前端（`dbd-ai-progress`）。事件是「即发即弃」的，
/// 前端用模块级缓冲订阅（切面板/keep-alive 也不会丢）。
fn emit_progress(app: &Option<tauri::AppHandle>, step: &str, extra: serde_json::Value) {
    use tauri::Emitter as _; // Tauri 2：emit 挂在 Emitter trait 上，不在 AppHandle 的固有方法里
    if let Some(handle) = app {
        let _ = handle.emit(
            "dbd-ai-progress",
            serde_json::json!({ "step": step, "data": extra }),
        );
    }
}

/// 生成过程的钩子：转发流式进度 + 响应「停止」。
struct DbdHooks {
    app: Option<tauri::AppHandle>,
    run_id: String,
}

impl ai::channel::ChannelHooks for DbdHooks {
    fn on_progress(&self, step: &str, extra: serde_json::Value) {
        emit_progress(&self.app, step, extra);
    }
    fn check_cancel(&self) -> Result<(), String> {
        if is_cancelled(&self.run_id) {
            return Err("已取消".to_string());
        }
        Ok(())
    }
}

// 设计文件的「标准格式」：JSON Schema + 完整示例，直接进提示词。
// 比堆一堆散文规则稳 —— 规则写多了模型容易漏执行，照着格式抄不会。
// **schema 必须与 `models.rs`（serde camelCase）保持同步**：那边加/改字段，这里同步改。
//
// 下面 `//` 开头的中文注释是**给模型看的**（JSONC，模型照抄格式不照抄注释）：
// type 的参数形态写在这里而不是只写在下面的散文字典里 —— schema 是最被照抄的一段。
const FORMAT_SCHEMA: &str = r#"{
  "name": "设计名（字符串）",
  "dialect": "mysql | postgres | sqlite",
  "nodes": [
    {
      "kind": "table | view",
      "name": "表名（小写蛇形，不允许重名）",
      "comment": "表的中文业务说明（可选）",
      "table": {
        "fields": [
          {
            "name": "字段名（小写蛇形）",
            "type": { "base": "varchar", "length": 32 },
            "comment": "字段的中文业务含义（可选）",
            "pk": true,
            "nullable": true,
            "default": "默认值（可选，字符串）",
            "autoIncrement": true,
            "unique": true
          }
        ],
        "indexes": [
          { "name": "idx_name", "kind": "index | unique | fulltext", "fields": ["col1", "col2"] }
        ]
      },
      "view": { "sql": "视图定义 SQL（仅 kind=view 时填）" }
    }
  ],
  // type 的参数形态（其余类型只写 base）：char / varchar 用 length；
  // decimal 用 precision + scale；enum 用 values（字符串数组）。
  //   {"base": "decimal", "precision": 10, "scale": 2}
  //   {"base": "enum", "values": ["new", "paid", "closed"]}
  "relations": [
    {
      "from": { "node": "子表（外键所在端）的节点 id 或表名", "field": "外键字段名" },
      "to": { "node": "父表（被引用端）的节点 id 或表名", "field": "被引用字段名（通常主键）" },
      "kind": "1-1 | 1-n | n-n",
      "onDelete": "CASCADE（可选）"
    }
  ]
}
"#;

/// 完整最小示例：两张表 + 一条关联，把 schema 里「可选字段怎么落」演示出来。
///
/// **示例必须自身通过 `store::validate`**，否则模型会忠实照抄违规写法。
/// 这是踩过两次的坑：第一次示例用 `id` 作主键、第二次用 `title`/`content`/`status`
/// 这类通用裸名 —— 两次用户看到的都是「AI 照着示例写出来的违规字段」。
/// 测试 `example_document_passes_our_own_validator` 会把示例过一遍真实校验器，
/// 以后新增任何规则都会自动被这条测试覆盖。
const FORMAT_EXAMPLE: &str = r#"{
  "name": "文章管理系统",
  "dialect": "mysql",
  "nodes": [
    {
      "kind": "table",
      "name": "user",
      "comment": "用户：系统账号",
      "table": {
        "fields": [
          { "name": "user_id", "type": { "base": "bigint" }, "pk": true, "autoIncrement": true, "comment": "用户主键" },
          { "name": "user_name", "type": { "base": "varchar", "length": 64 }, "unique": true, "comment": "登录名" },
          { "name": "created_at", "type": { "base": "datetime" }, "nullable": false, "comment": "创建时间" }
        ]
      }
    },
    {
      "kind": "table",
      "name": "article",
      "comment": "文章：用户发布的内容",
      "table": {
        "fields": [
          { "name": "article_id", "type": { "base": "bigint" }, "pk": true, "autoIncrement": true, "comment": "文章主键" },
          { "name": "user_id", "type": { "base": "bigint" }, "comment": "作者用户，引用 user.user_id" },
          { "name": "article_title", "type": { "base": "varchar", "length": 128 }, "nullable": false, "comment": "标题" },
          { "name": "article_content", "type": { "base": "text" }, "comment": "正文" },
          { "name": "article_status", "type": { "base": "enum", "values": ["draft", "published", "archived"] }, "nullable": false, "comment": "状态" }
        ],
        "indexes": [ { "name": "idx_article_user", "kind": "index", "fields": ["user_id"] } ]
      }
    }
  ],
  "relations": [
    {
      "from": { "node": "article", "field": "user_id" },
      "to": { "node": "user", "field": "user_id" },
      "kind": "1-n",
      "onDelete": "CASCADE"
    }
  ]
}
"#;

/// 把模型返回的 JSON 收成 [`AiOutcome`]：三种形状都要能认。
///
/// | 模型写了 | 怎么处理 |
/// |---|---|
/// | `{{"clarify":{{...}}}}` | 问问题，不落图 |
/// | `{{"document":{{...}},"decisions":[...]}}` | 正常产出 |
/// | `{{"name":...,"nodes":[...]}}`（旧形状） | 正常产出，`decisions` 当空 |
///
/// 第三行是为了向后兼容：模型不听话打出旧形状时仍然能用。
/// 格式不兼容不得导致整份产出作废（Q-0404 的教训）。
/// 模型返回的三种形状。分开返回的目的是让 `dbd_ai_generate` 知道自己该回全量
/// 还是回增量（增量要拿基线文档去合）。
enum ParsedOutcome {
    Design {
        document: DbDesignDocument,
        decisions: Vec<String>,
    },
    Patch {
        patch: crate::commands::db_designer::patch::DesignPatch,
        decisions: Vec<String>,
    },
    Clarify {
        question: String,
        options: Vec<String>,
    },
}

/// 自校验重修的上限。
///
/// 为什么是小数：每轮都是一次带整份文档的完整调用，耗时与花钱都流向提升。
/// 而多数需要重修的情况只是「模型漏写了一处」—— 第一轮就能救回来；
/// 需要第三轮通常说明模型并不理解需求，再试只是多花钱。
const MAX_SELF_REPAIR: u32 = 2;

/// 自校验结果：返回给前端展示用。
#[derive(serde::Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct RepairReport {
    /// 实际重修了几轮
    pub rounds: u32,
    /// 最终是否已无阻断级错误
    pub ok: bool,
    /// 仍剩的错误（有这些时设计**照样落图**，但保存/导出会被拦下）
    pub remaining_errors: Vec<String>,
    /// 是否因为「重修后反而更糟」而回滚到了更早的版本
    pub rolled_back: bool,
}

mod prompts;
pub use prompts::*;
mod parse;
pub use parse::*;
mod repair;
pub use repair::*;
mod generation;
pub use generation::*;
mod partial;
pub use partial::*;
#[cfg(test)]
mod tests;
