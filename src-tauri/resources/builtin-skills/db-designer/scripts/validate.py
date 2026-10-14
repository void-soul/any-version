#!/usr/bin/env python3
# -*- coding: utf-8 -*-
""".dbdesign.json 自检脚本（db-designer 技能自带）

用法：
    python scripts/validate.py <file.dbdesign.json>

设计原则：**只做结构与引用检查，不改文件**。
规则与应用的 store.rs::validate 对齐（error 会拦住保存/导出，warning 只是提示）。
权威实现是 Rust 那份；这里是给 Agent 用的轻量副本，规则变了要同步两边。
"""
import json
import re
import sys

BASE_TYPES = {
    "int", "bigint", "smallint", "tinyint", "decimal", "float", "double",
    "char", "varchar", "text", "date", "time", "datetime", "timestamp",
    "boolean", "json", "uuid", "blob", "enum",
}
NODE_KINDS = {"table", "view"}
REL_KINDS = {"1-1", "1-n", "n-n"}
INDEX_KINDS = {"primary", "unique", "index", "fulltext"}
FK_ACTIONS = {"RESTRICT", "CASCADE", "SET NULL", "SET DEFAULT", "NO ACTION"}
DIALECTS = {"mysql", "postgres", "sqlite"}

# ── 命名规范（与应用侧 store.rs::validate 的 validate_naming 逐条对齐）────────
#
# 这三条是 error 级：违反了应用会拦住保存与导出。缺了它们，这个自检脚本就会对
# 一份导不出来的设计报「全部通过」—— 那比不检查更糟，因为 Agent 会据此交付。
GENERIC_FIELD_NAMES = {
    "id", "name", "title", "label", "desc", "description", "price", "amount",
    "status", "type", "code", "value", "content", "text", "url", "uri",
    "phone", "email", "address", "age", "sex", "gender", "count", "num",
    "number", "total", "size", "weight", "height", "color", "comment",
    "note", "remark", "date", "time", "year", "month", "day", "sort",
    "order", "flag", "enabled",
}


def singularize(name):
    """表名去复数 -> 实体名。与 store.rs::singularize 同规则。

    刻意不处理不规则名词（users -> user 可以，people -> peopl 不会）：
    Rust 那份也是这样，宁可让用户自己起对名字，不要猜错。
    """
    n = (name or "").strip().lower()
    if n.endswith("ies") and len(n) > 3:
        return n[:-3] + "y"
    if n.endswith("ses") and len(n) > 3:
        return n[:-2]
    if n.endswith("s") and not n.endswith("ss"):
        return n[:-1]
    return n


def check_naming(name, table, pk_cols, idx_field_sets):
    """主键命名、通用裸名、外键索引覆盖。"""
    entity = singularize(name)

    # 1) 主键命名
    if len(pk_cols) == 1:
        expect = f"{entity}_id"
        if pk_cols[0] != expect:
            err(
                f"表 {name}：主键应命名为 {expect}（当前是 {pk_cols[0]}）"
                f"—— 裸 id 会让联表 SQL 分不清这一列属于谁"
            )
    elif len(pk_cols) > 1:
        names = [c.lower() for c in pk_cols]
        if "id" in names:
            err(f"表 {name}：复合主键里不能有裸 id（{', '.join(pk_cols)}）")
        elif not any("_" in c for c in names):
            err(
                f"表 {name}：复合主键（{', '.join(pk_cols)}）至少一个字段要带下划线，"
                f"建议其中一个叫 {entity}_id"
            )

    # 2) 通用裸名（不含下划线的才查：带下划线视为已带表前缀）
    for fname in [f.get("name") for f in table.get("fields") or []]:
        if not fname:
            continue
        low = fname.lower()
        if low in GENERIC_FIELD_NAMES and "_" not in fname:
            err(
                f"表 {name}：字段 {fname!r} 是通用裸名，跨表同名时无法分辨归属，"
                f"应改为 {entity}_{low}"
            )
    return entity

errors = []
warnings = []


def err(msg):
    errors.append(msg)


def warn(msg):
    warnings.append(msg)


