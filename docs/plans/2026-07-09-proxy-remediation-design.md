# AI 启动代理整改方案（设计稿）

> 目的：把"启动 AI 工具时必开的本地代理"从「只支持 Anthropic↔OpenAI 双协议、且 Google 直连」升级为「统一的三协议（Anthropic / OpenAI / Google）代理」，并把五项能力按"强制 / 自动强制 / 可选"明确分层。
>
> 评审对象：GLM5.2（以及人工）。本文为整改方案，不包含最终实现代码。

---

## 0. 结论速览

| 能力 | 性质 | 当前状态 | 改造要点 |
|------|------|----------|----------|
| ① 统计次数/用量 | **强制（不可关）** | 仅 Anthropic/OpenAI 代理记录了，Google 直连漏记 | 所有协议流量统一过代理，统一落库 |
| ② 协议转换 | **按需自动强制（不可关）** | 仅 Anthropic↔OpenAI | 新增 Google 互转，形成 3×3 转换矩阵 |
| ③ 差异抹平（整流器） | 可选 | 仅 Anthropic 形态 | 改造为协议感知，覆盖转换引入的差异 |
| ④ 请求优化（优化器） | 可选 | 仅 Anthropic 形态 | 改造为协议感知，按上游协议生效 |
| ⑤ 模型伪装 | 可选 | 仅 4 个角色别名（sonnet/opus/haiku/fable） | 泛化为"声明名 C → 实际模型 B"，响应恒报 C |

**核心架构决策**：把"按工具起两个端口（P1=Anthropic、P2=OpenAI）"改为**每次启动只起一个统一代理**，由工具 `api_protocol` 决定入站协议（`inbound_protocol`），由 Provider 实际拥有的 URL 决定出站协议（`outbound_protocol`）；两者不一致时转换**强制开启**（用户无开关）。

---

## 1. 现状与问题

### 1.1 代码现状
- 代理入口：`src-tauri/src/proxy/server.rs` + `transform.rs` + `optimizers.rs` + `sse.rs` + `types.rs`。
- 启动逻辑：`src-tauri/src/commands/ai/launch.rs` 中，凡 Provider 有 URL 就**同时**起 Anthropic 代理（`config.proxy_port`）和 OpenAI 代理（`proxy_port+1`）。
- 协议支持：`transform.rs` 只有 `anthropic_to_openai` / `openai_to_anthropic` 及其响应/SSE 转换。**没有 Google（Gemini）协议**。
- 工具协议（`ai-tools/*/config.json` 的 `apiProtocol`）：
  - `anthropic`：claude-code
  - `openai`：codex-cli、opencode、mimo-code、qwen-code、deveco（注：`deveco/paths.json` 写的是 `none`，`config.json` 写的是 `openai`，以 config.json 为准，需统一）
  - `google`：gemini-cli
- Google 的现状：`launch.rs` 对 `google` 协议直接把 `baseUrl`/`apiKey` 指向 `p.google_url`，**完全绕过代理** → ① 用量统计缺失，② 无法做伪装/优化/整流。

### 1.2 关键矛盾
1. **统计不统一**：Google 直连，用量漏记；同时 Anthropic/OpenAI 代理各自维护一份内存 `ProxyStats`（启动后丢弃），真正落库靠 `usage.rs` 的 SQLite。
2. **转换矩阵缺一角**：当工具协议 ≠ Provider 协议且涉及 Google 时，目前无解（如 gemini-cli 配 OpenAI-only 的中转站）。
3. **整流器/优化器只认 Anthropic**：`optimizers.rs` 的函数全部假设 body 是 Anthropic 形态；OpenAI 透传与 Google 形态都吃不到，且若未来转换把 Anthropic 转成 OpenAI 再发出，优化器应该在"出站协议"上生效而非"入站协议"。
4. **伪装能力被窄化**：`model_aliases` 只暴露 sonnet/opus/haiku/fable 四个角色，无法表达任意"声明名 C → 实际模型 B"的伪装（例如把 `claude-opus-4-20250514` 伪装成 `gpt-4o`）。

---

## 2. 能力模型与开关语义

把五项能力按"强制程度"重新定义，写入 `ProxyConfig`（注意：这里是**单次启动**的配置，是全局 `AiConfig` 里开关的投影）：

