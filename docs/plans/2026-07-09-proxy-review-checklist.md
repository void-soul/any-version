# 代理整改 · 代码审查清单

> 本文件用于 HY3 实施完成后的代码审查。审查依据为 `2026-07-09-proxy-remediation-review.md`（HY3 定稿方案）。
>
> 审查时需对照此清单逐项验证，并关注评审中发现但 HY3 方案未完全覆盖的点。

---

## 审查上下文

### 方案文件
- **实施依据**：`docs/plans/2026-07-09-proxy-remediation-review.md`（HY3 评审定稿）
- **原始方案**：`docs/plans/2026-07-09-proxy-remediation-design.md`（HY3 初稿）
- **对照方案**：`docs/plans/2026-07-09-proxy-remediation-design-GLM5.2.md`（GLM5.2 稿）
- **最终方案**：`docs/plans/2026-07-09-proxy-remediation-final.md`（合并方案）

### 核心架构决策（审查基准）
1. **每启动一个工具 = 一个独立代理实例 + 独立空闲端口**（不支持单端口共享多工具）
2. **不引入 IR 中间表示**（保留直接转换器，增量新增 4 个 Google 方向）
3. **优化器/整流器在 P_out（出站协议）形态上操作**
4. **统计在 StreamConverter 内边流边累加**（不引入会破坏流式的中间件）
5. **模型伪装泛化 model_aliases**（不新增 model_disguise 字段）
6. **入站路由按 inbound_protocol 锁定**（不同时注册所有协议路由）

### 管线顺序（9 步）
```
① 统计(强制): total_requests++
② 模型伪装(可选): C → B
③ 协议转换(自动): P_in → P_out
④ 优化器(可选, P_out 形态)
⑤ 转发
⑥ 反应式整流(可选, P_out 形态): 报错→修正→重试
⑦ 响应转换: P_out → P_in
⑧ 模型伪装回填: model 字段写回 C
⑨ 统计(强制): token 落库
```

---

## 审查清单

### P0: 骨架重构

#### ProxyConfig (types.rs)
- [ ] `upstream_protocol` 已移除，改为 `inbound_protocol` + `outbound_protocol`
- [ ] 新增 `tool_id: String` 和 `provider_id: String`
- [ ] 新增 `conversion_mode: String`（冗余观测字段）
- [ ] 新增 `upstream_google_url`（或复用现有字段）
- [ ] 所有新字段有 `#[serde(default)]`，旧配置可自动迁移
- [ ] `upstream_protocol` 有 `#[serde(alias)]` 向后兼容（或迁移逻辑）

#### launch.rs
- [ ] 不再"有 URL 就起 P1+P2 两个代理"，改为只起一个实例
- [ ] `pick_outbound(inbound, provider)` 函数实现正确（同协议透传优先 > anthropic > openai > google）
- [ ] `tool_id` 和 `provider_id` 正确传入 ProxyConfig
- [ ] Google 工具的 baseUrl 指向代理端口（不再直连 `p.google_url`）
- [ ] 端口分配：从 `proxy_port` 起为每个工具分配独立空闲端口
- [ ] **端口回收策略**：工具退出后代理是否停止？（当前代码 `tokio::spawn` 无 JoinHandle 无退出机制——需确认 HY3 是否处理了）

#### server.rs
- [ ] 路由按 `inbound_protocol` 注册（anthropic/openai/google 三选一）
- [ ] `record_proxy_usage` 不再硬编码 `"proxy"`，传入真实 `tool_id` 和 `provider_id`
- [ ] OpenAI 入站时注册 `/v1/models` 透传路由
- [ ] 统一处理管线（`process_request` 或类似函数）覆盖三协议

#### provider.rs
- [ ] `start_proxy` 适配新 ProxyConfig 字段

### P1: Google 协议转换

