# EchoBird 工具配置与启动机制完整分析

> 分析时间：2026-07-05
> 分析仓库：E:\pro\other-sdk\EchoBird (v5.4.2, 公开仓库)

---

## 一、重要声明

**EchoBird 的核心业务逻辑在私有 crate `echobird_core` 中**（来自私有仓库 `EchoBird-secret-`）。公开仓库仅包含：
- 前端 UI（React/TypeScript）
- 工具定义文件（`tools/<id>/config.json` + `paths.json`）
- 编译时静态资源
- Tauri 薄壳（`src-tauri/src/lib.rs` + `main.rs`）

所有实际的配置读写、进程启动、模型管理等逻辑在私有 crate 中，**公开仓库无法直接看到实现代码**。

---

## 二、项目结构

```
E:\pro\other-sdk\EchoBird\
├── src/                        # 前端 React/TypeScript
│   ├── api/tauri.ts            # Tauri IPC 调用层
│   ├── api/types.ts            # 共享类型定义
│   ├── pages/AppManager/       # 应用管理器页面
│   │   └── AppManagerProvider.tsx  # 核心启动逻辑
│   └── data/officialEndpoints.ts   # 官方恢复端点
├── tools/                      # 26个工具定义
│   ├── <tool-id>/
│   │   ├── config.json         # 模型配置读写映射
│   │   ├── paths.json          # 平台安装路径+元数据
│   │   └── models.json         # 仅 reversi/translator 有
│   └── README.md               # 配置系统文档
├── src-tauri/                  # Tauri 薄壳
│   └── src/lib.rs              # 仅调用 echobird_core::run()
└── docs/api/tools/install/     # 25个安装文档 JSON
```

---

## 三、配置系统核心概念

### 3.1 config.json 的两种模式

#### 模式 A：声明式映射（`custom: false`）

直接在 `config.json` 中定义 `read`/`write` 路径映射，**无需额外 Rust 代码**。

```json
{
    "docs": "https://docs.anthropic.com/...",
    "configFile": "~/.claude/settings.json",
    "format": "json",
    "read": {
        "model": ["env.ANTHROPIC_MODEL"],
        "baseUrl": ["env.ANTHROPIC_BASE_URL"],
        "apiKey": ["env.ANTHROPIC_AUTH_TOKEN", "env.ANTHROPIC_API_KEY"]
    },
    "write": {
        "env.ANTHROPIC_MODEL": "model",
        "env.ANTHROPIC_BASE_URL": "baseUrl",
        "env.ANTHROPIC_AUTH_TOKEN": "apiKey"
    }
}
```

#### 模式 B：自定义逻辑（`custom: true`）

只有 `configFile` 和 `format`，**读写映射在私有 Rust crate 中实现**。

```json
{
    "configFile": "~/.config/opencode/opencode.jsonc",
    "format": "json",
    "custom": true
}
```

### 3.2 `env.XXX` 语法

以 `env.` 为前缀的 key 表示**环境变量**，而非配置文件字段：

| 配置项 | 含义 |
|--------|------|
| `env.ANTHROPIC_MODEL` | 启动进程时设置 `ANTHROPIC_MODEL` 环境变量 |
| `env.ANTHROPIC_BASE_URL` | 启动进程时设置 `ANTHROPIC_BASE_URL` 环境变量 |
| `env.ANTHROPIC_AUTH_TOKEN` | 启动进程时设置 `ANTHROPIC_AUTH_TOKEN` 环境变量 |

**只有 Claude Code CLI 使用了 `env.` 语法**。其他工具都是直接写入配置文件。

### 3.3 API 协议支持

每个工具的 `paths.json` 中有 `apiProtocol` 数组：

- `openai` — 支持 OpenAI 兼容 API（`/v1/chat/completions`）
- `anthropic` — 支持 Anthropic 兼容 API（`/v1/messages`）

前端根据协议选择使用 `baseUrl`（OpenAI）还是 `anthropicUrl`（Anthropic）。

---

## 四、全部 26 个工具配置一览

### 4.1 `custom: false` 工具（声明式映射）

