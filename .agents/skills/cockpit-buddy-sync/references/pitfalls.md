# Buddy 移植踩坑清单（历次真实事故，重学时先跑本单）

格式：**坑 → 后果 → 检测命令**。检测命中即需处理；参考侧若新增了本单没有的同类语义，补进本单。

## A. 移植机械错误（上一个模型改乱时的事故集）

1. **子模块 super 深度** — `session_transfer/{workbuddy,codebuddy}.rs` 里 `use super::models/store` 落空（super 是 session_transfer）。检测：`grep -rn "use super::models\|use super::store\|super::session_transfer" src-tauri/src/commands/buddy/session_transfer/` 应为空。
2. **log crate 不存在** — 参考的 `log::info!`/`logger::` 直接抄会挂。检测：`grep -rn "log::" src-tauri/src/commands/buddy/` 应为空。
3. **函数内新建 LazyLock** — 每次调用新锁 = 无互斥。必须是 `static TRANSFER_LOCK`。
4. **尾表达式类型** — `transfer_local_sessions` 结尾写 `report.scanned_workspaces` 而签名是 `Result<Report>`（应为 `Ok(report)`）。
5. **`create_dir_all(x.parent())`** — parent 返回 Option，直接传编译错；正确是直接建目标目录本身。
6. **`Result<Option<T>>` 再 `.transpose()`** — 双重翻转误用；mod.rs 里 `transfer_on_switch(...)?` 直接拿 Option。
7. **`String == Option<String>`** — 当前账号比较要 `current_id.as_deref() == Some(id.as_str())`。
8. **tauri 命令返回结构体缺 `Serialize`** — 报错信息是诡异的 `blocking_kind`；新返回类型记得 derive（并 `rename_all="camelCase"`）。
9. **后端返回元组 + 前端 `invoke<string>`** — 对象进 React 子节点崩溃。切换返回 `(文本, 报告)` 时前端必须解构并把报告**格式化成文本**。

## B. 参考语义漂移（合并功能真实失败原因，最隐蔽）

10. **workbuddy.db 重映射被写成条件执行** — 参考是无条件（5.x 真正生效的合并）。只要走了 else 分支，会话就完全没合并。检测：
    `grep -n "remap_workbuddy_database_user_id" src-tauri/src/commands/buddy/session_transfer/workbuddy.rs` 的调用点**不能**在 `if extension_roots.is_empty()` 分支内。
11. **目标工作区不存在时只复制 index.json** — 参考是 `copy_dir_atomic(整个工作区)`；漏拷会话正文目录 = "合并成功但会话全是空的"。
12. **辅助目录是空桩 / 索引备份只建目录** — 参考有 `copy_auxiliary_workspace_roots`（5 种 kind）与真实 `fs::copy(index.json)`。
13. **sqlite 备份漏 `-wal/-shm`** — WAL 模式下未 checkpoint 的数据在 wal；只备份主文件 = 备份不可用。
14. **CN `sync_history` 返回 usize** — 丢 added/replaced 计数，报告恒 0。共享引擎必须返回完整 report。
15. **CN 注入三连漏** — state.vscdb 候选路径、`verify_state_db_injection` 注入后校验、Safe Storage 友好提示（参考 `codebuddy_cn_instance.rs`）。

## C. 工具链/环境坑

16. **sysinfo `refresh_processes` 参数个数随版本变**（0.31 单参 / 0.32 带 remove_dead）。改 sysinfo 后先 `cargo check`。
17. **i18n JSON 往 buddy 段插键**：锚定某 `"key": "value"` 行时注意**段内最后一个键没有尾逗号**，先补逗号再插。
18. **参考仓 `crates/cockpit-core/` 下的 `workbuddy_account/workbuddy_oauth/workbuddy_instance/codebuddy_cn_*` 是 cockpit-cli 用的副本，src-tauri 不编译它们**。学习/diff 一律以 `src-tauri/src/` 为准（relearn.sh 路径表已按此修正）；只改副本的 commit 不是 Buddy 行为变化。

