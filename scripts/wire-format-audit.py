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
                # 命令自己的 rename_all 决定**入参**命名：默认 camelCase，
                # 标了 rename_all = "snake_case" 的必须按 snake_case 传
                attr = l
                ra = re.search(r'rename_all\s*=\s*"([^"]+)"', attr)
                arg_case = ra.group(1) if ra else "camelCase"
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
                    params = []
                    depth = 0
                    started = False
                    for scan in range(j, min(j + 30, len(lines))):
                        seg = lines[scan]
                        # 收集参数名（跳过 tauri 注入的 State / AppHandle / Window，它们不是入参）
                        for pm in re.finditer(r"([a-z0-9_]+)\s*:\s*([^,]+)", seg):
                            pname, ptype = pm.group(1), pm.group(2).strip()
                            if ptype.startswith(("tauri::State", "tauri::AppHandle",
                                                "tauri::Window", "AppHandle", "State<")):
                                continue
                            if pname in ("state", "app", "window", "handle"):
                                continue
                            params.append((pname, ptype))
                        depth += seg.count("(") - seg.count(")")
                        if "(" in seg:
                            started = True
                        if "->" in seg:
                            ret = unwrap(seg.split("->", 1)[1])
                            break
                        if started and depth <= 0:
                            break
                        if seg.rstrip().endswith("{"):
                            break
                    cmds[m.group(1)] = {
                        "ret": ret,
                        "params": params,
                        "arg_case": arg_case,
                        "file": os.path.relpath(path, ROOT),
                    }
                    break
    return cmds


def parse_object_keys(txt, start, nested=None):
    """从 start（指向 '{'）开始，取出该对象字面量**第一层**的键名。

    `nested` 传入 dict 时，同时收集每个第一层键下面的第二层键名 —— pass 4 用它检查
    「结构体入参里的字段名」对不对（这类错不会被 tsc 发现：内联对象字面量直接传给
    invoke 时不做结构检查，而 Rust 侧反序列化失败会让命令**根本不执行**）。
    """
    keys = []
    depth = 0
    buf = ""
    quote = None
    cur = None
    i = start
    while i < len(txt):
        c = txt[i]
        if quote:
            if c == quote:
                quote = None
            i += 1
            continue
        if c in "\"'`":
            quote = c
            i += 1
            continue
        if c == "{":
            depth += 1
            if depth == 1:
                buf = ""
                i += 1
                continue
            if depth == 2 and nested is not None and cur:
                sub = parse_object_keys(txt, i)
                nested.setdefault(cur, []).extend(sub)
                # 跳过已解析的子对象
                d2 = 0
                q2 = None
                while i < len(txt):
                    ch = txt[i]
                    if q2:
                        if ch == q2:
                            q2 = None
                    elif ch in "\"'`":
                        q2 = ch
                    elif ch == "{":
                        d2 += 1
                    elif ch == "}":
                        d2 -= 1
                        if d2 == 0:
                            break
                    i += 1
                buf = ""
                cur = None
                i += 1
                continue
        elif c == "}":
            depth -= 1
            if depth <= 0:
                # 收尾时也要收一次：`{ platform }` 这种简写没有冒号和逗号，
                # 不收尾就会整个丢掉（表现为「TS sends = []」）
                k = buf.strip().strip("\"'")
                if k and re.match(r"^[A-Za-z0-9_]+$", k):
                    keys.append(k)
                break
        if depth == 1:
            if c == ":":
                k = buf.strip().strip("\"'")
                if re.match(r"^[A-Za-z0-9_]+$", k):
                    keys.append(k)
                    if nested is not None:
                        cur = k
                buf = ""
                i += 1
                continue
            if c == ",":
                k = buf.strip().strip("\"'")
                # 简写形式 { toolId } 没有冒号
                if k and re.match(r"^[A-Za-z0-9_]+$", k):
                    keys.append(k)
                buf = ""
                cur = None
                i += 1
                continue
            if not c.isspace():
                buf += c
        i += 1
    return keys


