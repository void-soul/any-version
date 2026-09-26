#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Validate a .mindmap.json file against the any-version mindmap import format v1.

Usage:
    python scripts/validate.py <file.json> [--strict]

Exit code: 0 = OK (warnings allowed), 1 = errors found (or warnings with --strict),
           2 = bad usage / unreadable file.

Checks mirror the Rust importer (mm_import_nodes / json_to_mindmap_nodes /
normalize_imported_nodes), so whatever passes here imports cleanly.
"""
import argparse
import json
import re
import sys
from collections import Counter

KINDS = {
    "root", "module", "component", "service", "route", "config",
    "file", "task", "requirement", "constraint", "risk", "other",
}
LAYOUTS = {"lr", "rl", "tb", "bt"}
COLOR_RE = re.compile(r"^#[0-9a-fA-F]{6}$")

MAX_PER_LEVEL = 12
MAX_DEPTH = 5
MAX_NAME = 40


def validate(data):
    errors, warnings = [], []

    if not isinstance(data, dict):
        return ["顶层必须是 JSON 对象"], []

    nodes = data.get("nodes")
    if nodes is None:
        return ["缺少 nodes 数组（导入会直接失败）"], []
    if not isinstance(nodes, list):
        return ["nodes 必须是数组"], []
    if not nodes:
        return ["nodes 是空的（导入会直接失败）"], []

    ids, parents = [], []
    for i, n in enumerate(nodes):
        tag = "nodes[%d]" % i
        if not isinstance(n, dict):
            errors.append("%s 不是对象" % tag)
            continue

        nid = n.get("id")
        if nid is not None:
            if not isinstance(nid, str) or not nid.strip():
                errors.append("%s 的 id 必须是非空字符串" % tag)
            else:
                ids.append(nid)

        name = n.get("name")
        if name is None or not str(name).strip():
            warnings.append("%s 缺少 name（导入后会显示「未命名」）" % tag)
        elif len(str(name)) > MAX_NAME:
            warnings.append("%s 的 name 有 %d 字（建议 <= %d，长文本请放进 detail）"
                            % (tag, len(str(name)), MAX_NAME))

        kind = n.get("kind")
        if kind is not None and kind not in KINDS:
            warnings.append("%s 的 kind=%r 不在白名单，导入后会变成 other" % (tag, kind))

        color = n.get("color")
        if color is not None and not COLOR_RE.match(str(color)):
            warnings.append("%s 的 color=%r 不是 #RRGGBB，导入后会用默认色" % (tag, color))

        p = n.get("parent_id", n.get("parentId"))
        if p is not None and str(p).strip() not in ("", "null"):
            parents.append((tag, str(p)))

        if not n.get("detail") and not n.get("description"):
            warnings.append("%s 没有 detail/description（节点会缺少说明）" % tag)

    dup = [k for k, v in Counter(ids).items() if v > 1]
    for d in dup:
        errors.append("id=%r 重复出现 %d 次（父子引用会指向错误的节点）" % (d, Counter(ids)[d]))

    known = set(ids)
    for tag, p in parents:
        if p not in known:
            warnings.append("%s 的 parent_id=%r 在文件里不存在 → 该节点会被当成一条新的主线"
                            % (tag, p))

    layout = data.get("layoutDir", data.get("layout_dir"))
    if layout is not None and layout not in LAYOUTS:
        warnings.append("layoutDir=%r 非法（可选值 %s），导入时会忽略" % (layout, "/".join(sorted(LAYOUTS))))

    if "name" not in data and "title" not in data:
        warnings.append("顶层没有 name/title，导入后文档名会取文件名")

    # 结构统计：每层子节点数 + 深度
    children = {}
    for n in nodes:
        p = n.get("parent_id", n.get("parentId"))
        p = None if p is None or str(p) in ("", "null") else str(p)
        children.setdefault(p, []).append(n)
    for parent, kids in children.items():
        if len(kids) > MAX_PER_LEVEL:
            warnings.append("父级 %s 下有 %d 个子节点（建议 <= %d）" % (parent or "(根)", len(kids), MAX_PER_LEVEL))

    def depth(node, seen=None):
        seen = seen or set()
        nid = node.get("id")
        if nid in seen:
            return MAX_DEPTH + 1
        seen = seen | ({nid} if nid else set())
        p = node.get("parent_id", node.get("parentId"))
        p = None if p is None or str(p) in ("", "null") else str(p)
        parent = None
        if p is not None:
            parent = next((x for x in nodes if x.get("id") == p), None)
        return 1 if parent is None else 1 + depth(parent, seen)

    deepest = max((depth(n) for n in nodes if isinstance(n, dict)), default=1)
    if deepest > MAX_DEPTH:
        warnings.append("最深 %d 层（建议 <= %d，再深在画布上读不了）" % (deepest, MAX_DEPTH))

    if len(nodes) > 150:
        warnings.append("共 %d 个节点，建议拆成多份导图" % len(nodes))

    return errors, warnings


def main():
    ap = argparse.ArgumentParser(description="Validate a mindmap import JSON file")
    ap.add_argument("file")
    ap.add_argument("--strict", action="store_true", help="treat warnings as failures")
    args = ap.parse_args()

    try:
        with open(args.file, "r", encoding="utf-8") as f:
            raw = f.read()
    except OSError as e:
        print("读取失败: %s" % e)
        return 2

    text = raw.strip()
    if text.startswith("```"):
        text = text.strip("`").strip()
        if text.lower().startswith("json"):
            text = text[4:].strip()
    try:
        data = json.loads(text)
    except json.JSONDecodeError as e:
        print("[ERROR] 不是合法 JSON: %s" % e)
        return 1

    errors, warnings = validate(data)

    nodes = data.get("nodes") if isinstance(data, dict) else None
    print("文件: %s" % args.file)
    print("节点数: %s" % (len(nodes) if isinstance(nodes, list) else "?"))
    for e in errors:
        print("[ERROR] %s" % e)
    for w in warnings:
        print("[WARN ] %s" % w)
    if not errors and not warnings:
        print("OK: 格式检查全部通过")

    if errors:
        return 1
    if warnings and args.strict:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
