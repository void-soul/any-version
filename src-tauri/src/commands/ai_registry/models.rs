use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfig {
    pub id: String,
    pub display_name: String,
    /// 协同模式中显示的头像（emoji 或文字，如 "🤖"）。为空时由前端按 id 回退。
    #[serde(default)]
    pub avatar: Option<String>,
    /// 协同模式中显示的昵称覆盖（为空时使用 display_name）。
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub website: String,
    /// 官网地址（优先于 website；website 可能指向 GitHub 仓库）
    #[serde(default)]
    pub homepage: Option<String>,
    /// GitHub 仓库地址（homepage / website 均缺省时兜底）
    #[serde(default)]
    pub github: Option<String>,
    /// 工具「原生」协议（兼容旧逻辑：one_m 后缀、配置清理判定）。
    /// 新逻辑以 supports_openai/anthropic/google 三标志为准。
    #[serde(default)]
    pub api_protocol: String,
    #[serde(default)]
    pub support_model: bool,
    #[serde(default)]
    pub support_fallback_model: bool,
    #[serde(default)]
    pub support_one_m_context: bool,
    /// 工具支持的入站协议（代理为这些协议都注册路由）。
    #[serde(default)]
    pub supports_openai: bool,
    #[serde(default)]
    pub supports_anthropic: bool,
    #[serde(default)]
    pub supports_google: bool,
    /// 内置模型名称列表（伪装模型名的预设值 C）。
    /// 非空时用户可把所选取的供应商模型 B「伪装」成其中某项 C。
    ///
    /// **必须按「新 → 旧」排序**：两个桌面端（Claude Desktop / ChatGPT 桌面端）在
    /// 「用户没填伪装名」时，会自动拿**清单第一项**当兜底声明名 —— 写死的兜底名迟早
    /// 会变成已废弃的型号，而用户看不出任何异常。清单是唯一的退役入口：官方下架一个
    /// 型号就从这里删掉，新型号加在最前面。
    #[serde(default)]
    pub builtin_models: Vec<String>,
    /// 该工具是否支持请求优化（启动页可开关）
    #[serde(default)]
    pub supports_optimizer: bool,
    /// 该工具是否支持抹平协议差异（整流器，启动页可开关）
    #[serde(default)]
    pub supports_rectifier: bool,
    /// 该工具是否使用官方**插件市场**（Codex 系是 `~/.codex/config.toml` 的
    /// `[marketplaces.*]`，Claude Code 是 `~/.claude/settings.json` 的 `extraKnownMarketplaces`）。
    #[serde(default)]
    pub supports_plugin_marketplace: bool,
    /// 插件市场走哪套后端：`"codex"`（`codex plugin`）或 `"claude"`（`claude plugin`）。
    ///
    /// 未声明时按 `"codex"` 处理 —— 向后兼容：Claude 接入之前，这个开关只有 Codex 系在用。
    #[serde(default)]
    pub plugin_marketplace_kind: Option<String>,
    pub resume_cmd: Option<String>,
    pub continue_cmd: Option<String>,
    /// 分叉命令模板（带 `{session_id}` 占位）：从指定会话复制一份新会话再进入，原会话不动。
    /// 只有 CLI 自己实现了 fork 才有意义（如 claude `--resume {session_id} --fork-session`）；
    /// 未声明的工具前端不显示「分叉」入口。
    #[serde(default)]
    pub fork_cmd: Option<String>,
    // 以下都是「可有可无」的声明。必须给 `#[serde(default)]`：漏一个字段整份 config.json
    // 就解析失败，而这个工具会被**整个从注册表里丢掉**（表现为「列表里莫名少了一个工具」，
    // 只有 stderr 上一行 parse 失败，界面完全看不出来）。现状：omp 因为少写 cacheDirs
    // 一直没进注册表。
    #[serde(default)]
    pub cache_dirs: Vec<String>,
    #[serde(default)]
    pub pkg_manager: Option<String>,
    #[serde(default)]
    pub pkg_name: Option<String>,
    #[serde(default)]
    pub config_file: Option<ConfigFileDef>,
    #[serde(default)]
    pub model_format: Option<ModelFormatDef>,
    #[serde(default)]
    pub sessions: Option<SessionScanDef>,
    #[serde(default)]
    pub skills_dir: Option<String>,
    /// 非交互派发命令模板（协同模式）：`{prompt_file}` 占位符会被替换为提示词文件路径（已加引号）。
    #[serde(default)]
    pub dispatch_cmd: Option<String>,
    /// 续聊派发命令模板：同一房间内对同一工具的后续派发走此模板（如 claude --continue），实现上下文连续。
    #[serde(default)]
    pub dispatch_continue_cmd: Option<String>,
    /// 续聊派发命令模板（带 {session_id} 占位，按工具原生会话 id 精确恢复上下文）。
    #[serde(default)]
    pub dispatch_resume_cmd: Option<String>,
    /// 派发运行模式："stream-json"(claude) / "codex-json"(codex) / "opencode-json"(opencode)，省略则一次性读取输出（兜底）。
    #[serde(default)]
    pub runner: Option<String>,
    /// 提示词传入方式："file"(--input-file 占位) / "stdin"(子进程 stdin 喂临时文件) / "arg"(`{prompt}` 内联)，省略默认 file。
    #[serde(default)]
    pub prompt_mode: Option<String>,
    #[serde(default)]
    pub skills_dir_xdg: Option<String>,
}

