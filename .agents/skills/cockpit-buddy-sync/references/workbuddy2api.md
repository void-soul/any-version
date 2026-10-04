# 参考仓 C：workbuddy2api（我们自己的 fork，Python，2API 协议真源）

> 与参考 A/B 不同：这一仓**我们自己是维护者**（fork 自 `ShouZhuo0413/codebuddy2openai`）。
> 学它的目的不是"抄功能"，而是**跟随上游协议/接口变化**，并保证 Rust 版 2API
> （`src-tauri/src/commands/buddy/twoapi/`）与它的协议语义**不漂移**。

## C.0 仓库与远程布局

| 项 | 值 |
|---|---|
| 路径 | `E:\pro\other-sdk\buddy\workbuddy2api`（可用 `WORKBUDDY2API_DIR` 覆盖） |
| `fork` | `https://github.com/void-soul/codebuddy2api.git` ← **我们的维护分支，推这个** |
| `origin` / `upstream` | `https://github.com/ShouZhuo0413/codebuddy2openai.git` ← 官方 |

**远程命名踩过的坑**：早期 `origin` 指向我们的 fork，后来加 `upstream` 指官方。
`git branch -vv` 显示的 "ahead 27, behind 18" 就是 `origin` 指向 fork 时的假象 ——
**判断是否落后必须看 `origin/main`（官方）**，或直接 `git fetch upstream && git log HEAD..upstream/main`。
推自己的改动一律用 `git push fork main`。

## C.1 Python → Rust 模块对照（改一处必须想另一处）

| Python（fork 内） | 我们的 Rust | 说明 |
|---|---|---|
| `core/converter.py`（FastAPI 路由） | `commands/buddy/twoapi/mod.rs` | 端点集合一致 |
| `core/workbuddy_atrest_crypto.py` | `commands/buddy/twoapi/atrest.rs` | at-rest 密钥获取 + AES-GCM 解密 |
| `core/credentials.py`（登录态读写 + 刷新） | `commands/buddy/twoapi/credentials.rs` | **我们多了 lost-update 防护**（见 C.4） |
| `core/converter.py::get_available_models` | `proxy/convert.rs::normalize_models_response` | 回落链不同（见 C.3） |
| —（Python 无） | `proxy/sse.rs::aggregate_chat_chunks` | **我们独有**：强制流式后聚合 |
| —（Python 无） | `proxy/types.rs::force_upstream_stream` | 我们独有，Python 是硬编码强流 |

**方向是单向的**：协议语义以 Python 版为准（它跑在真实客户端旁边、验证过上游），
实现细节以 Rust 版为准。发现不一致 → 先判断是"上游变了"还是"我们有意分歧"。

## C.2 上游契约（`https://copilot.tencent.com`）

这些是**实测确认**的，写死在前端/后端任何地方之前先确认是否仍成立：

| 事实 | 内容 |
|---|---|
| Base | `https://copilot.tencent.com/v2` |
| 对话 | `POST /v2/chat/completions`（OpenAI 兼容） |
| **权威模型目录** | `GET /v2/enterprises/personal/models` → `{"code":0,"data":{"models":[…]}}` |
| 模型目录**没有** OpenAI 惯例的 `/v2/models` | 我们试过，是 404 |
| **只收流式** | 非流式返回 `code 11101 Non-stream chat request is currently not supported` |
| 鉴权头 | `Authorization` / `X-User-Id` / `X-Enterprise-Id` / `X-Tenant-Id` / `X-Domain` / `User-Agent` |
| 登录态来源 | `%LOCALAPPDATA%/CodeBuddyExtension/Data/Public/auth/workbuddy-desktop.info` |
| at-rest 密钥 | 只能从 **WorkBuddy 桌面端**取（`ELECTRON_RUN_AS_NODE=1` 跑 JS 调原生 `loggerGet()`） |

**鉴权头是 WorkBuddy 特有的**（`X-User-Id` 等），不是 OpenAI 标准 —— proxy 的通用
Bearer 逻辑跑不通，必须走 `ProxyConfig.upstream_headers`。

## C.3 模型目录：三层来源，只有一层权威

| 来源 | 是什么 | 权威性 |
|---|---|---|
| `/v2/enterprises/personal/models` | 后端目录，客户端 UI 用的同一份（asar 里叫 `remote.models`） | ✅ **唯一权威** |
| 本机 `product.json` | 客户端本地清单（`resources/app.asar.unpacked/cli/product.json`） | ❌ 取决于 WorkBuddy 装在哪、版本偏旧、混着已下线模型 |
| `DEFAULT_MODELS`（Python 硬编码） | 读不到前两者的降级表 | ❌ 手写快照，永远漏新模型 |

**新模型只出现在后端目录**。实例：`space-bunny` 两份本地清单都没有，但直接用模型名调得通。
所以**不要拿任何本地清单当"可用模型"结论** —— 我们的 `/v1/models` 已转发后端目录。

过滤规则（图像/视频/内部功能）收敛在 `proxy/convert.rs::is_chat_model`，镜像 Python 的
`_filter_chat_models`。**只有一份**，`parse_catalog`（preflight）也调它。

## C.4 我们相对 Python 版的有意分歧

改动记录在 `sync-point.workbuddy2api.txt`。当前三条 + 各自理由：

1. **`/v1/models` 转发后端目录**（`a40edbd`）—— Python 侧同样改成转发（我们先改的，官方还没跟）
2. **模型目录优先级与回落链的测试**（`90b92d2`）
3. **`.gitignore` 忽略 `qa.db`**（`6fd292c`）—— 纯仓库卫生

Rust 侧另有 Python 没有的：token 自动刷新、切换 lost-update 防护、强制流式聚合。

## C.5 怎么跟进上游

```bash
cd E:/pro/other-sdk/buddy/workbuddy2api
git fetch upstream
git log --oneline HEAD..upstream/main       # 官方新增
git log --oneline upstream/main..HEAD       # 我们领先（推给官方前看这个）
```

**有上游新增时**：读 `core/converter.py` 的 diff → 对照 C.1 表 → 判断 Rust 侧是否要跟 →
跑 `relearn.sh 2api` 看是否落在监听路径上。

**推给官方前**：确认 `upstream/main..HEAD` 里没有 `.deps/`、`converter.log`、`qa.db`。
