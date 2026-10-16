---
name: db-designer
description: 为 any-version 的数据库设计器**生成可直接导入的设计文件**。当用户（或另一个 Agent）要求「根据这段需求设计表结构」「把这份需求/文档变成 ER 图」「产出数据库设计给我导入」「按 any-version 设计格式输出」时使用；也用于把零散的字段清单整理成带主键/外键/索引的完整设计。产物是一份符合格式的 `.dbdesign.json`，any-version 的数据库设计器可直接导入（纯文件，不连库、不改库；导出 SQL 由设计器负责）。导入格式、字段含义与校验规则见 `references/schema.md`，生成后用 `scripts/validate.py` 自检。
---

# 生成可导入的数据库设计文件（db-designer）

> 本技能随 Kira（any-version）**内置**分发，应用内可从技能管理页安装到任意目录
> （例如 `~/.agents/skills`、某个工具的技能目录）。安装后本目录下的 `references/` 与 `scripts/` 一起带过去。

## 0. 这份技能解决什么

any-version 的数据库设计器是**纯文件**的：画的是「表 / 视图」组成的图，改的是一份 JSON，
点导出才生成 SQL —— **不连库、不改库**。所以外部已经懂需求的 Agent（Codex / Claude Code / 脚本）
最省事的路径是：**直接产出一份符合格式的 `.dbdesign.json`，用户在应用里一键打开**。

设计器还顺手替你做了两件事，**不要重复劳动**：

1. **反向工程**：已有 `.sql`（mysqldump / pg_dump）或 `.db`（SQLite）时，用户用「逆向 DDL / 逆向 SQLite」
   就能拿到结构 —— **这种情况根本不需要本技能**，别去手写 JSON。
2. **三种方言导出**：MySQL / PostgreSQL / SQLite 的类型映射、索引写法全在后端，你只管把
   **逻辑类型**写对。

本技能只关心**怎么把设计文件写对**。

## 1. 文件长什么样（最小可用）

> **示例本身是合规的**：`user_id` / `order_id` 作主键、`user_email` 带表前缀、
> 外键 `user_id` 建了索引。示例违规的话 Agent 会照抄 —— 那比规则写漏了更常见。

```json
{
  "id": "d1",
  "name": "订单库",
  "dialect": "mysql",
  "nodes": [
    {
      "id": "t_users",
      "kind": "table",
      "name": "users",
      "comment": "用户表",
      "table": {
        "fields": [
          { "name": "user_id", "type": { "base": "bigint" }, "pk": true, "autoIncrement": true, "comment": "用户主键" },
          { "name": "user_email", "type": { "base": "varchar", "length": 255 }, "unique": true, "comment": "邮箱" },
          { "name": "created_at", "type": { "base": "datetime" }, "default": "CURRENT_TIMESTAMP" }
        ],
        "indexes": [
          { "name": "idx_users_user_email", "kind": "unique", "fields": ["user_email"] }
        ]
      }
    },
    {
      "id": "t_orders",
      "kind": "table",
      "name": "orders",
      "comment": "订单表",
      "table": {
        "fields": [
          { "name": "order_id", "type": { "base": "bigint" }, "pk": true, "autoIncrement": true, "comment": "订单主键" },
          { "name": "user_id", "type": { "base": "bigint" }, "comment": "下单人，引用 users.user_id" }
        ],
        "indexes": [
          { "name": "idx_orders_user_id", "kind": "index", "fields": ["user_id"] }
        ]
      }
    }
  ],
  "relations": [
    {
      "id": "r1",
      "from": { "node": "t_orders", "field": "user_id" },
      "to": { "node": "t_users", "field": "user_id" },
      "kind": "1-n",
      "onDelete": "RESTRICT",
      "onUpdate": "RESTRICT"
    }
  ]
}
```

顶层可带：`id`（可选，缺省时应用会补） / `name` / `description` / `dialect`（`mysql` 默认、`postgres`、`sqlite`）
/ `folderId` / **`commonFields`（公共字段，见 references/schema.md §8.2）** /
**`decisions`（设计决策记录，见 references/schema.md §8.3）** / `nodes` / `relations` / `updatedAt`。

> `decisions` 一句话一条记「为什么这么设计」，应用内 AI 每次生成都会读它 ——
> 需求里有明确取舍的地方（冗余字段的理由、枚举取值、软删除、时区口径…）就写一条，
> 别写「本轮新增 3 张表」这种过程记录。详见 §8.3。

> 若目标设计文件启用了 `commonFields`：**不要手动创建那些字段**（如 `created_at`），
> 导出时会自动附加；你手动写的会因重名被跳过（表自己的定义优先）。

## 2. 关键约定（最容易写错的地方）