```
能力              开关字段                  取值语义
─────────────────────────────────────────────────────────────────
① 统计           stats_enabled             固定 true（代码强制，UI 不暴露）
② 协议转换       conversion_mode           枚举，由矩阵推导，UI 只展示"状态"不提供开关
③ 整流器         rectifier.*（现有）       可选，默认开
④ 优化器         optimizer.*（现有）       可选，默认开
⑤ 模型伪装       masquerade（由 model_aliases 泛化） 可选
```

- **强制项（①）**：即便用户把所有可选开关关掉，统计也必须跑。实现上 `stats_enabled` 由构造 `ProxyConfig` 时写死 `true`，前端不提供复选框。
- **自动强制项（②）**：`conversion_mode` 不是用户选择，而是由 `inbound_protocol × outbound_protocol` 推导（见 §4）。UI 仅在启动确认页显示"本次需要协议转换：Anthropic→OpenAI"之类的只读提示。
- **可选项（③④⑤）**：保持现有 `rectifier` / `optimizer` 子开关；⑤ 新增但不强制。

---

## 3. 统一代理的整体架构

### 3.1 单次启动 = 一个代理实例
`launch.rs` 改为：读 `tool_config.api_protocol` → 得到 `inbound_protocol`；读 Provider 拥有的 URL → 推导 `outbound_protocol`；据此**只起一个**代理，监听一个空闲端口（沿用现有的端口自增探测，但只起一个实例，不再强行起两个）。

### 3.2 入站路由（按 inbound_protocol 注册）
- `anthropic`：注册 `/v1/messages`、`/v1/messages/count_tokens`
- `openai`：注册 `/v1/chat/completions`
- `google`：注册 `/v1beta/models/{model}:generateContent`、`/v1beta/models/{model}:streamGenerateContent`

> 用工具协议锁定入站路由，可避免"同一端口多协议歧义"。若未来要在一个端口同时服务多工具，可改为按 path 探测 + 工具协议白名单。

### 3.3 请求处理管线（核心）
对每条请求，按顺序执行：

```
[入站 P_in 请求]
  1. 统计(强制): total_requests++（归属 = 解析后的实际模型 B + provider）
  2. 模型伪装: 用 masquerade/alias 把"声明名 C"解析为"实际模型 B"
  3. 协议转换: 若 P_in ≠ P_out → 转换为 P_out 形态（强制，自动）
                若 P_in == P_out → 直接透传（仍做第 2 步的模型名替换）
  4. 优化器(可选, 在 P_out 形态上): 按出站协议应用启用的策略
  5. 转发: 携带对应鉴权头，发往 upstream
[上游响应 P_out]
  6. 整流器(可选, 反应式): 若上游报错且命中某规则 → 在 P_out 形态上修正并重试
  7. 协议转换(响应): P_out → P_in
  8. 模型伪装回填: 响应 model 字段写回"声明名 C"
  9. 统计(强制): 记录 input/output token（归属实际模型 B）
```

**关键设计点**：优化器与整流器都在 **出站协议（P_out）形态** 上操作，而不是入站形态。理由：它们的最终目标是"让上游能正确、省钱、鲁棒地接收请求"，在出站形态上做只需为"每个上游协议"写一套策略，而不是为"每个工具协议 × 每个上游协议"写 N×M 套。

---

## 4. 协议转换矩阵（② 的强制规则）

设工具协议 `T ∈ {anthropic, openai, google}`，Provider 拥有的 URL 集合为 `U`。选择出站协议 `P_out` 的优先级：

| T | U 含 anthropic | U 含 openai | U 含 google | 结果 (P_out / 转换) |
|---|---|---|---|---|
| anthropic | ✅ | — | — | anthropic / 透传 |
| anthropic | ❌ | ✅ | — | openai / **A→O（现有）** |
| anthropic | ❌ | ❌ | ✅ | google / **A→G（新增）** |
| openai | — | ✅ | — | openai / 透传 |
| openai | ✅ | ❌ | — | anthropic / **O→A（现有反向）** |
| openai | ❌ | ❌ | ✅ | google / **O→G（新增）** |
| google | — | ✅ | — | openai / **G→O（新增）** |
| google | ✅ | ❌ | — | anthropic / **G→A（新增）** |
| google | — | ❌ | ✅ | google / 透传 |

