# WorkDaddy 契约清单（参考 B）

- 时点：WorkDaddy `a0cdb0a0`（分支 `main`），`DAEMON_VERSION = '1.2.42'`，`DAEMON_BUILD_ID = 'release-1.2.42-20260914-streaming-session-transfer'`，2026-09-17 精读。
- 形态：纯 Node.js（无 `package.json` 构建管线）+ 本地回环 HTTP daemon（`scripts/daemon.js`）+ 注入到 WorkBuddy renderer 的面板（`scripts/inject.js`）。**真源是 `scripts/`，行为规范是 `test/`（每个模块都有同名测试）。**
- 本文件只记录**语义 / 格式 / 算法 / 时序**，不复制源码。每节末尾有 `锚点:`，学习前先跑 `§Z` 的检查确认未漂移；漂移了以源码为准并回写本文件。
- 文中「注入依赖」= 该能力靠 `Runtime.evaluate` 注入 renderer 实现，Rust/Tauri 无法照搬，只借判定规则与交互语义。

## §1 账号库与加密导出导入

- 账号库 = 目录扫描，没有单独索引表：`lib.js::accountsDir(dataDir)` = `<dataDir>/accounts`，每个账号一个 `<uid>.info`（原始登录文件文本，原子写 tmp+rename、权限 0600）；UID 即唯一键。
- `lib.js::backupAuthFile(dataDir,file,log)`：读登录文件 → 原子写备份；`backupCurrent` 遍历 `listAuthRecords()`，**已有同名备份且不是"当前权威文件"的历史存档不得覆盖**（真实事故防御）。
- 元数据 `meta.json`：`meta.accounts[uid]`（含 profile 绑定、`sort`）、`meta.accountOrderMode ∈ {expiry,fixed}`、`meta.autoCopy`（§12）。`lib.js::updateMeta(..., {preserveBinding})`。
- 写入目标决策链 `lib.js::resolveAuthTarget(dataDir,uid,authJson)`：官方固定文件优先 → `meta.authFileName`（需同通道校验）→ 唯一 uid 匹配 → 同通道唯一记录 → legacy 账号回退固定文件；**都不满足时拒绝猜测**（不写任何文件）。
- 加密导出 `secure-transfer.js`：`EXPORT_KDF='aes-256-gcm+scrypt'`、`EXPORT_VERSION=3`、`EXPORT_COMPRESSION='gzip'`、`MAX_PASSWORD_LENGTH=1024`；`encryptBinary` 用 `salt=16B`、`iv=12B`、打包布局 `base64(iv‖authTag‖cipher)`（`[0,12)` / `[12,28)` / `[28,…)`）；`createEncryptedExport(kind,payload,password)` **先 gzip 再加密**，信封 `{wbsExport:'WorkDaddy',version,exportType,createdAt,kdf,compression,salt,data}`；`openEncryptedExport` 接受 `version ∈ {2,3}`（v2 为不压缩文本），校验 `payload.exportType === 'WorkDaddy-'+kind`；`normalizeExportKind` = `/^[a-z][a-z0-9-]{0,63}$/`。
- v1 兼容：`daemon.js` `EXPORT_PASSPHRASE='workdaddy'` + `EXPORT_KDF_SALT='WorkDaddy-account-export-v1'` 的固定密钥 AES-GCM（`decryptLegacyExport`）。
- 导出/导入路由：`POST /api/accounts/export`（`{password,uids?≤500}`，payload `{exportType:'WorkDaddy-accounts',version:2,accounts:[{uid,info}]}`）、`POST /api/accounts/import`（分支：`format:'plain-json'` 明文 | 加密 v≥2 | v1 固定密码；**写盘前必须校验** uid 非 `.`/`..`、不含 `\ / \0`、且 `acct.uid === uid`）。
- 我们的对应：`store.rs`、`crypto.rs`；差异候选 = 我们的导出格式与参考 v3 信封不兼容（若要互操作需按上表复刻信封）。

锚点：`grep -n "EXPORT_VERSION\|EXPORT_KDF\|function createEncryptedExport\|function openEncryptedExport" scripts/secure-transfer.js`；`grep -n "function resolveAuthTarget\|function backupCurrent\|function accountsDir" scripts/lib.js`

## §2 账号互导 / 主账号 / 排序

- 账号互导 = §1 的导出/导入（加密信封）；跨账号**会话**同步 = §12 的 lineage 自动复制（两回事）。
- 排序：`lib.js::getAccountOrder/setAccountOrder`，`meta.accountOrderMode ∈ {expiry,fixed}` + `meta.accounts[uid].sort`。
- 主账号：`primary-account.js` 独立文件 `<dataDir>/primary-account.json`（`{uid}`，uid 正则 `/^[A-Za-z0-9_-]{1,128}$/`，原子写；读时账号不存在即清空），路由 `POST /api/accounts/primary`；删除账号时若为主账号则清空。
- 我们的对应：`store.rs::sync_accounts`（互导已对齐 cockpit）；主账号/排序暂无 → 缺口候选。

锚点：`grep -n "primary-account.json\|accountOrderMode" scripts/primary-account.js scripts/lib.js`

## §3 本地导入（登录态解析）

