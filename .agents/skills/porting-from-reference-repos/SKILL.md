---
name: porting-from-reference-repos
description: Use when the user says "抄作业", "check reference repos", "port features", "copy homework", or asks to check what the reference repositories (EchoBird, cc-switch, CodexPlusPlus, open-tag, orca) have updated and what can be ported to the current project. Also use when the user mentions specific reference repo names and wants to sync features for AI module: tool install/manage/model-set/launch, protocol alignment (Claude Code / Codex / ChatGPT Desktop), and multi-agent collaboration.
---

# Porting from Reference Repos (抄作业)

## Overview

只围绕 any-version **AI 模块**的五个能力面抄作业：

| 能力面 | 学习目标 | 参考仓 |
|--------|---------|--------|
| 工具安装（CLI + 桌面端） | 怎么装、装哪些（**只关注能改模型的工具**） | EchoBird |
| 设置模型 + 启动 | 怎么写工具配置、怎么拉起 | EchoBird |
| Claude Code 协议兼容 | 抹平第三方模型与 Claude Code 的差异 | cc-switch |
| Codex / ChatGPT Desktop 协议对齐 | Responses / Chat Completions / 桌面端差异整流 | CodexPlusPlus |
| 多 agent 协同 | 并行编排、任务分发、通信 | open-tag / orca |

## Reference Repos（范围已收窄）

| Repo | Path | 只学这一件事 |
|------|------|-------------|
| **EchoBird** | `E:\pro\other-sdk\ai-tools\EchoBird` | **工具安装 + 模型设置 + 启动**：哪些 CLI/桌面端能装、装的命令与来源、配置怎么写（声明 + 写入器 + 官方还原）、怎么拉起 |
| **cc-switch** | `E:\pro\other-sdk\ai-tools\cc-switch` | **Claude Code 协议对齐**：整流/优化器/按端点转换/故障转移 |
| **CodexPlusPlus** | `E:\pro\other-sdk\ai-tools\CodexPlusPlus` | **Codex + ChatGPT Desktop 协议对齐** |
| **open-tag** | `E:\pro\other-sdk\ai-tools\open-tag` | **多 agent 协同**（通信/任务分配） |
| **orca** | `E:\pro\other-sdk\ai-tools\orca` | **多 agent 并行编排**（worktree、一 prompt 多 agent、任务分发） |

> **路径说明**：所有参考仓库位于 `E:\pro\other-sdk\ai-tools\<repo>`。

**已移出跟踪范围**（不再在本技能里评估）：`headroom`（token 压缩，已作为托管服务集成只跟版本）、
`farming`、`ai-toolbox`、`claude-code-cli`。若将来要重新纳入，先跟用户确认再追加回本节。

**Target project (any-version):** `e:\pro\my\any-version` — AI Agent 桌面管理工具，Tauri + Rust + React (TS)。

## Workflow

> **⚠️ 核心规则（必须先确认再动手）**：发现可借鉴的内容后，**必须先在 Phase 4 向用户呈报方案并等待用户明确确认**，确认之后才开始实现/修改。任何情况下都**不要跳过确认直接改代码**。

### Phase 0: 查上次抄作业记录（必须先做）

读同目录 `sync-point.txt`：每个仓上次抄作业时的 HEAD hash 与日期，以及「落地了什么 / 哪些没抄 / 哪些经核实不适用」。

核对 any-version 侧实际落地（sync-point 可能滞后，以 git 为准）：

```bash
cd e:\pro\my\any-version
git log --oneline --all --grep="抄作业\|抄自\|移植自\|porting\|port\|sync\|EchoBird\|cc-switch\|CodexPlusPlus\|open-tag\|orca" --no-pager | cat
git log --oneline -30 --no-pager | cat
```

### Phase 1: Discover (抄什么)

以 `sync-point.txt` 里每个仓的 hash 为起点逐仓 diff：

```bash
cd <repo-path>
git log --oneline <pin>..HEAD --no-pager | cat
git diff --stat <pin>..HEAD --no-pager | cat
```

按学习目标收敛（别硬读全量）：

```bash
# 学习目标 1：安装 / 模型设置 / 启动（EchoBird）
git log --no-merges --grep="install\|config\|model\|launch\|restore" --pretty="%h %ad %s" --date=short <pin>..HEAD | cat
# 学习目标 2/3：协议对齐（cc-switch / CodexPlusPlus）
git log --no-merges --grep="proxy\|rectif\|transform\|compat\|protocol\|responses\|streaming" --pretty="%h %ad %s" --date=short <pin>..HEAD | cat
# 学习目标 4：多 agent 协同（open-tag / orca）
git log --no-merges --grep="agent\|orchestr\|worktree\|dispatch\|collab\|session" --pretty="%h %ad %s" --date=short <pin>..HEAD | cat
```

抄完当天**回写 `sync-point.txt`**：更新 hash / 日期，追加「落地了什么 / 哪些没抄 / 哪些经代码核实不适用」。

### Phase 2: Prioritize