> 原则：**优先"同协议透传"**；无法同协议时，按上表顺序选第一个可用的 `P_out`，并在 `P_in ≠ P_out` 时**强制转换**。UI 只展示最终结论，不给用户"要不要转换"的开关。

"新增"的四个方向（A→G、O→G、G→O、G→A）是本次改造的协议规范重头戏，要点见 §5 与 §9。

---

## 5. 三大协议互转的规范要点

> 现有 `transform.rs` 已实现 A↔O（参考 cc-switch，符合 Anthropic Messages API 与 OpenAI Chat Completions API）。以下聚焦**与 Google（Gemini Generative Language API）互转**必须对齐的字段，以及既有 A↔O 需要补强的点。

### 5.1 Google（Gemini）REST 规范（出站/入站都要处理）
- 端点：`POST {baseUrl}/v1beta/models/{model}:generateContent`（非流）、`:streamGenerateContent?alt=sse`（流）。
- 请求体（节选）：
  - `contents: [{ role: "user"|"model", parts: [{ text }, { inlineData: { mimeType, data } }] }]`
  - `systemInstruction: { parts: [{ text }] }`
  - `tools: [{ functionDeclarations: [{ name, description, parameters }] }]`
  - `toolConfig: { functionCallingConfig: { mode: "AUTO"|"ANY"|"NONE", allowedFunctionNames: [...] } }`
  - `generationConfig: { maxOutputTokens, temperature, topP, topK, stopSequences, thinkingConfig: { thinkingBudget, includeThoughts } }`
- 响应体（节选）：
  - `candidates: [{ content: { role: "model", parts: [{ text }, { functionCall: { name, args } }, { thought: true, text }] }, finishReason, index }]`
  - `usageMetadata: { promptTokenCount, candidatesTokenCount, totalTokenCount, thoughtsTokenCount }`
- 鉴权：出站到 Google 上游用 `x-goog-api-key: <key>`（或 `?key=`），不沿用 OpenAI 的 `Bearer`/Anthropic 的 `x-api-key`。
- 工具文档来源：`docs/tool-config/gemini-cli/configuration.md` 给出 `GEMINI_API_KEY`、`GOOGLE_GEMINI_BASE_URL`、`thinkingConfig`/`maxOutputTokens` 等字段名，可作为字段命名依据；完整请求/响应 schema 需对照 Google 官方 Generative Language API 参考补全（项目内暂无该参考，实现阶段要补一份 schema 备忘）。

### 5.2 三向映射核心字段（必须对齐）
| 语义 | Anthropic | OpenAI | Google |
|------|-----------|--------|--------|
| 系统提示 | `system`（顶层数组/字符串） | `messages[].role=="system"` | `systemInstruction.parts[].text` |
| 用户/助手轮 | `messages[].role` | `messages[].role` | `contents[].role`（`model`=`assistant`） |
| 文本块 | `content[].type=="text"` | `content` 字符串/数组 | `parts[].text` |
| 图片 | `content[].type=="image"` + base64 | `image_url`(data URI) | `parts[].inlineData{mimeType,data}` |
| 工具定义 | `tools[].{name,description,input_schema}` | `tools[].function.{name,description,parameters}` | `functionDeclarations[].{name,description,parameters}` |
| 工具调用(出) | `content[].type=="tool_use"` | `message.tool_calls[]` | `parts[].functionCall{name,args}` |
| 工具结果(回) | `content[].type=="tool_result"` | `role=="tool"` | `role:"user"` + `parts[].functionResponse{name,response}` |
| 思维链 | `content[].type=="thinking"` | `reasoning_content`（非标准） | `parts[].thought==true` + `thinkingConfig.includeThoughts` |
| 停止原因 | `stop_reason` | `finish_reason` | `finishReason`（`STOP`/`MAX_TOKENS`/`TOOL_CALL`→映射） |
| 用量 | `usage.{input_tokens,output_tokens}` | `usage.{prompt_tokens,completion_tokens}` | `usageMetadata.{promptTokenCount,candidatesTokenCount}` |
| 最大输出 | `max_tokens` | `max_completion_tokens`/`max_tokens` | `generationConfig.maxOutputTokens` |
| 思考开关 | `thinking.{type,budget_tokens}` | 各模型私有（如 `reasoning_effort`） | `generationConfig.thinkingConfig.{thinkingBudget,includeThoughts}` |