### 2.1 命名规范（**error 级，会拦住保存与导出**）

这三条与应用侧 `store.rs::validate` 逐条对齐，违反就导不出 SQL：

| 规则 | 对 / 错 |
|---|---|
| **主键禁止裸 `id`**，用 `<实体>_id` | `users` 表 → `user_id`；`orders` 表 → `order_id` |
| **通用裸名必须带表名前缀** | `email` ✗ → `user_email`；`status` ✗ → `order_status` |
| **每个外键字段都要有索引** | `orders.user_id` 是外键 → `indexes` 里必须有 `{"fields": ["user_id"]}` |
| **一张表只能有一个自增列** | 两个字段都写 `autoIncrement: true` → MySQL 建表直接失败（1067） |
| **索引名全文档唯一** | 两张表都用 `idx_user` → PostgreSQL / SQLite 建表失败（索引名要求全库唯一），一律带表名前缀 |

「通用裸名」指这些词单独出现时无法分辨属于哪张表：
`id` `name` `title` `label` `desc` `description` `price` `amount` `status` `type` `code`
`value` `content` `text` `url` `uri` `phone` `email` `address` `age` `sex` `gender`
`count` `num` `number` `total` `size` `weight` `height` `color` `comment` `note`
`remark` `date` `time` `year` `month` `day` `sort` `order` `flag` `enabled`

**已经带下划线的不用再加前缀** —— `created_at`、`user_id`、`order_status` 都合法。
判定口径是「**不含下划线** 且命中上面的词」，所以 `username`（含下划线）不会被要求改成 `user_username`。

> 复合主键（多个字段 `pk: true`）例外：不能出现裸 `id`，且至少一个字段带下划线。

### 2.2 其他约定

| 约定 | 说明 |
|---|---|
| **引用用的是名字，不是 id** | 关联和索引引用的是**表名字段名 / 字段名**。应用侧改名会级联更新，但你写文件时必须前后一致 |
| **方向别搞反** | `from` = **子表**（外键所在、"多"的一端），`to` = **父表**（被引用、"一"的一端）。外键只建在 `from` 上 |
| **`mirror`** | `true` = 「子表镜像父表主键」：父表以后加减主键，子表自动跟着变。整表引用式关联才填；只挑某几列时留空 |
| **`type` 是逻辑类型** | 写 `{ "base": "varchar", "length": 32 }`，**不要**写 `VARCHAR(32)` 整串 —— 导出时后端按方言映射 |
| **`primary` 索引不用写** | 主键由字段的 `pk: true` 表达。再写一条 `kind: "primary"` 会导出成两行 PRIMARY KEY |
| **只支持 table / view** | 函数节点不做（SQLite 没有函数、MySQL/PG 语法差异大） |

`type.base` 白名单（18 个）：
`int` `bigint` `smallint` `tinyint` `decimal` `float` `double` `char` `varchar`
`text` `date` `time` `datetime` `timestamp` `boolean` `json` `uuid` `blob` `enum`

字段其余属性：`nullable`（**默认 false**，写 `true` 才允许 NULL）、`default`（**SQL 字面量，一律写成字符串**）、
`comment`、`pk`、`autoIncrement`、`unique`。
索引：`{ name, kind, fields }`，`kind` ∈ `index` `unique` `fulltext`（`primary` 别用）。

### 2.3 `default` 怎么写（最容易踩）

1. **一律写成字符串**：`"default": "0"`、`"default": "CURRENT_TIMESTAMP"`。
   写裸值 `"default": false` / `0` 是模型最自然的写法，也是最常见的坑 ——
   应用侧现在能容错，但旧版本应用与严格 JSON 的工具会直接拒。
2. **字符串值应当自带 SQL 引号**：`"default": "'enabled'"`。设计文件里存的是
   **SQL 字面量**，引号是值的一部分。漏引号时应用导出会自动补（并在校验条警告），
   但别指望它 —— 自己写对，产物才和你想的一样。
3. **`boolean` 列按方言写**：`boolean` 只是设计器内部的抽象 —— MySQL 没有 BOOLEAN
   （导成 `TINYINT(1)`）、SQLite 导成 `INTEGER`，两边都是数值存储，写 `"0"` / `"1"`；
   只有 `dialect: "postgres"` 才写 `"false"` / `"true"`（PG 上 `BOOLEAN DEFAULT 0` 直接报错）。
4. **MySQL 的 TEXT / BLOB / JSON 列不要给默认值**（报 1101，导出时会被跳过）。

**导出选项不在设计文件里**：「这份 SQL 要不要带外键约束」是导出时的选择
（应用导出弹框 / `kira-dbd export-sql --no-foreign-keys`），**不要**为了导出一份
不带外键的 SQL 去改设计文件。