| 工具 ID | 配置文件 | 格式 | read/write 在 config.json |
|---------|---------|------|--------------------------|
| **claudecode** | `~/.claude/settings.json` | JSON | ✅ `env.XXX` 环境变量注入 |
| **claudedesktop** | `~/.claude/settings.json` | JSON | ❌ 非 custom 但无显式映射（桌面应用） |
| **claudescience** | `~/.claude-science/preferences.json` | JSON | ❌ noModelConfig=true |
| **coffeecli** | 无 | — | ❌ noModelConfig=true |
| **cursor** | 无 | — | ❌ noModelConfig=true |
| **geminidesktop** | `~/.gemini/settings.json` | JSON | ❌ 非 custom 但无显式映射 |
| **pi** | `~/.pi/agent/settings.json` | JSON | ❌ 非 custom 但无显式映射 |
| **vscode/windsurf/trae/traecn** | 无 | — | ❌ noModelConfig=true（IDE 工具） |

### 4.2 `custom: true` 工具（私有 crate 实现）

| 工具 ID | 配置文件 | 格式 | 说明 |
|---------|---------|------|------|
| **opencode** | `~/.config/opencode/opencode.jsonc` | JSON | CLI Code |
| **mimocode** | `~/.config/mimocode/mimocode.json` | JSON | CLI Code（小米） |
| **codex** | `~/.codex/config.toml` | TOML | CLI Code（有本地代理） |
| **codexdesktop** | `~/.codex/config.toml` | TOML | 桌面应用 |
| **aider** | `~/.aider.conf.yml` | YAML | CLI Code |
| **qwencode** | `~/.qwen/settings.json` | JSON | CLI Code（阿里） |
| **grok** | `~/.grok/config.toml` | TOML | CLI Code |
| **openclaw** | `~/.openclaw/openclaw.json` | JSON | CLI Code |
| **hermes** | `~/.echobird/hermes.json` | JSON | 桌面应用 |
| **opencodedesktop** | `~/.config/opencode/opencode.jsonc` | JSON | 桌面应用 |
| **workbuddy** | `~/.workbuddy/models.json` | JSON | 桌面应用（腾讯） |
| **zcode** | `~/.zcode/v2/config.json` | JSON | 桌面应用 |
| **vibe-trading** | `~/.vibe-trading/.env` | ENV | 量化交易 Agent |

### 4.3 嵌入式工具

| 工具 ID | models.json | 说明 |
|---------|------------|------|
| **reversi** | ✅ | AI 黑白棋游戏，`window.__MODEL_CONFIG__` 注入 |
| **translator** | ✅ | AI 翻译工具，`window.__MODEL_CONFIG__` 注入 |

---

## 五、Claude Code 配置写入机制（**唯一使用环境变量的工具**）

### config.json

```json
{
    "docs": "https://docs.anthropic.com/en/docs/claude-code/settings",
    "configFile": "~/.claude/settings.json",
    "format": "json",
    "read": {
        "model": ["env.ANTHROPIC_MODEL"],
        "baseUrl": ["env.ANTHROPIC_BASE_URL"],
        "apiKey": ["env.ANTHROPIC_AUTH_TOKEN", "env.ANTHROPIC_API_KEY"]
    },
    "write": {
        "env.ANTHROPIC_MODEL": "model",
        "env.ANTHROPIC_SMALL_FAST_MODEL": "model",
        "env.ANTHROPIC_DEFAULT_SONNET_MODEL": "model",
        "env.ANTHROPIC_DEFAULT_OPUS_MODEL": "model",
        "env.ANTHROPIC_DEFAULT_HAIKU_MODEL": "model",
        "env.ANTHROPIC_BASE_URL": "baseUrl",
        "env.ANTHROPIC_AUTH_TOKEN": "apiKey",
        "env.ANTHROPIC_API_KEY": "",
        "env.API_TIMEOUT_MS": "3000000",
        "env.CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1"
    }
}
```

### 写入效果

写入配置时，**也写入 `~/.claude/settings.json` 的 `env` 字段**：

```json
{
    "env": {
        "ANTHROPIC_MODEL": "用户选择的模型",
        "ANTHROPIC_SMALL_FAST_MODEL": "用户选择的模型",
        "ANTHROPIC_DEFAULT_SONNET_MODEL": "用户选择的模型",
        "ANTHROPIC_DEFAULT_OPUS_MODEL": "用户选择的模型",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL": "用户选择的模型",
        "ANTHROPIC_BASE_URL": "上游 API 地址",
        "ANTHROPIC_AUTH_TOKEN": "API Key",
        "ANTHROPIC_API_KEY": "",
        "API_TIMEOUT_MS": "3000000",
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1"
    }
}
```

