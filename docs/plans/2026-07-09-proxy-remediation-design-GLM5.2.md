让我再查看几个关键文件来完善方案：

现在我已经对整个系统有了全面深入的理解。让我完成最后的分析并输出完整整改方案。

---

# AI 代理系统整改方案

## 一、现状分析

### 1.1 现有架构总览

```
工具启动 → launch.rs
  ├─ Step 1: 启动代理（P1=Anthropic端口 / P2=OpenAI端口）
  ├─ Step 2: 写入工具配置文件（baseUrl → 127.0.0.1:proxy_port）
  ├─ Step 3: 构建 CLI 参数
  ├─ Step 4: spawn 子进程（注入 env.* 环境变量）
  └─ Step 5: 保存会话
```

代理核心由 5 个文件组成：

| 文件 | 职责 | 现状 |
|------|------|------|
| `types.rs` | `ProxyConfig` 配置结构 | 只有 `openai`/`anthropic` 两种 `upstream_protocol`，无 `google` |
| `server.rs` | Axum HTTP 服务 | 路由硬编码 `/v1/messages` 和 `/v1/chat/completions`，双端口 P1/P2 |
| `transform.rs` | 协议转换 | 仅实现 Anthropic↔OpenAI 双向转换，无 Google 协议 |
| `optimizers.rs` | 6 个整流器/优化器 | 全部操作 Anthropic 请求体格式，OpenAI 透传模式下大部分不生效 |
| `sse.rs` | SSE 解析 | 功能完整 |

### 1.2 七个工具的协议需求

| 工具 | apiProtocol | 配置文件写入方式 | 走代理？ |
|------|-------------|-----------------|---------|
| Claude Code | `anthropic` | env.ANTHROPIC_BASE_URL → P1 | ✅ |
| Codex CLI | `openai` | config.toml base_url → P2 | ✅ |
| Gemini CLI | `google` | env.GOOGLE_GEMINI_BASE_URL → 真实上游 | ❌ **不走代理** |
| OpenCode | `openai` | opencode.json baseURL → P2 | ✅ |
| MiMo Code | `openai` | mimocode.json baseURL → P2 | ✅ |
| Qwen Code | `openai` | env.OPENAI_BASE_URL → P2 | ✅ |
| Deveco Code | `openai` | opencode.json baseURL → P2 | ✅ |

### 1.3 供应商协议矩阵

从 `providers.json` 的 29 个预设来看：

| 类型 | 仅有 OpenAI URL | 仅有 Anthropic URL | 双协议 URL | 仅有 Google URL |
|------|-----------------|-------------------|-----------|----------------|
| 数量 | 11 个 | 6 个 | 11 个 | 1 个 |
| 示例 | OpenAI, xAI, Mistral, Groq... | NekoCode, Amux, FennoAI... | DeepSeek, 火山, Qwen, Kimi... | Google |

### 1.4 核心问题清单

| # | 问题 | 严重程度 | 说明 |
|---|------|---------|------|
| P1 | **Google 协议完全不支持** | 🔴 高 | Gemini CLI 的 `apiProtocol="google"` 直接绕过代理，无法统计用量、无法转协议 |
| P2 | **模型伪装与别名混淆** | 🔴 高 | 现有 `model_aliases` 是 "角色→模型" 映射（sonnet→deepseek-v4-pro），不是真正的"伪装"。用户想要"选 A 供应商的 B 模型，让工具以为调的是 C 模型"做不到 |
| P3 | **协议转换无自动决策** | 🟡 中 | launch.rs 不检测"工具协议 vs 供应商协议"是否匹配，盲目启动两个代理端口 |
| P4 | **整流器/优化器仅对 Anthropic 格式生效** | 🟡 中 | OpenAI 透传模式下，Cache Injector、Thinking Optimizer 等全部失效 |
| P5 | **统计不强制** | 🟡 中 | Google 协议工具完全绕过代理；OpenAI 透传模式下虽有统计但非强制 |
| P6 | **双端口架构冗余** | 🟢 低 | P1/P2 双端口增加了配置复杂度，且 `both` 协议需要同时启动两个代理实例 |

---

## 二、目标架构设计

### 2.1 核心设计理念：**统一代理管道（Unified Proxy Pipeline）**