- 登录文件：`profiles.js` 的 `authFile`，Windows = `%LOCALAPPDATA%\CodeBuddyExtension\Data\Public\auth\workbuddy-desktop.info`（AI = `-ai.info`）；可用 `WBSWITCH_AUTH_FILE` 覆盖。
- 多实例/多档案发现（`lib.js` `DYNAMIC_AUTH_DISCOVERY`）：当 profile 指定 authFile 且 `capabilities.accounts` 时，扫描整个 auth 目录下所有 `*.info`，用 `authRecordFromJson(file,json,{strict:true})` 过滤——**`accessToken` 必须存在且 `domain`/`issuer` 至少一个命中允许域集合**，否则不算有效账号。
- 允许域：由 JWT `iss` 解析（`tokenIssuerOrigin`）；cn = `www.workbuddy.cn` / `www.codebuddy.cn` / `copilot.tencent.com`，intl = `www.workbuddy.ai` / `www.codebuddy.ai`。
- 文件结构：`{account, auth, accounts:[], allAccounts:[]}`；`account` 取 `uid/nickname/uin/phoneNumber/type/lastLogin`，`auth` 取 `accessToken|access_token|token`、`refreshToken`、`domain|issuer`、`expiresAt`、`refreshExpiresAt`、`lastRefreshTime`。
- `.logged-out` 标记 = `<AUTH_FILE>.logged-out`：`resolveLogoutAuth` 在"auth 目录可读且无任何 `*.info`"时才回退固定路径，否则拒绝猜测；切换成功后 `retireLogoutMarker` 先 rename 成 `*.retired.<pid>.<ts>` 再删除。
- `resolveCurrentAuth` 三分支：单记录直接用；固定文件存在用它；否则仅当唯一 `lastLogin:true` 才接受；多条 → 返回 `ambiguous`（不猜）。
- 我们的对应：`workbuddy.rs`、`codebuddy_cn.rs`；差异校验点 = 多记录歧义处理与"允许域"过滤（cockpit 侧靠 token 形态/JWT sub）。

锚点：`grep -n "DYNAMIC_AUTH_DISCOVERY\|function authRecordFromJson\|function resolveCurrentAuth\|LOGOUT_MARKER" scripts/lib.js`

## §4 新增账号（无感 OAuth）

- 时序：`auth/state` 取 state → 系统浏览器打开授权页 → 轮询 `auth/token?state=`（code ∈ {0,200} 且含 `accessToken`）→ `login/account?state=`（`Authorization: Bearer`，可选 `X-Domain`）→ 拼官方 `.info` 入库。
- 常量（`daemon.js`）：`WB_API_PREFIX='/v2/plugin'`、`OAUTH_TIMEOUT_SECONDS=600`、`OAUTH_RESULT_RETENTION_SECONDS=300`；状态表 `oauthStates: Map<loginId,{state,expiresAt,done,result,error}>`。
- `buildSeamlessAuthFile(tokenData,accData)` 构造 `{account,auth,accounts,allAccounts}`：**保留官方额外字段（`idToken`/`sessionState` 等），只覆盖标准字段**；`allAccounts` 与现文件按 uid 去重合并。
- `saveSeamlessAccount` 只写 `accounts/<uid>.info` + meta，**不触碰当前登录文件**（免退出登录的核心）。
- 路由：`POST /api/oauth/start` → `{loginId,verificationUri,expiresIn}`；`GET /api/oauth/poll`。
- 时间戳归一 `normTs`：`<1e10` 视为秒 → ×1000。

锚点：`grep -n "OAUTH_TIMEOUT_SECONDS\|function buildSeamlessAuthFile\|function oauthPollOnce" scripts/daemon.js`

## §5 用量 / 积分（官方计费接口）

- 明细拉取 `credit-request-usage.js`：`POST {apiHost}/billing/meter/get-user-request-usage`，头 `authorization: Bearer`、`x-client-platform: web`，body `{startTime,endTime,pageNum,pageSize}`（本地时间 `YYYY-MM-DD HH:mm:ss`）；`normalizeUsageRow` 保留 `requestId/requestTime/usageDate/credit/model/client/agentPurpose`；`fetchUsageSinceAnchor` 从第 1 页倒序回溯到 `anchorRequestId`，**校验时间倒序 + 分页 total 稳定**，异常即报错（不静默）。
- 历史缓存 `credit-history-sync.js`：范围 1–90 天，按"缺失日 gaps"合并区间调 `fetchUsage`，按 `requestId` 去重后按 `usageDate` 累加；今日 TTL 60s，跨零点后 `final=true` 转不可变历史；行结构 `{uid,date,used,count,queriedAt,final}`；同范围并发返回 409。
- 积分段解析 `credit-segments.js`：字段名有**多套兼容别名**（剩余 `REMAINING_FIELDS`、总量 `TOTAL_FIELDS`、过期 `EXPIRY_FIELDS`、标签 `LABEL_FIELDS`，应对官方改名）；`extractCreditSegments` 展开 `SlicePeriodUsageDetails` 且 `remaining>0` 过滤；`mergeCreditSegments` 按 `[packageCode|source, expiresAt]` 合并；`parseEnterpriseUsage` 走 `POST /v2/billing/meter/get-enterprise-user-usage`，`limitNum === -1` 表示不限量，否则剩余 = `limitNum - credit`。
- 资源查询 `credit-resource-queries.js`：body `{PageNumber:1,PageSize:100,ProductCode:'p_tcaca',Status:[0,3],PackageEndTimeRangeBegin/End}`（End = now+101 年）。
- 落库 `credit-usage-store.js`（经 `session-db.js`，`profile_id` 维度隔离）：表 `credit_usage_records`（PK `(profile_id,uid,request_id)`，唯一索引 `(profile_id,uid,usage_date,request_time)`，`ON CONFLICT DO UPDATE` 幂等，批量 200/事务）、`credit_usage_sync_state`（`anchor_request_id`）、`credit_history_days`（覆盖率标记）、`daily_checkin_records`（PK `(profile_id,uid,checkin_date)`，`response_code===10001` 表示已签到）；聚合用 `ROUND(SUM(credit),2)`。
- 路由：`POST /api/credits`（单账号积分，token 过期返回 401 `{expired:true}`）、`GET/POST /api/credit-stats`、`GET /api/credit-stats/sync`。
- 我们的对应：`api.rs::refresh_payload_for_account`（单次查询，无按天历史/无 anchor 增量）；`daily_history.rs` 是签到/派旅行归档，语义不同。缺口候选 = 按天历史 + 增量同步 + 幂等落库。