1. **直接可移植** — 同架构（Tauri+Rust+React）优先
2. **用户可见价值** — 可见功能优先
3. **低风险** — 隔离功能优先
4. **现有缺口** — any-version 缺但参考仓有的

### Phase 3: Audit (避免重复)

```bash
cd e:\pro\my\any-version
rg -i "<feature-keyword>" src/ src-tauri/src/ --no-pager | cat
```

同时检查前端 `src/` 与后端 `src-tauri/src/`；跳过已有功能。

### Phase 4: 方案呈现（让用户决定）

```
## 抄作业调查报告

### 上次抄作业记录
- <commit hash> <message> — 改了什么

### 参考仓库新提交（按学习目标分组）

#### EchoBird — 安装 / 模型设置 / 启动
- <hash>: <message> — ✅有用 / ❌无用 / ⚠️待定

#### cc-switch — Claude Code 协议对齐
- <hash>: <message> — ✅有用 / ❌无用 / ⚠️待定

#### CodexPlusPlus — Codex / ChatGPT Desktop 协议对齐
- <hash>: <message> — ✅有用 / ❌无用 / ⚠️待定

#### open-tag / orca — 多 agent 协同
- <hash>: <message> — ✅有用 / ❌无用 / ⚠️待定

### 建议抄的内容
1. <功能名>（来源：<repo>，服务哪个学习目标）— 理由 + 整合方案

### 不建议抄的内容
- <功能名>（来源：<repo>）— 理由

请确认要抄哪些？
```

**等待用户确认后再开始实现。**

### Phase 5: Implement

顺序：Rust 后端 → React 前端 → 串联（`lib.rs` 命令注册 + `invoke()` 匹配）。

### Phase 6: Verify

```bash
cd e:\pro\my\any-version
cd src-tauri && cargo check --all-targets 2>&1 | cat
cd .. && npx tsc --noEmit 2>&1 | cat
```

两者**零错误**才算完成。

## 学习目标 1：工具安装 + 模型设置 + 启动（EchoBird）

> **只关注可以修改模型的工具**。EchoBird 的 `tools/<id>` 里没有 configFile、也没有对应 per-tool
> 写入模块 → 该工具不支持配置模型 → **不纳入 any-version 管理**（直接删 `ai-tools/<id>/`）。

### 去哪看（模块地图）

| 路径 | 看什么 |
|------|--------|
| `src-tauri/tools/<id>/config.json` | 工具声明：安装来源、`configFile`（path/format/write/custom） |
| `src-tauri/src/services/installer*` / `src-tauri/src/commands/*install*` | 安装命令与包管理器选择（npm/winget/brew/下载） |
| `src-tauri/src/services/tool_config_manager/<id>.rs` | per-tool 配置写入：写哪个文件、哪些字段 |
| `src-tauri/src/services/tool_config_manager.rs::restore_tool_to_official` | 官方模型还原语义 |
| **`src-tauri/src/services/process_manager.rs::start_tool`** | **启动优先级链**（`:68` 起）：Codex 原生 → 前端显式命令 → `startCommand` → **MSIX `shell:AppsFolder`（仅当磁盘上无 exe，`:178-205`）** → GUI exe → VS Code 扩展 → `command` |
| `src-tauri/src/services/codex_proxy/codex_binary.rs` | Store 应用的 AUMID 解析：`resolve_desktop_binary` / `resolve_desktop_launch_uri_scanned`（`:173`）/ `find_codex_store_family_via_appx`（`:196`） |
| 前端安装/模型设置面板 | 安装态、模型选择、启动按钮的交互与状态机 |

### 启动优先级（必须照抄的顺序）

桌面端启动有个「**AUMID 会假成功**」的陷阱，顺序错了就会中招：

1. **磁盘上的 exe 优先于 Store AUMID**。EchoBird `process_manager.rs:184-191` 的注释是实测结论：
   winget 装的 Claude Desktop 是 Squirrel 构建（在 `%LOCALAPPDATA%\AnthropicClaude\Claude.exe`），
   它的 AUMID `shell:AppsFolder\Claude_pzs8sxrjxfjjc!Claude` **并不解析** —— explorer.exe 会弹出一个
   文件夹窗口而不是应用。真正的 Store 装在声明路径上没有 exe，所以「找得到 exe → 用 exe」永远安全。
2. **AUMID 不能硬编码 publisher hash**。`paths.json` 里写死的 `OpenAI.Codex_2p2nqsd0c76g0` 只对
   stable 渠道有效；beta 渠道是 `OpenAI.CodexBeta_<别的 hash>`，用错了 AUMID 就拉不起来。
   正确做法（抄 `codex_binary.rs:173`）：**先 `Get-AppxPackage` 拿真实 PackageFamilyName**
   （stable 优先、beta 兜底，与 publisher hash 无关），拼 `shell:AppsFolder\<PFN>!<应用 Id>`，
   扫不到才回退 `paths.json`。数据源优先级：`Get-AppxPackage`（权威）> 扫 `%LOCALAPPDATA%\Packages`
   （兜底）。这个做法 EchoBird 注释里点名是跟 CodexPlusPlus 学的。