将代理从"按协议分端口"改为"**单端口多路由 + 管道式处理**"：

```
工具请求 → 本地代理（单端口）
             │
             ├─ 1. [强制] 用量统计拦截器
             ├─ 2. [自动] 协议识别 + 转换器（工具协议 → 供应商协议）
             ├─ 3. [可选] 差异抹平整流器
             ├─ 4. [可选] 请求优化器
             ├─ 5. [可选] 模型伪装器
             │
             └─ 转发到上游供应商
```

### 2.2 五大能力分层

| 层级 | 能力 | 开关 | 决策逻辑 |
|------|------|------|---------|
| L1 | **用量统计** | 强制开启 | 不可关闭，所有请求必须经过代理 |
| L2 | **协议转换** | 自动决策 | 根据工具 `apiProtocol` + 供应商可用 URL 自动判断 |
| L3 | **差异抹平** | 用户可选 | 默认开启，可在设置中关闭 |
| L4 | **请求优化** | 用户可选 | 默认开启，可在设置中关闭 |
| L5 | **模型伪装** | 用户可选 | 默认关闭，需用户显式配置 |

---

## 三、详细整改方案

### 3.1 L1 — 用量统计（强制开启）

#### 现状问题
- Google 协议工具完全绕过代理，用量丢失
- 统计逻辑散落在 `server.rs` 的 `record_proxy_usage()` 中，与协议处理耦合

#### 整改方案

**1) 所有协议工具必须走代理**

修改 `launch.rs`，将 Google 协议也纳入代理：

```rust
// 现有逻辑：google/none 协议不走代理
"google" => &p.google_url,  // ❌ 直连

// 改为：google 协议也走代理（新增 Google 代理路由）
"google" => format!("http://127.0.0.1:{}", config.proxy_port),  // ✅ 走代理
```

**2) 统计中间件独立化**

将用量统计从各 handler 中抽离为独立的 Axum 中间件层：

```rust
// proxy/middleware.rs (新增)
/// 用量统计中间件：拦截所有响应，解析 usage 并记录
pub async fn usage_tracking_middleware(
    State(state): State<ProxyState>,
    request: Request,
    next: Next,
) -> Response {
    // 请求前计数
    state.stats.total_requests += 1;
    
    // 转发请求
    let response = next.run(request).await;
    
    // 响应后解析 usage（从响应体中提取 token 数）
    // 支持 OpenAI usage / Anthropic usage / Google usage 三种格式
    ...
}
```

**3) 统计维度增强**

现有 `ai_usage` 表已有 `tool_id`、`model`、`provider` 字段，但代理记录的 `tool_id` 固定为 `"proxy"`。需要修改为在 `ProxyConfig` 中携带真实的 `tool_id` 和 `provider_id`，确保统计数据可按工具/供应商分组。

### 3.2 L2 — 协议转换（自动决策）

#### 现状问题
- 仅支持 Anthropic↔OpenAI 双向转换
- 无 Google 协议支持
- 转换决策逻辑散落在 `server.rs` 的 handler 中

#### 整改方案

**1) 协议转换矩阵**

需支持 \(3 \times 3 = 9\) 种转换方向（含同协议透传）：

| 工具协议 \ 供应商协议 | OpenAI | Anthropic | Google |
|---------------------|--------|-----------|--------|
| **OpenAI** | 透传 | OpenAI→Anthropic | OpenAI→Google |
| **Anthropic** | Anthropic→OpenAI | 透传 | Anthropic→Google |
| **Google** | Google→OpenAI | Google→Anthropic | 透传 |

**2) 自动决策逻辑**

在 `launch.rs` 中新增协议匹配检测：

```rust
/// 根据工具协议和供应商可用 URL，自动决定代理的协议转换策略
fn determine_proxy_strategy(tool_protocol: &str, provider: &AiProvider) -> ProxyStrategy {
    let provider_protocols = detect_provider_protocols(provider);
    // provider_protocols: Vec<"openai"|"anthropic"|"google"> 基于非空 URL
    
    // 工具协议 ∈ 供应商协议 → 透传，无需转换
    if provider_protocols.contains(&tool_protocol) {
        return ProxyStrategy::Passthrough(tool_protocol);
    }
    
    // 需要转换：选择供应商支持的第一个协议作为目标
    let target = provider_protocols.first().unwrap();
    ProxyStrategy::Translate {
        from: tool_protocol.to_string(),
        to: target.clone(),
    }
}

enum ProxyStrategy {
    Passthrough(String),           // 同协议透传
    Translate { from: String, to: String }, // 协议转换
}
```

