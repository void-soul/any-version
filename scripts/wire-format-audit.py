"""Cross-language field-name audit: Rust serde wire format vs TypeScript interfaces.

Read-only analysis. Two passes:
  pass 1  pair by type name (Rust struct <-> TS interface)
  pass 2  pair by command (Tauri command return type <-> TS invoke<T> generic)
"""
import collections
import os
import re

ROOT = r"e:\pro\my\any-version"
RS = os.path.join(ROOT, "src-tauri", "src")
TS = os.path.join(ROOT, "src")


def camel(n):
    p = n.split("_")
    return p[0] + "".join(x.capitalize() for x in p[1:])


def load_rust():
    rust = {}
    for dp, _, fs in os.walk(RS):
        for fn in fs:
            if not fn.endswith(".rs"):
                continue
            path = os.path.join(dp, fn)
            lines = open(path, encoding="utf-8").read().splitlines()
            i = 0
            while i < len(lines):
                m = re.match(r"\s*(?:pub\s+)?struct\s+([A-Za-z0-9_]+)\s*\{", lines[i])
                if not m:
                    i += 1
                    continue
                name = m.group(1)
                rename_all = None
                for back in range(1, 13):
                    j = i - back
                    if j < 0:
                        break
                    s = lines[j].strip()
                    ra = re.search(r'rename_all\s*=\s*"([^"]+)"', s)
                    if ra:
                        rename_all = ra.group(1)
                        break
                    if re.match(r"(pub\s+)?struct\s", s):
                        break
                fields = []
                k = i + 1
                while k < len(lines):
                    s = lines[k].strip()
                    if s.startswith("}"):
                        break
                    at = {}
                    while s.startswith(("#[", "//", "///")):
                        if s.startswith("#["):
                            rm = re.search(r'rename\s*=\s*"([^"]+)"', s)
                            if rm:
                                at["rename"] = rm.group(1)
                            if re.search(r"serde\(\s*skip[\s,\)]", s):
                                at["skip"] = True
                        k += 1
                        s = lines[k].strip() if k < len(lines) else ""
                    fm = re.match(r"(?:pub\s+)?([a-z0-9_]+)\s*:\s*(.+)", s)
                    if fm and not at.get("skip"):
                        o = fm.group(1)
                        w = at.get("rename") or (camel(o) if rename_all == "camelCase" else o)
                        fields.append((o, w))
                    k += 1
                rust[name] = {"fields": fields, "rename_all": rename_all,
                              "file": os.path.relpath(path, ROOT)}
                i = k
    return rust


def load_ts():
    ts = {}
    for dp, _, fs in os.walk(TS):
        if "node_modules" in dp:
            continue
        for fn in fs:
            if not (fn.endswith(".ts") or fn.endswith(".tsx")):
                continue
            path = os.path.join(dp, fn)
            lines = open(path, encoding="utf-8").read().splitlines()
            i = 0
            while i < len(lines):
                m = re.search(r"\binterface\s+([A-Za-z0-9_]+)\s*(?:extends\s+[^{]+)?\{", lines[i])
                if not m:
                    i += 1
                    continue
                name = m.group(1)
                depth = lines[i].count("{") - lines[i].count("}")
                fields = []
                k = i + 1
                while k < len(lines) and depth > 0:
                    s = lines[k].strip()
                    # 先用「当前」深度判定：`rectifier: {` 这行本身属顶层字段，
                    # 先增深度再判的话，所有带嵌套对象的字段都会被漏掉
                    at_top = depth == 1
                    depth += s.count("{") - s.count("}")
                    if depth <= 0:
                        break
                    if at_top and not s.startswith(("//", "*", "/*")):
                        fm = re.match(r"([A-Za-z0-9_]+)\s*\??\s*:", s)
                        if fm:
                            fields.append(fm.group(1))
                    k += 1
                ts[name] = {"fields": fields, "file": os.path.relpath(path, ROOT)}
                i = k
    return ts


def unwrap(t):
    # 签名行常带函数体开头的 "{"，先剥掉；否则尾部正则（,\s*String\s*>$）永远匹配不上
    t = t.strip()
    t = re.sub(r"\s*\{+\s*$", "", t).strip()
    t = re.sub(r"^Result<\s*", "", t)
    t = re.sub(r",\s*String\s*>$", "", t)
    t = re.sub(r"^Vec<\s*", "", t)
    t = re.sub(r"^Option<\s*", "", t)
    t = re.sub(r"^HashMap<\s*[^,]+,\s*", "", t)
    t = t.rstrip(">").strip()
    return t.split("<")[0].strip()


def load_cmds():
    cmds = {}
    for dp, _, fs in os.walk(RS):
        for fn in fs:
            if not fn.endswith(".rs"):
                continue
            path = os.path.join(dp, fn)
            lines = open(path, encoding="utf-8").read().splitlines()
            for idx, l in enumerate(lines):
                if "#[tauri::command]" not in l:
                    continue
                for fwd in range(1, 4):
                    j = idx + fwd
                    if j >= len(lines):
                        break
                    m = re.search(r"pub\s+(?:async\s+)?fn\s+([a-z0-9_]+)\s*\(", lines[j])
                    if not m:
                        continue
                    # 签名常跨多行（参数分行写）：往前扫到含 "->" 的那一行，
                    # 遇到函数体开始的 "{" 就停（那种是返回单元类型，没有结构性返回值）
                    ret = None
                    for scan in range(j, min(j + 20, len(lines))):
                        seg = lines[scan]
                        if "->" in seg:
                            ret = unwrap(seg.split("->", 1)[1])
                            break
                        if seg.rstrip().endswith("{"):
                            break
                    cmds[m.group(1)] = {"ret": ret, "file": os.path.relpath(path, ROOT)}
                    break
    return cmds


