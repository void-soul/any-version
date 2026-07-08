# AI 代理系统整改方案（最终版）

> 综合 HY3 与 GLM5.2 两份方案的优点，基于对现有代码的逐行审计，制定本实施蓝图。
>
> 核心原则：**增量改造、最小破坏、务实优先**。

---

## 0. 方案选型结论

| 决策点 | 选定方案 | 理由 |
|--------|---------|------|
| IR 中间表示 | **不引入** | 现有 `transform.rs` 的直接转换已可用，IR 会推倒重来且引入字段丢失风险；优化器/整流器改为在 P_out 形态上操作即可达到"协议无关"效果 |
| 单端口 vs 双端口 | **单端口** | 消除 P1/P2 冗余，Google 也有端口可用，配置写入逻辑简化 |
| 模型伪装 | **泛化 `model_aliases`** | 后端 `map_model_name` 已是 `HashMap<String,String>` 查找，只需放开前端 4 角色限制；不新增字段 |
| 优化器/整流器位置 | **P_out 形态** | 每个上游协议只写一套策略（如 cache_injection 只写 Anthropic 版），而非 N×M 套 |
| 入站路由 | **按 inbound_protocol 锁定** | 避免同一端口多协议歧义 |
| 整流器模式 | **预防式 + 反应式混合** | 高概率触发的（thinking signature）在转换后预防式剥离；不确定的（unknown field）反应式重试 |

---

## 1. 现状审计（基于代码逐行确认）

### 1.1 代理核心文件现状

| 文件 | 行数 | 关键发现 |
|------|------|---------|
| `types.rs` | 86 | `ProxyConfig` 有 `upstream_protocol: String`（仅 "openai"/"anthropic"），无 Google；`upstream_base_url` + `upstream_anthropic_url` 双 URL 字段；无 `tool_id`/`provider_id` |
| `server.rs` | 765 | 路由按 `upstream_protocol` 二选一注册；`messages_handler` 中 A→O 转换 + 优化器在 **Anthropic 形态（转换前）** 上操作；`chat_completions_handler` 不走优化器/整流器；`record_proxy_usage` 硬编码 `tool_id="proxy"` |
| `transform.rs` | 1057 | A↔O 双向转换完整（请求+响应+SSE）；`map_model_name` 已支持任意 key 精确匹配（第 39 行），但 `resolve_role` 内置 4 角色归并逻辑 |
| `optimizers.rs` | 390 | 6 个策略全部操作 Anthropic body；`is_deepseek_url` 按 URL 关键词匹配；`optimize_thinking` 按模型名分支 |
| `sse.rs` | 66 | 通用 SSE 解析，协议无关，无需改动 |
| `launch.rs` | 797 | 双端口启动（P1=anthropic, P2=openai）；Google 直接用 `p.google_url` 绕过代理；`effective_base_url` 按协议选端口 |

### 1.2 工具协议确认

| 工具 | apiProtocol | 走代理？ | 配置写入方式 |
|------|-------------|---------|-------------|
| claude-code | `anthropic` | ✅ P1 | env.ANTHROPIC_BASE_URL |
| codex-cli | `openai` | ✅ P2 | config.toml base_url |
| gemini-cli | `google` | ❌ **直连** | env.GOOGLE_GEMINI_BASE_URL |
| opencode | `openai` | ✅ P2 | opencode.json baseURL |
| mimocode | `openai` | ✅ P2 | mimocode.json baseURL |
| qwencode | `openai` | ✅ P2 | env.OPENAI_BASE_URL |
| deveco | `openai` | ✅ P2 | opencode.json baseURL |

### 1.3 核心问题清单

| # | 问题 | 根因（代码位置） | 严重度 |
|---|------|-----------------|--------|
| P1 | Google 完全绕过代理 | `launch.rs:139` `"google" => &p.google_url` + `launch.rs:313` `"google" => p.google_url.clone()` | 🔴 |
| P2 | 用量统计 tool_id 恒为 "proxy" | `server.rs:22` `log_usage_db("proxy", model, None, ...)` | 🔴 |
| P3 | 优化器/整流器仅在 Anthropic 入站时生效 | `server.rs:585-595` 优化器在 `messages_handler` 内调用；`chat_completions_handler` 无优化器 | 🟡 |
| P4 | 优化器在转换前操作（Anthropic 形态） | `server.rs:584` 先 `optimized_body` 再 `anthropic_to_openai`，若上游是 OpenAI 则优化器注入的 `cache_control` 等字段被转换丢弃 | 🟡 |
| P5 | 模型别名前端限制为 4 角色 | `ModelConfig.tsx` 下拉框仅 sonnet/opus/haiku/fable；后端 `map_model_name` 第 39 行已支持任意 key | 🟡 |
| P6 | 响应 model 字段回填不一致 | `openai_response_to_anthropic:418` 回填 `request_model`（即 C），但 `chat_completions_handler` 透传模式下不回填 | 🟢 |
| P7 | `deveco/paths.json` 写 `none` | 配置不一致 | 🟢 |

---

## 2. 目标架构

### 2.1 单端口统一代理

```
工具启动 → launch.rs
  ├─ Step 1: 推导 inbound_protocol（来自 tool_config.api_protocol）
  ├─ Step 2: 推导 outbound_protocol（来自 Provider 可用 URL，按矩阵选优）
  ├─ Step 3: 只起一个代理实例（单端口，按 inbound 注册路由）
  ├─ Step 4: 写工具配置文件（baseUrl 统一指向该端口）
  ├─ Step 5: spawn 子进程
  └─ Step 6: 保存会话
```

