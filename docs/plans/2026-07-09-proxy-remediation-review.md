# AI 代理整改方案 · 评审与最优合并方案

> 本文是对 `2026-07-09-proxy-remediation-design.md`（HY3 初稿）与 `2026-07-09-proxy-remediation-design-GLM5.2.md`（GLM5.2 稿）的对比评审，并给出**最终推荐的最优方案**（采用 HY3 的增量式架构 + 吸收 GLM5.2 的具体补充，修正其 3 处技术错误）。
>
> 不改动任何代码，仅作为实施前的定稿依据。

---

## 0. 评审结论（一句话）

GLM5.2 的**分析与文档呈现优于** HY3，且抓到了若干 HY3 没写清的具体 bug；但其实现骨架有 **3 处会直接导致失败/退化**（单端口并发、Axum 路由语法、流式统计中间件）。**最优路径 = HY3 的增量式并发架构 + GLM5.2 的几个具体补充**，放弃 GLM 的 IR 大重写与单端口共享。

---

## 1. GLM5.2 方案的评估

### 1.1 优于 HY3 初稿的部分（应采纳）
- **数据扎实**：统计出 `providers.json` 约 29 个预设，多数 OpenAI-only 或双协议、仅 1 个 Google-only，有力支撑"必须自动转换"。
- **具体 bug 命中**（已核实）：
  - `server.rs:22` 的 `log_usage_db("proxy", …)` 把 `tool_id` 硬编码为 `"proxy"`，导致统计无法按工具分组。HY3 初稿只说"归属实际模型+provider"，未点出此具体缺陷。
- **`/v1/models` 路由缺口**：当前代理只注册 `/v1/messages` 与 `/v1/chat/completions`，工具调用 `GET /v1/models` 会 404。HY3 初稿漏了。
- **伪装按工具区分**：Claude Code 要求伪装名必须是合法 Claude 名/别名，Gemini 的模型名在 URL 路径里。HY3 初稿未展开。
- **文档形态**：数据流图、文件改动清单、分阶段排期更"像交付物"。

### 1.2 必须纠正的 3 处技术错误
1. **单端口共享 = 多工具并发退化（§3.6 / §5.3）**
   GLM 主张"所有工具 baseUrl 统一指向 `proxy_port`、移除 P2"，一个代理实例注册全部路由。但用户会**同时开 Claude + Codex + Gemini**，各自可能选不同 Provider（不同 outbound、不同伪装表）。一个实例只有一份 `ProxyConfig`，无法在同一端口服务多个不同出站的会话。**正确做法是每启动一个独立实例 + 空闲端口**（HY3 初稿）。
2. **Axum 路由语法非法（§3.2.4 / §3.6）**
   `.route("/v1beta/models/:model:generateContent", …)` 不是合法 Axum 路径参数（冒号后跟字面量非法）。应写为 `/v1beta/models/{model}:generateContent`（Axum 0.8 `{param}` 风格）或自行 capture 解析 `:generateContent` 后缀。
3. **统计中间件与流式冲突（§3.1.2 / middleware.rs）**
   通用 Axum 中间件要"解析响应体 usage"必须消费整个 body，而代理对三协议均为 **SSE 流式**转发，会缓冲整条流、破坏流式体验。现有代码正确地在 `StreamConverter` 内边流边累加 usage；GLM 的中间件思路在流式场景不可行。

### 1.3 值得商榷：IR 大重写
GLM §3.3 的 `UnifiedRequest` 统一中间表示优雅，但：
- 要重做已验证可用的 A↔O 转换（GLM 自己承认 Phase 1 风险"需保证现有行为不变"）；
- 需让 IR 完整建模三协议所有字段（cache_control / thinkingConfig / reasoning_effort…），抽象不全就丢字段（它自己用 `raw` 兜底恰恰印证此风险）；
- **结论**：IR 适合作为长期演进，v1 采用 HY3 的**增量式**（保留直接转换器 + 新增 4 个 Google 方向 + 整流器/优化器在出站形态上做），更低风险、不碰已验证路径、流式安全。