def load_invokes():
    inv = {}
    for dp, _, fs in os.walk(TS):
        if "node_modules" in dp:
            continue
        for fn in fs:
            if not (fn.endswith(".ts") or fn.endswith(".tsx")):
                continue
            path = os.path.join(dp, fn)
            txt = open(path, encoding="utf-8").read()
            # 泛型可能是 `X` / `X[]` / `X | null` / `X[] | undefined` —— 取第一个标识符
            for m in re.finditer(
                r'invoke\s*<\s*([A-Za-z0-9_]+)\s*(?:\[\s*\])?\s*'
                r'(?:\|\s*(?:null|undefined)\s*)?\s*>\s*\(\s*"([a-z0-9_]+)"',
                txt,
            ):
                inv.setdefault(m.group(2), []).append(
                    (m.group(1), os.path.relpath(path, ROOT))
                )
    return inv


def verdict(pairs, tf):
    wire = [w for (_, w) in pairs]
    exact = [w for w in wire if w in tf]
    other = [o for (o, w) in pairs if o in tf and o not in exact]
    missing = [w for w in wire if w not in tf]
    if exact and other:
        return "MIXED", exact, other, len(wire), missing
    if exact:
        return "OK", exact, other, len(wire), missing
    if other:
        return "MISMATCH", exact, other, len(wire), missing
    return "NO-OVERLAP", exact, other, len(wire), missing


def coverage(missing, n):
    if n == 0:
        return 1.0
    return (n - len(missing)) / n


def norm(s):
    return s.replace("_", "").lower()


def main():
    rust = load_rust()
    ts = load_ts()
    cmds = load_cmds()
    inv = load_invokes()

    print("Rust structs: %d   TS interfaces: %d   commands: %d   invoke sites: %d"
          % (len(rust), len(ts), len(cmds), len(inv)))
    print()

    print("===== pass 1: match by type name =====")
    by_norm = {}
    for n, v in ts.items():
        by_norm.setdefault(norm(n), []).append((n, v))
    rows1 = []
    for rn, rv in rust.items():
        for tn, tv in by_norm.get(norm(rn), []):
            v, e, o, n, missing = verdict(rv["fields"], set(tv["fields"]))
            rows1.append((rn, tn, v, rv["rename_all"], rv["file"], tv["file"], e, o, n,
                          missing))
    c1 = collections.Counter(r[2] for r in rows1)
    print("pairs checked: %d   %s" % (len(rows1), dict(c1)))
    for r in sorted(rows1, key=lambda x: ({"MISMATCH": 0, "MIXED": 1,
                                           "NO-OVERLAP": 2, "OK": 3}[x[2]], x[0])):
        if r[2] == "OK":
            continue
        print("  [%s] Rust %s (%s) <-> TS %s" % (r[2], r[0], r[3] or "none", r[1]))
        print("        %s | %s" % (r[4], r[5]))
        print("        Rust wire=%s  TS-only=%s (%d fields)" % (r[6][:5] or "-",
                                                                r[7][:5] or "-", r[8]))
    print()

    print("===== pass 2: match by command return type <-> invoke generic =====")
    print("  (OK 只表示「至少一个字段对得上」；重命名字段若前端用了完全不同的名字则漏检，")
    print("   所以下面额外列出覆盖率偏低的 OK 项，供人工复核)")
    # 诊断：先看两边各自能解析出多少
    ret_struct = [c["ret"] for c in cmds.values() if c["ret"] and c["ret"] in rust]
    inv_struct = [(k, g) for k, v in inv.items() for (g, _) in v if g in ts]
    both = [k for k in set(c["ret"] for c in cmds.values() if c["ret"]) & set(k for k in inv)]
    print("  commands with resolvable struct return: %d" % len(ret_struct))
    print("  invokes with resolvable TS generic    : %d" % len(inv_struct))
    print("  command names present on both sides   : %d" % len(both))
    rows2 = []
    for cmd, c in cmds.items():
        ret = c["ret"]
        if not ret or ret not in rust or cmd not in inv:
            continue
        for (g, tf_) in inv[cmd]:
            if g not in ts:
                continue
            res = verdict(rust[ret]["fields"], set(ts[g]["fields"]))
            v, e, o, n, missing = res
            rows2.append((cmd, ret, g, v, rust[ret]["rename_all"], c["file"], tf_,
                          e, o, n, missing))
    c2 = collections.Counter(r[3] for r in rows2)
    print("pairs checked: %d   %s" % (len(rows2), dict(c2)))
    for r in sorted(rows2, key=lambda x: ({"MISMATCH": 0, "MIXED": 1,
                                           "NO-OVERLAP": 2, "OK": 3}[x[3]], x[0])):
        if r[3] == "OK":
            continue
        print("  [%s] cmd=%s   Rust %s (%s) -> TS %s"
              % (r[3], r[0], r[1], r[4] or "none", r[2]))
        print("        %s | %s" % (r[5], r[6]))
        print("        Rust wire=%s  TS-only=%s (%d fields)"
              % (r[7][:5] or "-", r[8][:5] or "-", r[9]))
    print()
    lows = [r for r in rows2 if r[3] == "OK" and coverage(r[10], r[9]) < 0.6]
    lows.sort(key=lambda r: coverage(r[10], r[9]))
    print("  -- 覆盖率 < 60%% 的 OK 项（人工复核）: %d" % len(lows))
    for r in lows[:25]:
        print("     %-28s Rust %s -> TS %s  cover=%.0f%%  missing=%s"
              % (r[0], r[1], r[2], coverage(r[10], r[9]) * 100,
                 r[10][:6] or "-"))


if __name__ == "__main__":
    main()