**3) Google 协议转换实现**

新增 `proxy/google.rs` 模块：

```rust
// Google Gemini API 格式 ↔ OpenAI/Anthropic 转换

/// OpenAI → Google Gemini (generateContent)
pub fn openai_to_google(body: &Value, model: &str) -> Value { ... }

/// Google Gemini → OpenAI 响应
pub fn google_to_openai_response(google_resp: &Value, model: &str) -> Value { ... }

/// Google SSE → OpenAI SSE
pub struct GoogleToOpenaiStreamConverter { ... }
```

Google Gemini API 的关键差异点（基于 `gemini-cli/configuration.md` 文档）：
- 端点：`/v1beta/models/{model}:generateContent`（非流式）/ `:streamGenerateContent`（流式）
- 认证：`x-goog-api-key` 头（非 Bearer Token）
- 请求体：`contents` 数组（非 `messages`），`parts` 结构（非 `content`）
- 响应体：`candidates[].content.parts[]` 结构
- 工具调用：`functionDeclarations` + `functionCall`（非 `tools` + `tool_use`）

**4) 路由层重构**

将 `server.rs` 的硬编码路由改为动态路由注册：

```rust
fn build_router(config: &ProxyConfig) -> Router {
    let mut app = Router::new().route("/health", get(health_handler));
    
    match config.strategy {
        ProxyStrategy::Passthrough("openai") | ProxyStrategy::Translate { to: "openai", .. } => {
            app = app.route("/v1/chat/completions", post(chat_completions_handler));
        }
        ProxyStrategy::Passthrough("anthropic") | ProxyStrategy::Translate { to: "anthropic", .. } => {
            app = app.route("/v1/messages", post(messages_handler))
                      .route("/v1/messages/count_tokens", post(count_tokens_handler));
        }
        ProxyStrategy::Passthrough("google") | ProxyStrategy::Translate { to: "google", .. } => {
            app = app.route("/v1beta/models/:model:generateContent", post(google_handler))
                      .route("/v1beta/models/:model:streamGenerateContent", post(google_stream_handler));
        }
    }
    app
}
```

**5) 统一入口路由**

关键改进：**无论工具用什么协议，代理都同时注册工具协议的入口路由和供应商协议的出口转换**。这样工具只需连接一个端口，代理自动处理转换：

```rust
// 工具发送 Anthropic 请求 → 代理接收 /v1/messages
// 代理检测到供应商只有 OpenAI URL → 转换为 /v1/chat/completions
// 响应再转回 Anthropic 格式给工具
```

### 3.3 L3 — 差异抹平整流器（可选）

#### 现状问题
- 整流器仅在 `messages_handler`（Anthropic 协议）中生效
- OpenAI 透传模式下，整流器完全失效
- 整流器操作的是 Anthropic 格式请求体，无法用于 OpenAI/Google 协议

#### 整改方案

**1) 整流器协议无关化**

将整流器从"操作 Anthropic 格式"改为"操作统一的中间表示（IR）"：

```rust
// proxy/ir.rs (新增)
/// 统一中间表示：无论原始协议是什么，整流器/优化器都在 IR 上操作
pub struct UnifiedRequest {
    pub model: String,
    pub system: Vec<TextBlock>,
    pub messages: Vec<UnifiedMessage>,
    pub tools: Vec<UnifiedTool>,
    pub thinking: Option<ThinkingConfig>,
    pub max_tokens: u64,
    pub temperature: Option<f64>,
    pub stream: bool,
}

impl UnifiedRequest {
    pub fn from_anthropic(body: &Value) -> Self { ... }
    pub fn from_openai(body: &Value) -> Self { ... }
    pub fn from_google(body: &Value) -> Self { ... }
    
    pub fn to_anthropic(&self) -> Value { ... }
    pub fn to_openai(&self) -> Value { ... }
    pub fn to_google(&self, model: &str) -> Value { ... }
}
```

