---
name: porting-from-reference-repos
description: Use when the user says "抄作业", "check reference repos", "port features", "copy homework", or asks to check what the reference repositories (EchoBird, cc-switch, CodexPlusPlus, open-tag, orca, headroom, farming) have updated and what can be ported to the current project. Also use when the user mentions specific reference repo names and wants to sync features.
---

# Porting from Reference Repos (抄作业)

## Overview

Systematically check reference repositories for recent updates, identify valuable features, and port them to the current project while avoiding duplication of existing functionality.

## Reference Repos

| Repo | Path | 核心能力 | 与 any-version 的关系 |
|------|------|---------|---------------------|
| **EchoBird** | `E:\pro\other-sdk\ai-tools\EchoBird` | **各 AI 工具的「模型管理」**：给每个工具写模型配置（配置文件路径 + 字段映射 + 自定义写入器）与「还原官方模型」；**不做协议转换代理**（那是 cc-switch/CodexPlusPlus 的参考点，也是 any-version 自己的代理） | **唯一借鉴点 = 各种工具的模型管理**：工具声明 + 写配置 + 官方还原（详见下方「EchoBird 模型管理移植」） |
| **cc-switch** | `E:\pro\other-sdk\ai-tools\cc-switch` | 提供 Claude Code 启动、管理。**支持协议对齐**、抹平第三方模型和 Claude Code 的差异 | 代理/协议转换能力是 any-version 的参考重点 |
| **CodexPlusPlus** | `E:\pro\other-sdk\ai-tools\CodexPlusPlus` | 提供 Codex 桌面端的启动、**协议对齐**，Codex 官方插件、技能的处理 | 插件/技能管理逻辑可参考 |
| **open-tag** | `E:\pro\other-sdk\ai-tools\open-tag` | 提供**多 agent 协同**，与 any-version 的 AI-协作功能非常相似 | 协作机制（多 agent 通信、任务分配）是重点参考对象 |
| **orca** | `E:\pro\other-sdk\ai-tools\orca` | **多 agent 协同编排器**（Orchestrator）：并行 worktree 运行多个 agent（Codex/ClaudeCode/OpenCode/Pi）、一 prompt 分发多 agent 比较结果、移动端监控。技术栈 Electron + TS/React（非 Tauri） | **多 agent 并行编排/任务分发/worktree 管理**机制是重点参考对象；架构不同（Electron），UI 可参考 |
| **headroom** | `E:\pro\other-sdk\ai-tools\headroom` | **LLM token 压缩层**（context compression）：压缩 AI agent 读取的所有内容（工具输出、日志、RAG、文件、对话历史）后发给 LLM，可减 60-95% token；提供 library（Python/TS `compress()`）、proxy、MCP server、`headroom wrap`、跨 agent 记忆、可逆压缩(CCR)。技术栈 Python + Rust（非 Tauri） | **发送给 LLM 前的内容智能压缩**是重点参考对象，可移植到 any-version 的 AI 对话/工具输出/历史记录场景；Rust 核心逻辑可参考 |
| **farming** | `E:\pro\other-sdk\ai-tools\farming` | **自托管多 agent 浏览器工作台**（Farming Code / Farming CRT）：在同一开发机上运行并监督多个 AI coding agent（Codex/Claude Code/Pi/OpenCode/Qoder/Qwen Code），浏览器或手机远程连接真实会话；支持结构化 Chat、Terminal、可恢复历史、项目文件浏览/审查、Agent 间共用项目浏览器、多 agent 仪表盘与使用量遥测。技术栈 Node.js + TS（前端 React/TSX + 后端 TS，非 Tauri） | **多 agent 会话监督/恢复历史/结构化 chat/远程监控/文件审查**机制是重点参考对象；桌面与移动端远程管理 agent 的架构与状态管理（TS 端）可参考，移植为 Tauri 时需自行实现 IPC |
| **ai-toolbox** | `E:\pro\other-sdk\ai-tools\ai-toolbox` | **个人 AI 工具箱**：一站式管理 AI 编程助手配置（Tauri + 前端，v1.1.x）。定位与 any-version 高度重合 | **同架构 + 同定位**，配置写入/多工具适配/UI 模式可直接对照，优先级高 |
| **claude-code-cli** | `E:\pro\other-sdk\ai-tools\claude-code-cli` | Claude Code CLI **源码学习与分析**项目（目录结构还原：cli/commands/context/coordinator 等） | 需要理解 Claude Code 内部行为（配置项、env、工具链）时可作逆向参考 |