锚点：`grep -n "get-user-request-usage" scripts/credit-request-usage.js`；`grep -n "credit_usage_records\|daily_checkin_records" scripts/credit-usage-store.js`

## §6 Token 统计（扫本地会话 JSONL）

- `token-stats.js`，仅 `PROFILE.kind === 'workbuddy'`；`walkJsonl(root)` 递归 `root/projects/**/*.jsonl`（跳过 `subagents`、隐藏文件，上限 5000 文件）。
- 字段别名：输入 `input_tokens|prompt_tokens|inputTokens|promptTokens`；输出 `output_tokens|completion_tokens|…`；缓存读/写 `cache_read_input_tokens|cache_read_tokens|cached_tokens`、`cache_creation_input_tokens|…`；记录识别用递归 `findUsage()`，`isSnapshotUpdate` 跳过。
- **去重**：整行 JSON 的 `sha256` + 同文件出现序号组成 `digest:occurrence`——同一份会话被导入成副本时不会被重复计费；跨文件同一 occurrence 视为副本。
- 账号归属：优先记录里的 `accountUid/accountId/uid/userId`，否则按 `sessionId` 查 `sessions` 表 `user_id`，再按文件名/目录名匹配。
- 聚合：以 `day\0account\0model` 分桶（`aggregateBuckets`），输出 `totals/daily/models/accounts`；`models`/`accounts` 按 `input+output` 降序。
- 缓存：`.workdaddy-token-stats-cache.json`，`CACHE_VERSION=8`、`MAX_CACHE_DAYS=90`，含 `timezone` 校验、`cutoff`；今日文件按 `mtimeMs+size` 增量；`tokenStatsCacheReady()` 供快查。
- 路由：`GET /api/token-stats?days&account&model&cacheStatus`。
- 我们的对应：`commands/ai/usage.rs`（是经代理/API 记账的 token 用量，**数据源不同**）+ `components/ai/UsageStats.tsx`；缺口候选 = 从本地 WorkBuddy 会话 JSONL 统计 token（可复用我们的会话目录解析）。

锚点：`grep -n "CACHE_VERSION\|function aggregateBuckets\|function walkJsonl\|isSnapshotUpdate" scripts/token-stats.js`

## §7 积分轮换建议（积分不足时换号）

- `credit-rotation.js`（纯函数，有单测 `test/credit-rotation.test.js`）：
  - `nearestExpiringSegment(segments,now)`：过滤已耗尽/已过期，永不外过期段排最后。
  - `wasNearestSegmentConsumed(prev,next,now)`：最近到期段的 key（`expiresAt|packageCode|source`）变化即视为被消耗。
  - `selectRotationCandidate(accounts,currentUid,now)`：**过滤非当前 uid 且 `remaining>0`，按"最早过期优先、同到期剩余多优先"取首个**。
- 触发：`POST /api/credit-rotation {uid}` → `{shouldSuggest,candidate{uid,nickname,remaining,expiresAt}}`；会话结束时调用（`daemon.js` 内 `selectRotationCandidate(cachedCreditRotationAccounts(), uid, now)`）。
- 我们的对应：暂无 → 缺口候选（纯逻辑，移植成本低）。

锚点：`grep -n "function selectRotationCandidate\|function nearestExpiringSegment" scripts/credit-rotation.js`

## §8 签到