**核心要点**：Claude Code 启动时从 `~/.claude/settings.json` 的 `env` 字段读取环境变量，而 EchoBird 在启动前把模型配置写入这个文件的 `env` 字段。**不需要传 `-m` 参数**。

### 1M 上下文处理

前端检查 `oneMContext` 标志，如果开启则在 model ID 后追加 `[1m]`：
```
model: "claude-sonnet-4-5[1m]"
```
Claude Code 解析时会识别 `[1m]` 后缀，启用 1M 上下文窗口，但发给上游 API 前会去掉该后缀。

---

## 六、Codex 特殊机制：本地协议翻译代理

Codex CLI 只能讲 **Responses API**（OpenAI 专有协议），而大多数第三方模型只支持 **Chat Completions API**。

### 架构

```
Codex CLI  ──Responses API──►  EchoBird Rust Proxy  ──Chat Completions──►  LLM Provider
            127.0.0.1:53682    (codex_proxy)                              (DeepSeek 等)
```

### 工作流程

1. **写入配置**：`apply_codex` 写入标准 13 行 `~/.codex/config.toml`，包含 `base_url = "http://127.0.0.1:53682/v1"` 和 `wire_api = "responses"`
2. **启动代理**：Tauri 启动时绑定 `127.0.0.1:53682`，处理 `POST /v1/responses`
3. **协议翻译**：Responses 请求 → Chat Completions 请求 → 上游 → SSE 响应 → Responses SSE 响应
4. **模型 ID 重写**：Codex 的 `gpt-5.4` → 用户选择的上游模型 ID

### Relay 模式

前端 `relayMode` 控制是否绕开代理：
- `relayMode=false`（默认）：Codex → 本地代理 → 上游
- `relayMode=true`（中继站已有 Responses 协议）：Codex → 直接连上游（`~/.codex/config.toml` 写入真实上游 URL）

---

## 七、启动流程（AppManager 完整链路）

### 7.1 前端 `handleLaunch()` (AppManagerProvider.tsx)

```
用户点击"启动"按钮
  │
  ├─ 1. 判断工具类型
  │   ├─ isLaunchable?     → 嵌入式工具（游戏/翻译器）
  │   ├─ noModelConfig?    → IDE/桌面应用，无需配置模型
  │   └─ CLI Code?         → 终端 CLI 工具
  │
  ├─ 2. 写入模型配置（条件：agreedConfigPolicy && !noModelConfig && !isLaunchable）
  │   ├─ applyModelConfig(toolId, modelInternalId)
  │   │   └─ 调用 Tauri IPC: invoke('apply_model_to_tool', { toolId, modelInfo })
  │   │       └─ [私有 crate]: 根据 config.json 的写映射，写入工具的配置文件
  │   │           ├─ claudecode → 写 ~/.claude/settings.json 的 env 字段
  │   │           ├─ opencode   → 写 ~/.config/opencode/opencode.jsonc [custom]
  │   │           ├─ mimocode   → 写 ~/.config/mimocode/mimocode.json [custom]
  │   │           ├─ codex      → 写 ~/.codex/config.toml + auth.json [custom]
  │   │           ├─ aider      → 写 ~/.aider.conf.yml [custom]
  │   │           └─ ...其他工具类似
  │   │
  │   └─ 检查写入结果：失败则显示错误，成功则继续
  │
  ├─ 3. 启动工具（条件：launchAfterApply || noModelConfig）
  │   ├─ isLaunchable?
  │   │   └─ launchGame(toolId, launchFile, modelConfig)
  │   │       └─ 通过 window.__MODEL_CONFIG__ 注入模型配置到 WebView
  │   │
  │   ├─ CLI Code 工具?
  │   │   ├─ 弹出文件夹选择器（选择 cwd 工作目录）
  │   │   └─ startTool(toolId, startCommand, cwd)
  │   │       └─ 调用 Tauri IPC: invoke('start_tool', { toolId, startCommand, cwd })
  │   │           └─ [私有 crate]: 查找二进制路径 → 构造启动命令 → 启动进程
  │   │               对于 claudecode: 同时设置 ANTHROPIC_* 环境变量（从 ~/.claude/settings.json 读取）
  │   │               对于 codex: 确保本地代理已运行
  │   │               对于其他: 直接启动二进制
  │   │
  │   └─ 桌面/IDE 工具?
  │       └─ startTool(toolId) 或 launchUri
  │           └─ 直接启动桌面应用
  │
  └─ 4. 完成
      └─ 3秒冷却期，防止重复点击
```