def _bool_default_literal(value, dialect):
    """boolean 列的默认值该写成什么。

    `boolean` 只是设计器内部的跨方言抽象：MySQL 根本没有 BOOLEAN（导出成 TINYINT(1)），
    SQLite 只有存储类 INTEGER（也导出成 INTEGER），两边都是数值存储 → 写 "0" / "1"；
    只有 PostgreSQL 是原生 BOOLEAN → 才写 "false" / "true"。
    """
    truthy = value is True or (isinstance(value, str) and value.strip().lower() in ("true", "t", "1", "yes"))
    if dialect == "postgres":
        return "true" if truthy else "false"
    return "1" if truthy else "0"


def check_type(t, where):
    if not isinstance(t, dict):
        err(f"{where}：type 必须是对象（如 {{\"base\": \"varchar\", \"length\": 32}}）")
        return
    base = t.get("base")
    if not base:
        err(f"{where}：type.base 缺失")
    elif base not in BASE_TYPES:
        err(f"{where}：type.base={base!r} 不在白名单里")
    if base in ("varchar", "char") and t.get("length") is None:
        warn(f"{where}：{base} 没写 length，导出时按 255 处理")
    if base == "enum" and not (t.get("values") or []):
        err(f"{where}：enum 必须给 values（如 [\"new\", \"paid\"]）")
    if base == "decimal":
        if t.get("precision") is None:
            warn(f"{where}：decimal 没写 precision，导出时按 10 处理")
        if t.get("scale") is None:
            warn(f"{where}：decimal 没写 scale，导出时按 2 处理")


