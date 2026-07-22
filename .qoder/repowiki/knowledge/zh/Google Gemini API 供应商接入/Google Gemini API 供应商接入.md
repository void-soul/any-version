---
kind: external_dependency
name: Google Gemini API 供应商接入
slug: google-gemini-api
category: external_dependency
category_hints:
    - auth_protocol
    - client_constraint
scope:
    - '**'
---

Google Gemini 协议作为原生协议之一，由 Gemini CLI 工具直接调用。供应商通过 google_url + GEMINI_API_KEY 配置，代理在 inbound=google 时直连。Gemini CLI 通过 security.auth.selectedType=gemini-api-key 和 GOOGLE_GEMINI_BASE_URL 控制认证与端点。