- 结果分类 `checkin-result.js`（纯函数，有单测）：`CHECKIN_PATHS=['/billing/meter/daily-checkin','/v2/billing/meter/daily-checkin']`，host 由 JWT issuer 决定（cn 追加 `codebuddy.cn` / `workbuddy.cn`）；`classifyCheckinResult({httpOk,code,message})` → `ok = !inactive && ((code===0 && httpOk) || already)`，`already = code===10001 && !inactive && ALREADY_MESSAGE.test(text)`，`inactive` 由 `INACTIVE_MESSAGE`（未开启/未开始/已过期/活动结束…）判定。
- 执行 `daemon.js::dailyCheckin(accessToken,account)`：多 endpoint 依次兜底，头含 `authorization: Bearer`、`x-user-id`、`x-domain`、`origin`/`referer`；401 优先返回"登录身份过期"；`CHECKIN_REQUEST_TIMEOUT_MS=12000`。
- `performAccountCheckin(uid)` 时序：SQLite 已确认 → skip；cache 命中 → 迁移到 SQLite；否则 `refreshAccountBackupToken`（惰性刷新）→ 取 token → 请求 → 写 `checkin-cache.json`（合并写）→ 成功写 SQLite。结果 `{uid,date,ok,already,inactive,code,message,at,verified}`；并发按 uid 去重（`checkinClaims`）。
- 授权（consent，opt-in）`checkin-consent.js`：`TASK_ID='daily-account-checkin'`，状态文件 `checkin-consent.json` `{version:1,initialized,decision,decidedAt}`；首次把内置任务 `enabled=false` 并 `decision:null`；**首个决策胜出**；写任务开关失败要回滚。路由 `GET/POST /api/automations/checkin-consent`。
- 内置任务 `scripts/builtin/automations/daily-account-checkin.json`：`trigger.types=['clientLoaded','panelOpened']` + `schedule {type:'interval',minutes:60}` + `enabled:false`，步骤 `account.forEach(switch:false)` → `account.checkin` → 断言 ok，成功间 `delay 250ms`。
- **注意**：WorkDaddy **没有**"随机时间窗"（是固定 60 分钟 interval + opt-in）；随机窗口是我们从 cockpit 学来的语义，勿反向覆盖。
- 我们的对应：`api.rs`（`perform_checkin` 语义）+ `auto_checkin.rs`（随机窗口调度）；可借鉴 = 多 endpoint 兜底顺序、"已签到"识别（`code===10001`）、inactive 文案判定、每日幂等（cache + 落库双写 + 惰性刷新 token）。

锚点：`grep -n "10001\|INACTIVE_MESSAGE\|CHECKIN_PATHS" scripts/checkin-result.js`；`grep -n "function dailyCheckin\|function performAccountCheckin" scripts/daemon.js`

## §9 会话列表

- 数据源 `profiles.js::sessionDb`：WorkBuddy `~/.workbuddy/workbuddy.db`（AI：`~/.workbuddy-ai/workbuddy.db`）；CodeBuddy `codebuddy-sessions.vscdb`。
- 访问层 `session-db.js`：`createSessionDb` 双后端（`node:sqlite` `DatabaseSync({readOnly:true})` 或 `sqlite3` CLI `-readonly -json`）；**安全闸**：`normalizeParameters` 类型白名单、`assertNoSqliteMetaCommands`、`assertSingleStatement`、`assertNoTransactionControl`；批量 id 限 `MAX_SESSION_ID_BATCH=100`、`MAX_SESSION_ID_LENGTH=200`。
- 列表查询：WorkBuddy 直接 SQL；CodeBuddy 走 `ItemTable WHERE key LIKE 'session:%'` 把 JSON 映射成行（`conversationId/userId/title/cwd/status/createdAt/updatedAt/isPlayground/mode/model`）。
- `GET /api/sessions?uid&range`：`uid` 缺省=当前账号、空=全部；`range ∈ today|7d|30d|all`；排序 `ORDER BY COALESCE(last_activity_at,updated_at,created_at) DESC, created_at DESC`，只取 `deleted_at IS NULL`。
- 辅助：`sessionPayloadExists`（判断会话正文目录是否真存在）、`repairMissingSessionWorkspaces`（仅 Win+workbuddy，每 5s 最多 2000 行）。
- 我们的对应：`sessions.rs`（已是 SQL/键扫描 + 过滤排序）；可借鉴 = 正文存在性探测与 cwd 修复。

锚点：`grep -n "function createSessionDb\|assertNoSqliteMetaCommands\|MAX_SESSION_ID_BATCH" scripts/session-db.js`

## §10 切换账号与假退出登录

- 文件层 `lib.js::switchTo(dataDir,uid,log)`（**不关客户端**）：
  1. 校验 `capabilities.accounts` → `migrateLegacyDataDir`；
  2. 读 `accounts/<uid>.info` 并**校验 `acct.uid === uid`**，不匹配即中止；
  3. `resolveAuthTarget` 决定写入目标（§1 决策链，拒绝猜测）；
  4. 原子写 `<target>.wbswitch.tmp` → rename → `chmod 0600`（macOS 沙箱 EPERM 才走 `osascript` 桥接，Windows 直接抛错）；
  5. `retireLogoutMarker` 清理 `.logged-out`；
  6. `updateMeta` 自愈"账号→登录文件"绑定与 domain/issuer；
  7. 不修改官方加密/校验字段，靠 uid 一致性 + 解析有效性校验。
- 上层 `POST /api/switch`（`{uid,reload}`）完整时序：`beginRendererReloadPriority`（注入依赖）→ `switchTo` → 若 `reload`：记录切换前选中会话标题 → `reloadWorkBuddyPage()`（`Page.getFrameTree` → 预挂注入 → `Page.reload` → 等组件挂载，超时 10s/5s）→ 兜底派发 `pageReady` → `autoFocusSessionByTitle`（虚拟滚动扫描 `.conversation-list`，800ms/轮约 30 轮）→ **CDP 刷新失败只记日志不抛错** → 自动复制任务（§12）→ 返回 `{ok,uid,nickname,reloaded,autoCopy,hint}`。
- 假退出 `POST /api/logout`：`resolveLogoutAuth` 无法唯一确认 → 409；**先 `quitWorkBuddy()` 优雅退出宿主**（避免内存里的旧身份被写回）→ 删登录文件 → 重新打开；token 备份保留。
- 我们的对应：`mod.rs::buddy_switch_account`（关客户端→合并→写回→重启，按 cockpit 时序，**勿与本节混用**）、`workbuddy.rs`/`codebuddy_cn.rs` 写回、`client_process.rs`。可借鉴 = `switchTo` 的 uid 一致性校验 + 目标决策链 + `.logged-out` 清理 + 假退出时序。