**2) 整流器管道化**

```rust
// proxy/rectifiers/mod.rs (重构)
pub trait Rectifier: Send + Sync {
    fn name(&self) -> &str;
    /// 在 IR 上执行整流操作
    fn apply(&self, req: &mut UnifiedRequest);
    /// 检查上游错误是否匹配此整流器
    fn matches_error(&self, status: u16, body: &str) -> bool;
}

// 现有 3 个整流器改为实现 trait
pub struct ThinkingSignatureRectifier;
pub struct ThinkingBudgetRectifier;
pub struct MediaFallbackRectifier;
```

**3) 整流器执行时机**

```
请求进入 → 解析为 IR → [整流器管道] → 转换为目标协议 → 发送上游
                                                      ↓
                                              上游报错？→ 匹配整流器 → 修正 IR → 重试
```

### 3.4 L4 — 请求优化器（可选）

#### 现状问题
- 与整流器相同，仅对 Anthropic 格式生效
- Cache Injector 注入的 `cache_control` 是 Anthropic 专有字段，OpenAI/Google 无此概念

#### 整改方案

**1) 优化器协议感知**

```rust
pub trait Optimizer: Send + Sync {
    fn name(&self) -> &str;
    /// 在 IR 上执行优化
    fn apply(&self, req: &mut UnifiedRequest, target_protocol: &str);
}
```

**2) 按目标协议调整优化策略**

| 优化器 | Anthropic 目标 | OpenAI 目标 | Google 目标 |
|--------|---------------|-------------|-------------|
| Cache Injector | 注入 `cache_control` | 转换为 OpenAI 的 `prompt_cache_key`（如支持）或跳过 | 跳过（Google 无 cache 机制） |
| Thinking Optimizer | 设置 `thinking.type` + `budget_tokens` | 设置 `reasoning_effort`（如 o1/o3 系列） | 设置 `thinkingConfig.thinkingBudget` |
| DeepSeek Normalize | 剥离 signature、注入占位 thinking | 跳过（DeepSeek OpenAI 端点不需要） | 跳过 |

### 3.5 L5 — 模型伪装（Model Disguise）

#### 现状问题
这是最复杂的功能。现有 `model_aliases` 是 "角色→模型" 映射（如 `sonnet → deepseek-v4-pro`），本质是**别名解析**而非**伪装**。

用户想要的是：**选 A 供应商的 B 模型，让工具认为自己调的是 C 模型，但实际代理转发给了 B**。

#### 需求分析

结合 `tool-config` 文档分析各工具的模型名处理方式：

| 工具 | 模型名来源 | 工具看到的模型名 | 代理可拦截？ |
|------|-----------|----------------|------------|
| Claude Code | env.ANTHROPIC_MODEL | 别名（sonnet/opus）或完整名 | ✅ 请求中的 `model` 字段 |
| Codex CLI | config.toml `model` | 完整模型名 | ✅ 请求中的 `model` 字段 |
| Gemini CLI | settings.json `model.name` | 完整模型名 | ✅ 请求 URL 中的 model 路径 |
| OpenCode | opencode.json `model` + `provider.models` | `provider_id/model_id` | ✅ 请求中的 `model` 字段 |
| MiMo Code | mimocode.json `model` | 完整模型名 | ✅ 请求中的 `model` 字段 |
| Qwen Code | settings.json `model.name` | 完整模型名 | ✅ 请求中的 `model` 字段 |
| Deveco Code | opencode.json `model` | 完整模型名 | ✅ 请求中的 `model` 字段 |

**关键发现**：所有工具的模型名最终都会出现在发往代理的 HTTP 请求中（`model` 字段或 URL 路径），因此代理可以统一拦截和替换。

#### 伪装方案设计

**1) 伪装配置模型**

在 `AiProvider` 中新增 `model_disguise` 字段：

```rust
// models.rs
pub struct AiProvider {
    // ... 现有字段 ...
    
    /// 模型伪装映射：工具看到的模型名 → 实际转发的模型名
    /// 例：{"claude-sonnet-4-5-20250514": "deepseek-v4-pro"}
    /// 工具以为调的是 claude-sonnet-4-5，实际代理转发给 deepseek-v4-pro
    #[serde(default)]
    pub model_disguise: HashMap<String, String>,
}
```