### 2.2 请求处理管线（9 步）

```
[入站 P_in 请求]
  ① 统计(强制): total_requests++  →  内存计数器
  ② 模型伪装(可选): 声明名 C → 实际模型 B  →  替换 body.model
  ③ 协议转换(自动): P_in → P_out  →  若相同则透传
  ④ 优化器(可选, P_out 形态): 按出站协议应用策略
  ⑤ 预防式整流(可选, P_out 形态): 高概率问题预先剥离
  ⑥ 转发: 携带对应鉴权头，发往 upstream
[上游响应 P_out]
  ⑦ 反应式整流(可选): 若上游报错 → 修正 P_out body → 重试一次
  ⑧ 协议转换(响应): P_out → P_in
  ⑨ 模型伪装回填: 响应 model 字段写回声明名 C
  ⑩ 统计(强制): 记录 input/output token →  SQLite
```

**关键设计决策**：

- **④⑤ 在 P_out 形态上操作**（HY3 方案核心）：优化器的目标是"让上游省钱/鲁棒"，应在上游能理解的格式上操作。例如 `cache_control` 是 Anthropic 请求体字段，若上游是 Anthropic，在 Anthropic 格式上注入即可，无需 IR 中转。
- **⑤ 预防式 + ⑦ 反应式混合**（吸收 GLM5.2 的好观点）：thinking signature 这类几乎必然触发的整流，在转换后预防式剥离（不等报错）；unknown field error 这类不确定的，反应式重试。

### 2.3 协议转换矩阵

| 工具协议 (P_in) | Provider 有 A URL | Provider 有 O URL | Provider 有 G URL | → P_out | 转换 |
|:-:|:-:|:-:|:-:|:-:|:-:|
| anthropic | ✅ | — | — | anthropic | 透传 |
| anthropic | ❌ | ✅ | — | openai | **A→O（现有）** |
| anthropic | ❌ | ❌ | ✅ | google | **A→G（新增）** |
| openai | — | ✅ | — | openai | 透传 |
| openai | ✅ | ❌ | — | anthropic | **O→A（现有）** |
| openai | ❌ | ❌ | ✅ | google | **O→G（新增）** |
| google | — | ✅ | — | openai | **G→O（新增）** |
| google | ✅ | ❌ | — | anthropic | **G→A（新增）** |
| google | — | ❌ | ✅ | google | 透传 |

**P_out 选择优先级**：同协议透传 > Anthropic > OpenAI > Google（按转换实现难度排序，已有的优先）。

---

## 3. 详细设计

### 3.1 ProxyConfig 改造（`types.rs`）

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub listen_address: String,
    pub listen_port: u16,

    // ─── 协议（替代原 upstream_protocol）───
    /// 入站协议：工具说话的协议（anthropic | openai | google）
    pub inbound_protocol: String,
    /// 出站协议：发往上游的协议（同上）
    pub outbound_protocol: String,
    /// 转换模式（冗余观测字段）："none" | "a2o" | "o2a" | "a2g" | "g2a" | "o2g" | "g2o"
    #[serde(default)]
    pub conversion_mode: String,

    // ─── 上游连接（三协议 URL，非空 = 可用）───
    pub upstream_api_key: String,
    #[serde(default)]
    pub upstream_openai_url: String,    // 原 upstream_base_url 改名
    #[serde(default)]
    pub upstream_anthropic_url: String,
    #[serde(default)]
    pub upstream_google_url: String,    // 新增

    // ─── 模型 ───
    pub target_model: String,
    #[serde(default)]
    pub model_aliases: HashMap<String, String>,
    #[serde(default)]
    pub default_model: Option<String>,

    // ─── 统计（强制）───
    /// 归属工具 ID（用于 SQLite 落库）
    pub tool_id: String,
    /// 归属供应商 ID
    #[serde(default)]
    pub provider_id: String,

    // ─── 整流器/优化器（沿用现有开关）───
    #[serde(default)] pub rectifier_enabled: bool,
    #[serde(default)] pub rectifier_thinking_signature: bool,
    #[serde(default)] pub rectifier_thinking_budget: bool,
    #[serde(default)] pub rectifier_media_fallback: bool,
    #[serde(default)] pub rectifier_protocol_mismatch: bool,  // 新增
    #[serde(default)] pub optimizer_enabled: bool,
    #[serde(default)] pub optimizer_cache_injection: bool,
    #[serde(default)] pub optimizer_thinking: bool,
    #[serde(default)] pub optimizer_deepseek: bool,

    pub timeout_secs: u64,
}
```

**变更要点**：
- `upstream_protocol` → `inbound_protocol` + `outbound_protocol` + `conversion_mode`
- `upstream_base_url` → `upstream_openai_url`（语义更清晰）
- 新增 `upstream_google_url`
- 新增 `tool_id` + `provider_id`（解决 P2：统计归属）
- 新增 `rectifier_protocol_mismatch`（处理转换残留字段被上游拒绝的情况）
- 统计无开关字段——代码中写死强制执行

### 3.2 launch.rs 改造

**核心变更**：从"双端口"改为"单端口 + 协议推导"。

```rust
// Step 1: 推导协议
let inbound_protocol = tool_config.api_protocol.as_str();
let outbound_protocol = pick_outbound(inbound_protocol, p);
let conversion_mode = derive_conversion_mode(inbound_protocol, outbound_protocol);