def check_table(node, dialect="mysql"):
    name = node.get("name", "?")
    table = node.get("table")
    if table is None:
        err(f"表 {name}：缺少 table 字段（视图才用 view）")
        return set()
    if not isinstance(table, dict):
        err(f"表 {name}：table 必须是对象")
        return set()
    fields = table.get("fields")
    if fields is None:
        err(f"表 {name}：缺少 table.fields")
        return set()
    if not isinstance(fields, list):
        err(f"表 {name}：table.fields 必须是数组")
        return set()
    if not fields:
        warn(f"表 {name}：一个字段都没有")

    seen = set()
    pk_cols = []
    auto_cols = []
    for i, f in enumerate(fields):
        where = f"表 {name} 第 {i + 1} 个字段"
        if not isinstance(f, dict):
            err(f"{where}：不是对象")
            continue
        fname = f.get("name")
        if not fname:
            err(f"{where}：缺少 name")
            continue
        if fname in seen:
            err(f"表 {name}：字段名重复 {fname!r}")
        seen.add(fname)
        check_type(f.get("type"), f"表 {name} 字段 {fname}")
        if f.get("pk"):
            pk_cols.append(fname)
            if f.get("nullable"):
                warn(f"表 {name} 字段 {fname}：主键不该可空（导出时强制 NOT NULL）")
        if f.get("autoIncrement"):
            auto_cols.append(fname)
        d = f.get("default")
        base = (f.get("type") or {}).get("base") if isinstance(f.get("type"), dict) else None
        if d is not None and not isinstance(d, str):
            # 裸 boolean / 数字：模型最自然的写法，也是最容易踩的坑。应用侧现在能容错，
            # 但约定上 default 一律是字符串（旧版本应用、以及把它当严格 JSON 读的其它
            # 工具会直接拒），所以这里按「写法不合规」提示，并给出该方言该怎么写。
            want = _bool_default_literal(d, dialect) if base == "boolean" else None
            tail = f"，{base} 列在 {dialect} 上建议写 {want!r}" if want else ""
            warn(
                f"表 {name} 字段 {fname}：default={d!r} 是裸值，"
                f"必须写成字符串（如 \"0\" / \"CURRENT_TIMESTAMP\"）{tail}"
            )
        elif isinstance(d, str) and d.strip():
            # 字符串字面量忘了加引号是高频错误
            v = d.strip()
            looks_literal = (
                v.startswith(("'", '"', "("))
                or re.match(r"^-?\d", v)
                or v.upper() in ("NULL", "TRUE", "FALSE", "CURRENT_TIMESTAMP", "NOW()")
            )
            if not looks_literal:
                warn(f"表 {name} 字段 {fname}：default={v!r} 看起来不像 SQL 字面量（字符串要自己加引号）")
            if base == "boolean" and v.lower() in ("true", "false"):
                want = _bool_default_literal(v, dialect)
                warn(
                    f"表 {name} 字段 {fname}：boolean 列在 {dialect} 上实际是数值存储，"
                    f"默认值建议写 {want!r}（写成 {v!r} 也能导，但列类型与字面量对不上）"
                )

    if len(pk_cols) > 1 and auto_cols:
        err(f"表 {name}：复合主键里不能有自增列（{', '.join(pk_cols)}），导出的 SQL 跑不起来")
    if len(auto_cols) > 1:
        # MySQL 报1067（there can be only one auto column）；PG/SQLite 建得出来但没人用第二个
        err(
            f"表 {name}：有 {len(auto_cols)} 个自增列（{', '.join(auto_cols)}）"
            f"—— 一张表只能有一个自增列，MySQL 建表会直接失败"
        )
    for c in auto_cols:
        if c not in pk_cols:
            err(f"表 {name}：自增列 {c} 不是主键（MySQL/PG 不允许），要么加 pk:true 要么去掉自增")
    if not pk_cols:
        warn(f"表 {name}：没有主键")

    indexes = table.get("indexes") or []
    if not isinstance(indexes, list):
        err(f"表 {name}：table.indexes 必须是数组")
        return seen
    idx_names = set()
    for i, idx in enumerate(indexes):
        where = f"表 {name} 第 {i + 1} 个索引"
        if not isinstance(idx, dict):
            err(f"{where}：不是对象")
            continue
        iname = idx.get("name")
        if not iname:
            err(f"{where}：缺少 name")
        elif iname in idx_names:
            err(f"表 {name}：索引名重复 {iname!r}")
        else:
            idx_names.add(iname)
        kind = idx.get("kind", "index")
        if kind not in INDEX_KINDS:
            err(f"{where}：kind={kind!r} 越界（允许 {'/'.join(sorted(INDEX_KINDS))}）")
        if kind == "primary":
            err(f"{where}：不要写 primary 索引 —— 主键用字段 pk:true 表达，重复写会导出两行 PRIMARY KEY")
        cols = idx.get("fields")
        if not isinstance(cols, list) or not cols:
            err(f"{where}：fields 必须是非空数组（列名）")
            continue
        for c in cols:
            if c not in seen:
                err(f"{where}：引用了不存在的字段 {c!r}")
        if kind == "unique" and cols and all(c in pk_cols for c in cols):
            warn(f"{where}：唯一索引的列全是主键，冗余")
        if kind == "fulltext" and dialect != "mysql":
            warn(
                f"{where}：{dialect} 没有全文索引语法（SQLite 要 FTS5 虚拟表），"
                f"导出时会降级成普通索引"
            )
        if dialect == "mysql" and kind != "fulltext":
            # MySQL 报 1170：BLOB/TEXT column used in key specification without a key length
            for c in cols:
                fld = next((f for f in fields if isinstance(f, dict) and f.get("name") == c), None)
                ftype = fld.get("type") if isinstance(fld, dict) else None
                fbase = ftype.get("base") if isinstance(ftype, dict) else None
                if fbase in ("text", "blob"):
                    warn(
                        f"{where}：建在 {c}（{fbase}）列上 —— MySQL 不允许 TEXT/BLOB 进普通或唯一索引"
                        f"（需要指定前缀长度，如 ({c}(191))），导出会跳过它"
                    )

    # 命名规范：主键命名 + 通用裸名
    check_naming(name, table, pk_cols, None)

    # MySQL 不允许 TEXT/BLOB/JSON 列带默认值（报 1101），导出时会跳过
    if dialect == "mysql":
        for f in fields:
            if not isinstance(f, dict):
                continue
            ftype = f.get("type") if isinstance(f.get("type"), dict) else None
            fbase = ftype.get("base") if isinstance(ftype, dict) else None
            dv = f.get("default")
            if fbase in ("text", "blob", "json") and isinstance(dv, str) and dv.strip():
                warn(
                    f"表 {name} 字段 {f.get('name')}（{fbase}）：默认值 {dv!r} 不会导出 —— "
                    f"MySQL 不允许 TEXT/BLOB/JSON 列带默认值（报 1101）"
                )

    # 外键索引覆盖：应用侧 store.rs::validate 会把「外键字段没索引」当 error，
    # 这里同口径 —— 否则自检通过、导入后照样导不出 SQL。
    idx_fields = {
        c
        for idx in indexes
        if isinstance(idx, dict)
        for c in (idx.get("fields") or [])
        if isinstance(c, str)
    }
    return seen