3. **拉起用 `explorer.exe <uri>`**（`start_shell_uri`，`:839`），不是 `cmd /c start`。
4. **只有 Windows 能走 AUMID**：`#[cfg(windows)]`，否则会遮蔽 macOS 的 `.app` 与 Linux 路径。
5. **一次 PowerShell 查完所有候选**：stable/beta 用 `if / elseif` 串在一条脚本里，别起两次进程。
6. **桌面应用直接 spawn exe，不套终端包装**：`cmd /c start cmd /k <exe>` 会留下一个黑色控制台
   窗口，GUI 应用完全用不上（EchoBird 的 Priority 3 是 `spawn_gui` 直起）。程序路径可能带空格，
   用 `Command::new(整条路径)`，不能 `split_whitespace` 拆词。
7. **启动入口不能沉在页面最底部**：桌面工具不需要「项目目录 / 终端」这些 CLI 区块，若把启动
   按钮放在面板末尾，用户滚到「安装路径 → 未检测到可执行文件」就以为没有启动入口了
   （2026-09-28 用户实际反馈）。桌面工具应在安装路径区后单独给一条启动按钮，
   并隐藏 CLI 专属区块。
8. **「未检测到可执行文件」对 Store 应用是误导**：它本来就没有常规 exe 安装路径。
   这类工具要把文案换成「启动走系统包注册，无需可执行文件路径」。

### 两边对应关系

| EchoBird | any-version | 说明 |
|----------|-------------|------|
| `tools/<id>/config.json`（`configFile` + `format` + `custom`） | `ai-tools/<id>/config.json`（`configFile: { path, format, write, custom, pathEnvDirs, xdgSubdir, preferExistingExtensions }`） | 工具声明；我们多一层 `write` 映射（点号路径 → 值模板） |
| `services/tool_config_manager/<id>.rs` | `commands/ai/launch.rs::write_tool_config_generic` | 声明式 `write`；schema 复杂的（WorkBuddy `models.json`）走自定义写入器 |
| 同上 `custom: true` 的工具 | `commands/ai/tool_config_custom.rs` | `write_config` / `read_model` 分派 |
| `restore_tool_to_official` + 各 `restore_*` | `commands/ai/tool_config_restore.rs::restore_tool_config` | 还原官方模型；无专属实现走「删整个文件」兜底 |

### 移植规则

1. **只管 EchoBird 有模型配置方式的工具**（见上）。
2. **逐字段对齐**：EchoBird 写哪个文件/字段我们就写哪个；EchoBird 刻意不写的（如 mimodesktop 顶层 `model`，
   会连带改掉共享该文件的 CLI 默认模型）我们也别写。provider 名用 `anyversion` 而不是 `echobird`。
3. **官方模型还原语义**：删「自己写的键」而不是整份覆盖 —— 按声明 `write` 反推；
   接管 `~/.codex/auth.json` 这类凭据文件前先备份（`~/.any-version/config-backups/<tool>/`），还原时先恢复备份。

### 易踩的坑（已踩）

- 注册表加载对声明**缺字段即整体丢弃**：`ToolConfig` 里所有「可有可无」字段都要 `#[serde(default)]`，
  否则少写一个 `cacheDirs` 会让整个工具从列表里消失（只留 stderr 一行 parse 失败）。
- 运行时读的是 `src-tauri/_up_/ai-tools` 副本：`build.rs::sync_dir` 必须「加 + 删」双同步。
- 写 JSONC（带注释）配置文件时先用 `strip_jsonc` 解析，否则解析失败会当空文档**整份覆盖**。
- 工具声明里的 `model` 值要带 `modelFormat.prefix`（如 `anyversion/`），否则 opencode 系工具选不中 provider。
- **启动 exe 解析必须带 Store 回退**（我方 2026-09-28 才补，EchoBird 早就有）：`launch_uri` 此前只在
  detect（策略 4）与缓存失效里用，启动完全不看它 → Store/MSIX 版桌面端明明检测到已安装，
  点启动却报「没有可用的启动命令」。回退必须放在「exe 找不到」之后，不能放在之前。
- **不抄** EchoBird 的「本地 anthropic 常驻代理 bridge 模式」：我们没有常驻代理，只走「直写本次
  base_url/api_key」的 relay 语义。
- **暂不抄**（用户 2026-09-28 明确否决）「启动前杀掉桌面端实例 + sleep 800ms」
  （`process_manager.rs:92-98`）：会误杀用户自己开的实例。注意 EchoBird 在这条上**反复过**：
  曾因「多重开启」移除，又因「模型切换静默失败影响面更大」加了回来。若将来我们出现
  「桌面应用开着时改模型不生效」，第一个要复查的就是这条。
