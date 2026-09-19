# WorkDaddy 契约清单（主参考）

- 时点：WorkDaddy `7126432f`（分支 `main`），`DAEMON_VERSION = '1.2.76'`，`DAEMON_BUILD_ID = 'release-1.2.76-20260919-session-fork-stream-alignment-fix'`，2026-09-19 精读（在 `a0cdb0a0` / 1.2.42 基础上增量：§17 会话同步明细、§18 会话分支、§19 成长计划、§20 自动化 V2 + 公开任务发现 + 安全评估 + 模型选择）。
- 形态：纯 Node.js（无 `package.json` 构建管线）+ 本地回环 HTTP daemon（`scripts/daemon.js`）+ 注入到 WorkBuddy renderer 的面板（`scripts/inject.js`）。**真源是 `scripts/`，行为规范是 `test/`（每个模块都有同名测试）。**
- 本文件只记录**语义 / 格式 / 算法 / 时序**，不复制源码。每节末尾有 `锚点:`，学习前先跑 `§Z` 的检查确认未漂移；漂移了以源码为准并回写本文件。
- 文中「注入依赖」= 该能力靠 `Runtime.evaluate` 注入 renderer 实现，Rust/Tauri 无法照搬，只借判定规则与交互语义。
- **平台范围**：WorkDaddy 只有 WorkBuddy（cn / ai）两个 profile，本文件学到的语义默认只落到 any-version 的 WorkBuddy 分支；除纯逻辑（统计聚合、任务模型、调度器、副作用分析）外不得据此改 CodeBuddy CN 行为。

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
- **1.2.76 新增 `dailyBreakdown`**：`aggregateCachedBuckets` 除上述四类外再吐一个**扁平行数组** `dailyBreakdown[] = { day, account, model, input, output, cacheRead, cacheWrite, calls }`，按 `day → account → model` 排序。用途 = 前端在"按天"图上叠加"按账号/按模型"筛选时不需要重新扫 JSONL。学习要点：聚合层一次算全维度、筛选在内存里做，**不要为每种筛选组合重扫文件**。
- 缓存：`.workdaddy-token-stats-cache.json`，`CACHE_VERSION=8`、`MAX_CACHE_DAYS=90`，含 `timezone` 校验、`cutoff`；今日文件按 `mtimeMs+size` 增量；`tokenStatsCacheReady()` 供快查。
- 路由：`GET /api/token-stats?days&account&model&cacheStatus`。
- 我们的对应：`commands/ai/usage.rs`（是经代理/API 记账的 token 用量，**数据源不同**）+ `components/ai/UsageStats.tsx`；缺口候选 = 从本地 WorkBuddy 会话 JSONL 统计 token（可复用我们的会话目录解析）。

锚点：`grep -n "CACHE_VERSION\|function aggregateBuckets\|function walkJsonl\|isSnapshotUpdate" scripts/token-stats.js`

## §7 积分轮换建议（积分不足时换号）

- `credit-rotation.js`（纯函数，有单测 `test/credit-rotation.test.js`）：
  - `nearestExpiringSegment(segments,now)`：过滤已耗尽/已过期，永不外过期段排最后。
  - `wasNearestSegmentConsumed(prev,next,now)`：最近到期段的 key（`expiresAt|packageCode|source`）变化即视为被消耗。
  - `selectRotationCandidate(accounts,currentUid,now)`（1.2.76 语义，已修正）：过滤非当前 uid → 各自取 `nearestExpiringSegment` → **只保留 `expiresAt !== null` 的候选**（永不外过期段不算可轮换）→ 按"最早过期优先、同到期剩余多优先"排序取首个；**再与当前账号自己的最近到期段比较：若当前账号的段更早或同时到期（`currentSegment.expiresAt <= nearest.expiresAt`）则返回 `null`**（没有轮换价值就不建议换号）。注意 `remaining` 不再参与过滤，只参与同到期排序。
- 触发：`POST /api/credit-rotation {uid}` → `{shouldSuggest,candidate{uid,nickname,remaining,expiresAt}}`；会话结束时调用（`daemon.js` 内 `selectRotationCandidate(cachedCreditRotationAccounts(), uid, now)`）。
- 我们的对应：暂无 → 缺口候选（纯逻辑，移植成本低）。

锚点：`grep -n "function selectRotationCandidate\|function nearestExpiringSegment" scripts/credit-rotation.js`

## §8 签到

- 结果分类 `checkin-result.js`（纯函数，有单测）：`CHECKIN_PATHS=['/billing/meter/daily-checkin','/v2/billing/meter/daily-checkin']`，host 由 JWT issuer 决定（cn 追加 `codebuddy.cn` / `workbuddy.cn`）；`classifyCheckinResult({httpOk,code,message})` → `ok = !inactive && ((code===0 && httpOk) || already)`，`already = code===10001 && !inactive && ALREADY_MESSAGE.test(text)`，`inactive` 由 `INACTIVE_MESSAGE`（未开启/未开始/已过期/活动结束…）判定。
- 执行 `daemon.js::dailyCheckin(accessToken,account)`：多 endpoint 依次兜底，头含 `authorization: Bearer`、`x-user-id`、`x-domain`、`origin`/`referer`；401 优先返回"登录身份过期"；`CHECKIN_REQUEST_TIMEOUT_MS=12000`。
- `performAccountCheckin(uid)` 时序：SQLite 已确认 → skip；cache 命中 → 迁移到 SQLite；否则 `refreshAccountBackupToken`（惰性刷新）→ 取 token → 请求 → 写 `checkin-cache.json`（合并写）→ 成功写 SQLite。结果 `{uid,date,ok,already,inactive,code,message,at,verified}`；并发按 uid 去重（`checkinClaims`）。
- ~~授权（consent，opt-in）`checkin-consent.js`~~：**1.2.76 已删除该模块与配套测试**（`scripts/checkin-consent.js`、`test/checkin-consent*.test.js` 均移除，`/api/automations/checkin-consent` 路由不存在）。签到直接由自动化任务 `daily-account-checkin` 承担，授权改由用户在自动化面板显式启用任务来表达。**不要再按旧契约实现 consent 流程。**
- 内置任务 `scripts/builtin/automations/daily-account-checkin.json`：`trigger.types=['clientLoaded','panelOpened']` + `schedule {type:'interval',minutes:60}` + `enabled:false`，步骤 `account.forEach(switch:false)` → `account.checkin` → 断言 ok，成功间 `delay 250ms`；1.2.76 起按协议 V2 加了 `log.write`（走 `params` 占位符 + 脱敏），见 §20。
- **注意**：WorkDaddy **没有**"随机时间窗"（是固定 60 分钟 interval）；随机窗口是我们从 cockpit 学来的语义，勿反向覆盖。
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