## D. 漂移探测 grep（两边各跑一次，输出结构对齐）

```bash
cd E:/pro/other-sdk/cockpit-tools   # 参考侧
grep -rn "check-point\|file-tree\|plan-task\|genie-cache\|connectors" src-tauri/src/modules/codebuddy_session_transfer.rs
grep -rn "deleted_at IS NULL" src-tauri/src/modules/workbuddy_session_transfer.rs
grep -rn "Tencent-Cloud.genie-ide-cn" src-tauri/src/commands/codebuddy_cn_instance.rs

cd E:/pro/my/any-version            # 我们侧（应逐条对得上）
grep -rn "AUXILIARY_KINDS" src-tauri/src/commands/buddy/session_transfer/
grep -rn "deleted_at IS NULL" src-tauri/src/commands/buddy/session_transfer/
grep -rn "Tencent-Cloud.genie-ide-cn" src-tauri/src/commands/buddy/
```

## E. 已知未移植/待观察（下次学习顺带确认）

- 参考 CN 的 `genie-cache` 在本机实际落在 `<uid>/<IDE>/genie-cache`（IDE 层），而非 `<IDE>/<uid>/genie-cache`（账号层）——参考实现按账号层取；真机若发现辅助数据缺失优先查这一处。
- 参考切换命令内含"启动失败仍完成认证写入"的兜底分支与 `app:path_missing` 事件；Kira 目前统一用"切换成功+启动失败提示"，若参考演化再对齐。
- 参考 `workbuddy_share_sessions_on_switch` / `codebuddy_cn_share_sessions_on_switch` 两个开关：Kira 恒开；若要做开关，加到设置 tab。
- 【2026-09-10 核对补充·属适配非缺口】参考自动签到用 `rand::random()`，Kira 依赖里没有 rand，改用 `getrandom` 自实现 `random_u32_below`——行为等价，勿"学回去"引 rand。
- 【同上】参考 `workbuddy_auto_checkin.rs::migrate_config_if_missing`（启动时确保配置存在）：Kira 的 `get_config_checked` 读不到即返回默认配置，无需迁移函数，缺它不算漏移植。
- 【2026-09-11 Kira 侧新增】优雅关闭客户端的 taskkill 参数：只能用 `taskkill /PID x`（不带 /T 不带 /F＝WM_CLOSE）。带 `/T` 会逐个向进程树发关闭请求，Electron 的 GPU/渲染/工具子进程没有窗口，刷一屏"只有强制终止才能终止此进程"且整体退出码非 0（参考侧 request_antigravity_graceful_close 带 /T 属同样问题，勿照搬）。/T 只保留在超时升级的强杀（`/T /F`）里。taskkill stderr 是 GBK，用 `file_io::decode_text_bytes` 解码，勿 from_utf8_lossy。
- 【2026-09-19 新增】Linux/打包类文件已从 `relearn.sh` 的监听范围排除（`*-linux*`、`platform.js`、`build-*`、`install-*`、`assets/` 等）——它们随版本发布会刷出一大堆与 Buddy 无关的 commit，掩盖真正的功能域改动。要调整排除表就改 `relearn.sh` 的 `excludes` 数组。

## F. 参考 B（WorkDaddy）移植坑（异架构，契约见 references/workdaddy.md）