// Step 2: 只起一个代理
let proxy_config = ProxyConfig {
    listen_port: config.proxy_port,
    inbound_protocol: inbound_protocol.to_string(),
    outbound_protocol: outbound_protocol.to_string(),
    conversion_mode,
    upstream_openai_url: p.openai_url.clone(),
    upstream_anthropic_url: p.anthropic_url.clone(),
    upstream_google_url: p.google_url.clone(),
    upstream_api_key: p.api_key.clone(),
    tool_id: req.tool_id.clone(),          // ← 关键：传递工具 ID
    provider_id: p.id.clone(),              // ← 关键：传递供应商 ID
    // ... rectifier/optimizer from config ...
};

// Step 3: 所有协议的 baseUrl 统一指向同一端口
let effective_base_url = format!("http://127.0.0.1:{}", config.proxy_port);
// ↑ google 也指向代理，不再直连！
```

**`pick_outbound` 函数**：
```rust
fn pick_outbound(inbound: &str, p: &AiProvider) -> &str {
    // 优先同协议透传
    match inbound {
        "anthropic" if !p.anthropic_url.is_empty() => "anthropic",
        "openai"    if !p.openai_url.is_empty()    => "openai",
        "google"    if !p.google_url.is_empty()    => "google",
        // 无法同协议，按优先级选第一个可用的
        _ => {
            if !p.anthropic_url.is_empty()    { "anthropic" }
            else if !p.openai_url.is_empty()  { "openai" }
            else if !p.google_url.is_empty()  { "google" }
            else { "openai" } // fallback
        }
    }
}
```

### 3.3 server.rs 改造

**路由注册**——按 `inbound_protocol` 锁定：

```rust
let mut app = Router::new().route("/health", get(health_handler));

match config.inbound_protocol.as_str() {
    "anthropic" => {
        app = app
            .route("/v1/messages", post(messages_handler))
            .route("/v1/messages/count_tokens", post(count_tokens_handler));
    }
    "openai" => {
        app = app.route("/v1/chat/completions", post(chat_completions_handler));
    }
    "google" => {
        app = app
            .route("/v1beta/models/:model/generateContent", post(google_handler))
            .route("/v1beta/models/:model/streamGenerateContent", post(google_stream_handler));
    }
    _ => return Err(format!("未知入站协议: {}", config.inbound_protocol)),
}
```

> **注意**：Axum 路由 path 中 `:generateContent` 这类带冒号的路径需要用 `/v1beta/models/{model}/generateContent`（Axum 0.7+ 语法）或拆分为 path + query param。实现时需验证 Axum 版本的路由语法。

**统一处理管线**——将 `messages_handler` 和 `chat_completions_handler` 的公共逻辑提取为统一函数：

```rust
async fn process_request(
    state: &ProxyState,
    headers: &HeaderMap,
    body: Value,          // P_in 形态
    inbound_protocol: &str,
) -> Response {
    let config = state.config.read().await.clone();

    // ① 统计：total_requests++
    { state.stats.write().await.total_requests += 1; }

    // ② 模型伪装：C → B
    let (claimed_model, actual_model) = resolve_model(&body, &config);
    let mut body = body;
    set_model(&mut body, &actual_model, inbound_protocol);

    // ③ 协议转换：P_in → P_out
    let outbound_body = convert_request(&body, inbound_protocol, &config.outbound_protocol, &config);

    // ④ 优化器（P_out 形态）
    let mut optimized = outbound_body;
    if config.optimizer_enabled {
        apply_optimizers(&mut optimized, &config);
    }

    // ⑤ 预防式整流（P_out 形态）
    if config.rectifier_enabled {
        apply_preventive_rectifiers(&mut optimized, &config);
    }

    // ⑥ 转发
    let (upstream_url, auth_headers) = build_upstream(&config);
    let resp = send_upstream(&state.client, &upstream_url, &auth_headers, &optimized).await;

    // ⑦ 反应式整流
    if !resp.status().is_success() {
        if let Some(rectified) = try_reactive_rectify(&resp, &optimized, &config).await {
            // 重试一次
            let retry_resp = send_upstream(&state.client, &upstream_url, &auth_headers, &rectified).await;
            if retry_resp.status().is_success() {
                return process_response(retry_resp, &config, &claimed_model, inbound_protocol).await;
            }
        }
    }

    // ⑧⑨⑩ 响应处理
    process_response(resp, &config, &claimed_model, inbound_protocol).await
}
```

### 3.4 transform.rs 改造

**保留现有 A↔O 转换不变**，新增 4 个方向的 Google 转换：

```rust
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  Google (Gemini) 转换（新增）
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Anthropic → Google Gemini 请求
pub fn anthropic_to_google(body: &Value, model: &str, aliases: Option<&ModelAliases>) -> Value {
    // system → systemInstruction.parts[].text
    // messages[].content[] → contents[].parts[]
    //   text → parts[].text
    //   image → parts[].inlineData{mimeType,data}
    //   tool_use → parts[].functionCall{name,args}
    //   tool_result → contents[].role:"user" + parts[].functionResponse{name,response}
    // tools[] → tools[].functionDeclarations[]
    // thinking.budget_tokens → generationConfig.thinkingConfig.thinkingBudget
    // max_tokens → generationConfig.maxOutputTokens
    // temperature → generationConfig.temperature
    // stream → 保持（Google 流式用 ?alt=sse query param）
}

