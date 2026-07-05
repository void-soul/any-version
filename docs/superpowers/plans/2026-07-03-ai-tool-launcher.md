# AI 工具启动器 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 重构 AI 工具启动流程：选择工具 → 选择模型 → 选择会话（新建/历史） → 选择目录/终端 → 启动。

**Architecture:** ToolLauncher 页面改为多步骤向导。后端新增会话扫描命令（扫描各工具的本地会话目录）。模型列表根据工具的 `apiProtocol` 过滤 Provider 中兼容的模型。会话恢复通过各工具的 CLI 参数实现（`claude --resume`, `codex resume`, `gemini --resume`）。

**Tech Stack:** React 19, Tauri 2 (Rust), TypeScript, Tailwind CSS 4

---

## 研究发现

### 工具会话恢复机制

| 工具 | 恢复命令 | 会话存储位置 | 模型参数 |
|------|----------|-------------|----------|
| Claude Code | `claude --resume <id>` 或 `claude --continue` | `~/.claude/projects/<encoded-path>/<session-id>.jsonl` | `--model <model>` |
| Codex CLI | `codex resume` 或 `codex resume --last` | Codex 内部存储 | `-m <MODEL>` |
| Gemini CLI | `gemini --resume latest` 或 `--resume <index>` | Gemini 内部存储 | `-m <model>` |
| Aider | 无内置会话恢复 | 无 | `--model <model>` |
| Deveco | 无内置会话恢复 | 无 | 无（不支持模型配置） |
| 其他工具 | 无内置会话恢复 | 无 | 无 |

### 工具协议与模型兼容性

| 工具 | 支持协议 | 支持模型配置 | 备注 |
|------|----------|-------------|------|
| Claude Code | Anthropic | ✅ | 只能用 Anthropic 协议模型 |
| Codex CLI | OpenAI | ✅ | 只能用 OpenAI 协议模型 |
| Aider | OpenAI + Anthropic | ✅ | 两种协议都支持 |
| Gemini CLI | Google | ✅ | 用自己的 API Key |
| Deveco | — | ❌ | `noModelConfig` |
| OpenClaw | OpenAI | ✅ | |
| OpenCode | OpenAI | ✅ | |
| Hermes | OpenAI | ✅ | |

### 终端检测

Windows 上可检测的终端：
- `cmd.exe` — 始终可用
- `powershell.exe` — 始终可用
- `pwsh.exe` — PowerShell 7+（如果安装）
- `wezterm.exe` — WezTerm（如果安装）
- `wt.exe` — Windows Terminal（如果安装）

---

## Task 1: 扩展工具检测 — 添加协议和模型配置支持标记

**Files:**
- Modify: `src-tauri/src/commands/ai.rs` — `AiToolDef` 和 `DetectedAiTool` 结构体

- [ ] **Step 1: 更新 AiToolDef 添加协议字段**

```rust
struct AiToolDef {
    id: &'static str,
    display_name: &'static str,
    detect_cmd: &'static str,
    install_cmd: &'static str,
    website: &'static str,
    /// 支持的协议："anthropic" | "openai" | "both" | "none"
    api_protocol: &'static str,
    /// 是否支持模型配置
    supports_model: bool,
    /// 恢复会话的命令模板（{session_id} 为占位符）
    resume_cmd: Option<&'static str>,
    /// 继续最近会话的命令
    continue_cmd: Option<&'static str>,
}
```

- [ ] **Step 2: 更新 AI_TOOLS 常量，为每个工具填入协议和会话信息**

```rust
AiToolDef {
    id: "claude-code",
    display_name: "Claude Code",
    detect_cmd: "claude --version",
    install_cmd: "npm install -g @anthropic-ai/claude-code",
    website: "https://claude.ai/code",
    api_protocol: "anthropic",
    supports_model: true,
    resume_cmd: Some("claude --resume {session_id}"),
    continue_cmd: Some("claude --continue"),
},
// ... 其他工具类似
```

- [ ] **Step 3: 更新 DetectedAiTool 结构体**

```rust
pub struct DetectedAiTool {
    pub id: String,
    pub display_name: String,
    pub installed: bool,
    pub version: Option<String>,
    pub install_cmd: String,
    pub website: String,
    pub api_protocol: String,
    pub supports_model: bool,
    pub resume_cmd: Option<String>,
    pub continue_cmd: Option<String>,
}
```

- [ ] **Step 4: 更新 detect_single_tool 填充新字段**

- [ ] **Step 5: cargo check**

---

## Task 2: 新增会话扫描命令

**Files:**
- Modify: `src-tauri/src/commands/ai.rs`

