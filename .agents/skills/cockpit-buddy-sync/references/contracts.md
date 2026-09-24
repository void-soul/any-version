# Buddy ↔ cockpit-tools 契约清单（参考 A；已逐字段验证，截至 v1.3.47 / deacbe44）

学习时**先对照本清单**：参考侧若改了这里的任何常量/时序/语义，即为必修差异。

> 参考仓路径现为 `E:\pro\other-sdk\buddy\cockpit-tools`（旧路径 `E:\pro\other-sdk\cockpit-tools` 已失效）。
> 第二个参考仓 **WorkDaddy**（Node.js + CDP 注入，异架构）的契约见同目录 `workdaddy.md`；两仓重叠的账号/会话域**以本文件（cockpit-tools）为准**。

## §1 账号存储与导出/导入

- 存储：`data_dir/buddy/{platform}_accounts/<id>.json` + 同名索引 json（id 仅 `A-Za-z0-9._-`，防穿越）；`current_accounts.json` 持久化"最后切换到的账号"（= 参考 provider_current_state）。
- upsert 去重：uid 优先，其次 email（非 unknown）；命中则保留目标已有 id/created_at/tags。
- 导入 JSON 容错：UTF-8 BOM/UTF-16/GBK 字节解码；顶层数组 | 单对象 | `{accounts|data|items|list|records:[...]}`；snake_case→camelCase；缺 platform/createdAt/lastUsed/id 自动补（id 用 `{platform}_{md5(uid|email|import_N)}`）。
  - 落地（2026-09-24）：解析逻辑抽成纯函数 `store::parse_accounts_json`（不落库），`import_accounts` = 解析 + `upsert_account`；第三方导入复用前者。
- **参考侧的账号导出 = 明文 JSON 数组**：`modules/{workbuddy_account,codebuddy_cn_account}.rs::export_accounts` 就是 `serde_json::to_string_pretty(Vec<Account>)`，字段 snake_case（`access_token/expires_at/...`），`id` 前缀 `workbuddy_` / `codebuddy_cn_`。因此我们的 JSON 导入天然可吃——**不要**再加一层专用解析。
  - 与我们对接：`third_party_import.rs::parse_cockpit_tools_export`（Kira「从本机导入」面板的 cockpit-tools 来源）。注意 `workbuddy_accounts.json` 只是摘要索引（无 `access_token`），**不是**导出文件，导入必须报可操作错误。

## §2 本地导入（从客户端）

- WorkBuddy 登录态两代并存：
  1. 新版 auth 文件（win）`%LOCALAPPDATA%\CodeBuddyExtension\Data\Public\auth\workbuddy-desktop.info`（mac `~/Library/Application Support/CodeBuddyExtension/Data/Public/auth/`），同级 `<file>.logged-out` 标记视为未登录；
  2. 旧版 `state.vscdb` secret：`ItemTable` key = `secret://{"extensionId":"tencent-cloud.coding-copilot","key":"planning-genie.new.accessTokencn"}`，safe-storage 解密。
- token 形态 `uid+access_token`：`+` 前为 uid；无前缀时回退 JWT `sub`。
- 导入后：官方 API 补全（资料/用量）→ upsert → **清理同 token 的占位账号**（email unknown/空 或 无 uid）→ 再刷新一次 token。

## §3 新增账号

- OAuth：start 返回 `{loginId, verifyUrl...}` → 前端轮询 complete → 落库；cancel 按 loginId。
- 直接 token：`build_payload_from_token(platform, token)` 校验并补全资料。

## §4 用量/配额

- `refresh_payload_for_account` 返回 (账号, 可选 quota 错误)。quota_raw 含 `userResource / dosage / payment` 组合；字段 `planType, dosageNotify{Code,Zh,En}, paymentType, usageUpdatedAt`；失败写 `quotaQueryLastError(+At)`。
- 全量刷新/查询返回成功计数，单条失败 eprintln 不中断。

## §5 签到

- 手动：`perform_checkin(token, uid, enterpriseId, domain)` → `{success, streakDays?, credit?, reward?, message?}`；成功写 `lastCheckinTime/checkinStreak(缺省+1)/checkinRewards(credit 兜底成 {credit})`。
- `get_checkin_status` → `{todayCheckedIn, active, streakDays, dailyCredit}`。
- 自动：配置（enabled + 每天随机时间窗 start/end + 账号级 schedule + lastCheckedDate 日去重）；日志上限裁剪；应用启动 `start_auto_checkin_scheduler(AppHandle)`；手动 run 带 force。

## §6 会话管理（列表）

- WorkBuddy：`~/.workbuddy/workbuddy.db` 表 `sessions(id,cwd,user_id,title,status,created_at,updated_at,deleted_at,is_playground)`，deleted 不展示。
- CodeBuddy CN：`<数据目录>/codebuddy-sessions.vscdb` `ItemTable` key `session:%`，value JSON `{conversationId,title,cwd,userId,status,createdAt,updatedAt,deletedAt,isPlayground}`。
- 过滤 keyword(标题/cwd) + status，按 updatedAt 倒序，跨实例合并 locations。

## §7 切换账号（核心时序，勿改序）

`关闭运行中客户端(20s 超时，失败则中断切换) → 会话合并 → 写入目标登录态 → 持久化 current → 重新启动(--new-window 分离；失败不回滚，消息追加"但 X 启动失败")`

