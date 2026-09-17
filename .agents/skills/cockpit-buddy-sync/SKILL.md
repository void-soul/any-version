---
name: cockpit-buddy-sync
description: 把两个参考仓的 Buddy / WorkBuddy（含 CodeBuddy CN/AI）功能学习并同步到 any-version 的既定流程——参考 A cockpit-tools（E:/pro/other-sdk/buddy/cockpit-tools，Rust/Tauri，可直接移植）、参考 B WorkDaddy（E:/pro/other-sdk/buddy/WorkDaddy，Node.js + CDP 注入，只作语义参考）。当参考仓升级、用户说"学习/对齐/复刻 cockpit-tools 或 WorkDaddy""继续学习目标项目""抄 Buddy 作业"，或 WorkBuddy/CodeBuddy 的 本地导入、新增账号、切换账号、假退出登录、手动/自动签到、会话列表、会话合并、会话归档导入导出、账号导出导入、账号互导、用量与积分统计、Token 统计、积分轮换建议、自动化任务、防休眠、免打扰、自动续接、暂存提示词、快捷短语、主题、模型管理 出现功能缺失或行为不一致时使用。先跑脚本取差异、读同步点与契约清单，再按差异增量学习，不要凭记忆全量重写。
---

# Buddy 参考仓增量学习（cockpit-buddy-sync）

## 0. 三个仓库

| 角色 | 路径 | 形态 | 学习方式 |
|---|---|---|---|
| 参考 A：cockpit-tools | `E:\pro\other-sdk\buddy\cockpit-tools` | Rust/Tauri，与目标**同架构** | 整函数移植，按 §4.1 适配 |
| 参考 B：WorkDaddy | `E:\pro\other-sdk\buddy\WorkDaddy` | 纯 Node.js 本地 daemon + CDP 注入（无构建管线，真源在 `scripts/`，行为规范在 `test/`） | **只借语义/格式/算法/时序**，不借架构与注入代码，按 §4.2 适配 |
| 目标：any-version | `E:\pro\my\any-version` | Tauri + Rust + React | 见下方文件地图 |