> **⚠️ 决策（2026-09-19）：本能力已评估但否决，不要重新实现。**
> 曾按本节点完整落地过一版（协议 V2 模型/校验/调度/执行器/发现/安全评估 + 内置「派猫猫旅行」任务 + 前端面板，
> Rust 约 3360 行 + TSX 688 行），随后**整体删除**。原因：
> 1) 参考任务的真正价值在 `dom.*` / `session.*` 与页面类触发——它们全部依赖向 WorkBuddy renderer 注入脚本（CDP），
>    我们的 Tauri 独立窗口没有这条链路，只能把一大半 op 标成"不可用"；
> 2) 剩下能跑的部分（账号遍历 + 官方接口 + 按天幂等 + 定时）**与我们既有的 `auto_checkin.rs` / `auto_travel.rs` 完全重叠**，
>    等于用 3000 行通用解释器重新实现两个已经写好的调度器；
> 3) 导入导出/发现/安全评估的价值依赖"社区能写出我们跑不了的任务"，收益与体量不成比例。
> 因此：**只有当我们将来真的具备页面注入能力（或有了非注入类的高价值任务生态）时才重新评估。**
> 下面保留完整契约，仅作边界知识与"如果要做该怎么做"的参考。

- 模型（`automation.js`，1.2.76 起 `SCHEMA_VERSION=2` + `SUPPORTED_SCHEMA_VERSIONS={1,2}`，见 §20；`MAX_TASKS=200`、`MAX_STEPS=200`、`MAX_DEPTH=12`、文本 ≤20000；id 正则 `/^[A-Za-z0-9][A-Za-z0-9_-]{0,79}$/`）：
  `{schemaVersion,id,name,description,enabled,trigger{type,types[],oncePerNavigation,restartOnNavigation},schedule{...},variables,steps[],onSuccess[],onFailure[]}`