锚点：`grep -n "function switchTo\|function resolveAuthTarget" scripts/lib.js`；`grep -n "'/api/switch'\|'/api/logout'" scripts/daemon.js`

## §11 会话归档导入导出（`.wds` v4）

- `session-transfer.js`（有单测 `test/session-transfer.test.js`）：
  - `MAGIC = 'WDS4\r\n\x1a\n'`、`HEADER_SIZE = 52`、`MAX_METADATA = 1MB`、`MAX_FILES = 20000`。
  - 头 52B 布局：`magic(8) + salt(16, [8,24)) + iv(12, [24,36)) + authTag(16, [36,52))`；**GCM 的 AAD = header[0,36)**（含 magic+salt+iv，防头篡改）。
  - 写出 `writeSessionTransfer(file,sessions,password)`：`scrypt(password,salt,32)` + `aes-256-gcm`，管道 `gzip → cipher`，`flag:'wx'`、0600，失败即删。
  - 读入 `readSessionTransfer(file,password,staging)`：**先验证完整 GCM tag 再解压**；逐帧解析（长度前缀），校验 id 正则、路径、重复 id、`meta.count ≤ MAX_FILES`、无多余数据；失败清理 staging。
  - `receiveSessionUpload(input,directory)` 把上传体写入 staging `<uuid>.wds`（元数据 `{password,targetUid}`）。
  - `remapSessionArchivePath(relativePath,oldId,newId)`：4 条正则映射（`projects/<hash>/<id>.jsonl`、`projects/<hash>/<id>/`、`workspace/sessions|tasks|file-history`、`artifact-index`），`resolveArchiveTarget` 防目录穿越。
- 路由：`POST /api/sessions/export`（≤100）、`POST /api/sessions/import`（支持 override uid；失败 `deleteSessionFiles` 回滚）。
- 我们的对应：**暂无**（`session_transfer/` 只做跨账号合并）→ 缺口候选；移植价值在于"会话可加密搬迁 + 导入校验"。

锚点：`grep -n "WDS4\|HEADER_SIZE\|setAAD" scripts/session-transfer.js`

## §12 会话合并 / 迁移（auto-copy lineage）

- 目录布局（WorkBuddy dataRoot）：`projects/<项目hash>/<id>.jsonl` 与 `projects/<项目hash>/<id>/`（正文）、`workspace/sessions/<id>/`、`tasks/<id>/`、`file-history/<id>/`、`artifact-index/<id>.json`；`app/sessions.json` 是窗口缓存（删除时只移条目）。
- `copySessionFiles(wbHome,oldId,newId,lineageIds)`：异步 `cp` 复制上述 5 类路径；**`artifact-index` 只重映射 `_meta.ownerConversationId ∈ {oldId,…lineageIds}` 的条目**并保留原 mtime。
- `sessionContentMtime`：以消息文件 mtime 为权威时间（不用 DB 字段）。
- `syncAutoCopyLineage(lineageId,targetUid)`：取 lineage 全部成员 → 查 `sessions` 活行 → 选最新成员（内容 mtime 优先）→ 复制到其他成员 → `UPDATE sessions SET title,custom_title,status,updated_at,last_activity_at WHERE id=? AND user_id=?`。
- `insertCopiedSession`：INSERT 到 `sessions`，**`user_id = targetUid`**（user_id 重映射核心），列集合 `SESSION_COPY_COLUMNS`（21 列）。
- `copySessionRecord(src,targetUid,options)`：优先复用 mapping（校验 `user_id===targetUid`）→ 否则按 lineage 成员找 canonical（created_at/updated_at 排序）→ 否则新建；带 `sessionCopyLocks` 每 `[lineageId,targetUid]` 串行锁。
- `buildAutoCopyPlan(sourceUid,targetUid)`：`normalizeAutoCopyLineages`（保证"每账号每 lineage 一条物理会话"）→ `getAutoCopyRules` → `dedupeAutoCopySessionRows` → `isAutoCopySessionSelected`；规则存 `meta.json.autoCopy`（v1→v2 迁移）。
- 注入依赖：`yieldAutoCopyToRenderer`（让位 renderer 重载/注入）、renderer 优先级抢占。
- 我们的对应：`session_transfer/`（按 cockpit 的"工作区目录 + index.json + 辅助目录 + DB user_id 重映射"模型）；**两边模型不同**（参考按项目 hash 目录 + lineage 概念）。学习时用作边界条件参考（如 artifact-index 归属校验、内容 mtime 权威性），不要整体替换我们的实现。

锚点：`grep -n "function syncAutoCopyLineage\|function copySessionRecord\|function buildAutoCopyPlan\|SESSION_COPY_COLUMNS" scripts/daemon.js`

## §13 profile / 客户端识别与能力矩阵

