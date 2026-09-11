---
name: cockpit-buddy-sync
description: 把 cockpit-tools（E:/pro/other-sdk/cockpit-tools）的 Buddy 相关功能学习并移植到 any-version 项目的既定流程。当 cockpit-tools 升级、用户说"学习/对齐/复刻 cockpit-tools""继续学习目标项目"、或 WorkBuddy/CodeBuddy CN 的 本地导入、新增账号、切换账号、手动/自动签到、会话合并、会话管理、用量查询、导出导入、账号互导 出现功能缺失或行为不一致时使用。先读本技能的同步点与契约清单，再按差异增量学习，不要凭记忆全量重写。
---

# Cockpit → Buddy 增量学习（cockpit-buddy-sync）

## 0. 两个仓库

- 参考源：`E:\pro\other-sdk\cockpit-tools`（git 仓，按版本发布；只读，禁止修改）
- 目标：`E:\pro\my\any-version`
  - 后端 `src-tauri/src/commands/buddy/`（mod / models / store / api / crypto / workbuddy / codebuddy_cn / sessions / auto_checkin / client_process / session_transfer{,.rs 含 workbuddy.rs+codebuddy.rs}）
  - 前端 `src/components/buddy/BuddyPanel.tsx`、i18n `src/i18n/locales/{zh,en}/translation.json`（键在 `buddy.*`）、命令注册 `src-tauri/src/lib.rs`

## 1. 同步点（先查再学）

`sync-point.txt` 记录了本仓库 Buddy 功能对齐到的 cockpit-tools commit。每次开始：

```bash
bash .agents/skills/cockpit-buddy-sync/scripts/relearn.sh
```

输出 = 自上次同步以来，参考侧 Buddy 相关文件的 commit 列表与 diff --stat。**以这个差异为学习范围**，只精读有变化的参考文件；无变化的功能域直接跳过（除非用户报告了 bug——那走 pitfalls.md 检查单）。

## 2. 功能域 → 参考文件对照表

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

## 3. 学习流程

1. `relearn.sh` 拿差异 → 对照上表圈出受影响功能域。
2. **精读参考实现**（整函数读，不要只 grep 片段），再读我们的对应实现。
3. 逐条比对：路径、常量（secret key、JSON 字段名）、时序、分支条件、错误语义。语义性差异（少做一步、条件写反、字段缺）必修；风格差异不修。
4. 应用 §4 适配规则后移植；每个语义点配一个单元测试。
5. 验证（§5）→ 更新 `sync-point.txt` 为新 commit 并追加一行学习记录 → 写 qa.db → 汇报差异清单。

## 4. Kira 固定适配（参考侧有、我们不照搬的部分，勿"学回去"）

- **无多实例**：参考按实例 user_data_dir 注入/启动；Kira 只有默认实例，固定用默认数据目录。
- **统一模型**：两平台共用 `BuddyAccount`，互导/转换不需要 payload 结构体。
- **日志**：无 `log` crate，用 `eprintln!`（勿引入 `log::` / 参考的 `logger::`）。
- **进度事件**：切换用 `tauri::AppHandle` emit `buddy-switch-progress`（stage: closing/merging/writing/launching/done），参考无此机制（只有 Codex 有进度弹窗）；合并报告返回前端后必须渲染为**文本**，绝不能把对象塞进 React 子节点。
- **备份**：参考用 `backup_storage::behavior_backup_dir` + 定期 prune；Kira 用简化目录 `<数据父目录>/.kira-{workbuddy,codebuddy}-session-backup/<uid>`。
- **命令形态**：互导合并为一条 `buddy_sync_accounts(from,to)`（参考是两条）。
- 前端文案一律走 `buddy.*` i18n 键，zh/en 同时补。

## 5. 验证协议

```bash
cd E:\pro\my\any-version\src-tauri
cargo check --no-default-features
cargo test --no-default-features --lib
cd .. && npx tsc --noEmit
```

改动了切换/合并/进程逻辑时，另需真机回归（本机布局速查见 contracts.md 末节）：启动 `yarn start`，实测一次切换，观察进度条、合并统计与 `%APPDATA%\CodeBuddy CN\.kira-codebuddy-session-backup` 备份目录是否生成。

## 6. 收尾

- qa.db：`qa-log` 技能，一条功能/修复记录（根因写"参考实现语义 vs 我们的差异"）。
- `sync-point.txt`：更新 commit 与日期。
- 若发现新坑（移植/漂移导致的 bug），追加到 references/pitfalls.md 检查单。