- 触发器：`manual|pageReady|pageLoaded|accountSwitched|clientLoaded|panelOpened`（组合用 `trigger.types[]`，空数组=仅手动）。
- 调度 `schedule.type`：`manual|interval(1–10080 分钟)|daily|weekly(days 0–6)|monthly(day 1–31)|once`（本地时区，`HH:MM`；缺失日期跳过）；`createScheduleTicker` 把 slot 落 `automation-schedule-state.json`，**先持久化再派发**，不补跑、运行中跳过。
- 步骤动作（`CAPABILITIES`/`SUPPORTED_OPS`）：`logic.*`（sequence/delay/if/switch/repeat/retry/catch/assert/forEach/break/waitUntil）、`account.*`（forEach{switch}/status/checkin/list/getCurrent/getPrimary）、`http.request`/`http.requestAsAccount`、`dom.*`（find/click/type/clear/press/readText/readAttribute/wait，locator 支持 css/xpath/text/role/ariaLabel/placeholder/attribute/coordinates）、`session.create/send/wait`、`state.*`、`vars.set`、`notify.toast/dismiss/afterAllTasks`；模板 `{{now}} {{event.*}} {{account.*}} {{vars.*}} {{response.*}} {{step.*}}`。
- 执行 `executeTask(taskInput,options)`：先 `validateSteps`（拒绝未支持 op/非法 locator/`state.*` 缺 scope/**禁止 `session.*` 进 `logic.retry`** 防重复发送）→ 跑 `steps` → 成功 `onSuccess`、失败写 `vars.error` 后 `onFailure`；`automation-runtime.js` 提供 `assertAccountRequestUrl`（账号请求仅限官方 HTTPS origin）、`createTaskState`、`cancellableWait`、`createRendererGate`、`probeSessionReceipt`。
- 导入导出：单任务 `.json`、多任务 `.zip`（`MAX_ENTRIES=512`，严格 store/deflate，拒绝 ZIP64/加密/分卷/symlink，CRC 校验）；导入**预览非权威**——提交时重读重校验；无 id 的本地文件任务赋 `id='import_'+sha256.slice(0,32)`。
- 兼容性 `automation-compatibility.js`：`requires` 允许键 `minWorkDaddyVersion/taskSchemaVersion/capabilities/profiles/platforms`（+`x-*`）；`taskSchemaVersion` 必须为 1。
- 任务包 `automation-packages.js` + `schemas/automation-package.v1.schema.json`：`kind='workdaddy.automation-package'`、`formatVersion=1`；`analyzeTask` 推导副作用（account-switch/checkin/send-message/page-input/network/account-credentials）；`inputs` ≤40，键禁 `__proto__/constructor/prototype`。
- 内置任务 `scripts/builtin/automations/`：`daily-account-checkin.json`、`close-buddy-popups.json`、`keep-accounts-active.json`（`installBuiltinTask` 用 revision/contentHash 增量升级，不覆盖用户改动）。
- 路由（1.2.76 实测，`daemon.js` 行号见锚点）：`GET /api/automations`、`POST /api/automations`、`/run`、`/run-status`、`/stop`、`/bulk`、`/validate`、`/dry-run`、`/export`、`/import(/preview)`、`/packages/preview`、`/agent-info`、`/agent-generate`、`/events`、`/capabilities`、`/logs/clear`，以及 1.2.76 新增的 `/discovery`、`/discovery/import`、`/safety-review`（§20）。
- 我们的对应：**暂无** → 最大缺口候选。纯逻辑部分（模型/校验/调度/导入导出/兼容性判定/发现/副作用分析）可移植；`dom.*`/`session.*` 步骤依赖注入，需另行设计降级（我们的优势是有 Tauri command 可做等价动作）。

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

## §17 会话同步明细（1.2.76 新增，账号切换后只处理有变化的会话）

- 参考位置：`daemon.js` 的 `syncAutoCopyLineage` / `copySessionRecord` / `buildAutoCopyPlan` / `startAutoCopyJob` / `publicAutoCopyJob` / `activeAutoCopyJob` / `pruneAutoCopyJobs`（约 4089~4800 行）。
- **Job 模型**（前端拿到的进度视图 `publicAutoCopyJob`）：
  ```
  { id, status: 'queued'|'running'|'done'|'partial'|'conflict'|'error',
    sourceUid, targetUid, sourceName, targetName,
    total, processed, copied, skipped, partial, failed, failedItems, conflicts,
    details: [ { id, label, status: 'running'|'copied'|'skipped'|'partial'|'conflict'|'failed',
                 failedFiles, conflicts, error? } ],
    error, currentLabel, startedAt, finishedAt }
  ```
  `details` 每条对应一个会话（`label` = `custom_title || title || cwd || '未命名会话'`，`error` 截 240 字）；`details` 上限 500 条，job 表最多保留 100 条、完成后 30 分钟自动删除。
- **队列串行化**：`autoCopyQueue` 单 worker，`sessionCopyLocks` 按 `[lineageId,targetUid]` 再串行一层。目的 = 快速连续切换（h→s→x）时后一个 job 能看到前一个刚 INSERT 的会话行；**每个 job 真正开跑时重新 `buildAutoCopyPlan`**（队首 re-plan），不用入队时的旧 plan。
- **"只处理有变化的会话"判据**（核心，三条分支）：
  1. `baselineAt = mapping.updatedAt`（该 lineage→targetUid 上次成功同步的时间戳）；
  2. `changedSinceBaseline = 成员中 contentMtime > baselineAt 的项`；
  3. 裁决：`changedSinceBaseline.length >= 2` → **`conflict`**（两边都改过，不按 mtime 猜谁赢、绝不静默覆盖）；`targetPresent && baselineAt > 0 && length === 0` → **`unchanged` → `skipped`**（继承旧行为不整份覆盖）；否则选 `selectLatestAutoCopyMember` 的最新成员作为源，复制到其他成员。
- `sessionContentMtime(dataRoot,id)` = 该会话全部落地路径的**最大 mtime**（递归目录）：`projects/<hash>/<id>.jsonl`、`projects/<hash>/<id>/`、`workspace/sessions/<id>`、`tasks/<id>`、`file-history/<id>`、`artifact-index/<id>.json`。**消息文件 mtime 是权威时间，不用 DB 字段。**
- 单条结果状态：`copied`（有实际复制）/ `skipped`（继承、无变化）/ `partial`（有文件失败）/ `conflict` / `failed`（抛异常）；job 汇总状态 = `conflicts? 'conflict' : (failed||partial ? 'partial' : 'done')`。
- 进度上报：`activeAutoCopyJob()` 优先返回 running，其次 queued，再退化为"15 秒内完成的最近一个"（让进度条收尾不闪回）；前端在进度条走完后才展示冲突明细（注释原话：surface a conflict for the user **after** the progress bar completes）。
- 失败文件计数与 `failedFiles` 分开：`failed` 是"文件级失败数"，`failedItems`/`partial` 是"会话级失败数"。
- 我们的对应：`session_transfer.rs::SessionTransferReport`（只有 `added_conversations` / `replaced_conversations` / `updated_session_rows` / `scanned_workspaces`）→ 缺口 = `details[]` 明细 + `copied/skipped/partial/conflict/failed` 计数 + "无变化即跳过"的 baseline 判据（我们现在 WorkBuddy 主库是**无条件全量** `UPDATE sessions SET user_id=? WHERE user_id!=?`，见 `session_transfer/workbuddy.rs`）。移植时注意我们的"会话"落盘布局与参考不同，baseline 应记在我们自己的会话记录上（mtime 或内容哈希）。

锚点：`grep -n "function syncAutoCopyLineage\|function startAutoCopyJob\|function publicAutoCopyJob\|function activeAutoCopyJob\|function sessionContentMtime" scripts/daemon.js`；`node --test test/auto-copy-rules.test.js`

## §18 会话分支 Fork（1.2.76 新增；**用户已否决，不移植**）

- 参考位置：`scripts/session-fork.js`（纯计算，404 行，有 CLI 入口）、`workbuddy-compat.js::findSessionForkSelection`、`test/session-fork*.test.js`。
- 语义：WorkBuddy 无原生分支按钮；本模块把会话本机 JSONL 记录按"消息锚点"切成**前缀**，产出新会话的记录文本，**源会话不被修改**；落库（写 `projects/<slug>/<id>.jsonl` + INSERT sessions）由 `daemon.js` 做，本模块**不写任何文件**。
- 纯函数：`parseRecords(text)`（逐行 JSONL，坏行跳过并计数——会话进行中追加写，最后一行可能是半截 JSON；**保留原始行文本**，切片时按原样输出，不重新序列化以免改写未知字段）、`buildAnchors`（只计 `user`/`assistant`，序号 1..N）、`resolveAnchor`（序号 / 时间 / `--points` 列表）、`cutIndexFor`、`planFork(text,spec)`、`planForkAtMessage(text,selection)`、`forkedTitle(title)` → `标题（分支）`（≤60 字）。
- 约束：**只支持"保留前缀到锚点为止"**，不做区间切片（丢掉前缀会让新会话从半轮开始、缺少开头 user 消息）；`MAX_RECORD_BYTES=64MiB`、`MAX_ANCHOR=100000`；`INJECTED_PREFIX` 跳过 harness 注入块（`<cb_summary|system-reminder|additional_data|identity_context|task-notification`）——注入块与用户真话可能同记录，先抽 `<user_query>` 再判定。
- `findSessionForkSelection(frame,messageStore)`：由消息 DOM 帧的 `data-cr-frame-id` 反查 messageStore，**必须满足 assistant 且 `complete !== false` 且 `isEnd !== false` 且 `finishTime` 为正安全整数**才可作分支点；返回 `{messageIndex, roles, finishedAt}`。这就是"改进流式回复和任务通知下的分支定位"（流式未结束 / 任务通知记录都不算落点）。
- **决策（2026-09-19）**：用户明确"不要会话分支"。因此**不移植**；本节点仅作为边界知识保留——若将来要做，上面这套锚点判定与"前缀切片"约束可直接转写为 Rust 纯函数。

锚点：`grep -n "function planFork\|function buildAnchors\|INJECTED_PREFIX\|MAX_RECORD_BYTES" scripts/session-fork.js`

## §19 成长计划 / Buddy 状态（1.2.76 新增）

- 参考位置：`scripts/growth-daily.js`（378 行）、`scripts/growth-active.js`（streak 扩展）、`daemon.js` 的 `/api/growth/*`。
- **每日进度聚合** `growth-daily.js::normalizeDailyProgress(tasks, travel, options)` → 纯函数，输出：
  ```
  { status: 'ready', fetchedAt,
    growth:  { completed, total, ratio, tasks: [ { taskCode, title, guide, tag, deadline,
                 reward: { credits, energy, buddy }, current, target, state } ] },
    rewards: { claimed, total, pending, ratio },
    cat:     { state, progress, arriveAt, dailyLimitReached, available, activeBuddy, buddies, reward },
    actions: { gacha: { available, count, energy, cost }, lottery: { available, count } },
    manualTasks: [ … ] }
  ```
  - `task.state ∈ claimed | completed | not_accepted | in_progress`；完成判据 `status ∈ {completed,claimed} || current >= target`（`current≥target` 兜底官方不推进 status 的情况）。
  - `AUTOMATABLE_TASK_CODES`（14 项白名单）+ `LOCKED_BUDDY_TASK_CODES = ['first_buddy','RichMeow_Chat']`：**未领养 Buddy 且任务带 task_code 时只展示这两个**；否则全部展示。不在白名单且未完成的任务进 `manualTasks`（UI 提示"需人工完成"）。
  - `cat.state`：`locked`（travel.available === false）/ `unknown`（buddy 信息未知）/ `needs_selection`（有 buddy 列表但没选当前 buddy 且状态 idle）/ 否则原样透传官方 state；`cat.progress`：`traveling` 或 `daily_limit_reached` → 1，`arrived` → 0.72，否则 0；`arriveAt` = 官方 `arrive_at` 秒 → 毫秒。
- **接口清单**（全部 `Authorization: Bearer` + `origin: apiHost` + `referer: {apiHost}/profile/growth-center` + `x-client-platform: web`，`redirect:'error'`，`code` 必须为 0）：
  | 用途 | 方法 | 路径（含回落） |
  |---|---|---|
  | 任务列表 | GET | `/v2/activity/growth/tasks` → `/activity/growth/tasks` |
  | 旅行状态 | GET | `/activity/growth/buddy/travel/status` → `/v2/activity/growth/buddy/travel/status` |
  | 当前 Buddy | GET | `/activity/growth/buddy/info`（optional） |
  | Buddy 列表 | GET | `/activity/growth/buddy/list`（optional） |
  | 盲盒额度 | GET | `/activity/growth/buddy/quota`（optional） |
  | 抽奖次数 | GET | `/activity/growth/lottery/chances`（optional） |
  | 接取任务 | POST | `/activity/growth/tasks/accept` body `{task_codes[]}`（≤20、去重、正则校验） |
  | 解锁首个 Buddy | POST | `/activity/growth/buddy/first`（无 body） |
  | 开盲盒 | POST | `/v2/activity/growth/buddy/open` body `{count:1}` |
  | 抽奖 | POST | `/v2/activity/growth/lottery/draw` body `{client_token:'draw-<uuid>'}` |
  | 旅行领奖 | POST | `/activity/growth/buddy/travel/claim` body `{}` |
  | 派出旅行 | POST | 先 GET `/v2/activity/growth/buddy/travel/config` 取 `locations[0].id`，再 POST `/v2/activity/growth/buddy/travel/depart` body `{location_id}` |
  | 选择 Buddy | POST | `/activity/growth/buddy/switch` body `{instance_id}`（正整数，严格 `String(id)===String(Number(id))`） |
  - 四个 optional 接口失败**返回 null 而非报错**，并用"是否有任一 optional 成功"推导 `buddyKnown`（未知时不要把 cat 判成 locked）。
- **连续活跃天数** `growth-active.js::fetchGrowthStreak`（1.2.76 扩展）：返回 `{ days, progressDays, nextTier, nextTierRemaining, makeupCards, tiers[{key:'7d'|'14d'|'28d', days, status}] }`；tiers 来源 `redemption_status.tiers`，缺失用默认 7/14/28 补齐，status 取 `redemption.tier_<key>_status`，兜底 `'locked'`；`progressDays` = `redemption.remaining_days` 兜底 `days`；`makeupCards` = `makeup_cards.balance`。
  - 缓存 `createGrowthStreakCache(load)`：**命中返回整份值对象**（旧版只回 `{days,status}`，改版时漏了这条会丢 tiers）；`get(uid,{force:true})` 强制刷新；ready 缓存 5 分钟、unavailable 30 秒；跨天（`day()` 变化）自动失效。
- 路由：`POST /api/growth/daily-progress`、`POST /api/growth/streak`、`POST /api/growth/today-active`、`POST /api/growth/activate`。
- 我们的对应：`auto_travel.rs`（旅行状态机 + 10 分钟/60 秒轮询调度，**已具备派旅行**）+ `daily_history.rs`（签到 / 旅行归档）。
  - 缺口：成长任务列表与接取、盲盒 / 抽奖、连续活跃天数与 7/14/28 档位、Buddy 列表与切换、`normalizeDailyProgress` 这一个"一次拉全、前端展示"的聚合视图。
- **平台范围：WorkBuddy only**（`apiHost` = `www.workbuddy.cn` / `copilot.tencent.com`）。**不要落到 CodeBuddy CN**。

锚点：`grep -n "function normalizeDailyProgress\|AUTOMATABLE_TASK_CODES\|LOCKED_BUDDY_TASK_CODES" scripts/growth-daily.js`；`grep -n "function fetchGrowthStreak\|function createGrowthStreakCache" scripts/growth-active.js`；`node --test test/growth-daily.test.js test/growth-rings-ui.test.js`

## §20 自动化协议 V2 / 公开任务发现 / 安全评估 / 模型选择（1.2.76 新增）

> **⚠️ 同 §14：整块已评估后否决（2026-09-19），实现已删除。** 本节保留只为了解协议、以及在"将来具备注入能力"时能直接复用。
> 唯一仍然成立、且已落在我们代码里的语义：**「按天幂等靠 state + 本地日期」「领奖后必须重查状态」「未领养账号优雅跳过」**
> —— 这三条 `auto_travel.rs` 早已实现；`normalizeAutomationModelId`（模型 ID 1–120 字符、trim、拒控制字符）可作将来"发送时选模型"的校验口径。

### 20.1 协议 V2（`automation.js`）

- `SCHEMA_VERSION = 2`，`SUPPORTED_SCHEMA_VERSIONS = {1,2}`；`isSupportedTaskSchema` 接受 1 与 2（**旧任务可执行**）。
- **任务包仍是 v1 才收**：`automation-packages.js::previewPackage` 判据 = `packaged ? (task.schemaVersion??1)===1 : [1,2].includes(task.schemaVersion??1)`，否则 `incompatible('unsupported_task_schema')`。即：外部任务包（package 信封）必须 schema 1；直接的裸任务 JSON 可以是 1 或 2。
- 新增 op：
  - `log.write`：`{ op:'log.write', message:'账号 {account}：{state}', params:{ account:'{{account.uid}}', state:'{{vars.catStatus.json.data.state}}' } }` —— message 里的 `{参数名}` 替换为 params 的**标量**值（params 支持模板）；**拒绝凭据字段（token/cookie/authorization/password 等路径），对手机号与令牌形态做脱敏**；只写运行日志，不弹通知。
  - `state.set` / `state.checkpoint` 支持 `ttlMs`（状态过期）。
  - `session.create` / `session.send` 支持可选 `model`：精确模型 ID，`normalizeAutomationModelId` = 1–120 字符、trim、拒绝控制字符；发送后恢复"新建任务"原模型偏好。
- 旧 op `session.sendCurrent` / `session.waitReply` / `notify.afterAllTasks` / `event.clientLoaded` 标 `deprecated`（仍可执行，`session.sendCurrent` 实际进入新建任务后发送）。
- 状态存储：key = `JSON.stringify(['v2', taskId, scope, uid, key])`，`STATE_V2_MARKER = '__workdaddyAutomationStateV2'` 用于旧状态迁移与命名空间隔离。
- **安全评估入口** `createSafetyReviewTask(dataDir, taskId)`（注意：**不是代码审计**）：
  - 生成一条**只读**评审任务，硬约束：只允许读 `automations.json` 里该条 + 协议说明两个输入；禁止 shell / 搜文件 / 打开源码 / 看日志 / 读第三方文件；禁止执行、启用或修改被评审任务；禁止输出真实 token / cookie / 账号备份 / 会话内容；不确定就写"无法判断"。
  - 固定 7 项结论（风险等级 / 这是做什么≤20 字 / 会导致数据泄露 / 会对电脑有危害 / 会造成账号、会话或任务数据丢失 / 其他风险 / 建议：可以启用·谨慎启用·不建议启用），整份 ≤120 字、只出结论不出过程；最终格式**覆盖**为两列 Markdown 表格。
  - **判定纪律（最容易写错）**：查询/读取请求本身不是写操作，**不得因为查询频率或请求次数推断会触发平台风控**；只有 JSON 或协议明确写出实际副作用（写入/发送/修改/删除）才列为风险。
  - 路由：`POST /api/automations/safety-review` → 起一条 run（`run.safetyReviewTaskId = id`）→ 202 `{ok, runId}`。
- **副作用静态分析**（`automation-packages.js::analyzeTask(task)`，**纯逻辑，应优先移植**）：递归 walk `steps / onSuccess / onFailure`，产出
  - `capabilities`：出现的 op 集合；
  - `effects`：`account-switch`（`account.forEach` 且 `switch===true|switchAccounts===true`）、`account-checkin`、`send-message`（`session.create|send|sendCurrent` 或 `notify.afterAllTasks`）、`page-input`（`dom.click|type|clear|press`）、`write-state`（`state.set|checkpoint`）、`network`（所有 `http.*`）、`account-credentials`（`http.requestAsAccount`）；
  - `origins`：`http.*` 的 URL origin，解析失败记 `'dynamic'`。
  - 递归容器：`logic.sequence|repeat|retry|catch|forEach|waitUntil|account.forEach`（走 `steps`）、`logic.catch`（另走 `onError`）、`logic.if`（then/else）、`logic.switch`（cases/default）。
  - `inputs` ≤40，键禁 `__proto__ / constructor / prototype`；`requires` 允许键 `minWorkDaddyVersion / taskSchemaVersion / capabilities / profiles / platforms`（+ `x-*`）。
- **运行 / 停止语义**：`POST /api/automations/run` → 202 `{ok,runId}`；`GET /api/automations/run-status?id=` → `automationPublicRun`；`POST /api/automations/stop {runId}` → running 则置 `cancelled` + 清 `pendingEvent`，再 `cleanupNotifications()`；`POST /api/automations/logs/clear`；`GET /api/automations/capabilities`（给 UI 列可用的 op 与示例）。

### 20.2 模型选择 `automation-model.js`（注入依赖，只借语义）

- `selectAutomationModel({model, conversationId, accountUid})` 的完整路径：找到可见的 `.cr-model-selector__trigger` → 打开 `.cr-model-selector__popover` → 从 `.cr-model-selector__item` 的 React fiber 逐层找 `memoizedProps.option.id + onSelect` → 读 `previousModel` → `target.props.disabled` 则拒绝 → `onSelect()` → 80ms×50 轮确认（`sessionStore.getState().model` + `configManager.model` 双侧一致，且触发器文案含模型名）。
  - **失败必须回滚**：`changed && previousModel` 时重开菜单选回原模型；`storageKey` 场景还原 `previousRaw`（`removeItem` / `setItem`）；回滚失败要把两条错误串起来抛出。
  - 引用会话时必须校验 `.cr-document[data-root-id] === conversationId` 且 `!sessionStore.getState().isBusy`；无会话上下文时用 `localStorage['cb-newtask:model:'+accountUid]`。
  - 配套：`verifyAutomationModel()`、`restoreNewTaskModelPreference({storageKey,model,previousModel,previousRaw})`。
- **我们的对应**：我们无法注入 renderer。等价语义要在我们自己的通道上实现——发送前选择模型 → 校验生效 → 失败回滚；并记录"这次 run 用了哪个模型"。协议层（`session.*` 的 `model` 字段 + `normalizeAutomationModelId` 校验）可以直接对齐字段名。

### 20.3 公开任务发现 `automation-discovery.js`（纯逻辑 + 接口，可完整转 Rust）

- 发现标记 `DISCOVERY_MARKER = 'WorkDaddyAutomationRepository'`：**只收录 description 里带这个标记的仓库**（防误抓）。GitHub：`api.github.com/search/repositories?q=<marker> in:description`；Gitee：`so.gitee.com/v1/search/widget/<widget>`（社区 widget id）。
- 仓库校验：`full_name` 正则 `^[A-Za-z0-9_.-]{1,100}/[A-Za-z0-9_.-]{1,100}$`，且 `html_url === 'https://github.com/'+full_name`（Gitee 同理校验 `fields.url === 'https://gitee.com/'+fullName`）。上限 `MAX_REPOSITORIES=1000`。
- 只扫仓库 `tasks/*.json`：单层目录（`entry.path === 'tasks/' + entry.name`）、文件名不含路径分隔符、每仓 ≤200 个（`MAX_TASKS_PER_REPOSITORY`）、单文件 ≤1MB、JSON ≤5MB、请求超时 15s（GitHub 25s）。
- **下载地址白名单（安全关键）**：GitHub 只接受 `https://raw.githubusercontent.com/<owner>/<repo>/<branch>/<path>`（pathname 前缀 + 后缀双向匹配、无 search/hash）；Gitee 只接受 `https` + host ∈ `{gitee.com, raw.giteeusercontent.com}` + pathname 以 `/<owner>/<repo>/` 开头 + 含 `/raw/`。**重定向手动处理（`redirect:'manual'`，≤3 跳），每一跳都重新校验 host 与 pathname。**
- 响应体读取有硬上限（`readBounded`：先看 `content-length`，再流式累加，超限即 cancel）。
- 缓存 `automation-discovery-cache.json`：`CACHE_VERSION=3`、`CACHE_TTL_MS=10min`；结构 `{version, checkedAt, refreshedAt, providers:{github[],gitee[]}, repositories:{key:{…,pushedAt,tasks[]}}, errors[]}`；**`pushedAt` 未变则复用缓存的 tasks**（增量，仓库多时省掉 N 次文件下载）；`readCache` 遇 v2 自动降级迁移；搜索与扫描错误都进 `errors[]` 不抛。并发抓取用 `mapLimit(…, 4)`。
- `catalogFromState`：按文件内容"规范化 JSON 的 sha256"去重——**同一份任务出现在多个仓库时合并成一条**（`sources[]` 记录 platform/repository/repositoryUrl/fileUrl/path/stars，`stars` 累加），按 stars 降序 + 名称 `zh-CN` 排序；`getTaskContent(key)` 按 sha256 取原文。
- 路由：`GET /api/automations/discovery`（取目录，带 `loading/stale/checkedAt/refreshedAt/errors/tasks`）、`POST /api/automations/discovery/import`（按 key 取原文 → 走 `import` 预览路径）。

### 20.4 内置任务 `scripts/builtin/automations/buddy-travel.json`（"派猫猫旅行"，可直接照抄语义）

- 头部：`schemaVersion: 2`、`id: 'daily-growth-and-buddy'`、`name: '派猫猫旅行'`、`concurrency: {policy:'skip'}`、`trigger: {type:'clientLoaded', types:['clientLoaded','panelOpened'], oncePerNavigation:false}`、`schedule: {type:'interval', minutes:10}`、`enabled: false`。
- 步骤骨架（**全部是官方接口 + 账号 token，不需要注入 → 我们能完整实现**）：
  1. `account.forEach {accounts:'all', switch:false}`（不切客户端）；
  2. `logic.catch` 包住 Buddy 查询：GET `buddy/info` → assert `code===0` → GET `buddy/list` → assert → 若 `info.data.buddy` 为空则遍历 `buddies` 找第一个 `instance_id` 匹配 `^[1-9][0-9]{0,14}$` 的，POST `buddy/switch`（`value.number` 把模板值转数字）；`buddy/list` 为空则记"尚未领养 Buddy，跳过旅行"并置 `skipTravel`；
  3. `logic.catch` 包住旅行：`state.get {scope:'account', key:'catSettledDay'}` 与 `{{runtime.time.localDate}}` 比较做**按天幂等** → GET `travel/status` → assert → 按 `state` 分支：
     - `arrived` → POST `travel/claim` → assert → **重查** `travel/status`（领奖后状态会变，必须重查再判断 idle 分支）；
     - `idle` → 若 `daily_limit_reached` 则 `state.set catSettledDay = 今天` 并记日志；否则再 GET `buddy/info` 确认有当前 Buddy → GET `/v2/.../travel/config` → `locations.length > 0` 才 POST `travel/depart`（`location_id = locations.0.id`）；
  4. 两个 `logic.catch` 的 `onError` 都把 `hadError=true` 并 `log.write`，**不中断其他账号**；末尾 `logic.assert({{vars.hadError}} falsy)` 让整轮在部分失败时标记失败。
- 学习要点：**按天幂等靠 `state` + 本地日期**（不是靠接口返回）；**领奖后必须重查状态**；**未领养账号要能优雅跳过**（提示去官网同意协议）；一个账号失败不影响其他账号，但整体要报"部分失败"。

锚点：`grep -n "SCHEMA_VERSION\|SUPPORTED_SCHEMA_VERSIONS\|function createSafetyReviewTask\|log.write" scripts/automation.js`；`grep -n "DISCOVERY_MARKER\|CACHE_VERSION\|function scanRepository\|function catalogFromState" scripts/automation-discovery.js`；`grep -n "function normalizeAutomationModelId\|function selectAutomationModel" scripts/automation-model.js`；`node --test test/automation-v2.test.js test/automation-discovery.test.js test/automation-safety-review.test.js test/automation-model.test.js`

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
- 路由族（详见 `daemon.js::handleApi`，2026-09-19 / 1.2.76 时点）：注入与交互 `/api/inject`、`/api/cdp-click`、`/api/click`、`/api/find`、`/api/clear-composer`、`/api/devtools-url`；账号 `/api/status`、`/api/accounts`(+`/primary`、`/order`)、`/api/switch`、`/api/logout`、`/api/backup`、`/api/delete`、`/api/current`、`/api/oauth/{start,poll}`、`/api/accounts/{export,import}`；用量 `/api/credits`、`/api/credit-rotation`、`/api/token-stats`、`/api/credit-stats(+/sync)`、`/api/growth/{daily-progress,streak,today-active,activate}`；会话 `/api/sessions(+ /workspaces、/auto-copy、/fork、/export、/import、/copy、/migrate、/delete、/restore)`；模型 `/api/models*`；自动化 `/api/automations`、`/run`、`/run-status`、`/stop`、`/bulk`、`/validate`、`/dry-run`、`/export`、`/import(/preview)`、`/packages/preview`、`/capabilities`、`/logs/clear`、`/events`、`/agent-info`、`/agent-generate`、**新增 `/discovery`、`/discovery/import`、`/safety-review`**；增强 `/api/{ask-mode,no-disturb,auto-continue,session-module,quick-phrase*}`；主题 `/api/{themes,wallpapers,custom-wallpapers,mask,blur,theme-text-shadow,theme-apply,theme-save}`;休眠 `/api/{sleep-mode,sleep-now}`；杂项 `/api/stash*`、`/api/open-url`、`/api/open-dir`、`/api/about`、`/api/update-*`。
  - **1.2.76 移除**：`/api/automations/checkin-consent`（consent 机制整体删除）。
  - **1.2.76 新增的非 Linux 平台件**（不移植）：`scripts/platform.js`、`linux-daemon-process.js`、`workbuddy-ai-linux.sh` 与各 `*-linux.sh` / systemd / deb 打包脚本。
- 用途：当我们要给某个能力设计 Tauri command 时，来这里查参考的请求/响应字段，避免自创字段名。

## §C 可移植性三分类（速判）

- **纯逻辑（可直接转 Rust）**：`lib.js` 账号解析/决策链/原子写、`secure-transfer.js`、`session-transfer.js`、`session-fork.js`（纯计算，但产品已否决）、`session-db.js` 安全闸、`token-stats.js`、`credit-segments.js`、`credit-rotation.js`、`credit-resource-queries.js`、`credit-usage-store.js`、`third-party-models.js`、`automation.js` 的模型/校验/调度/副作用分析/安全评审文案、`automation-packages.js::analyzeTask`、`automation-discovery.js`（含 URL 白名单与缓存策略）、`growth-daily.js`、`growth-active.js` 的 tiers 计算、`theme-*.js`、`ui-port.js`、`cdp-targets.js`、`daemon.js` 的会话同步 job（`syncAutoCopyLineage`/`startAutoCopyJob`/`publicAutoCopyJob`）、休眠控制与主题 CSS 生成。
- **官方接口（可转 Rust HTTP 客户端）**：`credit-request-usage.js`、`credit-history-sync.js`、OAuth `auth/state|token` + `login/account`、签到 `daily-checkin`、企业用量 `get-enterprise-user-usage`、成长中心全套（`/v2/activity/growth/tasks|tasks/accept|buddy/{info,list,quota,first,open,switch,travel/{status,config,depart,claim}}`、`/v2/activity/growth/lottery/{chances,draw}`）、`growth-active.js` 的 streak 接口、`automation-discovery.js` 的 GitHub / Gitee 搜索与 raw 下载。
- **注入依赖（不可照搬，只借语义）**：面板与各 pane、暂存/快捷短语按钮插入、免打扰点击、自动续接发送、`until-done` 忙碌判定、主题最终注入、`automation-model.js::selectAutomationModel`（模型下拉点击与回滚）、`automation-picker.js` 元素拾取、`reloadWorkBuddyPage`/`autoFocusSessionByTitle`/renderer 优先级抢占、`session-fork` 的 DOM 锚点选择（`findSessionForkSelection`）、`dom.*` 与 `session.*` 自动化步骤、`yieldAutoCopyToRenderer` 让位。

## §D WorkDaddy 独有 → any-version 缺口候选（按性价比排序）

1. **会话同步明细 + 只同步有变化的会话**（§17，1.2.76）：`SessionTransferReport` 加 `details[]` 与 `copied/skipped/partial/conflict/failed` 计数，并引入 baseline 判据（无变化即 `unchanged`→跳过、两侧都改即 `conflict` 不猜）。**平台范围：WorkBuddy 优先，CodeBuddy 侧仅复用"无变化跳过 + 明细结构"这类平台无关部分。**
2. ~~**自动化任务体系**（§14 + §20）~~ —— **已评估后否决（2026-09-19），实现已删除，不要再做**。理由见 §14 顶部：`dom.*` / `session.*` 与页面类触发依赖注入（我们没有），能跑的部分与既有 `auto_checkin.rs` / `auto_travel.rs` 完全重叠。若将来具备注入能力再重新评估。
3. **成长计划 / Buddy 状态展示**（§19）：签到、连续活跃天数（7/14/28 档位）、任务进度与接取、Buddy 解锁/切换、旅行、奖励（盲盒/抽奖）。我们已有 `auto_travel.rs` 与 `daily_history.rs` 作底座，只差聚合视图与新接口。**平台范围：WorkBuddy only。**
4. **Token / 积分 / 调用量统计多维筛选**（§5 §6）：① `token-stats.js` 增 `dailyBreakdown`（一次算全维度，筛选在内存做）；② 我们的 `ai_usage` 表补 account 维度与日期区间参数；③ 积分补按天历史 + anchor 增量 + 幂等落库（§5）。
5. **积分轮换建议**（§7）：纯函数，成本最低（注意 1.2.76 的"当前账号更早到期就不建议换"修正）。
6. **会话加密归档导入导出 `.wds` v4**（§11）：让会话可跨机搬迁，格式可自定义但建议对齐参考头布局。
7. **假退出登录**（§10）：先退宿主再删登录文件，token 保留。
8. **第三方模型（cc-switch）导入 + 端点连通测试**（§15）。
9. **防休眠（含 until-done）**（§16）：纯本地逻辑 + 我们的忙碌判定来源。
10. ~~会话分支 Fork~~（§18）：**用户 2026-09-19 明确不做**，不再列为候选。
11. 已对齐无需再做：主账号 + 账号排序（§2）、派旅行状态机（§19 的 cat 部分已由 `auto_travel.rs` 覆盖）。

## §Z 漂移校验锚点（学习前先跑）

```bash
cd E:/pro/other-sdk/buddy/WorkDaddy
grep -n "DAEMON_VERSION\|DAEMON_BUILD_ID" scripts/daemon.js | head -3
grep -n "EXPORT_VERSION\|MAGIC = Buffer" scripts/secure-transfer.js scripts/session-transfer.js
grep -n "SCHEMA_VERSION\|SUPPORTED_SCHEMA_VERSIONS\|const CAPABILITIES\|function createSafetyReviewTask" scripts/automation.js
grep -n "DISCOVERY_MARKER\|CACHE_VERSION" scripts/automation-discovery.js
grep -n "CACHE_VERSION" scripts/token-stats.js
grep -n "10001" scripts/checkin-result.js
grep -n "function syncAutoCopyLineage\|function startAutoCopyJob" scripts/daemon.js
grep -n "function normalizeDailyProgress" scripts/growth-daily.js
ls scripts/checkin-consent.js 2>/dev/null || echo "checkin-consent 已移除（1.2.76 起）"
node --test test/*.test.js   # 参考侧全量行为规范（改动会在这里体现）
```

以上任一项与本文档不符 → 该模块的契约以源码为准，学完后回写本文件并在 `sync-point.workdaddy.txt` 记录。