**2) 伪装与别名的区别**

| 特性 | 模型别名 (alias) | 模型伪装 (disguise) |
|------|-----------------|-------------------|
| 方向 | 角色 → 模型 (sonnet → deepseek-v4-pro) | 模型 → 模型 (claude-sonnet-4-5 → deepseek-v4-pro) |
| 使用场景 | 工具发送角色别名，代理解析为真实模型 | 工具发送"伪装名"，代理替换为真实模型 |
| 配置文件写入 | 写入 ANTHROPIC_DEFAULT_SONNET_MODEL 等 | 工具配置写伪装名，代理负责替换 |
| 响应中的模型名 | 返回真实模型名 | **返回伪装名**（让工具认为确实是伪装的模型） |

**3) 代理中的伪装执行**

```rust
// 在代理管道中，L5 层执行伪装
fn apply_model_disguise(req: &mut UnifiedRequest, disguise_map: &HashMap<String, String>) {
    if let Some(real_model) = disguise_map.get(&req.model) {
        // 记录原始模型名（用于响应中伪装回去）
        req.disguised_from = Some(req.model.clone());
        // 替换为真实模型名
        req.model = real_model.clone();
    }
}

// 响应处理时，将模型名伪装回去
fn disguise_response_model(resp: &mut Value, original_model: &str) {
    if let Some(model) = resp.get_mut("model").and_then(|v| v.as_str()) {
        // 将真实模型名替换为伪装名
        *model = original_model.to_string();
    }
}
```

**4) 配置文件写入策略**

伪装模式下，工具配置文件中写入**伪装名**（而非真实模型名）：

```rust
// launch.rs 中
if !provider.model_disguise.is_empty() {
    // 找到真实模型对应的伪装名
    let disguise_name = provider.model_disguise.iter()
        .find(|(_, real)| real == &selected_model)
        .map(|(disguise, _)| disguise.clone())
        .unwrap_or(&selected_model);
    
    // 配置文件写伪装名
    write_tool_config(tool, disguise_name, ...);
}
```

**5) 各工具的特殊处理**

- **Claude Code**：伪装名必须是 Claude 模型名或别名（如 `claude-sonnet-4-5-20250514` 或 `sonnet`），否则 Claude Code 会拒绝。代理收到后替换为真实模型。
- **Codex CLI**：伪装名可以是任意字符串，Codex 会原样发送。
- **Gemini CLI**：伪装名必须出现在 URL 路径中（`/v1beta/models/{disguise_name}:generateContent`），代理需要从 URL 中提取并替换。
- **OpenCode/Deveco/MiMo**：伪装名需要同时在 `provider.models` 中注册，否则工具不认识。

---

### 3.6 架构重构：单端口统一代理

#### 现状问题
P1（Anthropic 端口）和 P2（OpenAI 端口）双端口架构导致：
- `both` 协议工具需要同时启动两个代理实例
- 配置文件写入需要区分端口
- Google 协议无端口可用

#### 整改方案

**单端口 + 多路由**：

```rust
// 一个代理实例同时注册所有协议的路由
let app = Router::new()
    .route("/health", get(health_handler))
    // Anthropic 协议入口
    .route("/v1/messages", post(messages_handler))
    .route("/v1/messages/count_tokens", post(count_tokens_handler))
    // OpenAI 协议入口
    .route("/v1/chat/completions", post(chat_completions_handler))
    .route("/v1/models", get(list_models_handler))  // 新增：支持工具的模型列表请求
    // Google 协议入口
    .route("/v1beta/models/:model:generateContent", post(google_handler))
    .route("/v1beta/models/:model:streamGenerateContent", post(google_stream_handler))
    .with_state(state);
```

工具配置文件统一指向同一个端口：

```rust
// launch.rs
let effective_base_url = format!("http://127.0.0.1:{}", config.proxy_port);
// 无论 anthropic/openai/google/both，都指向同一个端口
```

**好处**：
- `both` 协议只需一个代理实例
- Google 协议也有端口可用
- 配置文件写入逻辑简化

---

