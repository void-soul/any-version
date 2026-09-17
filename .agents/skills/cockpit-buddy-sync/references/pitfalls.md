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

## F. 参考 B（WorkDaddy）移植坑（异架构，契约见 references/workdaddy.md）

19. **把 CDP 注入或 Node daemon 搬进来** — WorkDaddy 的面板/免打扰/自动续接/暂存都靠 `Runtime.evaluate` 注入 renderer，我们没这条链路。这些能力只能采用「纯函数判定规则 + 我们自己的 UI/命令」的形态，或明确放弃。
20. **两套切换时序混用** — WorkDaddy 切换是"写登录文件 + CDP 刷新，不关客户端"；我们的是"关客户端 → 会话合并 → 写回 → 重启"。把前者的"不关客户端"塞进我们的切换流程，会让运行中的客户端把内存里的旧身份写回，等于没切。假退出（`/api/logout`）才可以走"先退宿主再删文件"。
21. **Node 加密参数与 Rust crate 默认值不同（真要互操作归档/导出时才踩）** — Node `crypto.scryptSync` 默认 `N=16384, r=8, p=1`（对应 Rust `scrypt::Params::new(14, 8, 1, 32)`）；参考的 AES-GCM 打包布局是 `base64(iv‖authTag‖cipher)`，而 Rust `aes-gcm` crate 输出是 `ciphertext‖tag`、IV 需单独传，直接照搬字节顺序必然解不开。GCM 的 AAD 也要对齐（`.wds` v4 用 `header[0,36)`）。
22. **`code===10001` 单独判定"已签到"** — 参考是 `code===10001 && !inactive && ALREADY_MESSAGE.test(message)`；只看 code 会把"活动未开启/已过期"误判成签到成功。
23. **反向覆盖签到调度语义** — WorkDaddy 的签到是固定 60 分钟 interval + opt-in consent，**没有**随机时间窗；我们的随机窗口来自 cockpit-tools，勿用参考的 interval 替换。
24. **把参考的测试当可移植代码** — `test/*.test.js` 里一半是"从 `inject.js` 抽取源码片段 + DOM 桩"（见 `test/no-disturb-match.test.js` 头部注释），只作**行为规范**读，断言可转写成我们的 Rust 单测，代码不能搬。
25. **时区混用** — 参考中官方计费/历史按**本地日**聚合（`credit-history-sync` / `credit-usage-store`），匿名上报按 **UTC+8** 日（`usage-report.js`）。移植任何"按天"的功能前先确认用哪一种日界。

## G. 参考仓漂移探测（B 仓）

```bash
cd E:/pro/other-sdk/buddy/WorkDaddy
grep -n "DAEMON_VERSION" scripts/daemon.js | head -1   # 与本技能 sync-point.workdaddy.txt 记录对比
node --test test/*.test.js                             # 参考侧行为规范全量（改动会在这里体现）
```

差异出现即按 SKILL.md §2.2 圈功能域；本单 F 节若新增同类语义，直接追加。