> 旧路径 `E:\pro\other-sdk\cockpit-tools` 已失效；两个参考仓现同放 `E:\pro\other-sdk\buddy\`。可用环境变量 `COCKPIT_TOOLS_DIR` / `WORKDADDY_DIR` 覆盖路径。

目标侧文件地图：

- Buddy 后端 `src-tauri/src/commands/buddy/`：`mod.rs`（命令层 + 切换编排 + 进度事件）、`models.rs`、`store.rs`、`api.rs`（官方 API / 用量 / 签到）、`crypto.rs`、`workbuddy.rs`、`codebuddy_cn.rs`、`sessions.rs`、`auto_checkin.rs`、`auto_travel.rs`、`daily_history.rs`、`expiry.rs`、`action_log.rs`、`client_process.rs`、`session_transfer{.rs,/workbuddy.rs,/codebuddy.rs}`
- 前端 `src/components/buddy/BuddyPanel.tsx`；i18n `src/i18n/locales/{zh,en}/translation.json`（键在 `buddy.*`）；命令注册 `src-tauri/src/lib.rs`
- **AI/代理侧**（WorkDaddy 的用量统计、Token 统计、模型管理域对应到这里）：`src-tauri/src/commands/ai/{models.rs,provider.rs,usage.rs}`、`src-tauri/src/proxy/{mod.rs,server.rs}`、`src/components/ai/{UsageStats.tsx,ModelConfig.tsx}`

## 1. 同步点（先查再学）

`sync-point.txt`（cockpit-tools commit）与 `sync-point.workdaddy.txt`（WorkDaddy commit + `DAEMON_VERSION` / `DAEMON_BUILD_ID`）分别记录已对齐点。每次开始：

```bash
bash .agents/skills/cockpit-buddy-sync/scripts/relearn.sh            # 两个参考仓都跑
bash .agents/skills/cockpit-buddy-sync/scripts/relearn.sh cockpit    # 只跑参考 A
bash .agents/skills/cockpit-buddy-sync/scripts/relearn.sh workdaddy  # 只跑参考 B
```

输出 = 自上次同步以来 Buddy 相关文件的 commit 列表与 `diff --stat`（WorkDaddy 另打印当前 `DAEMON_VERSION`/`DAEMON_BUILD_ID`）。**以这个差异为学习范围**，只精读有变化的参考文件；无变化的功能域直接跳过（除非用户报告了 bug——那走 `references/pitfalls.md` 检查单）。

## 2. 功能域 → 参考文件对照表

### 2.1 参考 A：cockpit-tools（同架构，优先直接移植）

| 功能域 | cockpit-tools 参考位置 | 我们的位置 | 契约锚点 |
|---|---|---|---|
| 账号存储/去重/导出导入 | `modules/workbuddy_account.rs`、`modules/codebuddy_cn_account.rs` | `store.rs`、`mod.rs` | references/contracts.md §1 |
| 本地导入 | 同上 `import_payload_from_local` | `workbuddy.rs`、`codebuddy_cn.rs` | §2 |
| 新增（OAuth/token） | `modules/*_oauth.rs`、`commands/workbuddy.rs` | `api.rs` | §3 |
| 用量/配额 | `commands/*.rs` quota 系列 + `*_oauth` 的 refresh | `api.rs` `refresh_payload_for_account` | §4 |
| 手动签到/自动签到 | `commands/workbuddy.rs` checkin + `modules/*_checkin*` | `api.rs`、`auto_checkin.rs` | §5 |
| 会话管理（列表） | `modules/codebuddy_session*.rs` | `sessions.rs` | §6 |
| 切换账号 | `commands/workbuddy.rs::inject_workbuddy_to_vscode`、`commands/codebuddy_cn_instance.rs::inject_bound_account_for_instance_start_blocking`、`modules/workbuddy_instance.rs::resolve_workbuddy_runtime_dirs` | `mod.rs::buddy_switch_account`、`workbuddy.rs`/`codebuddy_cn.rs` `write_account_to_default_client`、`client_process.rs` | §7 |
| 会话合并 | `modules/workbuddy_session_transfer.rs`、`modules/codebuddy_session_transfer.rs` | `session_transfer{.rs,/workbuddy.rs,/codebuddy.rs}` | §8 |
| 账号互导 | `workbuddy_account::sync_accounts_to_codebuddy_cn`、`codebuddy_cn_account::sync_accounts_to_workbuddy` | `store.rs::sync_accounts` | §9 |

### 2.2 参考 B：WorkDaddy（异架构，借语义；完整契约见 `references/workdaddy.md`）

可移植性列含义：**纯逻辑** = 可直接转写成 Rust；**接口** = 官方 HTTPS 接口语义，可转 Rust HTTP 客户端；**注入** = 依赖 Electron renderer 注入，只借判定规则/交互语义，不照搬实现。

| 功能域 | WorkDaddy 参考位置 | 我们的位置 | 可移植性 | 锚点 |
|---|---|---|---|---|
| 账号库/备份/加密导出导入 | `scripts/lib.js`（`accountsDir`/`backupAuthFile`/`backupCurrent`）、`scripts/secure-transfer.js`（`EXPORT_VERSION=3`/`createEncryptedExport`/`openEncryptedExport`）、`daemon.js` `/api/accounts/{export,import}` | `store.rs`、`crypto.rs`、`mod.rs` | 纯逻辑 | `workdaddy.md` §1 §2 |
| 本地导入（登录态解析/多实例发现） | `lib.js` `authRecordFromJson`/`resolveCurrentAuth`/`resolveAuthTarget`/`retireLogoutMarker`、`profiles.js` `authFile` | `workbuddy.rs`、`codebuddy_cn.rs` | 纯逻辑 | §3 |
| 新增账号（无感 OAuth 免退出） | `daemon.js` `/api/oauth/{start,poll}`、`buildSeamlessAuthFile`、`saveSeamlessAccount`（`OAUTH_TIMEOUT_SECONDS=600`） | `api.rs` | 纯逻辑 + 接口 | §4 |
| 用量/积分（官方计费接口 + 按天落库） | `credit-request-usage.js`、`credit-history-sync.js`、`credit-usage-store.js`、`credit-segments.js` | `api.rs::refresh_payload_for_account`、`daily_history.rs` | 纯逻辑 + 接口 | §5 |
| Token 统计（扫本地会话 JSONL） | `token-stats.js`（`CACHE_VERSION=8`/`aggregateBuckets`） | `commands/ai/usage.rs`、`components/ai/UsageStats.tsx` | 纯逻辑 | §6 |
| 积分轮换建议（不足时换号） | `credit-rotation.js`（`selectRotationCandidate`） | 暂无 | 纯逻辑 | §7 |
| 签到（判定/多域兜底/consent） | `checkin-result.js`（`code===10001`）、`checkin-consent.js`、`daemon.js` `dailyCheckin`/`performAccountCheckin` | `api.rs`、`auto_checkin.rs` | 纯逻辑 + 接口 | §8 |
| 会话列表（DB 只读查询） | `session-db.js`、`daemon.js` `/api/sessions` | `sessions.rs` | 纯逻辑 | §9 |
| 切换账号 / 假退出登录 | `lib.js::switchTo`、`daemon.js` `/api/switch`、`/api/logout` | `mod.rs::buddy_switch_account`、`workbuddy.rs`、`client_process.rs` | 文件层纯逻辑；CDP 刷新属注入 | §10 |
| 会话归档导入导出（`.wds` v4 加密） | `session-transfer.js`（`MAGIC='WDS4\r\n\x1a\n'`/`HEADER_SIZE=52`）、`daemon.js` `/api/sessions/{export,import}` | 暂无（我们只有合并） | 纯逻辑 | §11 |
| 会话合并/迁移（auto-copy lineage） | `daemon.js` `copySessionRecord`/`syncAutoCopyLineage`/`buildAutoCopyPlan`、`lib.js` `meta.autoCopy` | `session_transfer/` | 纯逻辑；renderer 让位属注入 | §12 |
| 主账号与账号排序 | `primary-account.js`、`lib.js` `accountOrderMode` | 暂无 | 纯逻辑 | §2 |
| profile/客户端识别与能力矩阵 | `profiles.js`（`capabilities`）、`workbuddy-target.js`、`ui-port.js`、`cdp-targets.js` | `client_process.rs`、`models.rs::BuddyPlatform` | 识别纯逻辑；CDP 连接属注入 | §13 |
| 自动化任务（触发器/步骤/导入导出） | `automation*.js`、`schemas/automation-package.v1.schema.json`、`scripts/builtin/automations/*.json` | 暂无（大功能） | 模型/校验/调度纯逻辑；`dom.*`/`session.*` 步骤属注入 | §14 |
| 模型管理 / 第三方模型（CC Switch 导入） | `third-party-models.js`（`discoverCCSwitch`/`convertProvider`）、`daemon.js` `/api/models*` | `commands/ai/models.rs`、`provider.rs`、`components/ai/ModelConfig.tsx` | 纯逻辑 | §15 |
| 增强开关（免打扰/自动续接/暂存/快捷短语/主题/防休眠） | `inject.js` 各 pane + `daemon.js` 对应路由 + `theme-vars.js`/`theme-patches.js`/`theme-text-shadow.js` | 暂无 | 多数属注入；防休眠与主题 CSS 生成可搬 | §16 |
| 本地 HTTP API 路由清单 | `daemon.js` `handleApi` | 对应 Tauri command | 仅作接口语义清单 | §B |
| 行为规范（回归测试） | `test/*.test.js` | — | 移植前先读同名测试，用例断言可直接转写 | §Z |

> cockpit-tools 与 WorkDaddy 覆盖重叠的是「账号/会话」两域。**同域冲突时以 cockpit-tools 为准**（同架构、已对齐）；WorkDaddy 只用来补 cockpit 没有的能力（`references/workdaddy.md` §2、§5~§7、§11、§14~§16）与校验既有实现的边界条件。

## 3. 学习流程

1. `relearn.sh` 拿两仓差异 → 对照 §2 圈出受影响功能域。
2. **精读参考实现**（整函数读，不要只 grep 片段），再读我们的对应实现；`test/*.test.js` 与参考同源，可直接当行为规范读。
3. **判可移植性三分类**（WorkDaddy 特有）：① 纯逻辑；② 官方接口/HTTP；③ 必须渲染进程注入。`inject.js` 顶部被 `test/*.test.js` 直接抽取的纯函数（文案表、分类器、状态机）属 ①，优先移植；③ 只借鉴判定规则与交互语义，并在交付说明里写清降级方案。
4. 逐条比对：路径、常量（secret key、JSON 字段名）、时序、分支条件、错误语义。语义性差异（少做一步、条件写反、字段缺）必修；风格差异不修。
5. 应用 §4 适配规则后移植；每个语义点配一个单元测试（可直接把参考同名 test 的用例断言转写成 Rust `#[cfg(test)]`）。
6. 验证（§5）→ 更新**两个** sync-point 并追加学习记录 → 写 qa.db → 汇报差异清单。

## 4. 固定适配（参考侧有、我们不照搬的部分，勿"学回去"）

### 4.1 相对 cockpit-tools

- **无多实例**：参考按实例 user_data_dir 注入/启动；Kira 只有默认实例，固定用默认数据目录。
- **统一模型**：两平台共用 `BuddyAccount`，互导/转换不需要 payload 结构体。
- **日志**：无 `log` crate，用 `eprintln!`（勿引入 `log::` / 参考的 `logger::`）。
- **进度事件**：切换用 `tauri::AppHandle` emit `buddy-switch-progress`（stage: closing/merging/writing/launching/done），参考无此机制（只有 Codex 有进度弹窗）；合并报告返回前端后必须渲染为**文本**，绝不能把对象塞进 React 子节点。
- **备份**：参考用 `backup_storage::behavior_backup_dir` + 定期 prune；Kira 用简化目录 `<数据父目录>/.kira-{workbuddy,codebuddy}-session-backup/<uid>`。
- **命令形态**：互导合并为一条 `buddy_sync_accounts(from,to)`（参考是两条）。
- 前端文案一律走 `buddy.*` i18n 键，zh/en 同时补。

### 4.2 相对 WorkDaddy

- **不引入 CDP / Electron 注入，也不引入本地 Node daemon**：WorkDaddy 的面板是注入进 WorkBuddy renderer 的，any-version 是独立 Tauri 窗口。只复刻**数据与语义**（账号库、用量统计、归档格式、任务模型、签到判定），UI 用我们的组件范式重写。
- **不引入 Node 运行时与依赖**：参考实现里每个 Node API 都要找 Rust 等价物——`crypto.scryptSync`→`scrypt` crate、`aes-256-gcm`→`aes-gcm`、`zlib.gzipSync`→`flate2`、`node:sqlite`/`sqlite3` CLI→`rusqlite`（或复用现有 DB 助手）、`fs.rename` 原子写→`fs::rename`/现有 `file_io` 助手、`osascript` 沙箱回退→平台原生实现（Windows 我们不需要）。
- **profile 维度只保留两平台**：参考有 `workbuddy-cn` / `workbuddy-ai` / `codebuddy-cn` / `codebuddy-intl` 四 profile；我们只有 WorkBuddy 与 CodeBuddy CN。AI / intl 的差异（`-ai.info` 后缀、`~/.workbuddy-ai`、CDP 9223）仅作将来扩平台的参考，不预先落库。
- **切换时序不混用**：WorkDaddy 切换 = 写登录文件 + CDP 刷新（不关客户端）；我们的切换时序按 cockpit-tools（关客户端 → 会话合并 → 写回 → 重启）。WorkDaddy 的 `/api/logout`（先优雅退出宿主再删登录文件、token 保留）可作为"假退出登录"新功能的参考语义。
- **本地 HTTP API 只是语义清单**：`/api/*` 一律不作为架构引入，对应能力走 Tauri command。
- **不移植**：遥测/上报（`usage-report.js` 的 `workdaddy.dev/api/track`、Sentry）、Windows 提权与进程边界（`windows-process-boundary*`、`install-win.ps1`）、自动更新与打包发布（`build-win-release.ps1`、Inno Setup、DMG）。
- **参考里没有的东西**：签到"随机时间窗"在 WorkDaddy 中不存在（它是固定 60 分钟 interval 任务）；我们的随机窗口语义来自 cockpit-tools，勿互相覆盖。

## 5. 验证协议

```bash
cd E:\pro\my\any-version\src-tauri
cargo check --no-default-features
cargo test --no-default-features --lib
cd .. && npx tsc --noEmit
```

改动了切换/合并/进程逻辑时，另需真机回归（本机布局速查见 `references/contracts.md` 末节）：启动 `yarn start`，实测一次切换，观察进度条、合并统计与 `%APPDATA%\CodeBuddy CN\.kira-codebuddy-session-backup` 备份目录是否生成。

移植 WorkDaddy 的 ③ 类（注入依赖）能力时，必须额外说明降级方案与"我们做不到的部分"，不能只写"已移植"。

## 6. 收尾

- qa.db：`qa-log` 技能，一条功能/修复记录（根因写"参考实现语义 vs 我们的差异"，注明来自 A 还是 B）。
- `sync-point.txt` / `sync-point.workdaddy.txt`：更新 commit 与日期。
- 若发现新坑（移植/漂移导致的 bug），追加到 `references/pitfalls.md` 检查单；若发现 WorkDaddy 侧新语义，追加到 `references/workdaddy.md`。

## 7. 技能内文件

| 文件 | 用途 |
|---|---|
| `sync-point.txt` | 参考 A（cockpit-tools）对齐点 |
| `sync-point.workdaddy.txt` | 参考 B（WorkDaddy）对齐点（commit + daemon 版本） |
| `scripts/relearn.sh` | 生成两仓自同步点以来的相关 diff |
| `references/contracts.md` | 参考 A 的逐字段契约（已逐字段验证） |
| `references/workdaddy.md` | 参考 B 的契约清单 + 可移植性三分类 + 缺口候选 |
| `references/pitfalls.md` | 两仓移植踩坑单（含漂移探测 grep） |