> **路径说明**：所有参考仓库现统一位于 `E:\pro\other-sdk\ai-tools\<repo>`（旧路径 `E:\pro\other-sdk\<repo>` 已失效）。

**Target project (any-version):** `e:\pro\my\any-version` — AI Agent 桌面管理工具，Tauri + Rust + React (TS)。

## Workflow

> **⚠️ 核心规则（必须先确认再动手）**：发现可借鉴的内容后，**必须先在 Phase 4 向用户呈报方案并等待用户明确确认**，确认之后才开始实现/修改。任何情况下都**不要跳过确认直接改代码**。若没有可借鉴内容或用户未确认，则停止并等待指示。

### Phase 0: 查上次抄作业记录（必须先做）

**第一步：读 `sync-point.txt`（同目录）** —— 里面记着每个参考仓在「上次抄作业」时的 HEAD hash 与日期，是 Phase 1 的精确起点，也记着上次抄了什么、哪些还没抄。

**第二步：核对 any-version 侧的实际落地**（sync-point 可能滞后，以 git 为准）：

```bash
cd e:\pro\my\any-version
git log --oneline --all --grep="抄作业\|抄自\|移植自\|porting\|port\|sync\|EchoBird\|cc-switch\|CodexPlusPlus\|open-tag\|orca\|headroom\|farming\|ai-toolbox" --no-pager | cat
git log --oneline -30 --no-pager | cat
```

目的：
- 确认上次抄了什么、改动了哪些文件
- 避免重复抄已经抄过的功能
- 了解上次抄作业后是否有回滚或修改（例如 `Page` 模块先做后删、自动化任务体系先做后删）

### Phase 1: Discover (抄什么)

**以 `sync-point.txt` 里每个仓的 hash 为起点**逐仓 diff（比 `--since="2 weeks ago"` 精确：参考仓提交量差异极大，orca/CodexPlusPlus 两周可上千条）：

```bash
# 对每个参考仓库（<pin> = sync-point.txt 里该仓的 hash）
cd <repo-path>
git log --oneline <pin>..HEAD --no-pager | cat
git diff --stat <pin>..HEAD --no-pager | cat
```

提交量大的仓（orca、CodexPlusPlus、EchoBird）先按类型收敛，别硬读全量：

```bash
git log --no-merges --grep="^feat" --pretty="%h %ad %s" --date=short <pin>..HEAD | cat   # 只看功能
```

抄完当天**回写 `sync-point.txt`**：更新每个仓的 hash（`git -C <repo-path> rev-parse --short=8 HEAD`）、日期，追加本次「落地了什么 / 哪些没抄」，并把未落地项留在文件末尾作为下次的候选。

重点扫描方向：
- 新 UI 功能（React 组件、状态模式）
- 新后端命令（Rust `#[tauri::command]` 函数）
- 配置/设置变更
- package.json 依赖更新
- CLI 工具集成模式
- Bug 修复和稳定性改进
- **协议对齐/代理逻辑**（cc-switch、CodexPlusPlus 重点）
- **多 agent 协同机制**（open-tag 重点）
- **多 agent 并行编排/任务分发/worktree 管理**（orca 重点）
- **上下文/Token 压缩、工具输出与对话历史压缩**（headroom 重点）
- **多 agent 会话监督/恢复历史/结构化 chat/远程监控/文件审查**（farming 重点）

### Phase 2: Prioritize (抄什么优先级)

