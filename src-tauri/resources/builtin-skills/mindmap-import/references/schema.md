# 思维导图导入格式 v1（mindmap-import）

对应实现：`src-tauri/src/commands/mindmap/commands.rs::mm_import_nodes`
（内部复用 AI 导入的 `json_to_mindmap_nodes` + 本技能新增的 `normalize_imported_nodes`）。
本文是**对外契约**：其它 Agent / 脚本按它产出文件，用户导入即成图。

## 1. 顶层结构

```jsonc
{
  "name": "导图标题",            // 可选；缺省依次取 JSON 的 title → 文件名 → 「导入的导图」
  "description": "一句话说明",    // 可选，落到文档描述
  "layoutDir": "lr",            // 可选：lr(默认, 左→右) / rl / tb / bt；非法值忽略
  "nodes": [ /* 必填，至少一个 */ ]
}
```

- 顶层必须是**对象**，且 `nodes` 必须是**非空数组**，否则导入报错（不会建出空文档）
- 允许但不推荐：外层再套一层 `{"mindmap": {...}}` —— 导入端不认，会被当成缺 nodes

## 2. 节点对象

```jsonc
{
  "id": "a1",                 // 可选，文件内唯一；缺省按序号自动补
  "name": "订单服务",          // 节点标题（缺省「未命名」）
  "parent_id": "root",        // 可选；parentId 亦可。缺省/悬空 = 成为一条新主线
  "detail": "## 职责\n...",    // 可选，Markdown；旧字段 description 亦可
  "kind": "module",           // 可选，见白名单；缺省 other（根自动 root）
  "color": "#22d3ee",         // 可选，#RRGGBB；缺省按序号取调色板
  "sources": ["src/order/service.ts"]  // 可选，项目相对路径，去重限 6
}
```

### kind 白名单（12 个）

| kind | 适用 |
|---|---|
| `root` | 主线根（导入端对无父节点自动设为 root） |
| `module` | 子系统 / 顶层模块 |
| `component` | 组件、类、可复用单元 |
| `service` | 服务、接口层、后端能力 |
| `route` | 路由、页面入口、API 端点 |
| `config` | 配置项、环境变量、构建配置 |
| `file` | 具体文件 |
| `task` | 待办、步骤 |
| `requirement` | 需求点 |
| `constraint` | 约束、前提 |
| `risk` | 风险、未决问题 |
| `other` | 其它（兜底） |

越界值 → 导入时改为 `other` 并在结果里记一条 warning。

### 调色板（未指定 color 时按节点序号取）

`#22d3ee` `#34d399` `#fbbf24` `#60a5fa` `#fb7185` `#a78bfa` `#f97316` `#f59e0b` `#f8fafc` `#94a3b8`

## 3. 树关系规则（最容易出错的地方）

1. 每个节点的 `parent_id` **只能引用本文件中出现的 id**
2. 引用了不存在的 id → 该节点**成为一条新的主线**（不会报错、不会丢，但结构会变样）
3. 一个文件可以有**多条主线**（多个 `parent_id` 为空/悬空的节点），导入端支持多根
4. 不支持交叉引用 / 多父：想表达"同时属于 A 和 B"只能复制节点或写进 `detail`

## 4. 规范化与 warning

导入端会修以下情况并在 `warnings` 里逐条返回（不阻断导入）：

- `name` 为空 → 「未命名」
- `kind` 不在白名单 → `other`
- `color` 不是 `#RRGGBB` → `#94a3b8`
- `detail` 为空但 `description` 有值 → 用 `description` 补
- `sources` 逐项去掉前导 `./`、去空、去重，最多 6 个

## 5. 完整示例（三层 + 证据锚定）

```json
{
  "name": "订单系统",
  "description": "下单 → 支付 → 履约 的主链路",
  "layoutDir": "lr",
  "nodes": [
    { "id": "root",  "name": "订单系统", "kind": "root",   "detail": "面向 C 端的主交易链路，含超时补偿" },
    { "id": "m1",    "name": "下单",     "parent_id": "root", "kind": "module", "detail": "购物车校验 → 库存锁定 → 生成订单", "sources": ["src/order/create.ts"] },
    { "id": "m1-1",  "name": "库存锁定", "parent_id": "m1",   "kind": "service", "detail": "Redis 预扣，15 分钟未支付自动释放", "sources": ["src/inventory/lock.ts"] },
    { "id": "m2",    "name": "支付",     "parent_id": "root", "kind": "module", "detail": "三方支付对接 + 异步回调对账" },
    { "id": "m2-1",  "name": "超时关单", "parent_id": "m2",   "kind": "task",   "detail": "延时消息触发，先关单再退款", "color": "#fb7185" },
    { "id": "r1",    "name": "风险：重复支付", "parent_id": "root", "kind": "risk", "detail": "回调幂等依赖订单号 + 支付流水号唯一索引" }
  ]
}
```

## 6. 规模建议

| 项 | 建议值 | 原因 |
|---|---|---|
| 每层子节点 | 3～7（最多 12） | 画布横向宽度有限 |
| 层数 | ≤ 4 | 更深在画布上无法阅读 |
| `name` 长度 | ≤ 20 字 | 过长会把画布撑爆 |
| 总节点数 | ≤ 150 | 再多建议拆成多份导图 |

## 7. 导入接口（供脚本/其它命令参考）

- 命令：`mm_import_nodes`
- 入参：`{ path | content, documentId?, folderId?, replaceExisting? }`
  - `path` 与 `content` 二选一（都给以 `content` 为准）
  - 不给 `documentId` → 新建文档；给了 → 追加（`replaceExisting: true` 时先清空该文档节点）
  - 前端入口：思维导图面板左栏底部「导入 JSON 文件」→ 弹「导入到哪？」（新建 / 追加到当前文档 / 替换当前文档，替换需二次确认）
- 返回：`{ documentId, nodeCount, warnings }`