def check(doc):
    if not isinstance(doc, dict):
        err("顶层必须是对象")
        return
    if not doc.get("id"):
        warn("顶层缺 id（应用会自动补，但最好显式写）")
    if not doc.get("name"):
        warn("顶层缺 name")
    dialect = doc.get("dialect", "mysql")
    if dialect not in DIALECTS:
        err(f"dialect={dialect!r} 不是 mysql/postgres/sqlite")

    nodes = doc.get("nodes")
    if nodes is None:
        err("缺少顶层 nodes")
        return
    if not isinstance(nodes, list):
        err("nodes 必须是数组")
        return
    if not nodes:
        warn("一个节点都没有")

    by_id = {}
    names = set()
    fields_of = {}
    # 每张表被索引覆盖的列名：用于「外键字段必须有索引」那条 error
    idx_fields_of = {}
    # 索引名（小写）→ 首次出现的表名：用于「索引名跨表唯一」那条 error
    idx_name_owner = {}
    for i, n in enumerate(nodes):
        if not isinstance(n, dict):
            err(f"第 {i + 1} 个节点不是对象")
            continue
        nid = n.get("id")
        if not nid:
            err(f"第 {i + 1} 个节点缺少 id")
            continue
        if nid in by_id:
            err(f"节点 id 重复 {nid!r}")
        by_id[nid] = n
        name = n.get("name")
        if not name:
            err(f"节点 {nid}：缺少 name")
        elif name in names:
            err(f"节点名重复 {name!r}")
        else:
            names.add(name)
        kind = n.get("kind", "table")
        if kind not in NODE_KINDS:
            err(f"节点 {name or nid}：kind={kind!r} 越界（只支持 table / view）")
            continue
        if kind == "view":
            sql = (n.get("view") or {}).get("sql") if isinstance(n.get("view"), dict) else None
            if not sql or not str(sql).strip():
                err(f"视图 {name}：缺少 view.sql")
            fields_of[nid] = set()
        else:
            fields_of[nid] = check_table(n, dialect)
            tbl = n.get("table") or {}
            idx_fields_of[nid] = {
                c
                for idx in (tbl.get("indexes") or [])
                if isinstance(idx, dict)
                for c in (idx.get("fields") or [])
                if isinstance(c, str)
            }
            # 索引名跨表唯一：MySQL 是表内唯一，PG / SQLite 是**全库**唯一 ——
            # 后者索引建在表外，重名直接 relation already exists 建表失败。按最严口径判重。
            for idx in (tbl.get("indexes") or []):
                if not isinstance(idx, dict):
                    continue
                iname = idx.get("name")
                if not iname:
                    continue
                owner = idx_name_owner.get(iname.lower())
                if owner is None:
                    idx_name_owner[iname.lower()] = name or nid
                else:
                    err(
                        f"索引名 {iname!r} 被表 {owner} 和表 {name or nid} 共用 —— "
                        f"PostgreSQL / SQLite 要求索引名全库唯一，重名会建表失败，请加表名前缀"
                    )

    rels = doc.get("relations") or []
    if not isinstance(rels, list):
        err("relations 必须是数组")
        return
    for i, r in enumerate(rels):
        where = f"第 {i + 1} 条关联"
        if not isinstance(r, dict):
            err(f"{where}：不是对象")
            continue
        if not r.get("id"):
            warn(f"{where}：缺少 id")
        for side in ("from", "to"):
            end = r.get(side)
            if not isinstance(end, dict):
                err(f"{where}：{side} 必须是对象 {{node, field}}")
                continue
            node_id = end.get("node")
            field = end.get("field")
            node = by_id.get(node_id)
            if node is None:
                warn(f"{where}：{side} 指向的节点 {node_id!r} 不在本文件里（跨库外键，只提示）")
                continue
            if node.get("kind", "table") != "table":
                err(f"{where}：{side} 指向了视图 {node.get('name')!r}（视图不能有外键）")
                continue
            if field not in fields_of.get(node_id, set()):
                err(f"{where}：{side}.field={field!r} 不是表 {node.get('name')!r} 的字段")
        frm = (r.get("from") or {}).get("node")
        to = (r.get("to") or {}).get("node")
        if frm and to and frm == to:
            err(f"{where}：自关联暂不支持（from 与 to 指向同一张表）")
        kind = r.get("kind", "1-n")
        if kind not in REL_KINDS:
            err(f"{where}：kind={kind!r} 越界（允许 {'/'.join(sorted(REL_KINDS))}）")
        for act in ("onDelete", "onUpdate"):
            v = r.get(act)
            if v and str(v).upper() not in FK_ACTIONS:
                err(f"{where}：{act}={v!r} 越界（允许 {'/'.join(sorted(FK_ACTIONS))}）")

        # 外键字段必须有索引（error 级，与应用侧 validate_naming 同口径）
        # n-n 中间表例外：联合主键自带索引
        kind = r.get("kind", "1-n")
        if kind == "n-n":
            continue
        frm_node = by_id.get(frm)
        if frm_node is None or frm_node.get("kind", "table") != "table":
            continue
        fk_field = (r.get("from") or {}).get("field")
        tbl = frm_node.get("table") or {}
        pk_names = {f.get("name") for f in (tbl.get("fields") or []) if isinstance(f, dict) and f.get("pk")}
        if not fk_field or fk_field in pk_names:
            continue  # 主键本身不需要额外索引
        if fk_field in idx_fields_of.get(frm, set()):
            continue
        tname = frm_node.get("name") or frm
        err(
            f"表 {tname}：外键字段 {fk_field!r} 没有索引 —— "
            f"关联查它时全表扫。补一条 "
            f'{{"name": "idx_{tname}_{fk_field}", "kind": "index", "fields": ["{fk_field}"]}}'
        )