按以下顺序排序：
1. **直接可移植** — 同架构（Tauri+Rust+React）= 最高优先级
2. **用户可见价值** — 可见功能优先，后端优化其次
3. **低风险** — 隔离功能，不与现有代码深度耦合
4. **现有缺口** — any-version 缺失但参考仓库有的功能

### Phase 3: Audit (避免重复)

**关键步骤**：实现前必须审计 any-version 已有的功能：

```bash
cd e:\pro\my\any-version
rg -i "<feature-keyword>" src/ src-tauri/src/ --no-pager | cat
```

- 同时检查前端（`src/`）和后端（`src-tauri/src/`）
- 搜索相似的 Tauri 命令、React 组件、配置字段
- 跳过已存在的功能

### Phase 4: 方案呈现（让用户决定）

向用户报告发现，格式：

```
## 抄作业调查报告

### 上次抄作业记录
- <commit hash> <message> — 改了什么

### 参考仓库新提交

#### EchoBird
- <commit hash>: <message> — ✅有用 / ❌无用 / ⚠️待定
- ...

#### cc-switch
- <commit hash>: <message> — ✅有用 / ❌无用 / ⚠️待定
- ...

#### CodexPlusPlus
- <commit hash>: <message> — ✅有用 / ❌无用 / ⚠️待定
- ...

#### open-tag
- <commit hash>: <message> — ✅有用 / ❌无用 / ⚠️待定
- ...

#### orca
- <commit hash>: <message> — ✅有用 / ❌无用 / ⚠️待定
- ...

#### headroom
- <commit hash>: <message> — ✅有用 / ❌无用 / ⚠️待定
- ...

#### farming
- <commit hash>: <message> — ✅有用 / ❌无用 / ⚠️待定
- ...

#### ai-toolbox
- <commit hash>: <message> — ✅有用 / ❌无用 / ⚠️待定
- ...

#### claude-code-cli
- <commit hash>: <message> — ✅有用 / ❌无用 / ⚠️待定
- ...

### 建议抄的内容
1. <功能名>（来源：<repo>）— 理由 + 整合方案
2. ...

### 不建议抄的内容
- <功能名>（来源：<repo>）— 理由（已有/不适用/风险高）

请确认要抄哪些？
```

**等待用户确认后再开始实现。**

### Phase 5: Implement (开抄)

对每个要抄的功能，按以下顺序：

1. **Rust 后端优先** — 添加新 structs、commands、config fields
2. **React 前端其次** — 添加组件、状态、UI
3. **串联** — 更新 `lib.rs` 命令注册，确保 `invoke()` 调用匹配

关键移植模式：
- EchoBird 的 Tauri 命令 → 直接移植，适配命名到 any-version 约定
- cc-switch 的 React 组件 → 移植并适配样式
- 两者都用 TypeScript strict mode — 保持类型安全
- orca 是 **Electron**（非 Tauri），其 `#[tauri::command]` 不存在，参考重点是**多 agent 并行编排/任务分发/worktree 管理/移动端监控**的架构与状态机逻辑（TS 端），移植为 Rust 命令时需自行实现 IPC 与并发控制
- headroom 是 **Python + Rust**（非 Tauri），参考重点是**压缩算法/CCR 可逆压缩/内容感知压缩器**（Rust crates 可直接借鉴，Python 侧转译为 Rust 或按需调用）；压缩发生在发送给 LLM 之前，适合集成进 any-version 的 AI 消息发送链路
- farming 是 **Node.js + TS**（非 Tauri），参考重点是**多 agent 会话监督/可恢复历史/结构化 chat/远程浏览器监控/文件审查**的架构与会话状态管理（TS 端），移植为 Rust 命令时需自行实现 IPC 与并发控制
- ai-toolbox 是 **Tauri + 前端**（同架构）→ 可整块对照移植；free-router-proxy 是 **Node 网关**（非 Tauri），参考重点是**多供应商候选排序、免费模型探测、故障转移与冷却**的算法与状态机，需转写为 Rust 并接入 any-version 代理的上游选择环节

### Phase 6: Verify (检查)