/// OpenAI → Google Gemini 请求
pub fn openai_to_google(body: &Value, model: &str, aliases: Option<&ModelAliases>) -> Value { ... }

/// Google Gemini → Anthropic 响应
pub fn google_response_to_anthropic(google_resp: &Value, request_model: &str) -> Value {
    // candidates[].content.parts[] → content[]
    //   text → {type:"text",text}
    //   functionCall → {type:"tool_use",id,name,input}
    //   thought:true → {type:"thinking",thinking}
    // finishReason → stop_reason (STOP→end_turn, MAX_TOKENS→max_tokens, ...)
    // usageMetadata → usage{input_tokens,output_tokens}
}

/// Google Gemini → OpenAI 响应
pub fn google_response_to_openai(google_resp: &Value, request_model: &str) -> Value { ... }

/// Google SSE chunk → Anthropic SSE 事件
pub struct GoogleToAnthropicStreamConverter { ... }

/// Google SSE chunk → OpenAI SSE chunk
pub struct GoogleToOpenaiStreamConverter { ... }
```

**Google 请求体规范**（实现依据）：

```
端点: POST {baseUrl}/v1beta/models/{model}:generateContent  (非流)
      POST {baseUrl}/v1beta/models/{model}:streamGenerateContent?alt=sse  (流)
鉴权: x-goog-api-key: <key>

请求体:
{
  "contents": [
    {
      "role": "user" | "model",
      "parts": [
        { "text": "..." },
        { "inlineData": { "mimeType": "image/png", "data": "base64..." } },
        { "functionCall": { "name": "...", "args": {...} } },
        { "functionResponse": { "name": "...", "response": {...} } }
      ]
    }
  ],
  "systemInstruction": { "parts": [{ "text": "..." }] },
  "tools": [{ "functionDeclarations": [{ "name": "...", "description": "...", "parameters": {...} }] }],
  "toolConfig": { "functionCallingConfig": { "mode": "AUTO"|"ANY"|"NONE" } },
  "generationConfig": {
    "maxOutputTokens": 8192,
    "temperature": 1.0,
    "topP": 0.95,
    "topK": 40,
    "stopSequences": [...],
    "thinkingConfig": { "thinkingBudget": 0, "includeThoughts": false }
  }
}

响应体:
{
  "candidates": [{
    "content": {
      "role": "model",
      "parts": [
        { "text": "..." },
        { "functionCall": { "name": "...", "args": {...} } },
        { "thought": true, "text": "..." }
      ]
    },
    "finishReason": "STOP" | "MAX_TOKENS" | "SAFETY" | "RECITATION" | "OTHER",
    "index": 0
  }],
  "usageMetadata": {
    "promptTokenCount": 100,
    "candidatesTokenCount": 200,
    "totalTokenCount": 300,
    "thoughtsTokenCount": 50
  }
}
```

**三协议字段映射对照表**：

| 语义 | Anthropic | OpenAI | Google |
|------|-----------|--------|--------|
| 系统提示 | `system` | `messages[].role=="system"` | `systemInstruction.parts[].text` |
| 用户/助手轮 | `messages[].role` | `messages[].role` | `contents[].role`（`model`=`assistant`） |
| 文本块 | `content[].type=="text"` | `content` str/array | `parts[].text` |
| 图片 | `content[].type=="image"` + base64 | `image_url` data URI | `parts[].inlineData{mimeType,data}` |
| 工具定义 | `tools[].{name,description,input_schema}` | `tools[].function.{name,description,parameters}` | `functionDeclarations[].{name,description,parameters}` |
| 工具调用(出) | `content[].type=="tool_use"` | `message.tool_calls[]` | `parts[].functionCall{name,args}` |
| 工具结果(回) | `content[].type=="tool_result"` | `role:"tool"` | `role:"user"` + `parts[].functionResponse{name,response}` |
| 思维链 | `content[].type=="thinking"` | `reasoning_content` | `parts[].thought==true` |
| 停止原因 | `stop_reason` | `finish_reason` | `finishReason` |
| 用量 | `usage.{input_tokens,output_tokens}` | `usage.{prompt_tokens,completion_tokens}` | `usageMetadata.{promptTokenCount,candidatesTokenCount}` |
| 最大输出 | `max_tokens` | `max_completion_tokens` | `generationConfig.maxOutputTokens` |
| 思考开关 | `thinking.{type,budget_tokens}` | `reasoning_effort`(私有) | `generationConfig.thinkingConfig.{thinkingBudget,includeThoughts}` |

### 3.5 optimizers.rs 改造

**核心变更**：从"只在 Anthropic 形态上操作"改为"按 P_out 协议分派"。

```rust
/// 按出站协议应用优化器（在 P_out body 上操作）
pub fn apply_optimizers(body: &mut Value, outbound_protocol: &str, config: &ProxyConfig) {
    match outbound_protocol {
        "anthropic" => {
            if config.optimizer_cache_injection {
                inject_cache_breakpoints(body);  // 现有，无需改
            }
            if config.optimizer_thinking {
                optimize_thinking(body);  // 现有，无需改
            }
            if config.optimizer_deepseek {
                normalize_deepseek_thinking(body, &config.upstream_anthropic_url);
            }
        }
        "openai" => {
            if config.optimizer_thinking {
                optimize_thinking_openai(body);  // 新增：设 reasoning_effort 等
            }
            // cache_injection: OpenAI 无 cache_control 概念，跳过
            // deepseek_normalize: 仅 Anthropic 形态有意义，跳过
        }
        "google" => {
            if config.optimizer_thinking {
                optimize_thinking_google(body);  // 新增：设 thinkingConfig
            }
            // cache_injection: Google 无 prompt cache 字段，跳过
        }
        _ => {}
    }
}