- **写配置前建目录必须能处理「断链的重解析点」**（2026-09-29 真机实测，代价是一次功能全废）：
  用户把 `%LOCALAPPDATA%\Claude-3p` 用 **junction 重定向到了 `D:\sim-tool\Claude-3p`**
  （把应用数据挪到 D 盘，很常见），后来 D 盘那份被删 → junction 变成**断链**。
  此状态下裸 `std::fs::create_dir_all(该路径)` 会失败，报的却是
  `os error 183「当文件已存在时，无法创建该文件」` —— **完全指不到真正原因**，
  而 Claude Desktop 的 3P 写入正好落在这里 → 「设置模型」100% 失败。
  机理：`mkdir` 返回 `ERROR_ALREADY_EXISTS`，紧接着 std 用**跟随链接**的 `is_dir()` 判断，
  链接断了就得到 false，于是把 183 当硬错误抛出。
  做法：抽一个 `ensure_dir()`（`commands/ai/tool_config_custom.rs`）——正常目录直接放行；
  失败时 `read_link` 取目标（Windows 上要剥掉 `\??\` 前缀）**把目标目录补出来**，补不动才报错。
  **凡是往用户目录写配置的地方都该用它**，别直接 `create_dir_all`。
- **桌面端「装一遍再卸」是唯一可靠的验证方式**：本机 19 个工具里只有 3 个真装了，
  Claude Desktop / Claude Code 都没装 → 声明路径对不对、启动分支走不走得到、
  写入器能不能落盘，**全靠跑一遍才知道**（上面那条 183 就是这么做才暴露的）。
  能自动装的只有 winget 那两个（`claudedesktop` / `workbuddy`）；其余 6 个桌面端声明的
  安装命令是 `start <官网地址>`，只能手动下载，无法无人值守。
  卸载用同一条包 id（`winget uninstall --id <id>`），清残留时**先看 CreationTime 分清
  哪些是本次装的、哪些是用户原有**（这次就发现用户 5/30 装过 Claude、8/12 建过 junction）。

## 学习目标 2：Claude Code 协议对齐（cc-switch）

> cc-switch 演进很快，会**删除和收窄**功能，抄之前必须确认「现在还有没有、语义变没变」。

### 去哪看（模块地图）

| 路径 | 看什么 |
|------|--------|
| `src-tauri/src/proxy/types.rs` | `RectifierConfig` / `OptimizerConfig` / `AppProxyConfig` —— **判断一个能力是否存在的唯一权威** |
| `src-tauri/src/proxy/{cache_injector,thinking_optimizer,thinking_rectifier,thinking_budget_rectifier,media_sanitizer,tool_media,copilot_optimizer,model_mapper,reasoning_bridge}.rs` | 一个关注点一个文件的整流实现 |
| `src-tauri/src/proxy/providers/transform*.rs`、`streaming*.rs` | 按端点/协议的转换层 |
| `src-tauri/src/proxy/forwarder.rs` | 主流程：预防式改写 → 发送 → 报错后反应式整流重试 |
| `src/i18n/locales/zh.json` | 开关文案。**可能有遗留 key**，别拿它判断功能是否存在 |

### ⚠️ 抄之前必做：确认能力仍在、语义未变

已确认的三次变化（2026-09-26 核对）：

1. **「协议不匹配修复」（未知字段剥离重试）已删除** —— `RectifierConfig` 只剩 4 项。
2. **「请求优化器」改名「Bedrock 请求优化器」并收窄**：只在 `CLAUDE_CODE_USE_BEDROCK=1` 时生效。
3. **「DeepSeek 兼容」不再是用户开关**：下沉为按端点的内建转换。

核对顺序：**先看 `proxy/types.rs` → 再 rg 模块看实现 → 最后才看 i18n 文案**。

### 能力对照表（2026-09-26）

| 能力 | cc-switch | any-version | 结论 |
|------|-----------|-------------|------|
| Prompt 缓存注入 | 仅 Bedrock | 所有 anthropic 出站（`optimizers.rs::inject_cache_breakpoints`） | 我们更广 |
| Thinking 参数自适应 | 仅 Bedrock | anthropic / openai / google 三形态 | 我们更广 |
| DeepSeek / Moonshot 规范化 | 内建（按端点） | 用户开关 `deepseek_normalize` | 粒度不同 |
| Thinking 签名整流 | 报错后剥离 + 重试一次 | 预防式剥离 + 报错后重试一次 | 已对齐 |
| budget_tokens 修正 32000 | 有（+ max_tokens 64000） | 有（同值） | 一致 |
| 图片降级 | 声明纯文本 + 报错兜底 | 同样两条（`media_fallback` + `media_heuristic`） | 一致 |
| 纯文本模型预判 | `request_media_heuristic` | `TEXT_ONLY_MODEL_PREFIXES` + `is_text_only_model` | 已抄 |
| 协议残留字段剥离 | **已删除** | 有（`protocol_mismatch`） | 我们多一项 |
| Copilot 优化器 | 有 | **没有** | 可抄 |
| 每 app 自动故障转移 + max_retries + 流式首字超时 | 有 | **没有** | 可抄 |
| 模型能力注册表驱动图片/参数决策 | 有 | **没有** | 可抄 |

### 移植规则

1. **能力是协议层的，与工具形态无关**：只要工具走本地代理（配置里写 `127.0.0.1`）就该能用 →
   `ai-tools/<id>/config.json` 的 `supportsOptimizer` / `supportsRectifier` 必须对 `supportModel=true`
   的工具打开（`ai_registry.rs::model_capable_tools_allow_optimizer_and_rectifier` 守着这条不变量）。
2. **开关链路一次改全**：`proxy/types.rs::ProxyConfig` → `commands/ai/models.rs::RectifierConfig`
   （全局默认）→ `LaunchAiToolRequest` / `DispatchOptions`（按次覆盖）→ `launch.rs` / `provider.rs` /
   `collab.rs` 的映射 → 前端 `AiConfig` + `ToolLauncher` 复选框 + i18n **zh 与 en 都要加**（有 parity 测试）。
   前端还有兜底的默认 `AiConfig` 字面量（`ModelConfig.tsx`、`ToolLauncher.tsx`），漏改会 tsc 报错。
3. **预防式与反应式两条路都要考虑**；反应式只在确认能修这个错误时才认领（剥不出东西就别重试）。

### 易踩的坑（已踩）

- **别用 i18n 判断功能是否存在**（有遗留 key）；**别照旧截图抄**。
- **单测里不要构造 `ProxyConfig`**：它带 `#[serde(skip)] Option<tauri::AppHandle>`，一构造就把 tauri 运行时
  链接进测试二进制，Windows 下测试进程直接 `0xC0000139 STATUS_ENTRYPOINT_NOT_FOUND`（编译是通过的）。
  要测就把逻辑抽成不依赖 `ProxyConfig` 的纯函数。