> 特别注意：**thinking 在三个协议里语义差异最大**（Anthropic 的 `budget_tokens` vs Google 的 `thinkingBudget` vs OpenAI 各厂商私有字段）。这正是"差异抹平"要重点处理的转换坑（见 §6.3）。

### 5.3 现有 A↔O 需要补强
- 当前 `anthropic_to_openai` 已处理 system/messages/tools/tool_choice，但 `reasoning_content`（DeepSeek 等）只在响应侧回填，请求侧（上游 OpenAI 兼容端点支持 thinking 时）未正向转换 → 需在转换层补齐双向。
- 工具结果回传：Anthropic `tool_result` 在 OpenAI 用 `role:"tool"`；Google 用 `functionResponse`。三者都要在转换层正确处理"助手先 tool_use → 用户回 tool_result"的配对。

---

## 6. 五项能力的落地设计

### 6.1 ① 统计（强制）
- **不再依赖内存 `ProxyStats` 作为唯一来源**：每次请求进入即 `total_requests++`（内存，给 UI 实时展示），响应返回后**立即写入 SQLite**（`usage.rs::log_usage_db`），归属字段：
  - `tool_id`：来自启动上下文（已在 `launch.rs` 持有）
  - `model`：**实际模型 B**（伪装后的真实上游模型，保证计费准确）
  - `provider`：Provider id/name
  - `input_tokens` / `output_tokens`：从上游响应 usage 取（三协议各自解析，见 §5.2）
- **Google 也走代理**后，漏记问题解决。
- UI：用量面板无需改动，只保证三协议都能落库。可选增强：在用量记录里同时记一列 `claimed_model` 以便"按工具看到的名字"分组（非必须，留作扩展）。

### 6.2 ② 协议转换（强制/自动）
- `transform.rs` 新增：
  - `google_to_openai` / `openai_to_google`
  - `google_to_anthropic` / `anthropic_to_google`
  - 对应响应转换 `google_response_to_openai` / `openai_response_to_google` / `google_response_to_anthropic` / `anthropic_response_to_google`
  - Google 流转换器 `GoogleStreamConverter`（输入 Google SSE chunk → 输出 P_in 的 SSE/流格式）
- `server.rs` 按 `inbound_protocol` 选择 handler，handler 内部查 `conversion_mode` 决定走哪条转换路径。
- 转换层只做"结构映射"，**不做**优化/整流（那些在转换之后、P_out 形态上做）。

### 6.3 ③ 差异抹平（整流器，可选）
保留现有 4 条规则（thinking 签名、budget、media 降级 + 已有错误正则），并改造为 **P_out 形态 + 多协议**：
- **转换引入的差异**：例如工具发 Anthropic `thinking.budget_tokens` 给只支持 OpenAI 的上游（OpenAI 无此字段）→ 转换层本应丢弃；但若上游返回"unknown field"类错误，整流器应识别并剥离再重试。新增规则 `rectifier_protocol_mismatch`（可选子开关），专门处理"上游拒绝某个转换后仍残留的协议专有字段"。
- **Google 专属**：`thinkingConfig.includeThoughts` 在某些模型不支持时上游报错 → 整流器剥离 `thinkingConfig` 重试；`functionResponse` 缺失 `name` 等。
- 实现位置：在第 6 步（上游报错后），对 **P_out 形态 body** 应用，然后重新走"转换响应 → 伪装回填 → 统计"。

### 6.4 ④ 请求优化（优化器，可选）
把现有 4 个策略重构成"按 P_out 协议生效"：
- `cache_injection`：仅 Anthropic 上游有意义（注入 `cache_control`）。OpenAI/Google 上游跳过或做等价处理（Google 暂无官方 prompt cache 字段，先跳过）。
- `thinking_optimizer`：改为协议感知——
  - Anthropic：`thinking.{type:"adaptive",...}` + `anthropic_beta`（现有逻辑迁移）
  - Google：设 `generationConfig.thinkingConfig.{thinkingBudget, includeThoughts}`
  - OpenAI：对支持 reasoning 的模型设私有字段（如 `reasoning_effort`），不支持则跳过
