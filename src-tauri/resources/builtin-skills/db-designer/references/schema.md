# `.dbdesign.json` 格式契约 v1

> 真源是 `src-tauri/src/commands/db_designer/models.rs`（Rust，serde `rename_all = "camelCase"`）。
> 本文与它一一对应；**改了 models.rs 就同步改本文**。
> 应用侧的校验实现：`store.rs::validate`（下面「校验规则」一节列的就是它）。

## 0. 顶层

```jsonc
{
  "id": "d1",            // 必填。文档 id，任意字符串
  "name": "订单库",       // 必填。设计名
  "description": "",      // 可选
  "dialect": "mysql",    // 可选，默认 "mysql"。mysql | postgres | sqlite（其它值会被校验拦下）
  "folderId": null,      // 可选。预留字段
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
| `x` / `y` | number | 否 | 画布坐标。省略也能打开（会叠在左上角），建议用「自动布局」算好的值 |
| `table` | object / null | 表必填 | 见 §2 |
| `view` | object / null | 视图必填 | `{ "sql": "SELECT ..." }` |

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
| `autoIncrement` | bool | 否 | `false` | 自增。MySQL→`AUTO_INCREMENT`、PG→`SERIAL/BIGSERIAL`、SQLite→`INTEGER PRIMARY KEY AUTOINCREMENT` |
| `unique` | bool | 否 | `false` | 唯一约束（`UNIQUE`） |

约束：

- `pk: true` 隐含 NOT NULL
- SQLite 下自增必须**单列自键**（唯一主键）
- 自增但不是键的组合在 MySQL / PG 上导出的 SQL 跑不起来 —— 别这么写

## 4. 索引 `indexes[]`

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `name` | string | 是 | 索引名，同一表内不重名 |
| `kind` | string | 否 | `index`（默认）/ `unique` / `fulltext` / `primary` |
| `fields` | string[] | 是 | 参与索引的**列名**（不是 id），按索引顺序 |

- `primary` 由字段的 `pk` 表达，**不要**再写一条 `primary` 索引（会导出两行 PRIMARY KEY）
- 复合索引按顺序写多个：`{ "name": "idx_a_b", "kind": "index", "fields": ["a", "b"] }`
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
5. 索引引用了不存在的字段；同名索引重复；`kind` 越界
6. 关联引用了不存在的节点 / 字段，或字段所在节点不是表
7. `dialect` 不是三种之一
8. 缺 `id` / `name` 等必填字段

warning（能保存，但该知道）：

- 没有任何主键的表
- 自增字段不是主键（导出的 SQL 在 MySQL / PG 上会失败）
- 视图没有 `view.sql`
- 关联指向了不在本文件里的表（跨库外键，只提示）

## 9. 最小可用完整示例

见 `SKILL.md` §1。三个最容易踩的坑：

1. `from` 必须是子表（外键侧）
2. 类型写逻辑类型对象，不写 `VARCHAR(32)` 整串
3. 主键只用字段 `pk: true` 表达，别额外写 `primary` 索引