- [ ] **Step 1: 添加 scan_tool_sessions Tauri 命令**

```rust
#[derive(Serialize, Clone, Debug)]
pub struct ToolSession {
    pub session_id: String,
    pub project_path: String,
    pub last_used: String,
    pub summary: Option<String>,  // 会话的第一条用户消息作为摘要
}

#[tauri::command]
pub fn scan_tool_sessions(tool_id: String) -> Result<Vec<ToolSession>, String> {
    match tool_id.as_str() {
        "claude-code" => scan_claude_sessions(),
        "gemini-cli" => scan_gemini_sessions(),
        _ => Ok(Vec::new()), // 其他工具暂不支持会话扫描
    }
}
```

- [ ] **Step 2: 实现 scan_claude_sessions**

扫描 `~/.claude/projects/` 目录：
1. 遍历每个项目目录（如 `E--pro-my-any-version`）
2. 找到 `.jsonl` 会话文件
3. 读取文件的前几行，提取 `sessionId` 和第一条 `user` 类型消息作为摘要
4. 用文件修改时间作为 `last_used`
5. 将编码的目录名还原为实际路径（`E--pro-my-any-version` → `E:\pro\my\any-version`）

```rust
fn scan_claude_sessions() -> Result<Vec<ToolSession>, String> {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let projects_dir = PathBuf::from(&home).join(".claude").join("projects");
    let mut sessions = Vec::new();

    if !projects_dir.exists() {
        return Ok(sessions);
    }

    for entry in fs::read_dir(&projects_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let dir_name = entry.file_name().to_string_lossy().to_string();
        let project_path = decode_claude_project_path(&dir_name);

        // 扫描 .jsonl 文件
        for file_entry in fs::read_dir(entry.path()).map_err(|e| e.to_string())? {
            let file_entry = file_entry.map_err(|e| e.to_string())?;
            let name = file_entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".jsonl") { continue; }

            let session_id = name.trim_end_matches(".jsonl").to_string();
            let metadata = file_entry.metadata().map_err(|e| e.to_string())?;
            let last_modified = metadata.modified().map_err(|e| e.to_string())?;
            let last_used: chrono::DateTime<chrono::Local> = last_modified.into();

            // 读取前几行提取摘要
            let summary = extract_claude_session_summary(&file_entry.path());

            sessions.push(ToolSession {
                session_id,
                project_path: project_path.clone(),
                last_used: last_used.format("%Y-%m-%d %H:%M:%S").to_string(),
                summary,
            });
        }
    }

    // 按时间倒序
    sessions.sort_by(|a, b| b.last_used.cmp(&a.last_used));
    sessions.truncate(50);
    Ok(sessions)
}
```

- [ ] **Step 3: 实现 decode_claude_project_path 辅助函数**

将 `E--pro-my-any-version` 还原为 `E:\pro\my\any-version`（Windows 上 Claude Code 用 `-` 替换 `\` 和 `:`）

- [ ] **Step 4: 实现 extract_claude_session_summary**

读取 jsonl 文件，找到第一条 `type: "user"` 的消息，提取 `content` 的前 100 字符作为摘要。

- [ ] **Step 5: 实现 scan_gemini_sessions**

调用 `gemini --list-sessions` 命令解析输出。

- [ ] **Step 6: 在 lib.rs 注册 scan_tool_sessions**

- [ ] **Step 7: cargo check**

---

## Task 3: 新增终端检测命令

**Files:**
- Modify: `src-tauri/src/commands/ai.rs`

- [ ] **Step 1: 添加 detect_terminals Tauri 命令**

```rust
#[derive(Serialize, Clone, Debug)]
pub struct TerminalInfo {
    pub id: String,       // "cmd" | "powershell" | "pwsh" | "wezterm" | "wt"
    pub name: String,     // 显示名
    pub exe_path: String, // 可执行文件路径
}

#[tauri::command]
pub fn detect_terminals() -> Result<Vec<TerminalInfo>, String> {
    let mut terminals = Vec::new();

    // CMD — 始终可用
    terminals.push(TerminalInfo {
        id: "cmd".to_string(),
        name: "CMD".to_string(),
        exe_path: "cmd.exe".to_string(),
    });

    // PowerShell — 始终可用
    terminals.push(TerminalInfo {
        id: "powershell".to_string(),
        name: "PowerShell".to_string(),
        exe_path: "powershell.exe".to_string(),
    });

    // PowerShell 7+
    if which_exists("pwsh.exe") {
        terminals.push(TerminalInfo {
            id: "pwsh".to_string(),
            name: "PowerShell 7".to_string(),
            exe_path: "pwsh.exe".to_string(),
        });
    }

    // Windows Terminal
    if which_exists("wt.exe") {
        terminals.push(TerminalInfo {
            id: "wt".to_string(),
            name: "Windows Terminal".to_string(),
            exe_path: "wt.exe".to_string(),
        });
    }

    // WezTerm
    if which_exists("wezterm.exe") {
        terminals.push(TerminalInfo {
            id: "wezterm".to_string(),
            name: "WezTerm".to_string(),
            exe_path: "wezterm.exe".to_string(),
        });
    }

    Ok(terminals)
}