19. **把 CDP 注入或 Node daemon 搬进来** — WorkDaddy 的面板/免打扰/自动续接/暂存都靠 `Runtime.evaluate` 注入 renderer，我们没这条链路。这些能力只能采用「纯函数判定规则 + 我们自己的 UI/命令」的形态，或明确放弃。
20. **两套切换时序混用** — WorkDaddy 切换是"写登录文件 + CDP 刷新，不关客户端"；我们的是"关客户端 → 会话合并 → 写回 → 重启"。把前者的"不关客户端"塞进我们的切换流程，会让运行中的客户端把内存里的旧身份写回，等于没切。假退出（`/api/logout`）才可以走"先退宿主再删文件"。
21. **Node 加密参数与 Rust crate 默认值不同（真要互操作归档/导出时才踩）** — Node `crypto.scryptSync` 默认 `N=16384, r=8, p=1`（对应 Rust `scrypt::Params::new(14, 8, 1, 32)`）；参考的 AES-GCM 打包布局是 `base64(iv‖authTag‖cipher)`，而 Rust `aes-gcm` crate 输出是 `ciphertext‖tag`、IV 需单独传，直接照搬字节顺序必然解不开。GCM 的 AAD 也要对齐（`.wds` v4 用 `header[0,36)`）。
22. **`code===10001` 单独判定"已签到"** — 参考是 `code===10001 && !inactive && ALREADY_MESSAGE.test(message)`；只看 code 会把"活动未开启/已过期"误判成签到成功。
23. **反向覆盖签到调度语义** — WorkDaddy 的签到是固定 60 分钟 interval（1.2.76 起 consent 机制已整体删除），**没有**随机时间窗；我们的随机窗口来自 cockpit-tools，勿用参考的 interval 替换。
24. **把参考的测试当可移植代码** — `test/*.test.js` 里一半是"从 `inject.js` 抽取源码片段 + DOM 桩"（见 `test/no-disturb-match.test.js` 头部注释），只作**行为规范**读，断言可转写成我们的 Rust 单测，代码不能搬。
25. **时区混用** — 参考中官方计费/历史按**本地日**聚合（`credit-history-sync` / `credit-usage-store`），匿名上报按 **UTC+8** 日（`usage-report.js`）。移植任何"按天"的功能前先确认用哪一种日界。
26. **WorkDaddy 只做 WorkBuddy** — 参考只有 `workbuddy-cn` / `workbuddy-ai` 两个 profile，没有 CodeBuddy CN 的依据。从它学来的语义默认只落 WorkBuddy 分支；拿它去改 `codebuddy_cn.rs` / `session_transfer/codebuddy.rs` 的**行为**要单独论证（见 SKILL.md §0.1）。纯逻辑（统计、任务模型、调度、副作用分析）可以两侧共用。
27. **`session_transfer` 的"新增/替换"计数与"会话数"口径不同** — 前者是文件/数据库行数，后者是会话数。把 `sync.copied` 直接接到 `addedConversations` 上会得到一个两头都不对的数字；两套字段并存是对的，不要合并。
28. **按会话"增量同步"时忘掉"首次同步"分支** — 无基线（新账号/基线文件损坏/首次切换）必须退化为**全量处理**，否则会把"没基线"当成"无变化"从而一个会话都不合并。判据：基线缺失 → 一律 `Apply`。
29. **换新的 SQL 列前先探测** — WorkBuddy 不同版本的 `sessions` 表列不一致（旧版可能没有 `title` / `custom_title` / `updated_at`）。直接写进 SELECT 会在老库上失败（单测里用最小表结构也会失败）；用 `PRAGMA table_info(sessions)` 探测后再拼 SQL。
30. **Rust 闭包捕获 `&mut` 的轻微踩坑** — 在闭包里用 `*best = Some(..)` 更新外层 `Option<T>` 会报"cannot be dereferenced"。改成独立函数 `fn consider_path(best: &mut Option<(u128, String)>, path: &Path)`，显式传 `&mut`，比在闭包里绕更清楚。
31. **借用与移动同函数内冲突** — `let Some(id) = conversation_id(&value) else ...` 拿到的 `&str` 借用会在后面 `merged.push(value)`（移动）时报 E0505。先把判定/原因码算完、并 `let owned_id = id.to_string()`，再移动。
> **⚠️ 32~40 条是"自动化任务体系"落地时踩的坑，该体系已于 2026-09-19 整体删除（见 §14 决策）。**
> 保留它们只为将来"真具备页面注入能力"时参考；**不要把这份清单当成待办去重新实现自动化**（第 41 条）。