完整字段表见 `references/schema.md`。

## 3. 生成流程（按输入类型）

### 3.1 一段需求 / 一段话

1. 先数清**实体**（名词）与**关系**（动词）→ 每张表
2. 标出**主键**与**唯一键**。主键名用 `<实体>_id`（`users` 表用 `user_id`、`orders` 表用 `order_id`），
   业务键做主键时用业务键本身（如 `order_no`）。**不要用裸 `id`** —— 见 §2.1
3. 把需求里的限定词翻译成约束：「必须/不能为空」→ `nullable: false`；「默认…」→ `default`
4. 一对一写成外键 + `kind: "1-1"`；多对多**加一张中间表**

### 3.2 一个项目（代码仓库）

1. **先看真实表结构**（建表语句 / migration / ORM 模型），不要按想象的架构写
2. 从字段旁的注释、API 文档里抄 `comment`
3. 对照实际的索引与唯一约束填 `indexes` —— 漏索引在真实库里是性能债
4. 复合主键就**给多个字段 `pk: true`**（别用 `UNIQUE(a,b)` 冒充主键）

### 3.3 一份字段清单 / 文档

1. 原文里的**表名段落** → 一张表一张表来，别合并
2. 「用户ID（下单人）」这种"括号里的说明"→ 拆成 `name: "user_id"` + `comment: "下单人"`
3. 原文没提类型的字段**不要瞎猜**：先按最保守的类型给（名字/标题 `varchar(255)`，时间 `datetime`，数量 `bigint`），并在回复里列出「这些字段类型是猜的，请确认」

通用要求：
- 表数量按真实实体来；宁可少而准，不要为了显得完整编表
- 每张表 3～15 个字段；超过 20 个就该考虑拆表 / 留扩展字段
- `comment` 用中文写清楚业务含义（它是给人看的，也是 Agent 产出里最值钱的部分）

## 4. 自检（必做）

```bash
# 在本技能目录（SKILL.md 所在目录）下执行
python scripts/validate.py <file.dbdesign.json>

# 或在任意位置指定完整路径
python <技能安装目录>/db-designer/scripts/validate.py <file.dbdesign.json>
```

脚本会报：JSON 是否合法、必填字段、`type.base` 是否越界、索引/关联是否引用了不存在的字段、
表名与字段名是否重复、`pk` 与 `autoIncrement` 的组合是否合法、视图是否缺 `view.sql`。
**报错项修完再交付**；warning 要在回复里跟用户说明。

命令行还能自检（不需要装技能，另有一个随应用发布的 CLI）：

```bash
kira-dbd validate <file.dbdesign.json>     # 与应用内校验同一套规则
kira-dbd export-sql <file.dbdesign.json>   # 直接出建表 SQL
kira-dbd export-sql <file.dbdesign.json> --no-foreign-keys   # 不带外键约束（与应用导出弹框同一个开关）
```

## 5. 常见错误（踩过的坑）

1. **关联方向写反** → 外键建到了父表上。记忆：`from` 永远是**子表**（"多"的一端）
2. **写 `VARCHAR(32)` 整串** → 类型不在白名单里，导出 SQL 时才报错。写 `{ "base": "varchar", "length": 32 }`
3. **把 `primary` 索引也写一条** → 导出 SQL 出现两行 PRIMARY KEY。主键只用字段的 `pk: true`
4. **漏写 `nullable`** → 默认 false（NOT NULL）。想在需求里「可空」必须显式写 `nullable: true`
5. **索引引用了不存在的字段** → 校验直接失败，导出也会被拦下
6. **关系引用了视图** → 视图没有字段概念，不能作为外键的一端
7. **给 JSON 套 Markdown 代码围栏** —— 导入端能剥，但**不要依赖它**，直接输出纯 JSON
8. **多对多不建中间表** → 导出时应用会自己生成一张 `a_b` 中间表；但如果中间表需要额外字段（数量、备注），就要显式建出来
9. **`default` 写成裸值**（`false` / `0`）→ 旧版本应用直接拒收；字符串忘加引号（`"enabled"` 而不是 `"'enabled'"`）→ 导出虽会自动补，但产物与设计文件不一致。见 §2.3
10. **两张表用同一个索引名** → PostgreSQL / SQLite 建表失败（索引名要求全库唯一）。索引名一律带表名前缀
11. **一张表写多个自增列** → MySQL 建表直接失败（1067）
12. **为了导出一份不带外键的 SQL 去改设计文件** → 导出选项不属于设计文件，用导出弹框的「包含外键约束」开关（CLI：`--no-foreign-keys`）