impl ToolConfig {
    /// 工具支持的入站协议列表（用于代理为每种协议注册路由）。
    /// 若三个标志都为空（旧配置），按 `api_protocol` 回退推导。
    pub fn inbound_protocols(&self) -> Vec<String> {
        let mut v = Vec::new();
        if self.supports_openai {
            v.push("openai".to_string());
        }
        if self.supports_anthropic {
            v.push("anthropic".to_string());
        }
        if self.supports_google {
            v.push("google".to_string());
        }
        if v.is_empty() {
            // 旧配置兜底：both → anthropic
            match self.api_protocol.as_str() {
                "anthropic" | "both" => v.push("anthropic".to_string()),
                "google" => v.push("google".to_string()),
                _ => v.push("openai".to_string()),
            }
        }
        v
    }

    /// 工具的「原生」协议：用于协议转换消息展示与 one_m 后缀判定。
    pub fn native_protocol(&self) -> String {
        if self.supports_anthropic {
            "anthropic".to_string()
        } else if self.supports_google {
            "google".to_string()
        } else {
            "openai".to_string()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigFileDef {
    pub path: String,
    pub format: String,
    #[serde(default)]
    pub schema: Option<String>,
    pub write: Option<HashMap<String, String>>,
    /// 自定义写入器：schema 复杂到「路径 → 值」表达不了的（Claude Desktop 的 3P 网关要
    /// 跨 `Claude-3p/configLibrary/<id>.json` 与 `_meta.json` 互相引用），交给 Rust 侧整份生成。
    ///
    /// 抄 EchoBird：它的 `tools/*/config.json` 用 `"custom": true` 标记，实际写入在
    /// `src-tauri/src/services/tool_config_manager/<tool>.rs`（我们对应
    /// `commands/ai/tool_config_custom.rs`）。`true` = 用工具 id 当写入器名；也可以写字符串
    /// 显式指定（多个工具复用同一个写入器时用，只是路径不同）。
    #[serde(default, alias = "customWriter")]
    pub custom: Option<CustomWriter>,
    /// 目录级环境变量覆盖（按顺序取第一个非空值作为配置目录）：如 OpenCode v2 的
    /// `OPENCODE_CONFIG_DIR`。目录确定后文件名沿用 `path` 的文件名。
    /// 抄自 EchoBird c6f4bc25。
    #[serde(default, alias = "pathEnvDirs")]
    pub path_env_dirs: Vec<String>,
    /// 用 `XDG_CONFIG_HOME` 拼接的子目录名（如 `opencode`），在 `path_env_dirs` 之后生效。
    #[serde(default, alias = "xdgSubdir")]
    pub xdg_subdir: Option<String>,
    /// 同 stem 的其它扩展名文件已存在时优先沿用它（如 OpenCode v2 的 `opencode.jsonc`）。
    #[serde(default, alias = "preferExistingExtensions")]
    pub prefer_existing_extensions: Vec<String>,
    /// 写入前**先删掉**的键（点分路径，支持 `文件#子路径`）。
    ///
    /// 用途：声明改过时，旧键会**留在用户文件里**——写入侧是语义合并，只替换受管键、
    /// 其余原样保留，旧键于是继续生效。典型例子：Codex 早期用
    /// `model_providers.X.env_key` 指向环境变量，现在改走 `auth.json` +
    /// `requires_openai_auth`；不删掉残留的 `env_key`，Codex 仍会去环境变量找 key 并报错。
    /// （CodexPlusPlus 同样维护了一份待清理键列表，`relay_config.rs` 的 `remove_root_key`。）
    #[serde(default, alias = "removeKeys")]
    pub remove_keys: Vec<String>,
}

/// `configFile.custom` 的两种写法（untagged：`true` 或 `"workbuddy"`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CustomWriter {
    Flag(bool),
    Key(String),
}

impl ConfigFileDef {
    /// 该工具要用的自定义写入器名（None = 走通用的「路径 → 值」写入）。
    pub fn custom_writer(&self, tool_id: &str) -> Option<String> {
        match self.custom.as_ref()? {
            // `"custom": true` → 工具 id 就是写入器名（EchoBird 的写法）
            CustomWriter::Flag(true) => Some(tool_id.to_string()),
            CustomWriter::Flag(false) => None,
            CustomWriter::Key(key) => {
                let key = key.trim();
                if key.is_empty() {
                    None
                } else {
                    Some(key.to_string())
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFormatDef {
    pub prefix: Option<String>,
    #[serde(default)]
    pub extract_last: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionScanDef {
    #[serde(rename = "type")]
    pub scan_type: String,
    #[serde(default)]
    pub dirs: Vec<String>,
}

/// 非标准安装路径的检测提示。各字段仅在对应平台上生效。
/// 用于 GUI/桌面应用在 paths.json 硬编码路径之外的检测：
/// Windows 扫描注册表 Uninstall 键、macOS 查找 /Applications、Linux 扫描 .desktop。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InstallHints {
    /// Windows: 匹配注册表 `DisplayName`（精确、大小写不敏感）。
    #[serde(default)]
    pub windows_display_names: Vec<String>,
    /// Windows: 前缀匹配（用于 DisplayName 内嵌版本号的应用，如 "WorkBuddy 4.24.2"）。
    #[serde(default)]
    pub windows_display_name_prefixes: Vec<String>,
    /// Windows: 可选的 `Publisher` 过滤（消歧义）。
    #[serde(default)]
    pub windows_publisher: Option<String>,
    /// macOS: 要搜索的 `.app` 名称（在 /Applications 与 ~/Applications 下查找）。
    #[serde(default)]
    pub macos_app_name: Option<String>,
    /// Linux: 匹配 .desktop 文件 `Name=` 的名称列表。
    #[serde(default)]
    pub linux_desktop_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub api_protocol: Vec<String>,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub start_command: String,
    #[serde(default)]
    pub detect_cmd: String,
    #[serde(default)]
    pub install_cmd: String,
    #[serde(default)]
    pub uninstall_cmd: Option<String>,
    #[serde(default)]
    pub paths: HashMap<String, Vec<String>>,
    /// MSIX/Store 应用启动 URI（如 "shell:AppsFolder\\Claude_...!Claude"），
    /// 用于没有普通 .exe 路径的应用启动与检测。
    #[serde(default)]
    pub launch_uri: Option<String>,
    /// 非标准安装路径的检测提示（注册表 / Applications / .desktop）。
    #[serde(default)]
    pub install_hints: Option<InstallHints>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPreset {
    pub id: String,
    pub name: String,
    pub category: String,
    pub website: String,
    pub openai_url: String,
    pub anthropic_url: String,
    #[serde(default)]
    pub google_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsScanConfig {
    pub description: String,
    pub base_skills_dir: String,
    pub tool_skills_dirs: HashMap<String, Vec<String>>,
    /// 会主动读取 canonical 目录（~/.agents/skills）的工具列表。
    /// 这些工具不应再对其自身 skills 目录建 junction，否则同一批技能
    /// 会被「原生目录(junction→canonical)」和「canonical 直读」各读一遍 → 重复告警。
    #[serde(default)]
    pub reads_agents_skills: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolConfig {
    /// 该工具的中心 MCP 配置文件路径（支持 ~ 与 %VAR%）
    pub config_path: String,
    /// 配置格式：`claude`(Claude Code) | `gemini`(Qwen/Gemini) | `opencode`(OpenCode 系)
    pub format: String,
    /// 外部指定的 MCP 根 Key（如 "mcp" 或 "mcpServers"）
    #[serde(default)]
    pub mcp_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpConfig {
    pub description: String,
    pub tools: HashMap<String, McpToolConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalsConfig {
    pub description: String,
    pub terminals: HashMap<String, TerminalDef>,
    pub proxy_settings: ProxySettingsDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalDef {
    pub name: String,
    pub exe_names: Vec<String>,
    #[serde(default)]
    pub exe_path: Option<String>,
    #[serde(default)]
    pub always: bool,
    #[serde(default)]
    pub launch_args: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxySettingsDef {
    pub default_port: u16,
    pub listen_address: String,
    pub timeout_seconds: u32,
}

/// 与前端交互的 Provider 预设 DTO。
///
/// 注意：**不要**加 `rename_all = "camelCase"`。前端 `Preset` 类型与 AI 配置面
/// （`AiProvider`）一致按 snake_case 读 `openai_url` 等字段；一旦序列化成
/// `openaiUrl`，前端取不到 URL → 「添加预设供应商」只剩名称和官网。
/// （camelCase rename 只属于上面反序列化 `providers.json` 的 `ProviderPreset`。）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPresetDto {
    pub id: String,
    pub name: String,
    pub category: String,
    pub website: String,
    pub openai_url: String,
    pub anthropic_url: String,
    pub google_url: String,
}

/// 工具配置文件的简要信息（给前端看「模型会写到哪里」）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfigFileDto {
    pub path: String,
    pub format: String,
}
