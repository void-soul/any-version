---
name: bilibili-api-sync
description: 维护 any-version 收藏模块里 B站（bilibili）接口契约的既定流程。上游逆向文档仓 SocialSisterYi/bilibili-API-collect 已于 2026-01-28 收到 B站委托律所的律师函后**永久关停**（默认分支已改名为 deprecated，文档与源码均已删除），因此本技能的目标不是追上游提交，而是「把已依赖的契约固化在本项目内 + 定期自检签名与接口是否还活着」。当用户说"查 B站接口更新""同步 bilibili-API-collect""B站导入失败""WBI 签名报错 / 412 / -101""B站 Cookie 失效""要给 B站加取消收藏等写操作"，或收藏模块的 B站导入、失效检测、Cookie 校验出现行为异常时使用。先跑 scripts/sync.ps1 拿状态与自检结果，再对照 references/contracts.md 判差异；**改代码前必须先向用户呈报方案并等明确确认**。
---

# B站接口契约维护（bilibili-api-sync）

## 0. 上游现状（2026-09-26 核实，结论优先）

| 项 | 事实 |
|---|---|
| 仓库 | `https://github.com/SocialSisterYi/bilibili-API-collect` |
| 状态 | **已永久关停**。默认分支 = `deprecated`，内容只有 README + reason.jpg |
| 时间 | 2026-01-28 维护者收到 B站委托律所的律师函，指控"系统性收集、整理非公开 API 及访问控制/认证机制并公开传播"侵权 |
| 后果 | 文档与源码已删除；`git log` 只剩 `Update README` / `deprecated` 三条 |
| 本地克隆 | `E:\pro\other-sdk\tools\bilibili-API-collect`（HEAD `4c00347`），仅作"是否复活 / 是否出现继任来源"的观察点 |

**由此确立的三条方针：**

1. **不再以该仓为更新源**。网上流传的镜像/fork（如 `pskdje/bilibili-API-collect`、Gitee 镜像）是同一份被要求删除的内容，**不要整仓再分发进本仓库**——既不合规也会污染仓库体积。
2. **契约自持**：我们真正依赖的算法/常量/端点/错误码，全部固化在 `references/contracts.md`，来源标注为「既有实现 + 历史公开文档」，外链只作历史注记。
3. **以自检代替追更**：判定"接口变了"的权威信号是**我们自己的测试与真实请求**，不是上游文档。

> 现状补充：`src-tauri/src/commands/favorites/wbi.rs` 头注释仍写着「算法与常量来自 bilibili-API-collect 的 docs/misc/sign/wbi.md」——该链接已失效。契约以 `references/contracts.md` 为准，注释里的外链当作历史出处即可，**不要**据此再去联网找文档。

## 1. 我们依赖了什么（文件地图）

| 功能 | 位置 | 关键常量/端点 |
|---|---|---|
| WBI 签名 | `src-tauri/src/commands/favorites/wbi.rs` | 混洗表 `MIXIN_KEY_ENC_TAB[64]`（取前 32）、`mixin_key`、`url_encode`（空格→`%20`、大写十六进制、剔除 `!'()*`）、`sign_query`（参数按 key 升序 + `wts`，尾部拼 mixin_key 取 MD5 = `w_rid`） |
| 取 mid 与 WBI 口令 | `bilibili.rs::fetch_session` | `GET https://api.bilibili.com/x/web-interface/nav` → `data.wbi_img.img_url/sub_url`（`key_from_url` 取文件名主干）、`data.mid` |
| 收藏夹列表 | `bilibili.rs::fetch_folders` | `GET /x/v3/fav/folder/created/list-all`（`up_mid`、`type=2`），WBI 签名 |
| 收藏夹内容分页 | `bilibili.rs::fetch_folder_page` | `GET /x/v3/fav/resource/list`（`media_id`、`pn`、`ps=20`），WBI 签名 |
| 请求头与风控 | `bilibili.rs` | `USER_AGENT`（Chrome/120 伪装）、`REFERER=https://www.bilibili.com/`、页间隔 600ms（**账号安全措施，勿调小**）、HTTP 412 = 风控拦截 |
| 导入编排 | `commands.rs::fav_import_bilibili` / `import_bilibili_inner` | 任务标记 `TASK_BILIBILI`；Cookie 未配置 / `-101 未登录` 都要给可操作提示 |
| 失效检测 | `check.rs::probe` | B站条目走 HTTP 探测（HEAD → 回退带 Range 的 GET），不看 B站接口 |
| 凭据 | `fav_set_credential` + 表 `favorite_credential`（source=`bilibili`） | 存整串 Cookie，含 `SESSDATA`；**`bili_jct` 即 CSRF token，写操作必带** |
| 前端入口 | `src/components/favorites/FavoritesPanel.tsx` | B站导入按钮、Cookie 粘贴弹窗；i18n `favorites.*` |