41. **不要因为参考仓有就重新实现自动化任务体系** — 已落过一版并被否决删除（理由：`dom.*`/`session.*`/页面类触发依赖注入，我们没有；能跑的部分与既有 `auto_checkin.rs`/`auto_travel.rs` 重叠）。评审"要不要抄某个功能"时，先问三个问题：**① 它有多少比例依赖我们没有的链路？② 剩下那部分是否已经被我们写死了（只是形式不同）？③ 体量与收益是否成比例？** 本题三个答案分别是"一半以上""是""不成比例"。

32. **内置任务的触发方式必须换成本客户端支持的事件** — 参考 `buddy-travel.json` 用 `trigger.types=['clientLoaded','panelOpened']`（注入页面的加载/面板事件）。直接照抄会让 `install_builtins` 校验失败、任务装不进去。我们改成 `appStart`（= 客户端启动内完成一次），并已在 `dispatch_event("appStart", ...)` 里派发。
33. **`http.requestAsAccount` 的任务 URL 与我们的 host 不一致** — 参考任务写 `https://www.workbuddy.cn/...`，而 `api::travel_request` 打的是 `https://copilot.tencent.com`。运行期要**校验 origin 属官方白名单后，把 path(+query) 转发到我们的官方 endpoint**，否则要么凭据打到未验证 host，要么直接失败。白名单常量在 `automation/exec.rs::OFFICIAL_ORIGINS`，勿放宽。
34. **async 递归必须 `Pin<Box<dyn Future + Send>>`** — `run_step ⇄ run_steps` 直接递归 async fn 会报"size 无限"；且必须 `+ Send`（`tauri::async_runtime::spawn` 要求）。
35. **`break` 用独立错误变体传播** — 不能用普通错误字符串：`logic.catch` / `logic.retry` 会把错误吞掉，导致 break 被当成"失败被捕获"。用 `StepErr::Break` 单独一个变体，catch/retry 遇到它必须原样向上抛。
36. **发现功能的去重指纹要用"规范化 JSON 的 sha256"** — 用原始字节哈希时，同一份任务因字段顺序不同会被算成两条。`automation::canonical_json`（键排序）是唯一口径，`analyze/catalog/import` 三处都必须走它。
37. **内容去重/缓存键都必须"先校验再落盘"** — 公开任务目录缓存里存的是**原始文件文本**（不是解析后的对象），因为 key 是文本的规范化哈希；一旦提前解析再序列化，key 就对不上了（`getTaskContent` 会永远找不到）。
38. **能力清单里的"不可用"不是装饰** — `available:false` 的 op 必须在 `normalize_steps` 阶段直接返回 Err；只标记不拦截会让用户保存一个永远跑不通的任务。
39. **规则化安全评估的判定纪律** — 参考 §20.1 明确"查询请求本身不是写操作，不得因为查询频率或请求次数推断风控"。实现时**不要**因为出现 `http.requestAsAccount` 就加风险分（它只是"以账号身份读取"）；只有非 GET 写请求、非官方域名请求、发消息、页面输入、真实切换账号才计分，否则只读任务会被误判成中风险。
40. **Token 统计的账号归属是"有信息就用信息，没信息就猜路径"** — 记录里的 `accountUid/accountId/uid/userId` → 会话路径中含账号 uid → 否则记 `unknown`。别把 `unknown` 归到第一个账号上（会把统计做成假数据）。

## G. 参考仓漂移探测（B 仓）

```bash
cd E:/pro/other-sdk/buddy/WorkDaddy
grep -n "DAEMON_VERSION" scripts/daemon.js | head -1   # 与本技能 sync-point.workdaddy.txt 记录对比
node --test test/*.test.js                             # 参考侧行为规范全量（改动会在这里体现）
```

差异出现即按 SKILL.md §2.1 圈功能域；本单 F 节若新增同类语义，直接追加。