def main():
    # Windows 控制台默认 GBK：中文能打出来，但 ✓/✗ 这类符号会抛 UnicodeEncodeError
    # 把输出整成 UTF-8 且遇到编不了的字符就替换，别让脚本在最后一步崩掉。
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:
            pass
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    path = sys.argv[1]
    try:
        # utf-8-sig：容忍 Windows 编辑器 / 脚本写出的 BOM（裸 utf-8 遇到 BOM 会直接解析失败）
        with open(path, "r", encoding="utf-8-sig") as fh:
            doc = json.load(fh)
    except FileNotFoundError:
        print(f"[错误] 找不到文件：{path}")
        return 2
    except UnicodeDecodeError:
        print(f"[错误] {path} 不是 UTF-8 编码，请另存为 UTF-8")
        return 2
    except json.JSONDecodeError as e:
        print(f"[错误] JSON 解析失败：{e}")
        return 2

    check(doc)

    tables = sum(1 for n in (doc.get("nodes") or []) if isinstance(n, dict) and n.get("kind", "table") == "table")
    views = sum(1 for n in (doc.get("nodes") or []) if isinstance(n, dict) and n.get("kind") == "view")
    rels = len(doc.get("relations") or [])

    print(f"检查 {path}")
    print(f"  表 {tables} · 视图 {views} · 关联 {rels}")
    for w in warnings:
        print(f"  [警告] {w}")
    for e in errors:
        print(f"  [错误] {e}")
    if errors:
        print(f"\n[FAIL] {len(errors)} 个错误必须修完再交付")
        return 1
    if warnings:
        print(f"\n[OK] 可以用（{len(warnings)} 个警告，交付时跟用户说明）")
        return 0
    print("\n[OK] 全部通过")
    return 0


if __name__ == "__main__":
    sys.exit(main())
