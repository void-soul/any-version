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

errors = []
warnings = []


def err(msg):
    errors.append(msg)


def warn(msg):
    warnings.append(msg)


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


def check_table(node):
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
        if isinstance(d, str) and d.strip():
            # 字符串字面量忘了加引号是高频错误
            v = d.strip()
            looks_literal = (
                v.startswith(("'", '"', "("))
                or re.match(r"^-?\d", v)
                or v.upper() in ("NULL", "TRUE", "FALSE", "CURRENT_TIMESTAMP", "NOW()")
            )
            if not looks_literal:
                warn(f"表 {name} 字段 {fname}：default={v!r} 看起来不像 SQL 字面量（字符串要自己加引号）")

    if len(pk_cols) > 1 and auto_cols:
        err(f"表 {name}：复合主键里不能有自增列（{', '.join(pk_cols)}），导出的 SQL 跑不起来")
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
            fields_of[nid] = check_table(n)

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