```bash
cd e:\pro\my\any-version
# Rust 编译
cd src-tauri && cargo check 2>&1 | cat
# TypeScript 编译
cd .. && npx tsc --noEmit 2>&1 | cat
```

两者必须**零错误**才算完成。

## Implementation Patterns

### Porting a Tauri Command (Rust)

```rust
// Reference: EchoBird or cc-switch command
// 1. Copy the #[tauri::command] fn and its structs
// 2. Adapt field names to any-version conventions
// 3. Register in lib.rs: .invoke_handler(tauri::generate_handler![..., new_command])
```

### Porting a React Component (TSX)

```tsx
// Reference: EchoBird or cc-switch component
// 1. Copy component structure and state logic
// 2. Adapt imports to any-version paths
// 3. Adapt styling (Tailwind classes or CSS modules)
// 4. Wire up invoke() calls to match Rust command names
```

### Common Pitfalls

| Pitfall | Fix |
|---------|-----|
| 抄了已有的功能 | Phase 3 审计优先 |
| Tauri 命令签名不匹配 | 检查 Rust fn 名与前端 `invoke()` 字符串一致 |
| 前端缺少类型定义 | 复制组件时一起复制 type interfaces |
| Cargo.toml 缺少新依赖 | 检查 reference 的 Cargo.toml 所需 crates |
| 硬编码环境路径 | 替换为相对路径或配置驱动 |

## Quick Reference

```bash
# 完整发现周期（幂等：每个仓都从 sync-point.txt 的 hash 开始）
cd e:\pro\my\any-version
python - <<'PY'
import re, subprocess
root = r"E:\pro\other-sdk\ai-tools"
for line in open(r".agents\skills\porting-from-reference-repos\sync-point.txt", encoding="utf-8"):
    # 只认「仓名 hash 日期」三段（学习记录里的 “- <hash> ...” 不应被当成同步点）
    m = re.match(r"^([A-Za-z0-9][A-Za-z0-9_.-]*)\s+([0-9a-f]{6,40})\s+\d{4}-\d{2}-\d{2}\b", line)
    if not m:
        continue
    repo, pin = m.group(1), m.group(2)
    print(f"=== {repo} ({pin}..HEAD) ===")
    print(subprocess.run(["git", "-C", f"{root}\\{repo}", "log", "--oneline", "--no-merges",
                          f"{pin}..HEAD"], capture_output=True, text=True).stdout.rstrip() or "(无变化)")
PY
```

## EchoBird 模型管理移植（专项）

> 对 EchoBird 我们**只借鉴一件事**：它管理各 AI 工具「模型配置」的方式。协议转换、账号、UI 等
> 一律不抄（协议转换是我们的代理自研，参考点其实是 cc-switch / CodexPlusPlus）。

### 两边对应关系

| EchoBird | any-version | 说明 |
|----------|-------------|------|
| `tools/<id>/config.json`（`configFile` + `format` + `custom`） | `ai-tools/<id>/config.json`（`configFile: { path, format, write, custom, pathEnvDirs, xdgSubdir, preferExistingExtensions }`） | 工具声明；我们多一层 `write` 映射（「配置路径 → 值模板」），EchoBird 是 per-tool Rust 模块 |
| `src-tauri/src/services/tool_config_manager/<id>.rs`（per-tool 写入） | 通用写入器 `commands/ai/launch.rs::write_tool_config_generic` | 我们用声明式 `write` 表达「点号路径 → 标量」；schema 复杂的（WorkBuddy 的 `models.json`）才走自定义写入器 |
| `src-tauri/src/services/tool_config_manager/<id>.rs`（`custom: true` 的工具，如 workbuddy / claudedesktop） | `commands/ai/tool_config_custom.rs` | 自定义写入器：`write_config` / `read_model` 分派 |
| `tool_config_manager.rs::restore_tool_to_official` + 各 `restore_*` | `commands/ai/tool_config_restore.rs::restore_tool_config` | 还原官方模型：删掉自己写的 provider/键；无专属实现走「删整个文件」兜底 |

### 移植规则（用户定的）