### 3.7 数据流全景

```
┌─────────────────────────────────────────────────────────────────┐
│  工具（Claude Code / Codex / Gemini CLI / OpenCode / ...）      │
│  配置文件中 baseUrl = http://127.0.0.1:{proxy_port}             │
└──────────────────────────┬──────────────────────────────────────┘
                           │ HTTP 请求（工具协议格式）
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│  本地代理服务器（单端口，Axum）                                   │
│                                                                 │
│  ┌─ L1: 用量统计 ─────────────────────────────────────────────┐ │
│  │  拦截请求/响应，记录 tool_id, model, provider, tokens      │ │
│  └───────────────────────────────────────────────────────────┘ │
│                           ▼                                     │
│  ┌─ 入口路由：识别工具协议 ───────────────────────────────────┐ │
│  │  /v1/messages → Anthropic                                  │ │
│  │  /v1/chat/completions → OpenAI                             │ │
│  │  /v1beta/models/{model}:generateContent → Google           │ │
│  └───────────────────────────────────────────────────────────┘ │
│                           ▼                                     │
│  ┌─ 解析为统一中间表示（IR）───────────────────────────────────┐ │
│  │  UnifiedRequest { model, messages, tools, thinking, ... }  │ │
│  └───────────────────────────────────────────────────────────┘ │
│                           ▼                                     │
│  ┌─ L5: 模型伪装（可选）──────────────────────────────────────┐ │
│  │  伪装名 → 真实模型名替换                                    │ │
│  └───────────────────────────────────────────────────────────┘ │
│                           ▼                                     │
│  ┌─ L4: 请求优化（可选）──────────────────────────────────────┐ │
│  │  Cache Injector / Thinking Optimizer / DeepSeek Normalize  │ │
│  │  按目标协议调整策略                                         │ │
│  └───────────────────────────────────────────────────────────┘ │
│                           ▼                                     │
│  ┌─ L3: 差异抹平（可选）──────────────────────────────────────┐ │
│  │  Thinking Signature / Budget / Media Fallback              │ │
│  │  在 IR 上操作，协议无关                                      │ │
│  └───────────────────────────────────────────────────────────┘ │
│                           ▼                                     │
│  ┌─ L2: 协议转换（自动）──────────────────────────────────────┐ │
│  │  IR → 目标协议格式（OpenAI/Anthropic/Google）              │ │
│  │  自动决策：工具协议 vs 供应商协议是否匹配                    │ │
│  └───────────────────────────────────────────────────────────┘ │
│                           ▼                                     │
│  ┌─ 转发到上游供应商 ────────────────────────────────────────┐ │
│  │  选择正确的 URL + 认证头                                    │ │
│  │  OpenAI: Authorization: Bearer {key}                       │ │
│  │  Anthropic: x-api-key: {key} + anthropic-version           │ │
│  │  Google: x-goog-api-key: {key}                             │ │
│  └───────────────────────────────────────────────────────────┘ │
│                           ▼                                     │
│  ┌─ 响应处理 ────────────────────────────────────────────────┐ │
│  │  上游响应 → 解析为 IR → 转换为工具协议格式                   │ │
│  │  L5: 模型名伪装回去（真实名 → 伪装名）                      │ │
│  │  L1: 记录 token 用量                                        │ │
│  └───────────────────────────────────────────────────────────┘ │
└──────────────────────────┬──────────────────────────────────────┘
                           │ HTTP 响应（工具协议格式）
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│  工具收到响应（完全不知道经过了代理转换）                          │
└─────────────────────────────────────────────────────────────────┘
```

---

## 四、文件改动清单

### 4.1 后端 Rust 改动