---

## 2. 最优合并方案（定稿）

### 2.1 架构原则
- **每启动一个独立代理实例**，监听一个空闲端口（沿用现有端口自增探测，但只起一个实例，不再强行起 P1+P2）。工具配置文件 `baseUrl` 指向该实例端口。
- **入站协议** `inbound` 由工具 `api_protocol` 决定；**出站协议** `outbound` 由 Provider 实际拥有的 URL 按固定优先级推导。
- **所有协议流量统一过代理**（含 Google），由代理统一落库统计。

### 2.2 五大能力分层（强制 / 自动强制 / 可选）
| 层级 | 能力 | 开关 | 说明 |
|------|------|------|------|
| L1 | 用量统计 | **强制**（代码写死 true，UI 不暴露） | 所有请求过代理并落 SQLite，归属真实模型 B + provider + 真实 tool_id |
| L2 | 协议转换 | **自动强制**（矩阵推导，UI 仅展示结论） | `inbound ≠ outbound` 时强制转换，无用户开关 |
| L3 | 差异抹平（整流器） | 可选，默认开 | 反应式，在出站形态上修正并重试 |
| L4 | 请求优化（优化器） | 可选，默认开 | 主动式，在出站形态上按协议生效 |
| L5 | 模型伪装 | 可选，默认关 | 泛化别名映射 C→B，响应恒回填 C |

### 2.3 协议转换矩阵（L2 强制规则）
工具协议 `T` × Provider 拥有的 URL 集合 `U`，出站 `P_out` 选择优先级：**优先同协议透传；否则按 固定优先级** `anthropic > openai > google` 选第一个可用（确定性，不用 `.first()` 的任意顺序）：

| T \ U | 含 anthropic | 含 openai | 含 google | 结果 |
|-------|--------------|-----------|-----------|------|
| anthropic | ✅ | — | — | anthropic / 透传 |
| anthropic | ❌ | ✅ | — | openai / **A→O** |
| anthropic | ❌ | ❌ | ✅ | google / **A→G（新增）** |
| openai | — | ✅ | — | openai / 透传 |
| openai | ✅ | ❌ | — | anthropic / **O→A** |
| openai | ❌ | ❌ | ✅ | google / **O→G（新增）** |
| google | — | ✅ | — | openai / **G→O（新增）** |
| google | ✅ | ❌ | — | anthropic / **G→A（新增）** |
| google | — | ❌ | ✅ | google / 透传 |

> 注：实际 7 个工具中 `claude-code=anthropic`、`codex/opencode/mimo/qwen/deveco=openai`、`gemini-cli=google`；`deveco` 的 `paths.json` 写 `none` 与 `config.json` 的 `openai` 不一致，需统一为 `config.json` 为准。

### 2.4 请求处理管线（核心，流式安全）
```
[入站 P_in 请求]
  L1 统计(强制): total_requests++（归属 = 解析后的实际模型 B + provider + 真实 tool_id）
  L5 伪装解析: 用别名/伪装映射把"声明名 C"解析为"实际模型 B"
  L2 协议转换: P_in → P_out（不一致时强制；一致时透传并仍做模型名替换）
  L4 优化器(可选, 在 P_out 形态上): 按出站协议应用启用策略
  转发: 携带对应鉴权头发往 upstream
[上游响应 P_out]
  L3 整流器(可选, 反应式): 上游报错且命中规则 → 在 P_out 形态修正并重试
  L2 转换响应: P_out → P_in
  L5 伪装回填: 响应 model 字段写回"声明名 C"
  L1 统计(强制): 在流转换器内边流边累加 input/output token（归属实际模型 B）
```
**关键**：整流器/优化器都在**出站协议（P_out）形态**上操作，而非入站；统计在流转换器内累加（不引入会破坏流式的中间件）。

### 2.5 `ProxyConfig` 改动（`proxy/types.rs`）
- 移除 `upstream_protocol: String`，改为：
  - `inbound_protocol: String`（`anthropic`|`openai`|`google`）
  - `outbound_protocol: String`