- `profiles.js` 四 profile：`workbuddy-cn`（`appName:'WorkDaddy'`，authFile `workbuddy-desktop.info`，sessionDb `~/.workbuddy/workbuddy.db`，apiHost `https://www.codebuddy.cn`，`capabilities{accounts,sessions,models,stashPrompt,theme,checkin}` 全 true）、`workbuddy-ai`（`-ai.info`、`~/.workbuddy-ai`、`www.workbuddy.ai`）、`codebuddy-cn`（authFile null、`codebuddy-sessions.vscdb`、只 `sessions/checkin`）、`codebuddy-intl`（同前，`www.codebuddy.ai`）。
- `workbuddy-target.js`：客户端发现/自定义目标（`WBSWITCH_WORKBUDDY_BIN`/`_DIR`/`_VERSION`、`workbuddy-target.json`），企业版走 `apiHost` + `targetHints`。
- 端口：`ui-port.js` 每 profile 固定候选池（cn `47832/17832/27832/37832`、ai `47833…`、cbcn `47834…`、cbintl `47835…`）；CDP 端口 `{cn:9222,ai:9223,cbcn:9224,cbintl:9225}`，探测候选 = hint + 持久化 + `9222..9232` + `9333`。
- `cdp-targets.js`：`normalizeTargetUrl`（`%20`→空格）、`classifyTarget(url,title,desc)`（app 包路径/登录域名强信号 → profile id，无信号返回 null）、`isTargetForProfile`（只接受 `type==='page'`；CodeBuddy 走 `kind==='codebuddy'` 兜底并排除 devtools/chrome/about）。
- 我们的对应：`client_process.rs`（进程/路径匹配）、`models.rs::BuddyPlatform`（只两平台）；可借鉴 = 能力矩阵思路与"多候选必须给出唯一强信号否则拒绝"的防御式识别。

锚点：`grep -n "id: 'workbuddy-cn'\|capabilities:" scripts/profiles.js`；`grep -n "function classifyTarget\|function isTargetForProfile" scripts/cdp-targets.js`

## §14 自动化任务

- 模型（`automation.js`，`SCHEMA_VERSION=1`；`MAX_TASKS=200`、`MAX_STEPS=200`、`MAX_DEPTH=12`、文本 ≤20000；id 正则 `/^[A-Za-z0-9][A-Za-z0-9_-]{0,79}$/`）：
  `{schemaVersion,id,name,description,enabled,trigger{type,types[],oncePerNavigation,restartOnNavigation},schedule{...},variables,steps[],onSuccess[],onFailure[]}`