| 文件 | 改动类型 | 说明 |
|------|---------|------|
| `proxy/types.rs` | **重构** | 新增 `ProxyStrategy`、`tool_id`/`provider_id` 字段；`upstream_protocol` 改为 `target_protocol`；新增 `model_disguise` 字段 |
| `proxy/server.rs` | **重构** | 单端口多路由；handler 统一走 IR 管道；新增 Google 路由；统计中间件化 |
| `proxy/transform.rs` | **重构** | 拆分为 `from_*`/`to_*` 独立函数；现有转换逻辑改为 IR ↔ 协议格式 |
| `proxy/ir.rs` | **新增** | 统一中间表示 `UnifiedRequest`/`UnifiedResponse`；`from_anthropic`/`from_openai`/`from_google` + `to_*` |
| `proxy/google.rs` | **新增** | Google Gemini API 协议转换（请求/响应/SSE） |
| `proxy/rectifiers/mod.rs` | **重构** | 整流器 trait 化；在 IR 上操作；协议无关 |
| `proxy/optimizers.rs` | **重构** | 优化器 trait 化；按目标协议调整策略 |
| `proxy/middleware.rs` | **新增** | 用量统计中间件 |
| `commands/ai/launch.rs` | **修改** | 协议策略自动决策；单端口；Google 协议走代理；伪装名写入配置 |
| `commands/ai/models.rs` | **修改** | `AiProvider` 新增 `model_disguise` 字段 |
| `commands/ai/provider.rs` | **修改** | `start_proxy` 适配新 `ProxyConfig` |

### 4.2 前端 TypeScript 改动

| 文件 | 改动类型 | 说明 |
|------|---------|------|
| `components/ai/types.ts` | **修改** | `AiProvider` 新增 `model_disguise`；`AiConfig` 调整 rectifier/optimizer 结构 |
| `components/ai/ModelConfig.tsx` | **修改** | 新增模型伪装配置 UI（伪装名 ↔ 真实模型名映射表） |
| `components/ai/ToolLauncher.tsx` | **修改** | 启动时显示代理策略（透传/转换/伪装）；Google 协议也走代理 |
| `components/GlobalSettings.tsx` | **修改** | 整流器/优化器开关按协议分组展示 |

### 4.3 工具配置文件改动

| 文件 | 改动 |
|------|------|
| `ai-tools/gemini-cli/config.json` | baseUrl 改为指向代理端口（而非真实上游） |
| `ai-tools/claude-code/config.json` | 无需改动（已指向代理） |
| 其他工具 config.json | 无需改动（已指向代理） |

---

## 五、实施计划与风险

### 5.1 分阶段实施

| 阶段 | 内容 | 预估工作量 | 风险 |
|------|------|-----------|------|
| **Phase 1** | IR 统一中间表示 + 现有转换重构 | 3 天 | 中：需保证现有 Anthropic↔OpenAI 转换行为不变 |
| **Phase 2** | Google 协议转换实现 | 2 天 | 中：Google API 格式差异较大 |
| **Phase 3** | 单端口多路由 + 协议自动决策 | 2 天 | 低：主要是路由重构 |
| **Phase 4** | 整流器/优化器 IR 化 | 2 天 | 低：逻辑不变，改为在 IR 上操作 |
| **Phase 5** | 模型伪装功能 | 3 天 | 高：需处理各工具的特殊模型名校验 |
| **Phase 6** | 统计中间件化 + Google 统计 | 1 天 | 低 |
| **Phase 7** | 前端 UI 适配 | 2 天 | 低 |

### 5.2 关键风险与应对

| 风险 | 影响 | 应对 |
|------|------|------|
| **IR 抽象不完整** | 转换丢字段，工具行为异常 | 为 IR 保留 `raw` 字段，存储未识别的字段原样透传 |
| **Google 协议流式差异** | Gemini CLI 流式输出中断 | 参考 `gemini-cli/configuration.md` 文档，实现 `streamGenerateContent` 的 SSE 解析 |
| **模型伪装被工具校验** | Claude Code 拒绝非 Claude 模型名 | 伪装名必须是合法 Claude 模型名/别名；提供预设伪装名列表 |
| **单端口路由冲突** | 不同协议路由路径重叠 | 三个协议的路径天然不重叠（`/v1/messages` vs `/v1/chat/completions` vs `/v1beta/models/...`） |
| **向后兼容** | 现有用户配置丢失 | `ProxyConfig` 新增字段全部 `#[serde(default)]`；旧配置自动迁移 |

### 5.3 兼容性保障

1. **配置迁移**：`AiProvider` 的 `model_aliases` 保留，`model_disguise` 为新增字段，默认空。
2. **代理端口**：保持 `proxy_port` 不变，移除 P2 端口（`proxy_port + 1` 不再使用）。
3. **工具配置文件**：所有工具的 `config.json` 中 `baseUrl` 统一指向 `proxy_port`，无需改动（Gemini CLI 除外）。

