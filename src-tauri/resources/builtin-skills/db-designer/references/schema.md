# `.dbdesign.json` 格式契约 v1

> 真源是 `src-tauri/src/commands/db_designer/models.rs`（Rust，serde `rename_all = "camelCase"`）。
> 本文与它一一对应；**改了 models.rs 就同步改本文**。
> 应用侧的校验实现：`store.rs::validate`（下面「校验规则」一节列的就是它）。

## 0. 顶层

```jsonc
{
  "id": "d1",            // 可选。文档 id；缺省时应用会自己补一个
  "name": "订单库",       // 必填。设计名
  "description": "",      // 可选
  "dialect": "mysql",    // 可选，默认 "mysql"。mysql | postgres | sqlite（其它值会被校验拦下）
  "folderId": null,      // 可选。预留字段
  "commonFields": [],    // 可选。公共字段：导出时自动附加到每张表末尾，见 §8.2
  "decisions": [],       // 可选。设计决策与约束（一句话一条），见 §8.3
  "nodes": [],           // 必填（可为空数组）
  "relations": [],       // 可选
  "updatedAt": ""        // 可选，ISO8601
}
```

## 1. 节点 `nodes[]`

表和视图**共用**一个节点类型，用 `kind` 区分。

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `id` | string | 是 | 文档内唯一的节点 id。关联引用的是表名/字段名，但这个 id 也出现在文件里，保持稳定 |
| `kind` | string | 否 | `table`（默认）/ `view`。其它值（含历史遗留的 `function`）会被校验拦下 |
| `name` | string | 是 | 表名 / 视图名。**不允许重名**（同一文档内） |
| `comment` | string | 否 | 注释（中文说明），会进导出 SQL 的表注释 |
| `tags` | string[] | 否 | 标签，画布上按标签筛选。自由文本 |
| `color` | string | 否 | `#rrggbb`。节点色条与它的连线都用这个颜色；非法值按「未设置」处理 |
| `x` / `y` | number | 否 | 画布坐标。**省略的后果**：全部节点叠在 (0,0) 附近，文件仍能打开但画面是一团。给坐标就按下面的网格排 |
| `table` | object / null | 表必填 | 见 §2 |
| `view` | object / null | 视图必填 | `{ "sql": "SELECT ..." }` |

### 坐标怎么给（`x` / `y`）

`x` / `y` 都能省略，但**省略 = 全部节点叠在左上角**，文件照样能打开、只是画面没法看。
所以：要么干脆不写（留给导入方去布局），要么自己排开 —— 按这个网格排，与设计器内部落图用的
是同一套间距，不会互相压盖：

```
一行 5 张，第 i 张（i 从 0 开始）：
  x = 40 + (i % 5) * 320
  y = 40 + floor(i / 5) * 300
```

列宽 320、行高 300 是按字段行数较多的表留的余量；字段多（> 15 行）时把行高放到 400 更稳妥。

## 2. 表体 `table`

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `fields` | 字段[] | 是（可为空） | 见 §3。字段名不允许重复 |
| `indexes` | 索引[] | 否 | 见 §4。缺省当空数组 |

## 3. 字段 `fields[]`

| 字段 | 类型 | 必填 | 默认 | 说明 |
|---|---|---|---|---|
| `name` | string | 是 | — | 列名，不允许重复 |
| `type` | 对象 | 是 | — | 逻辑类型，见 §5。键名固定是 `type`（不是 `dataType`） |
| `nullable` | bool | 否 | `false` | `true` = 允许 NULL。**默认是 NOT NULL** |
| `default` | string / null | 否 | 无 | **SQL 字面量**：字符串自己加引号（`'未知'`）、函数写 `CURRENT_TIMESTAMP` |
| `comment` | string | 否 | `""` | 列注释，进导出 SQL |
| `pk` | bool | 否 | `false` | 主键。**复合主键就给多个字段设 true** |
| `autoIncrement` | bool | 否 | `false` | 自增。MySQL→`AUTO_INCREMENT`、PG→`SERIAL/BIGSERIAL`、SQLite→`INTEGER PRIMARY KEY AUTOINCREMENT`。**一张表最多一个** |
| `unique` | bool | 否 | `false` | 唯一约束（`UNIQUE`） |

约束：

- `pk: true` 隐含 NOT NULL
- SQLite 下自增必须**单列自键**（唯一主键）
- 自增但不是键的组合在 MySQL / PG 上导出的 SQL 跑不起来 —— 别这么写
- **一张表只能有一个自增列**（MySQL 报 1067）；复合主键里也不能有自增列

### `default` 的三条硬规则

1. **一律写成字符串**。写裸值（`false` / `0`）是模型最自然的写法，应用侧现在能容错，
   但旧版本应用与把它当严格 JSON 读的其它工具会直接拒；约定上就是字符串。
