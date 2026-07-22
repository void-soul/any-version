---
kind: external_dependency
name: Anthropic API 供应商接入
slug: anthropic-api
category: external_dependency
category_hints:
    - auth_protocol
    - client_constraint
scope:
    - '**'
---

Anthropic Messages 协议作为原生协议之一，由 Claude Code 工具直接调用。供应商通过 anthropic_url + ANTHROPIC_API_KEY 配置，代理在 inbound=anthropic 时直连。Claude Code 还通过环境变量 ANTHROPIC_MODEL / ANTHROPIC_DEFAULT_*_MODEL 注入模型名，ANONYMIC_BASE_URL 覆盖 base URL。