1. **只管理 EchoBird 有模型配置方式的工具**：EchoBird 的 `tools/<id>` 里没有 configFile、也没有对应
   per-tool 写入模块 → 该工具不支持配置模型 → **不纳入 any-version 管理**（直接删 `ai-tools/<id>/`）。
2. **逐字段对齐**：EchoBird 写哪个文件/哪些字段，我们就写哪个文件/哪些字段（provider 名用 `anyversion` 而不是
   `echobird`）；EchoBird 刻意不写的（如 mimodesktop 的顶层 `model`，因为它会连带改掉共享该文件的 CLI 默认模型）我们也别写。
3. **官方模型还原语义**：删「自己写的键」而不是整份覆盖 —— 按声明里的 `write` 映射反推要删哪些；
   接管像 `~/.codex/auth.json` 这种本来就有用户凭据的文件前先备份（`~/.any-version/config-backups/<tool>/`），
   还原时先恢复备份。自定义写入器的还原：WorkBuddy 删 `models.json`；Claude Desktop 把 `deploymentMode` 由 `3p` 切回 `1p` 并删 profile。

### 移植时易踩的坑（已踩过，记录在此）

- 注册表加载对声明**缺字段即整体丢弃**：`ToolConfig` 里所有「可有可无」字段都要 `#[serde(default)]`，
  否则少写一个 `cacheDirs` 会让整个工具从列表里消失（只留 stderr 一行 parse 失败）。
- 运行时读的是 `src-tauri/_up_/ai-tools` 副本：`build.rs::sync_dir` 必须「加 + 删」双同步，否则从源目录删工具不生效。
- 写 JSONC（带注释）配置文件时先用 `strip_jsonc` 解析，否则 `serde_json::from_str` 失败会当空文档**整份覆盖**。
- 工具声明里的 `model` 值要带 `modelFormat.prefix`（如 `anyversion/`），否则 opencode 系工具选不中 provider。

## cc-switch 代理能力移植（专项）

> cc-switch 的借鉴点是**代理 / 协议整流**。它演进很快，会**删除和收窄**功能，
> 所以抄之前必须确认「现在还有没有、语义变没变」——照着旧截图或旧印象抄会抄错。

### 去哪看（模块地图）

| 路径 | 看什么 |
|------|--------|
| `src-tauri/src/proxy/types.rs` | `RectifierConfig` / `OptimizerConfig` / `CopilotOptimizerConfig` / `AppProxyConfig` / `LogConfig` —— **判断一个能力是否存在的唯一权威** |
| `src-tauri/src/proxy/{cache_injector,thinking_optimizer,thinking_rectifier,thinking_budget_rectifier,media_sanitizer,tool_media,copilot_optimizer,model_mapper,reasoning_bridge}.rs` | 一个关注点一个文件的整流/优化实现 |
| `src-tauri/src/proxy/providers/transform*.rs`、`streaming*.rs` | 按端点/协议的转换层（codex responses、codex chat、gemini、moonshot schema…） |
| `src-tauri/src/proxy/forwarder.rs` | 主流程：预防式改写 → 发送 → 报错后反应式整流重试 |
| `src/i18n/locales/zh.json` | 开关文案。**可能有遗留 key**（如 `requestGroup/responseGroup` 还在，但配置里已无响应整流字段）→ 别拿它判断功能是否存在 |

### ⚠️ 抄之前必做：确认能力仍在、语义未变

已确认的三次变化（2026-09-26 核对）：

1. **「协议不匹配修复」（未知字段剥离重试）已删除** —— `RectifierConfig` 只剩 4 项。
2. **「请求优化器」改名「Bedrock 请求优化器」并收窄**：只在 `CLAUDE_CODE_USE_BEDROCK=1` 时生效（原来通用）。
3. **「DeepSeek 兼容」不再是用户开关**：下沉为按端点的内建转换（如 Moonshot/Kimi 工具 schema 的 `$ref` 兄弟键包进 `allOf`，Issue #6867）。

核对顺序：**先看 `proxy/types.rs` 的配置结构（有什么开关）→ 再 rg 模块看实现 → 最后才看 i18n 文案**。