### 7.2 Tauri 后端 IPC 接口

```typescript
// 写入模型配置到工具配置文件
invoke('apply_model_to_tool', {
    toolId: string,        // 如 "opencode"
    modelInfo: {
        id: string,        // 模型内部 ID
        name: string,      // 模型名称
        baseUrl: string,   // OpenAI 兼容 URL
        apiKey: string,    // API Key
        model: string,     // 模型 ID（如 "gpt-4"）
        protocol: string,  // "openai" | "anthropic"
        relayMode?: boolean,           // Codex 直连模式
        responsesPassthrough?: boolean, // Codex Responses 透传
        oneMContext?: boolean,          // Claude Code 1M 上下文
    }
})

// 启动工具进程
invoke('start_tool', {
    toolId: string,
    startCommand?: string,  // CLI 启动命令（默认从 paths.json 读取）
    cwd?: string,           // 工作目录（仅 CLI Code 类工具需要）
})
```

### 7.3 关键细节

1. **配置写入和启动是分开的两步**：先在 Rust 后端写入工具配置文件，然后才启动物理进程。工具启动时从自己的配置文件读取 API 信息。
2. **cwd 工作目录**：仅 `category === 'CLI Code'` 的工具会弹出文件夹选择器。桌面应用和 IDE 不需要 cwd。
3. **startCommand**：从前端传入，覆盖 `paths.json` 中定义的默认值。
4. **API Key 加密**：写入配置文件前会加密，存储在系统密钥链中。

---

## 八、各工具配置文件详细格式推断

由于 `custom: true` 工具的读写逻辑在私有 crate 中，以下格式基于各工具官方文档推断：

### 8.1 OpenCode (`~/.config/opencode/opencode.jsonc`)

```jsonc
{
    // model 字段：provider/model_name 格式
    "model": "anyversion/用户模型名",
    // provider 配置
    "provider": {
        "anyversion": {
            "options": {
                "apiKey": "sk-xxx",
                "baseURL": "https://api.example.com/v1"
            }
        }
    }
}
```

**注意**：EchoBird 使用固定 provider ID（类似于 `anyversion`），必须同时写 `model` 和 `provider.<id>.options`。

### 8.2 MiMo Code (`~/.config/mimocode/mimocode.json`)

```json
{
    "$schema": "https://mimo.xiaomi.com/config.json",
    "model": "anyversion/用户模型名",
    "small_model": "anyversion/用户模型名",
    "provider": {
        "anyversion": {
            "models": {
                "用户模型名": { "name": "用户模型名" }
            },
            "npm": "@ai-sdk/openai-compatible",
            "options": {
                "apiKey": "sk-xxx",
                "baseURL": "https://api.example.com/v1"
            }
        }
    }
}
```

### 8.3 Aider (`~/.aider.conf.yml`)

```yaml
model: openai/用户模型名
openai-api-base: https://api.example.com/v1
openai-api-key: sk-xxx
```

### 8.4 QwenCode (`~/.qwen/settings.json`)

```json
{
    "env": {
        "OPENAI_API_KEY": "sk-xxx",
        "OPENAI_API_BASE": "https://api.example.com/v1"
    },
    "model": "用户模型名"
}
```

### 8.5 Grok (`~/.grok/config.toml`)

```toml
[openai]
base_url = "https://api.example.com/v1"
api_key = "sk-xxx"
model = "用户模型名"
```

### 8.6 OpenClaw (`~/.openclaw/openclaw.json`)

```json
{
    "provider": {
        "openai": {
            "apiKey": "sk-xxx",
            "baseURL": "https://api.example.com/v1"
        }
    },
    "model": "openai/用户模型名"
}
```

### 8.7 Vibe-Trading (`~/.vibe-trading/.env`)

```env
OPENAI_API_KEY=sk-xxx
OPENAI_BASE_URL=https://api.example.com/v1
MODEL_NAME=用户模型名
```

---

## 九、与 any-version 当前实现的关键差异