fn which_exists(name: &str) -> bool {
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            if dir.join(name).exists() {
                return true;
            }
        }
    }
    false
}
```

- [ ] **Step 2: 在 lib.rs 注册 detect_terminals**

- [ ] **Step 3: cargo check**

---

## Task 4: 重构 ToolLauncher — 多步骤向导

**Files:**
- Modify: `src/components/ai/ToolLauncher.tsx`

这是核心 UI 改造。将当前的单页布局改为多步骤向导：

**Step 1: 选择工具** → 显示已安装的工具列表（带协议标签）
**Step 2: 选择模型** → 根据工具协议过滤可用模型，显示官方模型 + 用户配置的模型
**Step 3: 选择会话** → 新建会话 / 继续最近 / 选择历史会话
**Step 4: 配置启动** → 选择目录（默认/自定义）+ 选择终端

- [ ] **Step 1: 定义向导状态和数据结构**

```typescript
type WizardStep = "tool" | "model" | "session" | "launch";

interface ToolSession {
  session_id: string;
  project_path: string;
  last_used: string;
  summary: string | null;
}

interface TerminalInfo {
  id: string;
  name: string;
  exe_path: string;
}

// 向导状态
const [step, setStep] = useState<WizardStep>("tool");
const [selectedTool, setSelectedTool] = useState<DetectedAiTool | null>(null);
const [selectedModel, setSelectedModel] = useState<string>("");
const [sessionMode, setSessionMode] = useState<"new" | "continue" | "resume">("new");
const [selectedSession, setSelectedSession] = useState<ToolSession | null>(null);
const [projectPath, setProjectPath] = useState("");
const [defaultProjectPath, setDefaultProjectPath] = useState("");
const [selectedTerminal, setSelectedTerminal] = useState("cmd");
const [terminals, setTerminals] = useState<TerminalInfo[]>([]);
const [sessions, setSessions] = useState<ToolSession[]>([]);
```

- [ ] **Step 2: 实现 Step 1 — 工具选择**

显示已安装工具列表，每个工具卡片显示：
- 工具名称 + 版本
- 协议标签（Anthropic / OpenAI / Both / None）
- 是否支持模型配置

点击工具进入 Step 2。

- [ ] **Step 3: 实现 Step 2 — 模型选择**

根据工具的 `api_protocol` 过滤模型：
- `anthropic` → 只显示启用 Anthropic 协议的 Provider 中的模型
- `openai` → 只显示启用 OpenAI 协议的 Provider 中的模型
- `both` → 显示所有模型
- `none` → 显示"该工具不支持配置模型"

对于 Claude Code，额外显示官方模型列表（claude-sonnet-4-20250514, claude-haiku-4-5-20251001 等）。
对于 Codex CLI，显示 OpenAI 官方模型。

点击模型进入 Step 3。

- [ ] **Step 4: 实现 Step 3 — 会话选择**

三个选项：
1. **新建会话** — 进入 Step 4 选择目录
2. **继续最近会话** — 调用 `--continue`（仅 Claude Code 支持），跳到启动
3. **选择历史会话** — 显示会话列表（调用 `scan_tool_sessions`），点击恢复

会话列表每项显示：会话 ID（截断）、项目路径、最后使用时间、摘要。

如果工具不支持会话恢复（如 Aider、Deveco），只显示"新建会话"。

- [ ] **Step 5: 实现 Step 4 — 启动配置**

- 项目目录：默认目录（从设置读取）+ 自定义目录（浏览按钮）
- 终端选择：下拉框，显示检测到的终端列表
- 启动按钮

- [ ] **Step 6: 实现启动逻辑**

调用 `launch_ai_tool` 命令，传递：
- `tool_id`
- `project_path`
- `model_id`（如果支持）
- `session_id`（如果是恢复）
- `terminal_id`（选择的终端）
- `session_mode`（"new" | "continue" | "resume"）

- [ ] **Step 7: npx tsc --noEmit**

---

## Task 5: 更新 Rust 后端 launch_ai_tool 支持新参数

**Files:**
- Modify: `src-tauri/src/commands/ai.rs`

- [ ] **Step 1: 更新 LaunchAiToolRequest 结构体**

```rust
#[derive(Deserialize)]
pub struct LaunchAiToolRequest {
    pub tool_id: String,
    pub project_path: String,
    pub model_id: Option<String>,
    pub session_id: Option<String>,
    pub session_mode: Option<String>, // "new" | "continue" | "resume"
    pub terminal_id: Option<String>,  // "cmd" | "powershell" | "pwsh" | "wt" | "wezterm"
}
```

- [ ] **Step 2: 更新 launch_ai_tool 命令签名**

```rust
#[tauri::command]
pub async fn launch_ai_tool(req: LaunchAiToolRequest) -> Result<LaunchResult, String> {
```

- [ ] **Step 3: 根据 session_mode 构建命令行参数**

```rust
let mut tool_args: Vec<String> = Vec::new();

match req.session_mode.as_deref() {
    Some("continue") => {
        if let Some(ref cmd) = tool_def.continue_cmd {
            // 直接用 continue_cmd 替换
        }
    }
    Some("resume") => {
        if let (Some(ref cmd), Some(ref sid)) = (tool_def.resume_cmd, &req.session_id) {
            tool_args = cmd.replace("{session_id}", sid).split_whitespace().map(String::from).collect();
        }
    }
    _ => {
        // 新建会话
        if let Some(ref model) = req.model_id {
            // 根据工具添加模型参数
            match req.tool_id.as_str() {
                "claude-code" => tool_args.extend(["--model".into(), model.clone()]),
                "codex-cli" | "gemini-cli" => tool_args.extend(["-m".into(), model.clone()]),
                _ => {}
            }
        }
    }
}
```

- [ ] **Step 4: 根据 terminal_id 选择终端启动方式**

```rust
let terminal = req.terminal_id.as_deref().unwrap_or("cmd");
let mut cmd = match terminal {
    "powershell" => {
        let mut c = std::process::Command::new("powershell.exe");
        c.args(&["-NoExit", "-Command", &format!("cd '{}'; {}", req.project_path, tool_cmd_str)]);
        c
    }
    "pwsh" => {
        let mut c = std::process::Command::new("pwsh.exe");
        c.args(&["-NoExit", "-Command", &format!("cd '{}'; {}", req.project_path, tool_cmd_str)]);
        c
    }
    "wt" => {
        let mut c = std::process::Command::new("wt.exe");
        c.args(&["-d", &req.project_path, "cmd", "/k", &tool_cmd_str]);
        c
    }
    "wezterm" => {
        let mut c = std::process::Command::new("wezterm.exe");
        c.args(&["start", "--cwd", &req.project_path, "cmd", "/k", &tool_cmd_str]);
        c
    }
    _ => {
        // 默认 cmd
        let mut c = std::process::Command::new("cmd.exe");
        c.args(&["/k", &format!("cd /d \"{}\" && {}", req.project_path, tool_cmd_str)]);
        c
    }
};
```

- [ ] **Step 5: 设置环境变量（API Key, Base URL, Model）**

- [ ] **Step 6: 记录会话到 ai_sessions.json**

- [ ] **Step 7: cargo check**

---

## Task 6: 设置页面添加默认项目目录配置

**Files:**
- Modify: `src-tauri/src/commands/ai.rs` — AiConfig 添加 `default_project_path`
- Modify: `src/components/GlobalSettings.tsx` — 添加默认目录配置项

- [ ] **Step 1: AiConfig 添加 default_project_path 字段**

```rust
#[serde(default)]
pub default_project_path: String,
```

- [ ] **Step 2: GlobalSettings 添加默认项目目录配置 UI**

- [ ] **Step 3: ToolLauncher 启动时读取默认目录**

- [ ] **Step 4: cargo check + npx tsc --noEmit**

---

## Task 7: 集成测试与验证

- [ ] **Step 1: cargo check — 确认 Rust 编译通过**
- [ ] **Step 2: npx tsc --noEmit — 确认 TypeScript 编译通过**
- [ ] **Step 3: pnpm tauri dev — 启动应用验证**
- [ ] **Step 4: 测试完整流程：选择 Claude Code → 选择模型 → 新建会话 → 选择目录 → 启动**
- [ ] **Step 5: 测试会话恢复：选择 Claude Code → 历史会话 → 选择会话 → 恢复**
- [ ] **Step 6: 测试终端切换：分别用 CMD / PowerShell / Windows Terminal 启动**