- 抄「按端点特例」时**只在命中的端点上改写**，否则会破坏其它供应商的 prompt cache 前缀。

## 学习目标 3：Codex / ChatGPT Desktop 协议对齐（CodexPlusPlus）

### 去哪看（模块地图）

| 路径 | 看什么 |
|------|--------|
| `src-tauri/src/` 内代理 / 协议相关模块 | Responses API 与 Chat Completions 的差异整流、streaming 形态 |
| Codex 配置相关（`config.toml` 读写） | `[[profiles]]` / `mcp_servers` / `[[hooks]]` 表头重复的 TOML 合并陷阱 |
| ChatGPT Desktop 相关 | 桌面端接入方式与协议差异 |
| 前端模型 / 供应商面板 | 供应商元数据 catalog 的组织方式 |

### 已知结论（2026-09-24）

- **不抄**「原生浏览器识别与注入」（CodexPlusPlus 的 native Chrome/Edge 系列）：需要向宿主 renderer
  注入脚本，我们没有该链路（Page 模块先做后删，自动化任务体系同样因此否决）。
- **不抄** CodexPlusPlus 的「插件市场解锁注入」（`installPluginMarketplaceRequestPatch` /
  `patchPluginMarketplaceRequestParams` / `mergeLocalPluginMarketplaces` / `installPluginBuildFlavorFilterPatch`）：
  **Codex 官方 CLI 已经提供同一能力且官方 UI 原生认它**，注入在本版本三条路都断（详见下节实测表）。
  **走 CLI，不要走注入。**
- 候选（需先定用途）：15 家供应商模型元数据 catalog（`3933a12`）。
- ~~不抄 Responses API~~ **已于 2026-09-29 推翻**：新版 ChatGPT Desktop / Codex（26.901+）
  不再支持 `wire_api = "chat"`，**只会**打 `/responses`；不给它路由就直接
  `404 not found: http://127.0.0.1:<port>/responses`。我们已实现 Responses 入站
  （`proxy/responses.rs` + `server.rs::responses_handler`）。

### Codex / ChatGPT Desktop 接入的必知约定（2026-09-29 实测）

| 事项 | 结论 | 出处 |
|---|---|---|
| `wire_api = "chat"` | **26.901+ 已不支持**，写了会导致整份 config.toml 无效并回退默认 | `relay_config.rs:3557`、`provider_import.rs:260` |
| API Key 怎么写 | 用 `requires_openai_auth = true` + `~/.codex/auth.json` 的 `OPENAI_API_KEY`；**不要**用 `env_key`（那是 legacy 路径，去进程环境找 key，找不到就报「缺少 apikey」） | EchoBird `codex.rs:135-141,391-406`；CodexPlusPlus `relay_config.rs:1034-1046` 把 `env_key` 列为待清理键 |
| `OPENAI_*` 环境变量 | **不要注入**：EchoBird 对 codex 显式屏蔽，CodexPlusPlus 把 `OPENAI_` 前缀一律视为冲突 | `process_manager.rs:478-485`、`env_conflicts.rs:37-40` |
| TOML 值类型 | `requires_openai_auth` 必须是**布尔**；写成 `"true"` 会让 Codex 报 `invalid type: string "true", expected a boolean` 并拒绝启动 | 我们踩过 |
| 插件市场 | Codex **自带** plugin marketplace：市场放 `<root>/.agents/plugins/marketplace.json` + `plugins/<name>/`，在 config.toml 用 `[marketplaces.<name>] source_type="local" source="<marketplace.json 路径>"` 注册；**单个插件由客户端自己装** | `plugin_marketplace.rs:756-793` |
| 市场名 | **不能用 `openai-*`**（保留名，注册在它下面会被**静默忽略**，市场里一个插件都不显示，issue #1974/#1968）；改名必须连磁盘上的 `marketplace.json` 一起改 | `plugin_marketplace.rs:9-26` |