2. **字符串值应当自带 SQL 引号**：`"default": "'enabled'"`。设计文件里存的是
   **SQL 字面量**而不是「字符串内容」，所以引号是值的一部分。
   写 `"default": "enabled"` 应用导出时会自动补成 `'enabled'`（并在校验条里给警告），
   但别指望它：自己写对，导出的产物才和你想的一样。函数与关键字例外：`"CURRENT_TIMESTAMP"`。
3. **`boolean` 列的默认值按方言写**：`boolean` 只是设计器内部的跨方言抽象 ——
   MySQL 没有 BOOLEAN（导成 `TINYINT(1)`）、SQLite 导成 `INTEGER`，两边都是数值存储，
   写 `"0"` / `"1"`；只有 `dialect: "postgres"` 才写 `"false"` / `"true"`
   （PG 上 `BOOLEAN DEFAULT 0` 是直接报错的）。导出会自动归一，写对只是让产物更好读。

## 4. 索引 `indexes[]`

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `name` | string | 是 | 索引名。**全文档唯一**（MySQL 是表内唯一，PG / SQLite 是全库唯一 —— 按最严口径） |
| `kind` | string | 否 | `index`（默认）/ `unique` / `fulltext` / `primary` |
| `fields` | string[] | 是 | 参与索引的**列名**（不是 id），按索引顺序 |

- `primary` 由字段的 `pk` 表达，**不要**再写一条 `primary` 索引（会导出两行 PRIMARY KEY）
- 复合索引按顺序写多个：`{ "name": "idx_a_b", "kind": "index", "fields": ["a", "b"] }`
- 索引名跨表不可重复（PG / SQLite 重名建表失败），一律带表名前缀
- `fulltext` 只有 MySQL 有对应语法，PG / SQLite 导出会降级成普通索引
- MySQL 不允许 TEXT / BLOB 列进普通或唯一索引（需指定前缀长度），导出会跳过并留注释
- 引用了不存在的列 → 校验失败

## 5. 逻辑类型 `type`

```jsonc
{ "base": "varchar", "length": 32 }                  // 长度
{ "base": "decimal", "precision": 10, "scale": 2 }   // 精度 / 小数位
{ "base": "enum", "values": ["new", "paid"] }        // 取值列表
{ "base": "bigint" }                                 // 无参数类型只写 base
```

| 键 | 类型 | 适用 |
|---|---|---|
| `base` | string | **必填**，白名单见下 |
| `length` | number | `varchar` / `char` |
| `precision` / `scale` | number | `decimal` |
| `values` | string[] | `enum` |
| `unsigned` | bool | 预留 |

`base` 白名单（**18 个，越界即校验失败**）：
`int` `bigint` `smallint` `tinyint` `decimal` `float` `double` `char` `varchar`
`text` `date` `time` `datetime` `timestamp` `boolean` `json` `uuid` `blob` `enum`

各方言映射由后端 `export.rs` 负责，产出方不用管。

## 6. 关联 `relations[]`

```jsonc
{
  "id": "r1",
  "name": "",                                  // 可选，关系备注
  "from": { "node": "t_orders", "field": "user_id" },   // 子表（外键侧，"多"）
  "to":   { "node": "t_users",  "field": "id"      },   // 父表（被引用，"一"）
  "kind": "1-n",                               // 1-1 | 1-n（默认）| n-n
  "onDelete": "RESTRICT",                     // RESTRICT | CASCADE | SET NULL | SET DEFAULT | NO ACTION
  "onUpdate": "RESTRICT",
  "mirror": true                              // 可选，见 §7
}
```

**方向铁律**：`from` = 子表 = 外键所在的那一端。外键只建在 `from` 上，`to` 侧不会被挂外键。
`from.node` / `to.node` 是**节点 id**，`from.field` / `to.field` 是**字段名**（这是最容易混的地方）。

- `n-n`（多对多）：导出时后端自动生成一张 `a_b` 中间表；中间表要带额外字段（数量、备注）时自己显式建表
- 关联两端引用了不存在的字段 / 视图 → 校验失败
- 自关联（`from.node === to.node`）会被忽略，不要这么写

## 7. `mirror`（镜像主键）

`mirror: true` 表示「**子表镜像父表的主键**」——这层语义是应用侧的，导出 SQL 时不体现。

- `true`：父表以后**加/减主键、改主键类型**，子表的外键副本与连线自动跟着变（应用内 reconcile）
- 缺省 / `false`：这是独立的一条关系，父表后来加主键**不会**推给子表

只挑父表的某几列做引用时**不要**填 `true`（那等于让应用替你改设计）。

## 8. 校验规则（与 `store.rs::validate` 对齐）

error（会拦住保存 / 导出）：

