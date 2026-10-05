//! 通用「路径 → 值」写入之后的**附加结构化写入**。
//!
//! ## 为什么需要
//!
//! 声明里的 `configFile.write` 是「把某个点分路径设成某个值」的平铺映射，表达不了的语义有：
//!
//! - **数组内按 id 查找后再合并**（dsh 的 profile 序列、ZCode 的 `providerRules`）；
//! - **按上游域名决定写哪个值**（Codex 的 `web_search` / `model_catalog_json`）；
//! - **只改「值以我们前缀开头」的条目、其余原样保留**（OMP 的 `modelRoles`）；
//! - 顺手删掉一组遗留键、并生成一个被引用的附属文件（Codex 的 catalog）。
//!
//! 这些走本模块的 per-tool 步骤，对照 EchoBird `services/tool_config_manager/<tool>.rs`。
//! 执行顺序是**通用写入先跑，本模块在其结果上做追加修补** —— 所以这里读到的文件
//! 已经是「用户原有内容 ∪ 我们声明的键」。
//!
//! ## 与 `tool_config_custom` 的分工
//!
//! `tool_config_custom` 是**整份接管**（文件 schema 完全不同，声明里根本没有 write 映射，
//! 如 WorkBuddy / Claude Desktop）；本模块是**在通用写入之上补结构化语义**，
//! 声明里的 write 映射仍然照常生效。

use std::path::Path;

use crate::commands::ai::tool_config_restore::RestoreOutcome;

pub(crate) mod codex;
pub(crate) mod omp;
pub(crate) mod zcode;

/// 附加写入的上下文。
pub(crate) struct ExtrasCtx<'a> {
    pub tool_id: &'a str,
    /// `configFile.path` 解析后的**主配置文件**绝对路径（兄弟文件由各 handler 自己推导）。
    pub main_path: &'a Path,
    /// 要**写进配置**的端点。代理模式下这是本地代理（`http://127.0.0.1:PORT`）——
    /// 工具必须走代理，代理才能做统计、协议转换与模型伪装。
    pub base_url: &'a str,
    /// **真实上游**端点。只用来做「这家厂商是什么」的判断（域名匹配、厂商名展示）。
    /// 留空表示直连，此时 [`ExtrasCtx::vendor_url`] 会退回 `base_url`。
    pub upstream_url: &'a str,
    pub api_key: &'a str,
    /// 完整模型名（可能带声明的 provider 前缀，如 `anyversion/deepseek-v4-pro`）。
    pub model: &'a str,
    /// 去掉前缀的模型名（Ctx 里的「模型 id」）。
    ///
    /// 注意这是**声明名 C**：配了伪装时它是官方名（`gpt-5.1-codex`），不是真实模型。
    /// 凡是「按模型身份判定能力」的逻辑（上下文窗口、图像支持）必须用
    /// [`ExtrasCtx::real_model_name`]，否则查表必然落空。
    pub model_name: &'a str,
    /// 真实模型 B 的 id（供应商模型，去掉 provider 前缀；`[1m]` 后缀由各 handler 自己剥）。
    ///
    /// 与 [`ExtrasCtx::model_name`] 分开是有原因的：Codex 的 `model_context_window`
    /// 必须按**真实模型**的窗口写 —— 拿伪装名查表会把 204,800 的模型当成 1,000,000，
    /// Codex 于是永远等不到压缩，上游直接报超长。
    pub real_model_name: &'a str,
    /// 我们在该工具里使用的 provider 名（必须与声明里的键一致）。
    pub provider: &'a str,
    /// 实际选中的出站协议：`anthropic` / `openai`。
    pub chosen_protocol: &'a str,
    /// 用户是否开启了「联网搜索」开关。
    pub web_search: bool,
}

impl ExtrasCtx<'_> {
    /// 判断「是哪家厂商」时该用的地址 = 真实上游。
    ///
    /// **不要拿 `base_url` 做域名判断**：代理模式下它是 `127.0.0.1`，任何域名匹配都会
    /// 静默落空 —— 那正是「代码看着对、行为完全没变」的典型成因。
    pub(crate) fn vendor_url(&self) -> &str {
        if self.upstream_url.trim().is_empty() {
            self.base_url
        } else {
            self.upstream_url
        }
    }
}

/// 我们在各工具里使用的 **provider 名**。
///
/// 必须是**声明里用的那个键**（`providers.<name>` / `model_providers.<name>` / 角色值的前缀）——
/// 两边不一致的话，配置里注册的 provider 与这里改写的角色/规则会对不上，工具直接报找不到模型。
///
/// 历史原因 OMP 用的是 `echobird`（早期照 EchoBird 的配置写的），改它会让用户已有配置失效。
pub(crate) fn provider_for(tool_id: &str) -> &'static str {
    match tool_id {
        "omp" => "echobird",
        _ => "anyversion",
    }
}

/// 按工具 id 分派附加写入。返回**被改动的文件**列表（仅用于日志）。
///
/// 没有附加逻辑的工具返回空列表 —— 这是常态，不是错误。
pub(crate) fn apply_extras(ctx: &ExtrasCtx<'_>) -> Result<Vec<String>, String> {
    match ctx.tool_id {
        // Codex CLI 与 ChatGPT 桌面端**共用** `~/.codex/config.toml`，两边都得跑：
        // 只给一个跑的话，换另一个工具设模型时这些键就还是旧值。
        "codex-cli" | "chatgptdesktop" => codex::apply(ctx),
        "zcode" => zcode::apply(ctx),
        "omp" => omp::apply(ctx),
        _ => Ok(Vec::new()),
    }
}

/// 还原本模块写下的东西（见 [`apply_extras`]）。
///
/// 通用还原删的是**声明 `write` 映射里的键**，本模块写的内容**不在那份映射里**
/// （另一个文件、`refs` 层级、数组内条目…），所以必须单独还原。不还原的后果不是「少了点清理」，
/// 而是「勾了『使用官方模型』之后工具仍然跑在我们的 provider 上」—— ZCode 的 providerRules
/// 就是这样：那条配置还在，ZCode 就还听它的。
pub(crate) fn restore_extras(tool_id: &str, main_path: &Path) -> Result<RestoreOutcome, String> {
    match tool_id {
        // 两个工具共用 `~/.codex/config.toml`，还原也要两边都走
        "codex-cli" | "chatgptdesktop" => codex::restore(main_path),
        "zcode" => zcode::restore(main_path),
        "omp" => omp::restore(main_path),
        _ => Ok(RestoreOutcome::default()),
    }
}

/// 读文件；不存在或读失败都当空串（写回时只动我们的键）。
pub(super) fn read_or_empty(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// 原子写文件，顺带建父目录。返回展示用的路径。
pub(super) fn write_file(path: &Path, content: &str) -> Result<String, String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建目录 {} 失败: {e}", parent.display()))?;
        }
    }
    crate::commands::config::atomic_write_file(path, content.as_bytes())
        .map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;
    Ok(path.display().to_string())
}

/// 主配置文件的同目录兄弟文件（`~/.zcode/v2/config.json` → `~/.zcode/v2/provider_config.json`）。
pub(super) fn sibling(main: &Path, name: &str) -> std::path::PathBuf {
    main.parent().unwrap_or(Path::new(".")).join(name)
}