- 新增 `stats_enabled: bool`（构造写死 `true`）。
- 新增 `tool_id: String` 与 `provider_id: String`（修正 `"proxy"` 硬编码，使统计可按工具/供应商分组）。
- 新增 `conversion_mode: String`（冗余观测字段：`none|a2o|o2a|a2g|g2a|o2g|g2o`）。
- 保留 `model_aliases` / `default_model` / `rectifier_*` / `optimizer_*` / `masquerade`（由 alias 非空推导）。

### 2.6 路由与 Google 支持（修正 GLM 语法错误）
`server.rs` 按 `inbound_protocol` 注册入口路由（每实例只注册该工具的入站路由 + health）：
- `anthropic`：`/v1/messages`、`/v1/messages/count_tokens`
- `openai`：`/v1/chat/completions`、**`/v1/models`**（GLM 补充，透传上游模型列表）
- `google`：`/v1beta/models/{model}:generateContent`、`/v1beta/models/{model}:streamGenerateContent`（**正确 Axum 0.8 语法**）

转换层（`transform.rs`）新增 4 个方向 + 对应响应/SSE 转换：
- `google_to_openai` / `openai_to_google`
- `google_to_anthropic` / `anthropic_to_google`
- 响应：`google_response_to_openai` / `openai_response_to_google` / `google_response_to_anthropic` / `anthropic_response_to_google`
- `GoogleStreamConverter`（输入 Google SSE chunk → 输出 P_in 的 SSE 格式，参考现有 `AnthropicToOpenaiStreamConverter` 的增量合并）

Google 规范要点（依据 `gemini-cli/configuration.md` + Google Generative Language API）：
- 端点：`/v1beta/models/{model}:generateContent`（非流）、`:streamGenerateContent?alt=sse`（流）
- 请求：`contents[]` / `systemInstruction` / `tools[].functionDeclarations[]` / `toolConfig.functionCallingConfig` / `generationConfig.{maxOutputTokens,thinkingConfig}`
- 响应：`candidates[].content.parts[]`（含 `functionCall`、 `thought`）/ `usageMetadata.{promptTokenCount,candidatesTokenCount,thoughtsTokenCount}`
- 鉴权出站用 `x-goog-api-key`（非 Bearer）

### 2.7 整流器 / 优化器（协议感知，在 P_out 形态）
- `optimizers.rs` 现有 4 策略重构成"按 outbound 生效"：
  - `cache_injection`：仅 Anthropic 出站注入 `cache_control`；OpenAI/Google 跳过。
  - `thinking_optimizer`：Anthropic→`thinking.{type,budget_tokens}`+`anthropic_beta`；Google→`thinkingConfig.{thinkingBudget,includeThoughts}`；OpenAI→私有字段（如 `reasoning_effort`），不支持则跳过。
  - `deepseek_normalize` / `media_fallback`：保留（按出站 URL/协议判断）。
- 整流器新增 `rectifier_protocol_mismatch`：专门处理"上游拒绝转换后仍残留的协议专有字段"（如 Anthropic `thinking.budget_tokens` 落到只支持 OpenAI 的上游）。
- 触发时机：整流器为反应式（上游报错后，在 P_out 形态修正并重试）；优化器为请求前（P_out 形态）。

### 2.8 模型伪装（L5，泛化别名 + 按工具校验）
- **机制**：泛化现有 `model_aliases` 为**任意声明名 C → 实际模型 B** 映射（现有 `map_model_name` 已支持任意 key，仅前端放开 4 角色限制）。响应 model 字段恒回填 C（现有 `openai_response_to_anthropic` 已把 model 设为 `request_model`；Google/OpenAI 响应转换同样回填 C）。
- **与别名的关系**：不另起 `model_disguise` 字段（GLM 的两套机制冗余）；同一套映射既服务角色别名也服务精确伪装。UI 上把"模型映射"区从固定 4 角色下拉改为可增删的键值对（C→B）。
- **按工具校验（采纳 GLM）**：
  - Claude Code：C 必须是合法 Claude 名/别名，否则被拒。
  - Gemini：C 出现在 URL 路径 `{model}` 中，代理需从路径提取并替换。
  - OpenCode/Deveco/MiMo：C 需在 `provider.models` 注册，否则工具不认识。

