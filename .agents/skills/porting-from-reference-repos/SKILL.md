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
| **EchoBird** | `E:\pro\other-sdk\ai-tools\EchoBird` | 提供各 agent CLI 终端的启动，启动时可修改模型，**但不支持代理**（如将 OpenAI 协议转为 Anthropic 协议） | 架构相似（Tauri+Rust+React），UI/启动逻辑可参考 |
| **cc-switch** | `E:\pro\other-sdk\ai-tools\cc-switch` | 提供 Claude Code 启动、管理。**支持协议对齐**、抹平第三方模型和 Claude Code 的差异 | 代理/协议转换能力是 any-version 的参考重点 |
| **CodexPlusPlus** | `E:\pro\other-sdk\ai-tools\CodexPlusPlus` | 提供 Codex 桌面端的启动、**协议对齐**，Codex 官方插件、技能的处理 | 插件/技能管理逻辑可参考 |
| **open-tag** | `E:\pro\other-sdk\ai-tools\open-tag` | 提供**多 agent 协同**，与 any-version 的 AI-协作功能非常相似 | 协作机制（多 agent 通信、任务分配）是重点参考对象 |
| **orca** | `E:\pro\other-sdk\ai-tools\orca` | **多 agent 协同编排器**（Orchestrator）：并行 worktree 运行多个 agent（Codex/ClaudeCode/OpenCode/Pi）、一 prompt 分发多 agent 比较结果、移动端监控。技术栈 Electron + TS/React（非 Tauri） | **多 agent 并行编排/任务分发/worktree 管理**机制是重点参考对象；架构不同（Electron），UI 可参考 |
| **headroom** | `E:\pro\other-sdk\ai-tools\headroom` | **LLM token 压缩层**（context compression）：压缩 AI agent 读取的所有内容（工具输出、日志、RAG、文件、对话历史）后发给 LLM，可减 60-95% token；提供 library（Python/TS `compress()`）、proxy、MCP server、`headroom wrap`、跨 agent 记忆、可逆压缩(CCR)。技术栈 Python + Rust（非 Tauri） | **发送给 LLM 前的内容智能压缩**是重点参考对象，可移植到 any-version 的 AI 对话/工具输出/历史记录场景；Rust 核心逻辑可参考 |
| **farming** | `E:\pro\other-sdk\ai-tools\farming` | **自托管多 agent 浏览器工作台**（Farming Code / Farming CRT）：在同一开发机上运行并监督多个 AI coding agent（Codex/Claude Code/Pi/OpenCode/Qoder/Qwen Code），浏览器或手机远程连接真实会话；支持结构化 Chat、Terminal、可恢复历史、项目文件浏览/审查、Agent 间共用项目浏览器、多 agent 仪表盘与使用量遥测。技术栈 Node.js + TS（前端 React/TSX + 后端 TS，非 Tauri） | **多 agent 会话监督/恢复历史/结构化 chat/远程监控/文件审查**机制是重点参考对象；桌面与移动端远程管理 agent 的架构与状态管理（TS 端）可参考，移植为 Tauri 时需自行实现 IPC |
| **ai-toolbox** | `E:\pro\other-sdk\ai-tools\ai-toolbox` | **个人 AI 工具箱**：一站式管理 AI 编程助手配置（Tauri + 前端，v1.1.x）。定位与 any-version 高度重合 | **同架构 + 同定位**，配置写入/多工具适配/UI 模式可直接对照，优先级高 |
| **free-router-proxy** | `E:\pro\other-sdk\ai-tools\free-router-proxy` | **本地 OpenAI 兼容网关**（Node，v1.1.0）：跨可插拔供应商**排序当前免费模型**，以虚拟模型 `free-best` 暴露；单个供应商限流/宕机/空返回时**自动故障转移**；缺 key 的供应商直接剔除 | **多供应商候选排序 + 故障转移**是 any-version 代理的参考重点（与 EchoBird 有序路由同源问题） |
| **claude-code-cli** | `E:\pro\other-sdk\ai-tools\claude-code-cli` | Claude Code CLI **源码学习与分析**项目（目录结构还原：cli/commands/context/coordinator 等） | 需要理解 Claude Code 内部行为（配置项、env、工具链）时可作逆向参考 |
| **page-agent** | `E:\pro\other-sdk\ai-tools\page-agent` | **浏览器内 AI agent**（阿里，TS monorepo + Chrome 扩展，v1.12.x）：在页面里用自然语言驱动浏览器操作 | 与 any-version 定位较远，暂列备查 |

> **路径说明**：所有参考仓库现统一位于 `E:\pro\other-sdk\ai-tools\<repo>`（旧路径 `E:\pro\other-sdk\<repo>` 已失效）。

**Target project (any-version):** `e:\pro\my\any-version` — AI Agent 桌面管理工具，Tauri + Rust + React (TS)。

## Workflow

> **⚠️ 核心规则（必须先确认再动手）**：发现可借鉴的内容后，**必须先在 Phase 4 向用户呈报方案并等待用户明确确认**，确认之后才开始实现/修改。任何情况下都**不要跳过确认直接改代码**。若没有可借鉴内容或用户未确认，则停止并等待指示。

### Phase 0: 查上次抄作业记录（必须先做）

在读取参考仓库之前，**先调查 any-version 仓库中上次"抄作业"相关的 git 提交记录**：

```bash
cd e:\pro\my\any-version
git log --oneline --all --grep="抄作业\|抄自\|移植自\|porting\|port\|sync\|EchoBird\|cc-switch\|CodexPlusPlus\|open-tag\|orca\|headroom\|farming\|ai-toolbox\|free-router-proxy" --no-pager | cat
git log --oneline -30 --no-pager | cat
```

目的：
- 确认上次抄了什么、改动了哪些文件
- 避免重复抄已经抄过的功能
- 了解上次抄作业后是否有回滚或修改

### Phase 1: Discover (抄什么)

检查每个参考仓库的近期提交（从上次抄作业之后开始）：

```bash
# 对每个参考仓库
cd <repo-path>
git log --oneline --since="2 weeks ago" --no-pager | cat
git diff --stat HEAD~10..HEAD --no-pager | cat
```

如果 Phase 0 发现了上次抄作业的 commit hash，则用该 hash 作为起点：

```bash
cd <repo-path>
git log --oneline <last-ported-commit>..HEAD --no-pager | cat
```

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

#### free-router-proxy
- <commit hash>: <message> — ✅有用 / ❌无用 / ⚠️待定
- ...

#### claude-code-cli
- <commit hash>: <message> — ✅有用 / ❌无用 / ⚠️待定
- ...

#### page-agent
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
# 完整发现周期（从 any-version 根目录运行）
for repo in EchoBird cc-switch CodexPlusPlus open-tag orca headroom farming ai-toolbox free-router-proxy claude-code-cli page-agent; do
  echo "=== $repo ==="
  cd "E:\pro\other-sdk\ai-tools\$repo" && git log --oneline -20 | cat
done
cd e:\pro\my\any-version
```

## When NOT to Use

- 任务是完全在 any-version 内部的 bug 修复或功能开发（无需参考）
- 用户明确要求原创实现
- 参考仓库没有相关新变更