#### transform.rs
- [ ] 新增 `anthropic_to_google` 请求转换
- [ ] 新增 `openai_to_google` 请求转换
- [ ] 新增 `google_response_to_anthropic` 响应转换
- [ ] 新增 `google_response_to_openai` 响应转换
- [ ] 新增 `GoogleToAnthropicStreamConverter`（Google SSE → Anthropic SSE）
- [ ] 新增 `GoogleToOpenaiStreamConverter`（Google SSE → OpenAI SSE）
- [ ] Google 请求转换正确处理：system→systemInstruction, messages→contents, tools→functionDeclarations, tool_result→functionResponse, thinking→thinkingConfig
- [ ] Google 响应转换正确处理：candidates→content/choices, functionCall→tool_use/tool_calls, thought→thinking/reasoning, finishReason→stop_reason/finish_reason, usageMetadata→usage
- [ ] Google StreamConverter 内累加 usage（边流边记，不缓冲整条流）

#### server.rs (Google handler)
- [ ] Google 入站路由注册（`/v1beta/models/{model}:generateContent` 等）
- [ ] **Axum 路由语法验证**：`{model}:generateContent` 是否能正确匹配 `/v1beta/models/gemini-pro:generateContent`？需确认 Axum 版本的 `{param}` 是否会匹配整个路径段（含 `:generateContent` 后缀）。若不行，是否用了 `{model_and_action}` + 手动 split 的方案？
- [ ] Google 出站鉴权头 `x-goog-api-key`（不是 Bearer）
- [ ] Google 模型名从 URL 路径提取（入站时 model 在 path 中，不在 body 中）
- [ ] Google 流式端点 `?alt=sse` query param 处理

#### Google countTokens
- [ ] 当 inbound=anthropic 调用 `/v1/messages/count_tokens` 且 outbound=google 时，是否映射到 Gemini `:countTokens` 端点？
- [ ] 当 outbound=openai/anthropic 时，count_tokens 是否返回估算值或透传？

### P2: 统计完善

- [ ] 三协议的 StreamConverter 都在流内累加 usage
- [ ] 非流式响应正确解析三协议各自的 usage 格式
- [ ] Google `usageMetadata` 的 `thoughtsTokenCount` 计入 output
- [ ] SQLite 中 `tool_id` 为真实工具 ID（如 "claude-code"、"gemini-cli"）
- [ ] SQLite 中 `model` 为实际模型 B（伪装后的真实上游模型）
- [ ] SQLite 中 `provider` 不为 None
- [ ] `/v1/models` 透传正常工作

### P3: 优化器/整流器协议感知

#### optimizers.rs
- [ ] `apply_optimizers(body, outbound_protocol, config)` 按出站协议分派
- [ ] `cache_injection`：仅 Anthropic 出站注入 `cache_control`；OpenAI/Google 跳过
- [ ] `thinking_optimizer`：Anthropic→`thinking.{type,budget_tokens}`+`anthropic_beta`；Google→`thinkingConfig.{thinkingBudget,includeThoughts}`；OpenAI→`reasoning_effort`（仅 o1/o3 系列）
- [ ] `deepseek_normalize`：保留，按出站 URL 判断
- [ ] `media_fallback`：三协议通用（图片块降级为文本）
- [ ] **优化器在转换后（P_out 形态）调用，不是转换前**（当前代码 `server.rs:584` 是转换前调用——需确认已修正）

#### 整流器
- [ ] 反应式整流：上游报错后在 P_out body 上修正并重试
- [ ] `thinking_signature`：三协议各自的剥离逻辑
- [ ] `thinking_budget`：三协议各自的修正逻辑
- [ ] `media_fallback`：三协议通用
- [ ] 新增 `rectifier_protocol_mismatch`：处理转换残留字段被上游拒绝
- [ ] 重试只重试一次（不无限重试）
- [ ] **预防式整流**：thinking signature 这类高概率触发的，是否在转换后预防式剥离？（HY3 方案是纯反应式——审查时确认是否退步，是否导致 DeepSeek 等场景每次双往返）

### P4: 模型伪装泛化

- [ ] `map_model_name` 支持任意 key（现有代码第 39 行已支持，确认未被破坏）
- [ ] 响应 model 字段恒回填声明名 C（三协议响应转换都回填）
- [ ] Google 入站：model 名从 URL path 提取后做伪装替换
- [ ] Claude Code 的伪装名约束：必须是合法 Claude 模型名/别名
- [ ] 前端 `ModelConfig.tsx`：模型映射区从固定 4 角色下拉改为可增删键值对
- [ ] 前端 `ToolLauncher.tsx`：启动确认区显示"入站 X → 出站 Y｜伪装 C→B"
- [ ] 前端 `types.ts`：同步新字段
- [ ] 前端 `GlobalSettings.tsx`：新增 `protocol_mismatch` 整流器开关