> `config.toml` 的合并/删除我们已统一走 `toml_edit` 语义合并（保注释、保未托管表），
> 上面「重复表头 / `mcp_servers` 打散 / `[[hooks]]` 数组表头」三项随之解决。

### Codex 插件市场：**不要抄注入，用官方 CLI**（2026-09-29 实机全链路验证）

**结论先行**：Codex 官方 CLI 提供完整的插件市场管理能力，且 **ChatGPT Desktop 的官方 UI 原生认它**
—— 注册市场 + 装插件之后重启客户端，「公开」标签直接列出全部可装插件、「Installed」列出已装插件。
**这一整条链路不需要任何 renderer 注入。**

```bash
codex plugin marketplace add <本地路径 | owner/repo[@ref] | https/ssh Git URL>   # 注册市场
codex plugin marketplace list                                                   # 市场列表（含 ROOT）
codex plugin marketplace upgrade                                                # 刷新 Git 市场快照
codex plugin marketplace remove <name>
codex plugin list  [--marketplace M] [--available] --json                        # 插件列表
codex plugin add    <插件>@<市场> --json
codex plugin remove <插件>@<市场> --json
```

| 实测事实 | 证据 / 后果 |
|---|---|
| **保留名会被拒**，且报错明说 | `marketplace add` 一个 `name: openai-curated` 的目录 → `Error: marketplace \`openai-curated\` is reserved and cannot be added from this source`。**手写 config.toml 同样被静默忽略**（`marketplace list` 里根本不出现）—— 这就是「装好了但界面一个插件都没有」的真因 |
| **手写 `config.toml` 与 CLI `marketplace add` 等效** | 手写 `[marketplaces.<非保留名>] source_type="local" source='\\?\C:\…'` 后，`marketplace list` 能列出、`plugin list --available` 能列出 120 个、`plugin add` 能装成功 → 不必强依赖 CLI 注册 |
| `--json` 的结构是 `{"installed":[…],"available":[…]}` | **必须带 `--available`** 才包含未安装的插件；条目字段 `name / version / installed / enabled / authPolicy / source.path / marketplaceSource` |
| `plugin add` 不只是写配置 | 它把插件落到 `~/.codex/plugins/cache/<市场>/<插件>/<版本>/` 并处理清单里的 `authPolicy` —— 只写 `config.toml` 会在客户端里「看得见、用不了」。所以**装/卸必须走 CLI** |
| 官方仓库清单里的 `name` 是 `openai-curated` | 下官方市场包落盘后**必须把 `marketplace.json` 的 `name` 改成自己的名字**（如 `anyversion-curated`），否则磁盘名 ≠ config 表名 → 整个市场被忽略 |
| 插件条目里路径在 `source.path` | 顶层没有 `path`；只读顶层会得到一列空路径 |
| 内置市场目录**每次启动重建** | `~/.codex/.tmp/bundled-marketplaces/<…>` 的 CreationTime == 本次启动时刻 → 往里面塞插件会被抹掉，别走这条路 |
| UI 展示位置 | 侧栏 `Customize → Installed` 列已装；「插件 → 公开」按清单里的 `category` 分组列可装。**注册/安装后需要重启客户端**才刷新 |

**为什么「不抄 CodexPlusPlus 的注入」**（本版本 `OpenAI.Codex 26.924.2738.0` 逐条实测）：

| 它依赖的钩子 | 本机实测结果 |
|---|---|
| 包 `window.electronBridge.sendMessageFromView` | `electronBridge` **深度冻结**：`Object.isFrozen === true`、`writable:false, configurable:false`，赋值测试返回 false → **包不了** |
| `import()` 已知 asset（`use-host-config-` 等）拿 app-server client | 主 UI 是**单 bundle**（`app://-/assets/index-<hash>.js`），`import()` 导出数 **0** → 拿不到 |
| 改 `marketplaceKinds` 请求参数 / merge 响应 | 请求经冻结的 contextBridge，无可写钩子；响应侧 `Array.prototype.filter` 可 patch（实测放行后界面确实多出插件），但**放行只能显示已有数据、造不出数据** —— 而数据源里根本没有本地市场 |
| 靠 `bundled-marketplaces` 目录 | 每次启动重建（见上表） |