/// Google thinking 优化
fn optimize_thinking_google(body: &mut Value) {
    // 在 generationConfig.thinkingConfig 中设置 thinkingBudget + includeThoughts
    if let Some(gc) = body.get_mut("generationConfig") {
        if let Some(o) = gc.as_object_mut() {
            o.insert("thinkingConfig".into(), json!({
                "thinkingBudget": 0,  // 0 = 动态思考（Gemini 2.5+）
                "includeThoughts": true
            }));
        }
    }
}

/// OpenAI thinking 优化
fn optimize_thinking_openai(body: &mut Value) {
    // 对支持 reasoning 的模型设 reasoning_effort
    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("");
    if model.contains("o1") || model.contains("o3") || model.contains("o4") {
        if let Some(o) = body.as_object_mut() {
            o.insert("reasoning_effort".into(), json!("high"));
        }
    }
}
```

**现有优化器保留情况**：

| 优化器 | Anthropic P_out | OpenAI P_out | Google P_out | 改动 |
|--------|:-:|:-:|:-:|------|
| `inject_cache_breakpoints` | ✅ 原样 | ❌ 跳过 | ❌ 跳过 | 无 |
| `optimize_thinking` | ✅ 原样 | ✅ 新增 `_openai` | ✅ 新增 `_google` | 拆分 |
| `normalize_deepseek_thinking` | ✅ 原样 | ❌ 跳过 | ❌ 跳过 | 无 |
| `strip_thinking_blocks` | ✅ 原样 | ✅ 新增 `_openai` | ✅ 新增 `_google` | 拆分 |
| `fix_thinking_budget` | ✅ 原样 | ✅ 新增 | ✅ 新增 | 拆分 |
| `replace_image_blocks` | ✅ 原样 | ✅ 通用（已支持 image_url） | ✅ 新增 `_google` | 微调 |

### 3.6 整流器改造（预防式 + 反应式）

```rust
/// 预防式整流：在发送前对 P_out body 做高概率修正
pub fn apply_preventive_rectifiers(body: &mut Value, outbound_protocol: &str, config: &ProxyConfig) {
    if !config.rectifier_enabled { return; }

    match outbound_protocol {
        "anthropic" => {
            // thinking signature：几乎必然触发的场景才预防式
            if config.rectifier_thinking_signature {
                strip_thinking_blocks(body);  // 剥离历史 thinking + signature
            }
        }
        "openai" => {
            // OpenAI 形态无 thinking signature 问题，跳过
        }
        "google" => {
            // Google: 某些模型不支持 includeThoughts，预防式设为 false
            // （若用户显式开了 includeThoughts 但模型不支持，反应式再修）
        }
        _ => {}
    }
}