### 能力对照表（2026-09-26）

| 能力 | cc-switch | any-version | 结论 |
|------|-----------|-------------|------|
| Prompt 缓存注入 | **仅 Bedrock** | 所有 anthropic 出站（`optimizers.rs::inject_cache_breakpoints`） | 我们更广 |
| Thinking 参数自适应 | **仅 Bedrock** | anthropic / openai / google 三形态 | 我们更广 |
| DeepSeek / Moonshot 规范化 | 内建（按端点） | 用户开关 `deepseek_normalize`（deepseek/moonshot/kimi/mimo） | 粒度不同 |
| Thinking 签名整流 | 报错后剥离 + 重试一次 | 预防式剥离 + **报错后重试一次** | 已对齐 |
| budget_tokens 修正 32000 | 有（+ max_tokens 64000） | 有（同值） | 一致 |
| 图片降级 | 两条路径：声明纯文本 + 报错兜底 | 同样两条（`media_fallback` + `media_heuristic`） | 一致 |
| 纯文本模型预判（注册表） | `request_media_heuristic` | `TEXT_ONLY_MODEL_PREFIXES` + `is_text_only_model` | 已抄 |
| 协议残留字段剥离 | **已删除** | 有（`protocol_mismatch`） | 我们多一项 |
| Copilot 优化器（x-initiator 分类 / 孤立 tool_result 清理 / 合并 / compact 识别） | 有（Issue #1813） | **没有** | 可抄 |
| 每 app 自动故障转移 + max_retries + 流式首字超时 | 有 | **没有** | 可抄 |
| 模型能力注册表（`model_capabilities.rs`）驱动图片/参数决策 | 有 | **没有** | 可抄（配合图片预判） |

### 移植规则

1. **能力是协议层的，与工具形态无关**：只要工具走本地代理（写进它配置的是 `127.0.0.1`），就该能用 →
   `ai-tools/<id>/config.json` 的 `supportsOptimizer` / `supportsRectifier` 必须对 `supportModel=true` 的工具打开
   （`ai_registry.rs::model_capable_tools_allow_optimizer_and_rectifier` 守着这条不变量）。
2. **开关链路一次改全**：`proxy/types.rs::ProxyConfig` → `commands/ai/models.rs::RectifierConfig`（全局默认）→
   `LaunchAiToolRequest` / `DispatchOptions`（按次覆盖）→ `launch.rs` / `provider.rs` / `collab.rs` 的映射 →
   前端 `AiConfig` + `ToolLauncher` 复选框 + i18n **zh 与 en 都要加**（有 parity 测试）。
   别忘了前端还有兜底的默认 AiConfig 字面量（`ModelConfig.tsx`、`ToolLauncher.tsx`），漏改会 tsc 报错。
3. **预防式与反应式两条路都要考虑**：预防式省一次往返，反应式兜未知端点差异；
   反应式必须**只在确认能修这个错误时才认领**（剥不出东西就别重试，把错误让给后面的分支）。

### 易踩的坑（已踩）

- **别用 i18n 判断功能是否存在**（有遗留 key）。
- **别照旧截图抄**（会抄到已删除 / 已收窄的能力）。
- **单测里不要构造 `ProxyConfig`**：它带 `#[serde(skip)] Option<tauri::AppHandle>`，一构造就把 tauri 运行时
  链接进测试二进制，Windows 下测试进程直接 `0xC0000139 STATUS_ENTRYPOINT_NOT_FOUND` 起不来（编译是通过的）。
  要测就把逻辑抽成不依赖 ProxyConfig 的纯函数（如 `strip_images_for_text_only_model`）。
- 抄「按端点特例」（Moonshot/Kimi 之类）时，**只在命中的端点上改写**，否则会破坏其它供应商的 prompt cache 前缀。

## When NOT to Use

- 任务是完全在 any-version 内部的 bug 修复或功能开发（无需参考）
- 用户明确要求原创实现
- 参考仓库没有相关新变更