它的本体是 **505KB / 11193 行** 的注入脚本，靠 `Function.prototype.toString` 匹配压缩后的 JS
（如 `!u(e.marketplaceName)||e.marketplaceName===r`），自己都写了「连续 8 次 miss 就禁用」。
**只在「要接 ChatGPT 账号维度的远程市场（`created-by-me-remote` / `shared-with-me`）」时才值得考虑**，
本地/官方仓库场景一律走 CLI。

### Claude Code 也有**同构**的插件市场 CLI（2026-09-29 装真机实测）

`claude plugin` 与 `codex plugin` 是**同一套形状**，所以两个后端可以共用一份 UI 与一份实现思路：

```bash
claude plugin marketplace add <URL|路径|owner/repo>   # 也支持 --claudeai（claude.ai 托管的市场）/ --scope / --sparse
claude plugin marketplace list                        # 人读输出：`❯ <名字>` + `Source: GitHub (…)`
claude plugin marketplace remove|rm <name>
claude plugin marketplace update [name]
claude plugin install|i <插件>[@市场] [--json]
claude plugin uninstall|remove <插件>[@市场]
claude plugin list [--json] [--available]
claude plugin enable/disable/details/update/validate/prune/init|new/tag/eval
```

| 差异点 | Codex | Claude Code |
|---|---|---|
| CLI 形态 | app 自带的 `codex.exe` | npm 全局包 → Windows 上是 **`claude.cmd`（不是 .exe）**，`CreateProcess` 不能直接跑，**必须经 `cmd /c`** |
| 市场声明 | `~/.codex/config.toml` 的 `[marketplaces.<name>]` | `~/.claude/settings.json` 的 `extraKnownMarketplaces` + 装完写的 `enabledPlugins` |
| 市场名 | 官方仓库里是**保留名** `openai-curated`，必须改名 | **没有保留名机制**：市场名由仓库的 `.claude-plugin/marketplace.json` 自己声明（`anthropics/skills` 声明出来叫 `anthropic-agent-skills`） |
| `list --json` | `{installed:[name/version/installed/enabled], available:[…]}` | `{installed:[**id**/version/scope/enabled/installPath], available:[pluginId/name/**description**/marketplaceName]}` —— **两段字段名不同**，要按 `pluginId`↔`id` 合并，描述只在 available 段 |
| 落盘 | `~/.codex/plugins/cache/<市场>/<插件>/<版本>/` | `~/.claude/plugins/cache/<市场>/<插件>/<版本>/`，另有 `installed_plugins.json` / `known_marketplaces.json` / `plugin-catalog-cache.json` |
| ⚠️ 安全 | 无 | `install` 有 **`--accept-command <sha256>`**（批准「市场声明的命令」）。**绝不自动传** —— 那是替用户批准执行命令；命令源插件宁可装失败，让用户去官方 CLI 自己确认 |

skill（技能）侧：`~/.claude/skills/<名>` 会被当成 `<名>@skills-dir` 插件加载，我们的 per-skill junction 部署落点正确；
但 **`claude plugin list/details` 不覆盖 skills-dir**（那是在会话内解析的），所以「Claude 是否真的加载了」需要登录会话才能验证。

### ⚠️ 声明路径只覆盖「默认安装位置」——这是反复踩到的检测缺口

`ai-tools/<id>/paths.json` 的 `paths.win32` 写的都是默认位置，真机上**经常一条都不命中**：

| 实例 | 声明 | 实机 | 后果 |
|---|---|---|---|
| `codex-cli` | `%APPDATA%/npm/codex.cmd` 等 3 条 | ChatGPT Desktop 的 codex 在 `%LOCALAPPDATA%\OpenAI\Codex\bin\<hash>\codex.exe`（**路径带 hash，无法硬编码**） | `command: codex` 又不在 PATH → **显示未安装**（只能靠 `config.toml` 的 `CODEX_CLI_PATH` 兜底，而那只用在插件安装上，检测层没用） |
| `workbuddy` | `%LOCALAPPDATA%\Programs\WorkBuddy\WorkBuddy.exe` | 实际在 **`D:\sim-tool\WorkBuddy\WorkBuddy.exe`**；`command`/`detectCmd` 都是空 | **检测必然失败**，装了也显示未安装 |
| 所有 npm 系 CLI | `%APPDATA%/npm/<cmd>.cmd` 等 | npm 全局前缀被改到 `D:\any-versions\sdk\nodejs` → 声明 0 命中 | 目前只靠 PATH 兜住 |

可用的兜底来源（按可靠性）：**npm 实际全局前缀**（`npm prefix -g`）→ PATH → 注册表卸载项 `InstallLocation` /
winget 记录 → 开始菜单 `.lnk` 的 Target。`launch.rs` / `claude_plugins.rs` 里已有「动态查 npm prefix」的实现可复用。

### 测试桌面端功能的必知约定（MSIX 版 ChatGPT Desktop）

要在 ChatGPT Desktop 上做任何自动化验证（读 UI、注入、探状态），**必须先拿到 CDP target**：