### P5: 测试与文档

- [ ] 9 种转换矩阵各有单元测试
- [ ] 7 工具 × 不同 Provider 端到端验证
- [ ] Google 流式输出不中断
- [ ] 伪装端到端：工具收到 C，上游收到 B
- [ ] 统计准确性：by_model 反映 B
- [ ] 补 `docs/tool-config/google-api.md`

---

## 重点关注项（评审中发现但 HY3 方案未完全覆盖）

### 1. 端口生命周期管理
当前代码 `tokio::spawn` 启动代理后没有 JoinHandle，工具进程退出后代理仍在运行。HY3 提到"端口管理"但未展开回收策略。审查时确认：
- 代理是否随工具进程退出而停止？
- 端口分配表存在哪里？
- 多个代理实例如何管理？

### 2. 预防式 vs 纯反应式整流
HY3 方案是纯反应式整流。但 thinking signature 错误在 DeepSeek 等中转站几乎每次都触发，纯反应式意味着每个请求都多发一次到上游。审查时确认是否引入了预防式剥离（至少对高概率场景）。

### 3. Axum 路由 path 参数与冒号后缀
Google API URL `/v1beta/models/gemini-pro:generateContent` 中 `:generateContent` 是路径段后缀。Axum 的 `{param}` 匹配整个路径段。需确认 HY3 的实现是否能正确拆分 model 名和 action。

### 4. Google countTokens 端点
HY3 评审完全没提及 countTokens。Claude Code 会调用 `/v1/messages/count_tokens`，若 outbound 是 Google 需映射到 `:countTokens`。审查时确认是否处理。

### 5. stats_enabled / masquerade 冗余字段
HY3 提议新增 `stats_enabled`（永远 true）和 `masquerade`（由 alias 非空推导）。这两个字段不必要——统计直接无条件执行，伪装运行时检查 alias 是否非空即可。审查时确认是否加了冗余字段。

### 6. 出站优先级合理性
固定优先级 `anthropic > openai > google` 在 Provider 有多个 URL 时生效。需确认是否有场景下选错出站导致发到错误上游。

### 7. 优化器位置变更的回归
当前代码 `server.rs:584` 优化器在 `anthropic_to_openai` 转换**之前**调用。改为转换后调用可能改变现有行为。需重点回归测试 Claude Code + DeepSeek 组合。

---

## 涉及的核心文件

| 文件 | 路径 | 预期改动 |
|------|------|---------|
| ProxyConfig | `src-tauri/src/proxy/types.rs` | 字段重构 |
| 代理服务器 | `src-tauri/src/proxy/server.rs` | 单端口路由 + 统一管线 + Google handler |
| 协议转换 | `src-tauri/src/proxy/transform.rs` | 新增 4 方向 Google 转换 + StreamConverter |
| 优化器 | `src-tauri/src/proxy/optimizers.rs` | 按协议分派 + 预防/反应式整流 |
| SSE 解析 | `src-tauri/src/proxy/sse.rs` | 不改 |
| 启动逻辑 | `src-tauri/src/commands/ai/launch.rs` | 单实例 + pick_outbound + tool_id 传入 |
| 供应商命令 | `src-tauri/src/commands/ai/provider.rs` | start_proxy 适配 |
| 数据模型 | `src-tauri/src/commands/ai/models.rs` | 可能不改（已有 google_url） |
| 用量统计 | `src-tauri/src/commands/ai/usage.rs` | 不改（已支持 provider 参数） |
| 前端类型 | `src/components/ai/types.ts` | 同步新字段 |
| 模型配置 | `src/components/ai/ModelConfig.tsx` | 键值对 UI |
| 工具启动 | `src/components/ai/ToolLauncher.tsx` | 启动信息条 |
| 全局设置 | `src/components/GlobalSettings.tsx` | protocol_mismatch 开关 |

### 7 个工具的协议
| 工具 | apiProtocol |
|------|-------------|
| claude-code | anthropic |
| codex-cli | openai |
| gemini-cli | google |
| opencode | openai |
| mimocode | openai |
| qwencode | openai |
| deveco | openai（paths.json 写 none 需修正） |