- `deepseek_normalize`：保留（OpenAI 兼容端点，剥离 thinking 签名等）。
- `media_fallback`：保留（跨协议通用，图片块降级为文本）。
- 触发时机：第 4 步，在 P_out 形态上。

### 6.5 ⑤ 模型伪装（可选，泛化）
- 把 `model_aliases` 从"仅 4 个角色"泛化为**任意声明名 → 实际模型**的映射表（现有 `map_model_name` 已支持任意 key，只需放开 UI 限制）。
- 新增语义约束：**响应 model 字段恒写"声明名 C"**（当前 `openai_response_to_anthropic` 已把 model 设为 `request_model`，即 C；Google/OpenAI 响应转换也要同样回填 C）。这样工具自始至终以为自己在调用 C。
- UI（见 §8）：在 Provider 模型配置里允许为某个实际模型 B 指定"声明名 C"；在工具启动选择模型时，可选择"以 C 的名义启动"。
- 与转换的关系：伪装解析（C→B）发生在第 2 步，**早于**协议转换，因此转换层拿到的 model 已经是 B，转发给上游的就是 B；响应再回填 C。

---

## 7. 配置与数据模型变更

### 7.1 `proxy::types::ProxyConfig`
- 移除 `upstream_protocol: String`（"openai"|"anthropic"），改为：
  - `inbound_protocol: String`（`anthropic`|`openai`|`google`）
  - `outbound_protocol: String`（同上）
- 新增 `stats_enabled: bool`（构造时写死 `true`）。
- 新增 `masquerade_enabled: bool`（从 `model_aliases` 是否非空推导，亦可显式）。
- 保留 `model_aliases` / `default_model` / `rectifier_*` / `optimizer_*`。
- `conversion_mode`（可选冗余字段，便于观测）：`none | a2o | o2a | a2g | g2a | o2g | g2o`。

### 7.2 `ai::models::AiConfig`（全局）
- ① 统计：不新增开关（强制）。
- ② 转换：不新增开关（自动）。
- ③④：保留 `rectifier` / `optimizer` 现有子开关。
- ⑤ 伪装：可新增 `AiConfig.masquerade: { enabled: bool }` 作为全局总开关（可选），或直接在 Provider 的 `model_aliases` 上体现。建议保留 `model_aliases` 作为唯一事实源，UI 放开任意 key。
- Provider 结构（`AiProvider`）已含 `openai_url`/`anthropic_url`/`google_url`/`model_aliases`/`default_model`，无需改；仅需确保 `model_aliases` 的 key 不再被前端限制为 4 角色。

### 7.3 `launch.rs` 改造
- 不再"有 URL 就起两个代理"，改为：
  1. `inbound = tool_config.api_protocol`
  2. `outbound = pick_outbound(inbound, provider)`（按 §4 矩阵）
  3. 仅起一个代理：`ProxyConfig { inbound_protocol, outbound_protocol, model_aliases, default_model, rectifier_*, optimizer_*, stats_enabled:true, ... }`
  4. 工具配置文件仍写 `baseUrl` 指向该代理端口、`apiKey` 写代理使用的本地 key（或留空由代理带真实 key）。
  5. 端口管理：沿用自增探测，但单一实例；多工具并行时各自分配空闲端口。

---

## 8. 前端改造（`src/components/ai`）

- `ModelConfig.tsx`：
  - 模型映射区从"固定 4 角色下拉"改为**可增删的键值对列表**（声明名 C → 实际模型 B），支持任意声明的伪装名。
  - 三个协议 URL 输入框已存在，保持不变；在保存校验里增加"若工具协议与所选 Provider 协议不同，提示将自动转换"。
- `ToolLauncher.tsx`：
  - 启动确认区新增只读信息条：**"本次代理：入站 X → 出站 Y（自动转换）｜统计已开启｜伪装：C→B"**，让用户清楚强制项与可选项。
  - 整流器/优化器开关沿用现有全局配置入口（建议在设置页，而非每次启动都选）。
- `types.ts`：`AiConfig` 可补 `masquerade?`；`ProxyConfig` 对应 Rust 侧字段同步。