---

## 六、与现有代码的对照

### 6.1 现有整流器/优化器保留情况

| 现有功能 | 整改后 | 说明 |
|---------|--------|------|
| `inject_cache_breakpoints` | ✅ 保留 | 改为在 IR 上操作，仅 Anthropic 目标时注入 `cache_control` |
| `is_thinking_signature_error` + `strip_thinking_blocks` | ✅ 保留 | 改为在 IR 上操作 |
| `is_thinking_budget_error` + `fix_thinking_budget` | ✅ 保留 | 改为在 IR 上操作 |
| `normalize_deepseek_thinking` | ✅ 保留 | 改为在 IR 上操作 |
| `is_unsupported_image_error` + `replace_image_blocks` | ✅ 保留 | 改为在 IR 上操作 |
| `optimize_thinking` | ✅ 保留 | 按目标协议调整：Anthropic 用 `thinking.type`，OpenAI 用 `reasoning_effort`，Google 用 `thinkingConfig` |
| `map_model_name` + `ModelAliases` | ✅ 保留 | 作为 L5 模型伪装的底层实现 |
| `anthropic_to_openai` / `openai_to_anthropic` | ✅ 保留 | 重构为 IR 的 `from_*`/`to_*` 方法 |
| `StreamConverter` / `AnthropicToOpenaiStreamConverter` | ✅ 保留 | 新增 `GoogleToOpenaiStreamConverter` 等 |

### 6.2 现有 `model_aliases` 与新增 `model_disguise` 的协作

```
场景：用户选 DeepSeek 供应商的 deepseek-v4-pro 模型，启动 Claude Code

现有行为（model_aliases）：
  配置文件写入：ANTHROPIC_MODEL = sonnet（角色别名）
  配置文件写入：ANTHROPIC_DEFAULT_SONNET_MODEL = deepseek-v4-pro
  Claude Code 发送：model = sonnet
  代理 map_model_name("sonnet") → deepseek-v4-pro ✅

新增行为（model_disguise）：
  配置文件写入：ANTHROPIC_MODEL = claude-sonnet-4-5-20250514（伪装名）
  代理收到：model = claude-sonnet-4-5-20250514
  代理 model_disguise["claude-sonnet-4-5-20250514"] → deepseek-v4-pro ✅
  代理转发给 DeepSeek：model = deepseek-v4-pro
  DeepSeek 响应：model = deepseek-v4-pro
  代理伪装回去：model = claude-sonnet-4-5-20250514
  Claude Code 收到：model = claude-sonnet-4-5-20250514 ✅（完全无感）
```

两者可共存：`model_aliases` 用于角色别名场景，`model_disguise` 用于精确模型名伪装场景。

---

## 七、总结

本方案的核心改进点：

1. **统一管道**：所有请求经过同一条管道（L1→L5），各层职责清晰
2. **协议完备**：补齐 Google 协议支持，实现三种协议任意方向转换
3. **单端口**：消除 P1/P2 双端口复杂度
4. **IR 中间表示**：整流器/优化器协议无关化，一次实现三协议通用
5. **模型伪装**：作为独立 L5 层，与别名解耦，支持响应中模型名伪装回去
6. **统计强制**：所有协议工具（含 Google）必须走代理，统计无遗漏

方案充分考虑了与 `tool-config` 官方文档的兼容性，特别是各工具的模型名校验规则（Claude Code 的别名系统、OpenCode 的 `provider_id/model_id` 格式、Gemini CLI 的 URL 路径模型名等），确保伪装功能在实际使用中不会被工具拒绝。

---

以上方案供 HY3 评审。如需进一步细化某个模块的实现伪代码或补充特定工具的兼容性分析，我可以随时深入。

方案已完成。以上是完整的 AI 代理系统整改方案，涵盖了五大能力的分层设计、Google 协议补齐、单端口统一管道架构、IR 中间表示、模型伪装独立层、以及分阶段实施计划和风险分析。请交由 HY3 评审，我随时准备根据评审反馈进一步调整或直接进入实施阶段。