/// 反应式整流：上游报错后尝试修正并重试
pub async fn try_reactive_rectify(
    resp: &reqwest::Response,
    body: &Value,
    config: &ProxyConfig,
) -> Option<Value> {
    let status = resp.status().as_u16();
    let error_body = resp.text().await.unwrap_or_default();

    if config.rectifier_thinking_budget && is_thinking_budget_error(status, &error_body) {
        let mut fixed = body.clone();
        fix_thinking_budget(&mut fixed);  // 按协议分派
        return Some(fixed);
    }

    if config.rectifier_media_fallback && is_unsupported_image_error(status, &error_body) {
        let mut media_body = body.clone();
        if replace_image_blocks(&mut media_body) > 0 {
            return Some(media_body);
        }
    }

    // 新增：协议不匹配整流
    if config.rectifier_protocol_mismatch && is_unknown_field_error(status, &error_body) {
        let mut cleaned = body.clone();
        strip_unknown_fields(&mut cleaned);  // 剥离转换残留的协议专有字段
        return Some(cleaned);
    }

    None
}
```

### 3.7 模型伪装泛化

**后端**：`map_model_name` 已支持任意 key（第 39 行精确匹配），无需改。只需确保转换函数在调用 `map_model_name` 后，用返回值替换 `body.model`。

**响应回填**：所有响应转换函数的 `model` 字段统一写回"声明名 C"（即请求中的原始模型名）。

```rust
// 在 process_response 中统一处理
fn process_response(resp: Value, claimed_model: &str, inbound_protocol: &str) -> Response {
    let mut resp = resp;
    // 统一回填声明名 C
    set_response_model(&mut resp, claimed_model, inbound_protocol);
    // ...
}
```

**前端**：`ModelConfig.tsx` 的模型映射区从"固定 4 角色下拉"改为**可增删的键值对列表**。

**各工具的模型名校验规则**（实现伪装时必须遵守）：

| 工具 | 伪装名约束 | 拦截点 |
|------|-----------|--------|
| Claude Code | 必须是 Claude 模型名或角色别名（`claude-sonnet-4-*` / `sonnet` / `opus` 等） | 请求 body.model |
| Codex CLI | 任意字符串 | 请求 body.model |
| Gemini CLI | 任意模型名（出现在 URL 路径中） | URL path: `/v1beta/models/{model}:generateContent` |
| OpenCode/Deveco | 需在 `provider.models` 中注册 | 请求 body.model |
| MiMo Code | 任意字符串 | 请求 body.model |
| Qwen Code | 任意模型名 | 请求 body.model |

### 3.8 统计改造

```rust
// server.rs 中
fn record_proxy_usage(
    tool_id: &str,
    provider_id: &str,
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
) {
    crate::commands::ai::usage::log_usage_db(
        tool_id,
        model,
        Some(provider_id),
        input_tokens,
        output_tokens,
    );
}
```

**usage.rs 的 `log_usage_db` 已支持 `provider: Option<&str>`**，无需改表结构。只需：
1. `ProxyConfig` 携带 `tool_id` + `provider_id`
2. `server.rs` 调用时传入真实值（不再硬编码 `"proxy"`）

**Google usage 解析**：
```rust
// Google usageMetadata → (input, output)
fn parse_google_usage(resp: &Value) -> (u64, u64) {
    let usage = resp.get("usageMetadata");
    let input = usage
        .and_then(|u| u.get("promptTokenCount"))
        .and_then(|v| v.as_u64()).unwrap_or(0);
    let output = usage
        .and_then(|u| u.get("candidatesTokenCount"))
        .and_then(|v| v.as_u64()).unwrap_or(0);
    // thoughtsTokenCount 计入 output
    let thoughts = usage
        .and_then(|u| u.get("thoughtsTokenCount"))
        .and_then(|v| v.as_u64()).unwrap_or(0);
    (input, output + thoughts)
}
```

---

## 4. 文件改动清单

### 4.1 后端 Rust

| 文件 | 改动类型 | 改动量 | 说明 |
|------|---------|--------|------|
| `proxy/types.rs` | **重构** | 中 | `ProxyConfig` 改字段：`inbound/outbound_protocol`、`upstream_openai_url`、`tool_id`/`provider_id`、`rectifier_protocol_mismatch` |
| `proxy/server.rs` | **重构** | 大 | 单端口路由；统一 `process_request` 管线；Google handler；统计传真实 tool_id/provider_id |
| `proxy/transform.rs` | **扩展** | 大 | 保留 A↔O；新增 4 方向 Google 转换 + 2 个 Google StreamConverter；`map_model_name` 响应回填 |
| `proxy/optimizers.rs` | **重构** | 中 | `apply_optimizers` 按协议分派；新增 `optimize_thinking_openai`/`_google`；`apply_preventive_rectifiers`/`try_reactive_rectify` |
| `proxy/sse.rs` | 不改 | — | 通用 SSE 解析，协议无关 |
| `commands/ai/launch.rs` | **重构** | 中 | 单端口；`pick_outbound`；Google 走代理；`tool_id`/`provider_id` 传入 |
| `commands/ai/provider.rs` | **修改** | 小 | `start_proxy` 适配新 `ProxyConfig` |
| `commands/ai/models.rs` | 不改 | — | `AiProvider` 已有 `google_url`，`model_aliases` 已是 `HashMap` |
| `commands/ai/usage.rs` | 不改 | — | `log_usage_db` 已支持 provider 参数 |

### 4.2 前端 TypeScript

| 文件 | 改动类型 | 说明 |
|------|---------|------|
| `components/ai/types.ts` | **修改** | `AiConfig` 补 `rectifier.protocol_mismatch` |
| `components/ai/ModelConfig.tsx` | **修改** | 模型映射区改为可增删键值对列表 |
| `components/ai/ToolLauncher.tsx` | **修改** | 启动确认区显示"入站 X → 出站 Y（自动转换）｜统计已开启" |
| `components/GlobalSettings.tsx` | **修改** | 新增 `protocol_mismatch` 整流器开关 |

### 4.3 工具配置

| 文件 | 改动 |
|------|------|
| `ai-tools/gemini-cli/config.json` | baseUrl 改为指向代理端口 |
| `ai-tools/deveco/paths.json` | `none` → `openai`（修复不一致） |

---

## 5. Google 协议实现要点（风险最高部分）

### 5.1 请求转换关键点

1. **messages → contents**：Anthropic/OpenAI 的 `messages[]` 转为 Google 的 `contents[]`，role `assistant` → `model`
2. **system 处理**：Anthropic `system` / OpenAI `role:"system"` → Google `systemInstruction`
3. **tool_use → functionCall**：Anthropic `content[].type=="tool_use"` → Google `parts[].functionCall{name,args}`
4. **tool_result → functionResponse**：Anthropic `content[].type=="tool_result"` → Google `role:"user"` + `parts[].functionResponse{name,response}`（注意：Google 把工具结果放在 `user` 角色下）
5. **thinking → thinkingConfig**：`thinking.budget_tokens` → `generationConfig.thinkingConfig.thinkingBudget`
6. **图片**：base64 data → `inlineData{mimeType,data}`
7. **model 名从 URL 提取**：Google 入站时，model 在 URL 路径中（`/v1beta/models/{model}:generateContent`），需从 path param 提取

### 5.2 响应转换关键点

1. **candidates → content/choices**：`candidates[0].content.parts[]` → Anthropic `content[]` / OpenAI `choices[0].message`
2. **finishReason 映射**：`STOP`→`end_turn`/`stop`，`MAX_TOKENS`→`max_tokens`/`length`，`SAFETY`/`RECITATION`→`end_turn`/`stop`
3. **functionCall → tool_use/tool_calls**：同请求侧反向
4. **thought → thinking/reasoning_content**：`parts[].thought==true` 的 text → thinking block
5. **usageMetadata 解析**：`promptTokenCount` → input，`candidatesTokenCount` + `thoughtsTokenCount` → output

### 5.3 SSE 流转换

Google 流式响应格式（`streamGenerateContent?alt=sse`）：
```
data: {"candidates":[{"content":{"role":"model","parts":[{"text":"Hello"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":5}}
```

与 OpenAI/Anthropic SSE 的差异：
- Google 的每个 SSE chunk 是一个完整的 `GenerateContentResponse`（含 candidates + usageMetadata）
- 不像 OpenAI 那样有 `delta` 增量结构——Google 每个 chunk 的 `parts[].text` 就是增量文本
- 最后一个 chunk 包含 `usageMetadata` 和 `finishReason`

**`GoogleToAnthropicStreamConverter`** 实现思路：
- 每个 chunk 的 `candidates[0].content.parts` 遍历，text → `text_delta` 事件
- `functionCall` → `content_block_start` (tool_use) + `input_json_delta`
- 首个 chunk → `message_start` 事件
- `finishReason` 出现 → `content_block_stop` + `message_delta` + `message_stop`
- `usageMetadata` → 累积 token 计数

### 5.4 鉴权头

```rust
fn build_auth_headers(config: &ProxyConfig) -> Vec<(String, String)> {
    match config.outbound_protocol.as_str() {
        "anthropic" => vec![
            ("x-api-key".into(), config.upstream_api_key.clone()),
            ("anthropic-version".into(), "2023-06-01".into()),
        ],
        "openai" => vec![
            ("Authorization".into(), format!("Bearer {}", config.upstream_api_key)),
        ],
        "google" => vec![
            ("x-goog-api-key".into(), config.upstream_api_key.clone()),
        ],
        _ => vec![],
    }
}
```

### 5.5 count_tokens 等价

Gemini 有 `:countTokens` 端点。当 inbound 是 anthropic 且 Claude Code 调用 `/v1/messages/count_tokens` 时：
- 若 outbound 是 google → 转换为 `:countTokens` 调用
- 若 outbound 是 openai → 返回估算值（现有行为）
- 若 outbound 是 anthropic → 透传

---

## 6. 实施阶段

### Phase 0: 骨架重构（2 天）

**目标**：单端口 + 统计归属，不改变现有 A↔O 转换行为。

| 任务 | 文件 | 验收 |
|------|------|------|
| `ProxyConfig` 改字段 | `types.rs` | 编译通过，旧配置自动迁移 |
| `launch.rs` 改单端口 + `pick_outbound` | `launch.rs` | 7 工具中 6 个（除 gemini）正常启动 |
| `server.rs` 按 `inbound_protocol` 注册路由 | `server.rs` | A→O / O→A / 透传 三种模式正常 |
| 统计传 `tool_id`/`provider_id` | `server.rs` | SQLite 中 tool_id 不再是 "proxy" |
| `provider.rs::start_proxy` 适配 | `provider.rs` | 编译通过 |

**风险**：`ProxyConfig` 字段重命名需同步所有引用点（`launch.rs`、`provider.rs`、`server.rs`）。建议用 `#[serde(alias)]` 做向后兼容。

### Phase 1: Google 协议转换（3 天）

**目标**：Gemini CLI 走代理，三协议互转可用。

| 任务 | 文件 | 验收 |
|------|------|------|
| `anthropic_to_google` + 响应 + SSE | `transform.rs` | Claude Code → Google 上游能对话 |
| `openai_to_google` + 响应 + SSE | `transform.rs` | Codex CLI → Google 上游能对话 |
| `google_to_anthropic` + 响应 + SSE | `transform.rs` | Gemini CLI → Anthropic 上游能对话 |
| `google_to_openai` + 响应 + SSE | `transform.rs` | Gemini CLI → OpenAI 上游能对话 |
| `google_handler` + `google_stream_handler` | `server.rs` | Gemini CLI 走代理，用量落库 |
| Google 鉴权头 `x-goog-api-key` | `server.rs` | Google 上游鉴权通过 |
| Google `countTokens` 映射 | `server.rs` | Claude Code 的 count_tokens 正常 |

**风险**：Google SSE 格式与 OpenAI/Anthropic 差异大，需充分测试流式输出。

### Phase 2: 优化器/整流器协议感知（2 天）

**目标**：优化器/整流器在 P_out 形态上按协议生效。

| 任务 | 文件 | 验收 |
|------|------|------|
| `apply_optimizers` 按协议分派 | `optimizers.rs` | Anthropic 上游注入 cache_control；OpenAI/Google 跳过 |
| `optimize_thinking_openai` | `optimizers.rs` | o1/o3 模型设 reasoning_effort |
| `optimize_thinking_google` | `optimizers.rs` | 设 thinkingConfig |
| `apply_preventive_rectifiers` | `optimizers.rs` | thinking signature 预防式剥离 |
| `try_reactive_rectify` | `optimizers.rs` | budget/media/protocol_mismatch 反应式重试 |
| 现有整流器拆分按协议 | `optimizers.rs` | 三协议各自的整流规则正确触发 |

**风险**：优化器从"转换前"移到"转换后"可能改变现有行为。需回归测试 Claude Code + DeepSeek 组合。

### Phase 3: 模型伪装泛化（2 天）

**目标**：任意"声明名 C → 实际模型 B"映射。

| 任务 | 文件 | 验收 |
|------|------|------|
| 响应 model 字段统一回填声明名 C | `server.rs`/`transform.rs` | 工具收到的 model 恒为 C |
| Google 入站从 URL 提取 model | `server.rs` | Gemini CLI 的 model 名从 URL path 解析 |
| 前端模型映射改为键值对列表 | `ModelConfig.tsx` | 可添加任意 key→value 映射 |
| 启动确认信息条 | `ToolLauncher.tsx` | 显示"入站 X → 出站 Y｜伪装：C→B" |
| `GlobalSettings` 新增 protocol_mismatch 开关 | `GlobalSettings.tsx` | 开关可切换 |

**风险**：Claude Code 的模型名校验严格，伪装名必须是合法 Claude 模型名/别名。

### Phase 4: 测试与文档（2 天）

| 任务 | 验收 |
|------|------|
| 9 种转换矩阵单测 | 每种方向至少 1 个 test case |
| 7 工具 × 不同 Provider 端到端 | 全部能正常对话 |
| Google 流式输出测试 | Gemini CLI 流式不中断 |
| 伪装端到端测试 | 工具收到的 model 为 C，上游收到 B |
| 统计准确性测试 | by_model 反映实际模型 B |
| 补 `docs/tool-config/google-api.md` | Gemini REST schema 备忘 |

**总工时**：约 11 天。

---

## 7. 风险与应对

| 风险 | 影响 | 应对 |
|------|------|------|
| **Google SSE 格式差异** | Gemini CLI 流式中断 | 参考 OpenAI StreamConverter 的增量合并思路，逐 chunk 映射 parts |
| **Axum 路由 path 语法** | Google `:generateContent` 路由可能不兼容 | 用 `/v1beta/models/{model}/generateContent`（slash 分隔）或手动解析 path |
| **thinking 跨协议不可无损互转** | budget_tokens ↔ thinkingBudget ↔ reasoning_effort 语义不等价 | 定义降级优先级：有 thinking 字段 > 无；budget 取近似值 |
| **优化器位置变更的回归风险** | 现有 Claude Code + DeepSeek 行为可能变化 | Phase 2 完成后重点回归测试此组合 |
| **配置向后兼容** | 旧 `ProxyConfig` 序列化失败 | 所有新字段 `#[serde(default)]`；`upstream_protocol` 保留 `#[serde(alias)]` |
| **多工具并行端口冲突** | 同时启动两个工具端口碰撞 | 沿用端口自增探测（已有），每个工具分配独立端口 |
| **Claude Code 模型名校验** | 伪装名被拒绝 | 提供 Claude 模型名预设列表；非 Claude 名时不写入 ANTHROPIC_MODEL |

---

## 8. 验收标准

- [ ] 7 个工具在"同协议 / 跨协议（含 Google）"Provider 下均能正常对话
- [ ] Gemini CLI 走代理，用量进入 SQLite，tool_id 为 "gemini-cli"
- [ ] SQLite 中 `by_model` 反映实际模型 B（伪装后）
- [ ] 当 P_in ≠ P_out 时转换自动发生，无用户开关可关闭
- [ ] 优化器/整流器可独立开关，且只对出站协议生效
- [ ] 配置"声明名 C → 实际模型 B"后，工具收到的响应 `model` 恒为 C
- [ ] 转换矩阵 9 种情况各有单元测试覆盖
- [ ] Google 流式输出不中断
- [ ] 现有 Claude Code + DeepSeek 组合行为不退化

---

## 9. 与两份方案的对照

| 方面 | HY3 方案 | GLM5.2 方案 | 本方案（最终） |
|------|---------|------------|--------------|
| IR 中间表示 | ❌ 不引入 | ✅ 引入 UnifiedRequest | ❌ 不引入（采纳 HY3） |
| 优化器位置 | P_out 形态 | IR 形态（转换前） | P_out 形态（采纳 HY3） |
| 整流器模式 | 纯反应式 | 纯预防式 | **混合**（高概率预防 + 不确定反应） |
| 模型伪装 | 泛化 model_aliases | 新增 model_disguise | 泛化 model_aliases（采纳 HY3） |
| 入站路由 | 按 inbound 锁定 | 全协议注册 | 按 inbound 锁定（采纳 HY3） |
| 各工具模型名校验 | 未详述 | ✅ 详细表格 | ✅ 采纳 GLM5.2 表格 |
| raw 字段保底 | 未提及 | ✅ 提出 | 转换函数对未识别字段 log warning |
| 预防式整流 | ❌ 未考虑 | ❌ 未区分 | ✅ 区分高概率/不确定 |
| Google countTokens | 提及但未细化 | 未提及 | ✅ 明确三种出站的处理方式 |
| 统计归属 | 提及 tool_id | 提及 tool_id | ✅ 明确代码改动点（`server.rs:22`） |
| 实施工时 | P0-P4 未估天 | 7 阶段 15 天 | 5 阶段 11 天 |