### 2.9 `launch.rs` 改造
1. `inbound = tool_config.api_protocol`
2. `outbound = pick_outbound(inbound, provider)`（按 §2.3 固定优先级）
3. 仅起一个代理：`ProxyConfig { inbound, outbound, tool_id, provider_id, model_aliases, default_model, rectifier_*, optimizer_*, stats_enabled:true, masquerade:!aliases.is_empty() }`
4. 工具配置文件 `baseUrl` 指向该实例端口（Google 工具也从"直连"改为指向代理）
5. 端口管理：从 `proxy_port` 起为每个运行中的工具分配独立空闲端口（支持并发）

### 2.10 前端（`src/components/ai`）
- `ModelConfig.tsx`：模型映射区改为可增删键值对（C→B）；三协议 URL 不变；保存校验提示"工具协议与 Provider 协议不同将自动转换"。
- `ToolLauncher.tsx`：启动确认区加只读信息条——"代理：入站 X → 出站 Y（自动转换）｜统计已开启｜伪装 C→B"。
- `types.ts`：`AiProvider` 同步 `model_aliases` 任意 key；`ProxyConfig` 对应 Rust 字段同步。

---

## 3. 实施阶段（合并排期）

| 阶段 | 内容 | 对应错误/补充 |
|------|------|---------------|
| P0 | `ProxyConfig` 改 `inbound/outbound` + 携带 `tool_id/provider_id`；`launch.rs` 改为单实例+空闲端口；统计落库修正 `"proxy"` 硬编码 | 修 GLM 错误1；采纳 GLM bug 命中 |
| P1 | Google 入/出转换 + `GoogleStreamConverter` + 正确路由语法 + `x-goog-api-key` | 修 GLM 错误2 |
| P2 | 统计在流转换器内累加（三协议）；补 `/v1/models` 透传 | 修 GLM 错误3；采纳 GLM `/v1/models` |
| P3 | 整流器/优化器协议感知（P_out 形态）+ Google 相关规则 + thinking 跨协议 | — |
| P4 | 模型伪装泛化（任意 C→B）+ 响应回填 C + 按工具校验 + 前端键值对 UI | 采纳 GLM 伪装 nuance |
| P5 | 补 `docs/tool-config/google-api.md`（Gemini REST schema 备忘）；转换矩阵单测；端到端验证 7 工具 × 不同 Provider | 防 GLM 风险"Google schema 缺口" |

---

## 4. 风险与开放问题（合并）

1. **Google 官方 schema 缺口**：项目内仅有 `gemini-cli/configuration.md` 字段片段，缺完整 Gemini 请求/响应 schema。P5 先补 `google-api.md`（必要时联网核对 Generative Language API）。
2. **thinking 跨协议语义**：Anthropic `budget_tokens` / Google `thinkingBudget` / OpenAI 私有字段无法无损互转，需定义降级优先级（P3 处理）。
3. **端口与并发**：单实例 + 空闲端口分配已覆盖；多工具并行各自独立。
4. **`deveco` 协议不一致**：统一为 `config.json` 的 `openai` 为事实源。
5. **统计归属口径**：用量按"实际模型 B"保证计费准确，UI 可同时展示声明名 C。

---

## 5. 验收标准（定稿）

- [ ] 7 个工具在"同协议 / 跨协议（含 Google）"Provider 下均能正常对话（含并发启动）。
- [ ] 任何组合用量进 SQLite，`tool_id` 为真实工具、`model` 为实际模型 B、`provider` 正确。
- [ ] `T ≠ P_out` 时转换自动发生，无用户开关可关。
- [ ] 整流器/优化器可独立开关，且仅对出站协议生效。
- [ ] 配置 C→B 后，工具收到响应 `model` 恒为 C，上游收到 B。
- [ ] 转换矩阵 9 种情况各有单测。
- [ ] `GET /v1/models` 正常透传。