---

## 9. Google 协议支持的实现要点（风险最高部分）

1. **请求转换**：`anthropic_to_google` / `openai_to_google` 需把 messages 规整成 `contents[]`，把 tools 转 `functionDeclarations`，把 thinking 转 `thinkingConfig`。
2. **响应转换**：Google `candidates[].content.parts` → Anthropic `content[]` / OpenAI `choices[].message`；`functionCall` → `tool_use`/`tool_calls`；`thought` 部分 → thinking/reasoning。
3. **SSE**：Google 流是 `data: {candidates:[...]}` 的 JSON 行；需新写 `GoogleStreamConverter`，把累积的 candidates 增量映射回 P_in 的 SSE 事件（参考现有 `AnthropicToOpenaiStreamConverter` 的增量合并思路）。
4. **鉴权头**：出站 Google 用 `x-goog-api-key`，不要沿用 Bearer。
5. **usage 解析**：`usageMetadata` 的 `promptTokenCount`/`candidatesTokenCount`/`thoughtsTokenCount` 分别计入 input/output（thoughts 计入 output 或单独，需在统计层决定归类，建议 thoughts 计入 output）。
6. **`count_tokens` 等价**：Gemini 有 `:countTokens`；若需要，可在 google 入站时把 `/v1/messages/count_tokens` 映射过去，否则返回估算值（现有 Anthropic 端已是估算）。

> 实施前建议先在 `docs/tool-config` 补充一份 `google-api.md`，沉淀 Gemini REST 请求/响应 schema 与字段口径，作为转换层实现的"标准"依据（目前只有 configuration.md 的字段名片段）。

---

## 10. 实施阶段（建议）

- **P0 重构骨架**：`ProxyConfig` 改 `inbound/outbound_protocol`；`launch.rs` 改为单实例；`server.rs` 按入站协议路由；统计统一落库（含 Google 经代理）。
- **P1 Google 入/出转换**：`transform.rs` 新增 4 个方向 + `GoogleStreamConverter`；`server.rs` google handler；鉴权头处理。
- **P2 整流器/优化器协议感知**：把现有策略从"Anthropic 形态"迁移到"P_out 形态"，补充 Google 相关规则与 thinking 跨协议归一。
- **P3 模型伪装泛化**：放开 `model_aliases` 任意 key；响应 model 恒回填 C；前端键值对 UI；启动信息条。
- **P4 测试与文档**：补 `docs/tool-config/google-api.md`；为转换矩阵每个单元写单测（参考 `config.rs` 现有 `#[cfg(test)]` 风格）；端到端验证 7 个工具 × 不同 Provider 组合。

---

## 11. 风险与开放问题

1. **Google 官方 schema 缺口**：项目内仅有 `gemini-cli/configuration.md` 的字段片段，缺完整请求/响应 schema。实现前需补官方参考（或联网核对 Generative Language API）。
2. **thinking 跨协议语义**：Anthropic `budget_tokens`、Google `thinkingBudget`、OpenAI 私有字段三者不能无损互转，伪装/转换时需定义"降级优先级"。
3. **端口冲突与多工具并行**：单实例代理 + 多工具同时运行时的端口分配策略需明确（建议从 `proxy_port` 起为每个运行中的工具分配独立空闲端口）。
4. **`deveco` 协议不一致**：`paths.json` 写 `none`、`config.json` 写 `openai`，需统一为单一事实源。
5. **统计归属口径**：伪装场景下用量按"实际模型 B"还是"声明名 C"分组？建议按 B 保证计费准确，UI 可同时展示 C。

---

## 12. 验收标准（草稿）

- [ ] 7 个工具在"同协议 / 跨协议（含 Google）"Provider 下均能正常对话。
- [ ] 任何组合的用量都进入 SQLite，且 `by_model` 反映实际模型 B。
- [ ] 当 `T ≠ P_out` 时转换自动发生，且无用户开关可关闭它。
- [ ] 整流器/优化器可独立开关，且只对出站协议生效。
- [ ] 配置"声明名 C → 实际 B"后，工具收到的响应 `model` 字段恒为 C，上游收到的是 B。
- [ ] 转换矩阵 9 种情况各有单元测试覆盖。
