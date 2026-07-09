# Google Gemini (Generative Language API) 速查

> 代理模块 `src-tauri/src/proxy/google.rs` 的协议转换依据。
> 覆盖 Gemini CLI（`apiProtocol: "google"`）经代理与 OpenAI / Anthropic 互转所需的核心字段。

## 1. 端点与鉴权

| 用途 | 方法 & 路径 |
| --- | --- |
| 非流式 | `POST {base}/v1beta/models/{model}:generateContent` |
| 流式 | `POST {base}/v1beta/models/{model}:streamGenerateContent?alt=sse` |
| 仅计数 / 列表 | `GET {base}/v1beta/models` |

- 鉴权：**`x-goog-api-key: <API_KEY>`**（请求头，而非 `Authorization: Bearer`）。
- `{model}` 取模型名，如 `gemini-2.5-pro`、`gemini-2.5-flash`。
- 流式 SSE 事件名：`[GEMINI_SSE_DEFAULT]`（data-only）；每条 data 为 `GenerateContentResponse` JSON。
- 代理对 Google 入站统一在 `server.rs` 的 `google_handler` 中：从 URL path 取 model，按 `uri.path().contains("streamGenerateContent")` 判定是否流式。

## 2. 请求体（generateContent）核心字段

```jsonc
{
  "contents": [                       // 多轮对话，每条交替 role
    { "role": "user",      "parts": [ { "text": "..." } ] },
    { "role": "model",     "parts": [ { "text": "..." } ] }
  ],
  "systemInstruction": { "parts": [ { "text": "系统提示" } ] },
  "tools": [
    { "functionDeclarations": [       // 函数声明列表
      { "name": "get_weather", "description": "…", "parameters": { "type": "object", … } }
    ] }
  ],
  "toolConfig": {                     // 函数调用约束
    "functionCallingConfig": {
      "mode": "AUTO" | "ANY" | "NONE",
      "allowedFunctionNames": ["get_weather"]   // 仅 mode=ANY 时可选
    }
  },
  "thinkingConfig": {                 // 思考/推理控制
    "thinkingBudget": 1024,           // 0 = 关闭思考；>0 开启
    "includeThoughts": true           // 在响应 parts 中返回 thought 块
  },
  "generationConfig": {
    "temperature": 0.7,
    "maxOutputTokens": 8192,
    "stopSequences": ["…"],
    "responseMimeType": "application/json"
  }
}
```

### 与 Anthropic / OpenAI 的字段映射（代理实现）

| Google 字段 | Anthropic 来源 | OpenAI 来源 |
| --- | --- | --- |
| `contents[].parts[].text` | `messages[].content`（text 块） | `messages[].content`（string / text 块） |
| `contents` 顺序 | user/model 交替 | 按 role 映射：user→user，assistant→model |
| `systemInstruction` | `system`（string / 顶层 system 块） | `messages` 中 `role:"system"` |
| `tools[].functionDeclarations[]` | `tools[].{name,description,input_schema}` | `tools[].function.{name,description,parameters}` |
| `toolConfig.functionCallingConfig` | `tool_choice`：`any`→ANY，`tool`→ANY+allowed，`auto`→AUTO | `tool_choice`：`required`→ANY，`none`→NONE，object→ANY+allowed，默认 AUTO |
| `thinkingConfig.thinkingBudget` | `thinking.budget_tokens`（0 或 >0） | `reasoning_effort` / 自定义 thinking 字段 |
| `thinkingConfig.includeThoughts` | 固定 `true`（开启思考时） | 固定 `true` |
| `generationConfig.maxOutputTokens` | `max_tokens` | `max_tokens` |
| `generationConfig.temperature` | `temperature` | `temperature` |

- 思考模式（`thinking.type == "adaptive"`）映射为 `{"thinkingBudget": 0, "includeThoughts": true}`。
- 普通 `thinking`（含 `budget_tokens`）映射为 `{"thinkingBudget": <budget>, "includeThoughts": true}`。

## 3. 响应体（GenerateContentResponse）核心字段

```jsonc
{
  "candidates": [
    {
      "content": {
        "role": "model",
        "parts": [
          { "text": "最终回答" },
          { "thought": true, "text": "思考过程…" },          // includeThoughts=true 时出现
          { "functionCall": { "name": "get_weather", "args": {…} } }  // 工具调用
        ]
      },
      "finishReason": "STOP" | "MAX_TOKENS" | "STOP" | "SAFETY" | "OTHER",
      "index": 0
    }
  ],
  "usageMetadata": {
    "promptTokenCount": 123,
    "candidatesTokenCount": 45,
    "totalTokenCount": 168,
    "thoughtsTokenCount": 12            // 思考 token（可选）
  }
}
```

- 函数调用以 `parts[].functionCall` 表达（非独立 message）。
- 工具结果回填：客户端随后发送一条 `role:"user"` 且 `parts` 含 `functionResponse` 的 content。
- 流式响应：`usageMetadata` 通常只在**最后一个** SSE data 块出现。

### 响应回写 Anthropic / OpenAI 的字段映射（代理实现）

| Google 响应 | Anthropic 目标 | OpenAI 目标 |
| --- | --- | --- |
| `candidates[].content.parts[].text` | `content[].text`（text 块） | `choices[].message.content` |
| `parts[].thought=true` 文本 | `content[]` 中 `type:"thinking"` 块 | `choices[].message.reasoning_content` |
| `parts[].functionCall` | `content[].type:"tool_use"`（id 自动生成） | `choices[].message.tool_calls[]` |
| `usageMetadata.promptTokenCount` | `usage.input_tokens` | `usage.prompt_tokens` |
| `usageMetadata.candidatesTokenCount` | `usage.output_tokens` | `usage.completion_tokens` |
| `usageMetadata.totalTokenCount` | 派生 = input+output | `usage.total_tokens` |
| `usageMetadata.thoughtsTokenCount` | 计入 `output_tokens` | 计入 `completion_tokens` |

## 4. 代理转换方向（共 6 种流式/非流式组合）

- Google 入站：
  - `google → anthropic`（`GoogleToAnthropicStreamConverter` 处理 SSE）
  - `google → openai`（`GoogleToOpenaiStreamConverter` 处理 SSE）
- Google 出站：
  - `anthropic → google`（`AnthropicToOpenai`/GoogleStreamConverter）
  - `openai → google`（GoogleStreamConverter）
- 同协议（inbound == outbound）：透传，不转换。

## 5. 注意事项

- Google **不支持** Anthropic 的顶层 `metadata`、`top_k`/`top_p` 语义不完全一致；转换时残留的 Anthropic 专有字段由 `rectifier_protocol_mismatch` 在重试前剥离。
- `thinkingBudget: 0` 表示**关闭**思考；非零表示开启并限制预算。
- 函数名在 `allowedFunctionNames` 中需与 `functionDeclarations[].name` 完全一致。
- 流式 SSE 仅用 `data:` 行；结束以流关闭或最后一个含 `usageMetadata` 的块判定。