1. 节点 `kind` 不在 `table` / `view` 里
2. 节点名重复
3. 字段 `type.base` 不在 18 个白名单里
4. 同一表内字段名重复
5. 索引引用了不存在的字段；`kind` 越界；索引名跨表重复
6. 一张表有多个自增列
6. 关联引用了不存在的节点 / 字段，或字段所在节点不是表
7. `dialect` 不是三种之一
8. 缺 `id` / `name` 等必填字段

warning（能保存，但该知道）：

- 没有任何主键的表
- 自增字段不是主键（导出的 SQL 在 MySQL / PG 上会失败）
- 视图没有 `view.sql`
- 关联指向了不在本文件里的表（跨库外键，只提示）
- `default` 写成裸值（boolean / 数字），或字符串值忘了加 SQL 引号
- MySQL：TEXT / BLOB / JSON 列的默认值不会导出（报 1101）
- MySQL：TEXT / BLOB 列上的普通 / 唯一索引会被跳过（报 1170）
- 非 MySQL：`fulltext` 索引会降级成普通索引

## 8.1 不在设计文件里的东西：导出选项

「是否输出外键约束」「导出到文件还是剪贴板」这类**导出选项不属于设计文件**，
它们只作用于应用里那一次导出。写进 `.dbdesign.json` 只会让拿到文件的人看到一份
自己不记得设过的开关。

所以需要一份不带外键约束的 SQL 时，**不要去改设计文件**：在应用的导出弹框里关掉
「包含外键约束」即可（CLI：`kira-dbd export-sql --no-foreign-keys`）。

## 8.2 公共字段 `commonFields`（可选）

**一份「所有表都该有的字段」的可勾选配置**：导出 SQL 时自动附加到**每张表字段末尾**。

```jsonc
"commonFields": [
  { "name": "created_at", "type": { "base": "datetime" }, "nullable": false,
    "default": "CURRENT_TIMESTAMP", "comment": "创建时间" },
  { "name": "is_deleted", "type": { "base": "boolean" }, "nullable": false,
    "default": "false", "comment": "是否逻辑删除" }
]
```

规则（Agent 产出时必须知道，否则会白写一遍）：

1. **配置里启用哪些、以什么名字启用，一律由文件决定** —— 存的是完整定义（自包含），
   缺省 / 空数组 = 不附加。
2. **表里已有同名字段时以表自己的为准**：不重复添加、不合并、不覆盖。
   比对时**忽略大小写与下划线**（`CreatedAt` 与 `created_at` 算同一个 —— MySQL 列名
   不区分大小写，两个都写会 `Duplicate column name` 建表失败）。
3. **所以：产出设计时不要手动创建这些字段。** 用户若在设计文件里启用了公共字段，
   你写在自己表里的 `created_at` 会因为重名被跳过（表自己的赢），等于白写，
   还让用户误以为配置没生效。
4. 公共字段不参与「通用裸名」「外键索引」等表级校验（它是文档级配置，不是某张表的字段）。
5. 中间表（n-n 自动生成）**不附加**公共字段。

## 8.3 决策记录 `decisions`（可选）

**一句话一条记「为什么这么设计」**，存成字符串数组。应用内的 AI 助手**每次生成都会读它**，
所以它不是日志，是**长期记忆**：下次改这张表时，模型会知道当初为什么这么定。

```jsonc
"decisions": [
  "订单明细冗余 product_name 与 product_price 快照，保证订单完成后仍能追溯下单时的商品与价格",
  "status 改成枚举并加入已关闭，因为财务要求保留关单记录"
]
```

规则：

1. **只写「下次改动时需要知道的东西」**。判断标准：这句话能不能让你在几周后避免改错东西？
   不能就别写。
2. **不要写进 `decisions` 的三类内容**：
   - **过程记录**：「其余 11 张表原样保留」「本轮新增 1 张表」—— 用户从界面已经知道，重复只是噪声；
   - **规则复述**：「已遵守命名规范」—— 规范本来就写在文档里；
   - **实现细节**：「导出 SQL 会自动加反引号」「位置由界面决定」—— 那是工具行为，不是设计决策。
3. **缺省空数组 = 无影响**，旧设计文件与不写它的产出完全不受影响。
4. **写入是安全的**：应用内 AI 每轮都会回吐整份文档、理论上可能把你的决策改写，
   但应用侧的做法是「**保留原文档的这份，只追加模型本轮新增的**」——
   所以你写进去的决策不会被下一轮生成冲掉。

> 与应用内 AI 的差别只有一处：AI 每轮把 `decisions` 和设计**分开**输出（便于只追加新增的那些），
> 而 `.dbdesign.json` 是**单一文件**，`decisions` 就写在顶层。

## 9. 最小可用完整示例

见 `SKILL.md` §1。三个最容易踩的坑：

1. `from` 必须是子表（外键侧）
2. 类型写逻辑类型对象，不写 `VARCHAR(32)` 整串
3. 主键只用字段 `pk: true` 表达，别额外写 `primary` 索引