模块红线（`commands/favorites/mod.rs:6`）：**只读取平台数据，不反向操作平台**（不做 unstar / 取消收藏 / 新建收藏夹）。任何写操作都必须先改这条规则并经用户确认。

## 2. 触发场景

- "查一下 B站接口有没有更新 / 同步 bilibili-API-collect"
- 收藏 → 导入 B站收藏失败，或报 `412` / `-101` / "-403" / "未能从 B站获取 WBI 口令"
- `cargo test --lib` 里 wbi 相关用例失败（**最高优先级信号**：签名算法或常量变了）
- 要给 B站加写操作（取消收藏、建收藏夹）
- 换机器/换账号后验证 Cookie 链路

## 3. 工作流

### Phase 0：先读同步点

读同目录 `sync-point.txt`：记录上次自检时间、本地克隆 HEAD、契约版本、以及"上游已关停"这一状态。以它为起点，避免重复排查。

### Phase 1：取状态与自检

```powershell
powershell -NoProfile -File .agents/skills/bilibili-api-sync/scripts/sync.ps1
# 可选：带真实 Cookie 做一次线上探活（只发 /x/web-interface/nav，只读）
$env:BILI_COOKIE = "<粘贴的 Cookie>"; powershell -NoProfile -File .agents/skills/bilibili-api-sync/scripts/sync.ps1 -Live
```

脚本做四件事：打印 sync-point → 检查本地克隆是否有新提交/是否已复活 → 跑 `cargo test --lib wbi`（签名向量自检）→（`-Live`）打 `nav` 看 `code==0` 且 `data.wbi_img` 是否还在。

### Phase 2：判差异（只关心这四类）

1. **WBI 口令获取变了**：`nav` 不再返回 `wbi_img`，或 key 不再从图片文件名取 → `fetch_session` 报错优先怀疑这里
2. **签名算法/常量变了**：混洗表、`wts` 参数、排序规则、编码细节（空格/大小写/`!'()*`）任一变化 → wbi 单测先炸
3. **接口路径或认证方式变了**：`/x/v3/fav/...` 换路径、改 `type` 语义、要求 app 端鉴权 → 导入报业务错误码
4. **风控阈值变了**：频繁 412、Cookie 提前失效 → 不要靠"重试"，去看页间隔与请求节奏

### Phase 3：呈报（硬规则）

**必须先向用户呈报"发现了什么 + 打算怎么改 + 风险"，等明确确认后才动代码。** 禁止自行判定后直接改 `wbi.rs` / `bilibili.rs`。没有可改的就不要改。

### Phase 4：落地与记录

- 改代码 → 补/更新 wbi 单测向量（真实响应为准）→ 跑 `cargo test --lib favorites`
- 更新 `references/contracts.md` 与 `sync-point.txt`
- 按 `qa-log` 技能登记 qa.db

## 4. 契约卡

完整契约（算法步骤、官方测试向量、端点参数表、错误码表、风控参数）在 **`references/contracts.md`**，需要改动实现时必读；只做状态巡检时不必加载。

## 5. 红线与适配规则

- **默认只读**：收藏模块至今只有 GET。新增任何写操作（取消收藏等）都要：改 `mod.rs` 顶部硬规则说明 → 前端二次确认 → 操作日志 → 明确"平台失败则保留本地条目"的补偿语义。
- **`bili_jct` = CSRF token = 完整写权限**：Cookie 只存本地、只发往 `*.bilibili.com`，不得转发第三方。
- **不要为了快调小页间隔**（600ms）或去掉 412 处理；那是账号安全参数。
- **不要引入 CDP / 浏览器注入 / 本地代理**：我们是独立 Tauri 应用，只走带 Cookie 的 HTTP。
- **不要把第三方镜像仓库的文档整仓拷进本仓库**：只提炼我们实际用到的契约条目。
- **法律与合规背景**：上游因律师函关停。写操作（改动用户账号数据）的风险显著高于只读拉取；新增前先让用户知情并确认。

## 6. 验证清单

- `cargo test --lib favorites`（含 wbi 官方向量）
- 手测：收藏模块 → 配置 B站 Cookie → 导入 B站收藏 → 能看到收藏夹与条目；二次导入 `added = 0`（幂等）
- 负向：清空 Cookie 导入应提示"未配置 B站 Cookie"；过期 Cookie 应提示"Cookie 未生效"
- 改过签名后：重跑导入确认 `w_rid` 被服务端接受（返回 `code == 0`）
