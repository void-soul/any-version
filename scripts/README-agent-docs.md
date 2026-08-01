# Agent 文档工具 — 执行速查

独立脚本，不等主程序运行即可使用。需要先安装 requests 库（可任选其一）：
```
pip install requests
uv pip install requests
```

## 命令

| 命令 | 作用 |
|------|------|
| `python scripts/update_agent_docs.py --status` | 查看所有 Agent 文档的状态 |
| `python scripts/update_agent_docs.py --check` | 检查哪些 Agent 有更新 |
| `python scripts/update_agent_docs.py` | 更新全部 Agent 文档 |
| `python scripts/update_agent_docs.py claude-code` | 仅更新 Claude Code |
| `python scripts/update_agent_docs.py --serve` | 启动 Web 浏览服务 (127.0.0.1:8765) |
| `python scripts/update_agent_docs.py --manifest` | 查看清单中的 Agent 列表 |
| `python scripts/update_agent_docs.py --help` | 帮助信息 |

## 文件结构

```
any-version/
├── scripts/
│   ├── update_agent_docs.py       ← 主脚本（更新 + 浏览 + 状态）
│   └── agent_doc_manifest.json    ← Agent 清单（20 个 Agent 的文档源）
├── docs/
│   └── agent-doc/                 ← 文档存放目录（由脚本自动创建）
│       ├── claude-code/
│       ├── codex-cli/
│       ├── hermes-agent/
│       └── ...
└── agent-docs-state.json          ← 运行时状态（自动生成，可删）
```

## manifest.json 字段说明

```json
{
  "id": "unique-id",
  "displayName": "显示名",
  "category": "cli-code|ide-extension|ide|multi-platform",
  "priority": 1,
  "docsSource": {
    "indexUrl": "https://.../llms.txt  索引文件 URL",
    "baseUrl": "https://.../             基础 URL",
    "pagesUrlTemplate": "https://.../{page}.md   页面模板（可选）"
  },
  "localDir": "本地目录名",
  "description": "简述",
  "supportsProtocols": ["openai", "anthropic", "google"]
}
```

## 扩展

编辑 `scripts/agent_doc_manifest.json` 添加新 Agent，然后运行：
```
python scripts/update_agent_docs.py <agent-id>
```

## 原理

1. 从 `indexUrl` 读取 `llms.txt`（每行一个页面路径）
2. 解析页面路径列表
3. 用 `pagesUrlTemplate` 构造实际 URL（若未设置则用 baseUrl 拼接）
4. 逐页 GET 下载，保存到 `docs/agent-doc/<localDir>/` 
5. 状态写入 `agent-docs-state.json`