- WorkBuddy 写回 auth 文件：
  - 含 `$wbEncrypted` 包装字段 → 拒写（防破坏官方加密态）；
  - 现有文件先比对 SHA256 再**哈希一致才原子写**（被官方客户端改过→报"已停止覆盖请重试"）；
  - 会话结构必须含 `account/auth/accounts/allAccounts`，非目标账号 `lastLogin:false`，目标合并进 accounts/allAccounts；写回后校验 accessToken 与四个字段；清理 `.logged-out` 标记（同样哈希比对）。
- CodeBuddy CN 写回 `state.vscdb`（候选路径 `User/globalStorage → globalStorage → 根`，无则建父目录注入）：
  - secret key 同 §2；会话 JSON **逐字段固定结构**：`id:"Tencent-Cloud.genie-ide-cn", token, refreshToken, expiresAt, domain, accessToken:"uid+token", converted:true, account:{id,uid,label,nickname,enterpriseId,enterpriseName,pluginEnabled:true,lastLogin:true}, auth:{accessToken,refreshToken,tokenType,domain,expiresAt,expiresIn:expiresAt,refreshExpiresIn:0,refreshExpiresAt:0,lastRefreshTime:now}`；
  - 注入失败含 Safe Storage/Local State/Keychain → 追加"先手动登录一次"引导；
  - 注入后必须 `verify_state_db_injection`（重查 key 非空）。
- 进程：参考 PowerShell+sysinfo 按 exe 路径匹配；Kira sysinfo 按进程名（`WorkBuddy.exe` / `CodeBuddy CN.exe|CodeBuddy.exe`）匹配 + `taskkill /T /F` + 等待；启动路径 `buddy/client_paths.json` 覆盖优先（设置 tab 可配），否则常见安装位置候选。

## §8 会话合并（结构最深，漂移重灾区）

- 目录布局：`<extensionData>/<uid>/<IDE名>/<uid>/history/<workspace>/{index.json, <conversationId>/...}`；
  extensionData = `%LOCALAPPDATA%\WorkBuddyExtension\Data`（新）→ 无则 `CodeBuddyExtension\Data`（legacy，WB 与 CN 共用）。
- 合并算法（参考 codebuddy_session_transfer 为共享引擎，workbuddy 侧复用）：
  1. 来源工作区在目标**不存在** → `copy_dir_atomic(整个工作区目录)`（**不是只拷 index.json**）+ 辅助目录；added=来源会话数。
  2. 存在 → 按 conversationId 去重合并：来源更新（`lastMessageAt`，支持 int 或 RFC3339）才 `replace_dir_atomic` 并计 replaced；目标索引有该 id 但目录缺失也替换；新 id → 目录复制 + 备份孤儿目录 + added；`current` 仅在来源 current 会话存在于合并结果时转移；`index.json` 改动前**先备份**（真复制 index.json）；排序按 recency 倒序；原子写。
  3. 辅助目录 kinds：`check-point, file-tree, plan-task, genie-cache, connectors`，工作区级 `<accountRoot>/<kind>/<workspace>` 与会话级 `<...>/<conversationId>`；replace 时先备份再替换。
  4. 安全：`validate_uid` 仅单级路径名；所有读写点 `reject_symlink`；全程 try-lock 互斥（static LazyLock，**不可函数内新建**）。
- 数据库层（WorkBuddy 5.x 真正生效的合并）：
  - **无条件**执行 `UPDATE sessions SET user_id=<target> WHERE user_id != <target> AND deleted_at IS NULL`（`~/.workbuddy/workbuddy.db`；先建备份目录并复制 **db+ -wal + -shm**，事务提交）。
  - **无条件**再查旧版 vscdb（electron 与 config 两处的 `codebuddy-sessions.vscdb`，取第一个存在者）：`session:%` 的 JSON `userId` 从 source→target（备份主库文件后事务更新）。
  - 目录级 roots 为空只是跳过目录合并，**数据库重映射照做**（历史大坑）。
- CN 入口数据目录：`%APPDATA%\CodeBuddy CN`；来源 uid 判定：读当前 secret 的 `accessToken` 前缀（或 Kira：store 当前账号的 uid），与目标不等才合并。

## §9 账号互导

- 方向：当前平台 → 对侧，全量；字段级复制 identity/token/quota/raw/status；**签数字段与 tags 不携带**（参考 payload 结构没有这些字段）；
- 目标 upsert 去重（uid/email）→ 新账号 id 按目标前缀重新生成（`workbuddy_` / `codebuddy_cn_` + md5(uid|email|"平台_user")）；
- 单条失败仅日志，返回成功计数；前端「导入 <对侧>」按钮 + `成功导入 N 个账号` / `导入失败: err` toast，无确认。

## 本机真机布局速查（win，排查用）

```
~/.workbuddy/{app/, workbuddy.db(+ -wal/-shm)}
%APPDATA%/CodeBuddy CN/{codebuddy-sessions.vscdb, last-session.json, User/globalStorage/state.vscdb, Local State}
%LOCALAPPDATA%/CodeBuddyExtension/Data/<uid>/...          （legacy 扩展数据，含 Public/auth/workbuddy-desktop.info）
%LOCALAPPDATA%/WorkBuddyExtension/Data                     （可能不存在，正常）
```
合并备份：`~/.workbuddy/.kira-workbuddy-session-backup/<uid>`、`%APPDATA%/.kira-codebuddy-session-backup/<uid>`。