def load_invoke_args():
    """TS 侧 invoke("cmd", { ... }) 的第一层入参键名。"""
    out = {}
    for dp, _, fs in os.walk(TS):
        if "node_modules" in dp:
            continue
        for fn in fs:
            if not (fn.endswith(".ts") or fn.endswith(".tsx")):
                continue
            path = os.path.join(dp, fn)
            txt = open(path, encoding="utf-8").read()
            for m in re.finditer(
                r'invoke\s*(?:<[^>]*>)?\s*\(\s*"([a-z0-9_]+)"\s*,\s*\{', txt
            ):
                nested = {}
                keys = parse_object_keys(txt, m.end() - 1, nested)
                out.setdefault(m.group(1), []).append(
                    (keys, nested, os.path.relpath(path, ROOT))
                )
    return out


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
    print()

    print("===== pass 4: struct-typed argument payloads =====")
    print("  顶层参数名对了不代表结构体里的字段对了：内联对象字面量直接传给 invoke 时")
    print("  tsc **不做**结构检查，Rust 侧反序列化失败则命令根本不执行（被 .catch 吞掉）。")
    print("  save_last_launch_config 就是这个坑：顶层 toolId/config 都对，config 里的")
    print("  provider_id 等 snake_case 字段导致整条命令静默失败。")
    rows4 = []
    invargs = load_invoke_args()
    for cmd, c in cmds.items():
        if cmd not in invargs:
            continue
        for (pname, ptype) in c.get("params", []):
            base = unwrap(ptype)
            if base not in rust:
                continue
            s = rust[base]
            wire = [w for (_o, w) in s["fields"]]
            srcs = [o for (o, _w) in s["fields"]]
            want_case = c.get("arg_case") or "camelCase"
            if s["rename_all"] == "camelCase" or (s["rename_all"] is None
                                                  and want_case == "camelCase"):
                good_names, bad_names = wire, srcs
            else:
                good_names, bad_names = srcs, wire
            for (keys, nested, tsf) in invargs[cmd]:
                sub = nested.get(pname if want_case != "camelCase" else camel(pname), [])
                if not sub:
                    continue
                ks = set(sub)
                good = [k for k in ks if k in good_names]
                bad = [k for k in ks if k in bad_names and k not in good_names]
                if bad and not good:
                    v = "MISMATCH"
                elif bad:
                    v = "MIXED"
                elif good:
                    v = "OK"
                else:
                    v = "NO-OVERLAP"
                rows4.append((cmd, pname, base, v, s["rename_all"], c["file"], tsf,
                              good[:5], bad[:5], len(wire)))
    c4 = collections.Counter(r[3] for r in rows4)
    print("struct-typed payloads checked: %d   %s" % (len(rows4), dict(c4)))
    # 只有「内联对象字面量」能查到；用变量传的（config: lc）键名看不见 ——
    # 但那种若变量有类型标注，tsc 会替我们校验，所以剩下的风险面是「无类型标注的对象」。
    print("  注：仅覆盖内联字面量；变量传入的由 tsc 按类型校验，故不重复检查。")
    for r in sorted(rows4, key=lambda x: ({"MISMATCH": 0, "MIXED": 1,
                                           "NO-OVERLAP": 2, "OK": 3}[x[3]], x[0])):
        print("      %-9s cmd=%-26s 参数 %s : %s" % (r[3], r[0], r[1], r[2]))
        if r[3] == "OK":
            continue
        print("  [%s] cmd=%s  参数 %s : Rust %s (%s)" % (r[3], r[0], r[1], r[2],
                                                        r[4] or "none"))
        print("        %s | %s" % (r[5], r[6]))
        print("        期望=%s  实际错风格=%s (%d fields)" % (r[7] or "-", r[8] or "-", r[9]))
    print()

    print("===== pass 3: TS -> Rust command arguments =====")
    print("  入参命名由命令自己的 rename_all 决定：默认 camelCase，")
    print("  #[tauri::command(rename_all = \"snake_case\")] 的必须按 snake_case 传。")
    print("  传错不会报错 —— 参数反序列化失败后命令直接不执行（常被 .catch 吞掉），")
    print("  表现为「点了没反应 / 配置存不进去」，属于最难查的一类。")
    invargs = load_invoke_args()
    rows3 = []
    for cmd, c in cmds.items():
        if cmd not in invargs:
            continue
        want_case = c.get("arg_case") or "camelCase"
        pnames = [p for (p, _t) in c.get("params", [])]
        if not pnames:
            continue
        expected = {camel(p) if want_case == "camelCase" else p: p for p in pnames}
        for (keys, nested, tsf) in invargs[cmd]:
            ks = set(keys)
            good = [k for k in ks if k in expected]
            wrong_style = []
            for p in pnames:
                other = p if want_case == "camelCase" else camel(p)
                if other in ks and (camel(p) if want_case == "camelCase" else p) not in ks:
                    wrong_style.append((p, other))
            extra = [k for k in ks if k not in expected]
            if wrong_style:
                v = "MISMATCH"
            elif not good and pnames:
                v = "NO-OVERLAP"
            else:
                v = "OK"
            rows3.append((cmd, v, want_case, pnames, keys, wrong_style, extra,
                          c["file"], tsf))
    c3 = collections.Counter(r[1] for r in rows3)
    print("commands invoked with an arg object: %d   verdicts: %s"
          % (len(rows3), dict(c3)))
    order3 = {"MISMATCH": 0, "NO-OVERLAP": 1, "OK": 2}
    for r in sorted(rows3, key=lambda x: (order3[x[1]], x[0])):
        if r[1] == "OK":
            continue
        print("  [%s] cmd=%s (expect %s)" % (r[1], r[0], r[2]))
        print("        %s | %s" % (r[7], r[8]))
        print("        Rust params=%s" % (r[3], ))
        print("        TS sends  =%s" % (r[4], ))
        if r[5]:
            print("        风格错配  =%s" % (r[5], ))
        if r[6]:
            print("        多余键    =%s" % (r[6], ))

    lows = [r for r in rows2 if r[3] == "OK" and coverage(r[10], r[9]) < 0.6]
    lows.sort(key=lambda r: coverage(r[10], r[9]))
    print("  -- 覆盖率 < 60%% 的 OK 项（人工复核）: %d" % len(lows))
    for r in lows[:25]:
        print("     %-28s Rust %s -> TS %s  cover=%.0f%%  missing=%s"
              % (r[0], r[1], r[2], coverage(r[10], r[9]) * 100,
                 r[10][:6] or "-"))


if __name__ == "__main__":
    main()