| 维度 | EchoBird | any-version 当前 |
|------|----------|-----------------|
| **配置写入时机** | 启动前由 Rust 后端写入工具配置文件 | 启动时在 Rust 中写入（仅 OpenCode/MiMo） |
| **Claude Code** | 写 `~/.claude/settings.json` 的 `env` 字段 | 写 `~/.claude/settings.json` 的 `env` 字段 ✅ 基本正确 |
| **OpenCode** | 写 `~/.config/opencode/opencode.jsonc`（注意是 `.jsonc`） | 写 `~/.config/opencode/opencode.json`（缺少 `c`，路径可能不同） |
| **MiMo Code** | `custom: true`，私有 crate 处理 | 手动写 `~/.config/mimocode/mimocode.json` |
| **Codex** | 启动本地代理 127.0.0.1:53682，写 `~/.codex/config.toml` | **完全未处理** |
| **Aider** | 写 `~/.aider.conf.yml` | **完全未处理** |
| **QwenCode** | 写 `~/.qwen/settings.json` | **完全未处理** |
| **Grok** | 写 `~/.grok/config.toml` | **完全未处理** |
| **OpenClaw** | 写 `~/.openclaw/openclaw.json` | **完全未处理** |
| **Hermes** | 写 `~/.echobird/hermes.json` | **完全未处理** |
| **ZCode** | 写 `~/.zcode/v2/config.json` | **完全未处理** |
| **WorkBuddy** | 写 `~/.workbuddy/models.json` | **完全未处理** |
| **Vibe-Trading** | 写 `~/.vibe-trading/.env` | **完全未处理** |
| **桌面/IDE 工具** | 启动前无配置写入（因为应用自身处理） | **完全未处理** |
| **API Key 加密** | 写入配置文件前加密 | ❌ 明文写入 |
| **配置读写解耦** | 写入配置和启动工具分离（apply vs start） | 写入和启动混合在 `launch_ai_tool` 中 |

---

## 十、EchoBird 架构总结

```
┌─────────────────────────────────────────────────────┐
│                    前端 (React)                       │
│  AppManagerProvider.tsx                              │
│  ├─ applyModelConfig(toolId, model) → IPC            │
│  └─ startTool(toolId, cmd, cwd) → IPC                │
└─────────────────────┬───────────────────────────────┘
                      │ Tauri IPC
┌─────────────────────▼───────────────────────────────┐
│              私有 Crate: echobird_core                │
│                                                       │
│  apply_model_to_tool:                                 │
│  ├─ 读取 tools/<id>/config.json 的 write 映射         │
│  ├─ custom: false → 按声明式映射写入配置文件            │
│  │   ├─ env.XXX → 写入配置文件的 env 字段              │
│  │   └─ 直接路径 → 写入配置文件对应路径                 │
│  └─ custom: true → 调用内置的写入函数                   │
│       ├─ opencode_custom_write()                      │
│       ├─ mimocode_custom_write()                      │
│       ├─ codex_custom_write()                         │
│       └─ ...                                          │
│                                                       │
│  start_tool:                                          │
│  ├─ 读取 tools/<id>/paths.json 获取二进制路径          │
│  ├─ 检查各平台路径是否存在                              │
│  ├─ 对 claudecode: 额外设置环境变量                     │
│  ├─ 对 codex: 确保本地代理已运行                        │
│  └─ spawn 子进程                                       │
└─────────────────────────────────────────────────────┘
```

---

## 十一、关键教训

1. **EchoBird 的核心逻辑在私有 crate 中**，公开代码只告诉我们"做什么"而不是"怎么做"。我们必须自己实现等价逻辑。

2. **配置写入和进程启动是两个独立步骤**：先写配置文件，再启动工具。工具从自己配置文件读取 API 信息。

3. **不同工具写到完全不同的配置文件**：有 `.jsonc`、`.json`、`.toml`、`.yml`、`.env` 等多种格式，且路径各不相同。

4. **只有 Claude Code CLI 使用环境变量注入**：`ANTHROPIC_MODEL`、`ANTHROPIC_BASE_URL`、`ANTHROPIC_AUTH_TOKEN`。

5. **Codex 需要完整的协议翻译代理**：这不是简单的配置文件写入能解决的，需要实现 Responses ↔ Chat Completions 的协议转换服务。

6. **API Key 加密**：写入配置文件前需要加密，需要系统密钥链支持。

7. **custom: true 工具 = 需要定制化代码处理**：不能简单映射路径，需要理解各工具的配置文件格式并精确写入。

8. **OpenCode 的配置文件是 `.jsonc`，不是 `.json`**：这是一个容易忽略的差异。