- 触发器：`manual|pageReady|pageLoaded|accountSwitched|clientLoaded|panelOpened`（组合用 `trigger.types[]`，空数组=仅手动）。
- 调度 `schedule.type`：`manual|interval(1–10080 分钟)|daily|weekly(days 0–6)|monthly(day 1–31)|once`（本地时区，`HH:MM`；缺失日期跳过）；`createScheduleTicker` 把 slot 落 `automation-schedule-state.json`，**先持久化再派发**，不补跑、运行中跳过。
- 步骤动作（`CAPABILITIES`/`SUPPORTED_OPS`）：`logic.*`（sequence/delay/if/switch/repeat/retry/catch/assert/forEach/break/waitUntil）、`account.*`（forEach{switch}/status/checkin/list/getCurrent/getPrimary）、`http.request`/`http.requestAsAccount`、`dom.*`（find/click/type/clear/press/readText/readAttribute/wait，locator 支持 css/xpath/text/role/ariaLabel/placeholder/attribute/coordinates）、`session.create/send/wait`、`state.*`、`vars.set`、`notify.toast/dismiss/afterAllTasks`；模板 `{{now}} {{event.*}} {{account.*}} {{vars.*}} {{response.*}} {{step.*}}`。
- 执行 `executeTask(taskInput,options)`：先 `validateSteps`（拒绝未支持 op/非法 locator/`state.*` 缺 scope/**禁止 `session.*` 进 `logic.retry`** 防重复发送）→ 跑 `steps` → 成功 `onSuccess`、失败写 `vars.error` 后 `onFailure`；`automation-runtime.js` 提供 `assertAccountRequestUrl`（账号请求仅限官方 HTTPS origin）、`createTaskState`、`cancellableWait`、`createRendererGate`、`probeSessionReceipt`。
- 导入导出：单任务 `.json`、多任务 `.zip`（`MAX_ENTRIES=512`，严格 store/deflate，拒绝 ZIP64/加密/分卷/symlink，CRC 校验）；导入**预览非权威**——提交时重读重校验；无 id 的本地文件任务赋 `id='import_'+sha256.slice(0,32)`。
- 兼容性 `automation-compatibility.js`：`requires` 允许键 `minWorkDaddyVersion/taskSchemaVersion/capabilities/profiles/platforms`（+`x-*`）；`taskSchemaVersion` 必须为 1。
- 任务包 `automation-packages.js` + `schemas/automation-package.v1.schema.json`：`kind='workdaddy.automation-package'`、`formatVersion=1`；`analyzeTask` 推导副作用（account-switch/checkin/send-message/page-input/network/account-credentials）；`inputs` ≤40，键禁 `__proto__/constructor/prototype`。
- 内置任务 `scripts/builtin/automations/`：`daily-account-checkin.json`、`close-buddy-popups.json`、`keep-accounts-active.json`（`installBuiltinTask` 用 revision/contentHash 增量升级，不覆盖用户改动）。
- 路由：`GET /api/automations`、`POST /api/automations`、`/run`、`/run-status`、`/stop`、`/bulk`、`/validate`、`/dry-run`、`/export`、`/import(/preview)`、`/packages/preview`、`/agent-info`、`/agent-generate`、`/events`。
- 我们的对应：**暂无** → 最大缺口候选。纯逻辑部分（模型/校验/调度/导入导出/兼容性判定）可移植；`dom.*`/`session.*` 步骤依赖注入，需另行设计降级（我们的优势是有 Tauri command 可做等价动作）。

锚点：`grep -n "SCHEMA_VERSION\|MAX_TASKS\|const CAPABILITIES\|function validateTask\|function executeTask" scripts/automation.js`；`node --test test/automation*.test.js`

## §15 模型管理 / 第三方模型（CC Switch 导入）

- `third-party-models.js`：`discoverCCSwitch()` 找 `~/.cc-switch/{cc-switch.db,config.json}`（sqlite 或 json 两种格式，注释对齐 `farion1231/cc-switch` 某 commit）；`parseRoutingToml(text)`（OpenCode 风格 routing toml）；`convertProvider(provider,env)` 把第三方供应商转成目标 `models.json` 结构；`createThirdPartyImport` 提供 `discover/preview/import`，写目标前先备份成 `<file>.before-cc-switch-<uuid>.bak`。
- 路由：`GET /api/models`（官方+备份+跨端导入源）、`POST /api/models/import`、`/api/models/third-party(GET)`、`/preview`、`/import`、`/backup`、`/delete-official`、`/test`（连通测试）、`/copy`、`/edit`、`/delete`、`/enable`。
- 我们的对应：`commands/ai/models.rs`、`provider.rs`、`components/ai/ModelConfig.tsx`（已有多供应商/模型配置）；可借鉴 = 从 cc-switch / cc-switch.db 一键导入供应商 + 导入前自动备份 + 端点连通探测。

锚点：`grep -n "function discoverCCSwitch\|function convertProvider\|before-cc-switch" scripts/third-party-models.js`

## §16 增强开关（多为注入依赖）

| 能力 | 参考位置 | 依赖 | 可移植性 |
|---|---|---|---|
| 暂存提示词（stash） | `inject.js` `insertStash`/`positionStash`/`shouldShowStash`/`syncStash`；`daemon.js` `/api/stash*` | 官方发送按钮/工具栏 DOM + 官方消息队列适配器 | 注入依赖；仅"入队后暂停自动发送、按会话独立保存"的语义可参考 |
| 快捷短语 | `inject.js` `renderExploreOptions`；`daemon.js` `/api/quick-phrase*`、`/api/quick-phrases/{export,import}`（加密） | 同上 | 数据/导入导出可搬，悬浮菜单不可 |
| 权限弹窗免打扰 | `inject.js` `classifyNoDisturbApprovalCandidate`（纯函数）+ `ND_ALLOW_ONCE_LABELS`/`ND_ALLOW_SESSION_LABELS`/`ND_*_HINTS`/`ND_MAX_DECISION_BUTTONS=12`、`startNoDisturbAutoApprove`/`scanNoDisturbApproval`/`toNdAudit`；`daemon.js` `/api/no-disturb*` | 官方确认弹窗 DOM | 判定规则（含"凭证外发/批量删除/提权确认不误点"白名单）可借鉴；点击必须注入 |
| 异常中断自动续接 | `inject.js` `classifyAutoContinueReply`/`classifyAutoContinueControllerSnapshot`/`autoContinueControllerCompleted`/`selectAutoContinueAssistant`、`AC_CONTINUE_TEXT`、`wireAutoContinuePane`；`daemon.js` `/api/auto-continue*` | 官方 controller/messageStore 回执 | 状态机语义可借鉴；实现必须注入 |
| 防休眠 | `daemon.js` `applySleepMode`/`startCaffeinate`/`startUserActivityLoop`/`sleepNow`/`restoreSleepMode`；模式 `allow|keep|until-done` | 系统 API（macOS `caffeinate`、Win `SetThreadExecutionState`）；"until-done" 需注入判定会话忙碌 | **防休眠本体是纯本地逻辑**，可在 Tauri 侧实现（跨平台需自选 crate / 平台 API） |
| 主题（毛玻璃/壁纸） | `daemon.js` `themeVarsCss`/`themeExtrasCss`/`applyThemeByCdp`/`restoreSavedTheme`；`theme-vars.js`、`theme-patches.js`（`[{id,desc,css}]` 按 `themeId`/`setting` 过滤）、`theme-text-shadow.js` | 官方主题 token（`--wb-*`/`--cb-*`、`data-vscode-theme-name`）+ DOM 注入 | CSS 变量层/补丁表/蒙版雾化参数可搬；最终注入不可 |

## §A 参考侧本地文件与数据目录（排查用）

```
%APPDATA%\WorkDaddy\            （Win 数据根；mac ~/Library/Application Support/WorkDaddy）
  accounts\<uid>.info           账号备份（原子写 0600）
  meta.json                     账号元数据 + autoCopy 规则 + accountOrderMode
  primary-account.json          主账号
  models\                       模型备份
  checkin-cache.json            签到每日缓存        checkin-consent.json  签到授权状态
  automation-schedule-state.json 定时槽位           automation-builtins.json 内置任务版本
  .api-token / ui-port.json / cdp-port.json / daemon.log / .daemon.lock
  profiles\<id>\                非 cn profile 的数据目录
%LOCALAPPDATA%\CodeBuddyExtension\Data\Public\auth\workbuddy-desktop[.info|-ai.info]
~/.workbuddy\{workbuddy.db, projects\<hash>\<id>.jsonl, workspace\, tasks\, file-history\, artifact-index\, models.json, settings.json}
~/.workbuddy-ai\...             国际版同构
%APPDATA%\CodeBuddy CN\codebuddy-sessions.vscdb
环境变量：WBSWITCH_{AUTH_FILE,DATA_DIR,PROFILE,SHARED_DATA_DIR,CDP_PORT,PORT,WORKBUDDY_BIN,DIR,VERSION,TARGET_PLATFORM}
```

## §B 本地 HTTP API 路由（接口语义参考，不引入实现）

- 鉴权：仅 `127.0.0.1`；`isApiRequestAuthorized` —— 有 `Origin` 且非 loopback/官方域即拒绝；免 token 白名单仅 `/api/status`、`/api/about`、`/api/update-check`、`/api/update-status`；`POST /api/inject` 无 Origin 时放行；其余必须 `X-WorkDaddy-Token`（`timingSafeEqual`）。
- 路由族（详见 `daemon.js::handleApi`，2026-09-17 时点）：注入与交互 `/api/inject`、`/api/cdp-click`、`/api/click`、`/api/find`、`/api/clear-composer`、`/api/devtools-url`；账号 `/api/status`、`/api/accounts`(+`/primary`、`/order`)、`/api/switch`、`/api/logout`、`/api/backup`、`/api/delete`、`/api/current`、`/api/oauth/{start,poll}`、`/api/accounts/{export,import}`；用量 `/api/credits`、`/api/credit-rotation`、`/api/token-stats`、`/api/credit-stats(+/sync)`、`/api/growth/*`；会话 `/api/sessions(+ /workspaces、/auto-copy、/export、/import、/copy、/migrate、/delete、/restore)`；模型 `/api/models*`；自动化 `/api/automations*`；增强 `/api/{ask-mode,no-disturb,auto-continue,session-module,quick-phrase*}`；主题 `/api/{themes,wallpapers,custom-wallpapers,mask,blur,theme-text-shadow,theme-apply,theme-save}`;休眠 `/api/{sleep-mode,sleep-now}`；杂项 `/api/stash*`、`/api/open-url`、`/api/open-dir`、`/api/about`、`/api/update-*`。
- 用途：当我们要给某个能力设计 Tauri command 时，来这里查参考的请求/响应字段，避免自创字段名。

## §C 可移植性三分类（速判）

- **纯逻辑（可直接转 Rust）**：`lib.js` 账号解析/决策链/原子写、`secure-transfer.js`、`session-transfer.js`、`session-db.js` 安全闸、`token-stats.js`、`credit-segments.js`、`credit-rotation.js`、`credit-resource-queries.js`、`credit-usage-store.js`、`third-party-models.js`、`automation*.js`、`theme-*.js`、`ui-port.js`、`cdp-targets.js`、`daemon.js` 的休眠控制与主题 CSS 生成。
- **官方接口（可转 Rust HTTP 客户端）**：`credit-request-usage.js`、`credit-history-sync.js`、OAuth `auth/state|token` + `login/account`、签到 `daily-checkin`、企业用量 `get-enterprise-user-usage`。
- **注入依赖（不可照搬，只借语义）**：面板与各 pane、暂存/快捷短语按钮插入、免打扰点击、自动续接发送、`until-done` 忙碌判定、主题最终注入、`automation-picker.js` 元素拾取、`reloadWorkBuddyPage`/`autoFocusSessionByTitle`/renderer 优先级抢占。

## §D WorkDaddy 独有 → any-version 缺口候选（按性价比排序）

1. **积分按天历史 + anchor 增量同步 + 幂等落库**（§5）：补 `api.rs` 只做单次查询的不足。
2. **Token 统计（本地会话 JSONL，含副本去重）**（§6）：可与 `UsageStats` 合并展示。
3. **积分轮换建议**（§7）：纯函数，成本最低。
4. **会话加密归档导入导出 `.wds` v4**（§11）：让会话可跨机搬迁，格式可自定义但建议对齐参考头布局。
5. **主账号 + 账号排序**（§2）。
6. **假退出登录**（§10）：先退宿主再删登录文件，token 保留。
7. **第三方模型（cc-switch）导入 + 端点连通测试**（§15）。
8. **自动化任务**（§14）：最大工程，建议先做"任务模型 + 校验 + 定时器 + JSON/ZIP 导入导出"，步骤动作按我们已有的 Tauri 能力子集实现。
9. **防休眠（含 until-done）**（§16）：纯本地逻辑 + 我们的忙碌判定来源。

## §Z 漂移校验锚点（学习前先跑）

```bash
cd E:/pro/other-sdk/buddy/WorkDaddy
grep -n "DAEMON_VERSION\|DAEMON_BUILD_ID" scripts/daemon.js | head -3
grep -n "EXPORT_VERSION\|MAGIC = Buffer" scripts/secure-transfer.js scripts/session-transfer.js
grep -n "SCHEMA_VERSION\|const CAPABILITIES" scripts/automation.js
grep -n "CACHE_VERSION" scripts/token-stats.js
grep -n "10001" scripts/checkin-result.js
node --test test/*.test.js   # 参考侧全量行为规范（改动会在这里体现）
```

以上任一项与本文档不符 → 该模块的契约以源码为准，学完后回写本文件并在 `sync-point.workdaddy.txt` 记录。