| 步骤 | 做法 | 踩过的坑 |
|---|---|---|
| 1. 带参数启动 | 只能走 COM：`IApplicationActivationManager::ActivateApplication(AUMID, "--remote-debugging-port=<p> --remote-allow-origins=http://127.0.0.1:<p>", 0)`，CLSID `45BA127D-10A8-46EA-8AB7-56EA9078943C`，AUMID = PackageFamilyName + `!App` | **直接 spawn `WindowsApps\…\app\ChatGPT.exe` 带参数 → 端口能起、`/json` 与 `Target.getTargets` 全是空的**（无包身份，renderer 不暴露 target） |
| 2. 等 target | 轮询 `GET http://127.0.0.1:<p>/json`，约 15~20s 后出现 | 太早查会扑空 |
| 3. 选对 target | 取 `url === "app://-/index.html"` 那条 `type=page` | `/json` 里还有 `codex-sandbox://…`（webview）与 `?initialRoute=%2Favatar-overlay`，取第一个会打到沙箱文档（`bodyLen=0`，白折腾） |
| 4. 操作 UI | `Runtime.evaluate` 执行 `element.click()`；元素多是 `button[role=button]`，侧栏标签是 DIV，按文本匹配时要遍历 `*` | 中文界面文案是「插件 / 添加 / 公开 / 个人」 |
| 5. 收尾 | 验证完把带调试端口的实例关掉，恢复用户自己的启动方式 | 调试端口本机可连 |

## 学习目标 4：多 agent 协同（open-tag / orca）

### 去哪看（模块地图）

| Repo | 路径 | 看什么 |
|------|------|--------|
| open-tag | 协同 / 通信 / 权限相关模块 | agent 间通信协议、任务分配、authz 权限域（sealed mode / reply policy / protected inboxes） |
| orca | Orchestrator 相关 TS | **并行 worktree 运行多 agent**（Codex/ClaudeCode/OpenCode/Pi）、一 prompt 分发多 agent 比较结果、移动端监控 |

### 已知结论

- orca 是 **Electron**（非 Tauri），没有 `#[tauri::command]`；参考重点是**并行编排 / 任务分发 /
  worktree 管理**的架构与状态机逻辑（TS 端），移植为 Rust 命令时需自行实现 IPC 与并发控制。
- open-tag 2026-08-23 起约一个月无新提交；**authz 权限域需先对齐我们的协同权限模型再评估**。
- farming 的「保留侧聊 / 相关会话面板 / 只读分享链接」与我们 `CollabRoom` 模型不同，需先对齐语义
  （farming 已移出跟踪范围，仅在此留档）。

## 实现时的一体化约束（必读）

any-version 的 AI 模块是**一张网，不是一堆孤立功能**。新增任何能力前先确认它在网里的位置：

```
工具声明 ai-tools/<id>/config.json
   ↓ supportModel / supportsOptimizer / supportsRectifier
detect（检测）→ install（安装）→ models（模型列表）→ launch（写配置 + 起本地代理 + 拉起工具）
                                                          ↓
                                        proxy（统一协议转换：anthropic ↔ openai ↔ google）
                                                          ↓
                                    aggregate（本地模型聚合：多供应商候选排序 / 故障转移 / 冷却）
                                                          ↓
                                    collab（多 agent 协同：同一套代理 + 同一套整流 + 同一套聚合）
```

硬性要求：

1. **协议转换只有一份**：任何「某工具/某模型的兼容开关」必须落到统一的 proxy 整流层，
   不允许在 launch / install / collab 里各写一份特例。按端点的特例只在命中的端点生效。
2. **本地模型聚合服务（aggregate）是出站唯一入口**：新增供应商能力（故障转移、max_retries、
   首字超时、冷却）要落在聚合层，不要在单条路径上另起炉灶。
3. **多 agent 协同复用同一条链路**：`collab.rs` 与 `launch.rs` 必须共享「写配置 → 起代理 →
   整流 → 聚合」的实现；协同里新增的协议/聚合能力，单工具启动也应当能用到。
4. **开关链路一次改全**（见学习目标 2 的规则 2）：Rust 配置 → 按次覆盖 → 三个映射点 → 前端 → i18n（zh+en）。
5. **不留死代码**：为「将来可能支持」的端点/形态写的代码一律不写（前车之鉴：Responses API）。

## Common Pitfalls

| Pitfall | Fix |
|---------|-----|
| 抄了已有的功能 | Phase 3 审计优先 |
| Tauri 命令签名不匹配 | 检查 Rust fn 名与前端 `invoke()` 字符串一致 |
| 前端缺少类型定义 | 复制组件时一起复制 type interfaces |
| Cargo.toml 缺少新依赖 | 检查 reference 的 Cargo.toml 所需 crates |
| 硬编码环境路径 | 替换为相对路径或配置驱动 |
| 抄了已删除/已收窄的能力 | 先看 `types.rs` 配置结构，别看 i18n / 旧截图 |

## When NOT to Use

- 任务是完全在 any-version 内部的 bug 修复或功能开发（无需参考）
- 用户明确要求原创实现
- 参考仓库没有相关新变更
- 要抄的东西属于「已移出跟踪范围」的四个仓
