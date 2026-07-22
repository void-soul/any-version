---
kind: external_dependency
name: OpenAI 协议供应商接入
slug: openai-api
category: external_dependency
category_hints:
    - auth_protocol
    - client_constraint
scope:
    - '**'
---

项目将 OpenAI Chat Completions 作为三大原生协议之一（另两个为 Anthropic Messages、Google Gemini）。供应商通过 AiProvider.openai_url + api_key 配置端点，代理在 inbound=openai 时直连上游不做转换。多个 CLI 工具（Codex CLI、Qwen Code、OpenCode、MiMo Code、Deveco Code）均以 openai 协议对接。