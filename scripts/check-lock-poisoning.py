"""CI 闸：禁止生产 Rust 代码出现裸 `.lock().unwrap()` / `.read().unwrap()` /
`.write().unwrap()`（std::sync 锁中毒会连锁崩溃，AUDIT-043）。

允许：
- 测试代码（#[cfg(test)] / tests.rs / tests/ 目录）
- 已使用 unwrap_or_else(|e| e.into_inner()) 的写法
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
PATTERN = re.compile(r"\.(lock|read|write)\(\)\s*\.unwrap\(\)")
ALLOW_TEST = re.compile(r"#\[cfg\(test\)\]")

bad = []
for base in ("src-tauri/src", "crates/cert-deployment/src"):
    for p in (ROOT / base).rglob("*.rs"):
        rel = p.relative_to(ROOT).as_posix()
        if "/tests/" in rel or rel.endswith("tests.rs") or "/tests.rs" in rel:
            continue
        text = p.read_text(encoding="utf-8", errors="replace")
        # 粗粒度：含 cfg(test) 模块的文件只对模块起点之前的部分检查
        check_text = text
        m = ALLOW_TEST.search(text)
        if m:
            check_text = text[: m.start()]
        for match in PATTERN.finditer(check_text):
            line = check_text.count("\n", 0, match.start()) + 1
            bad.append((rel, line, match.group(0).replace("\n", " ")))

if bad:
    print("FAIL: 发现 %d 处裸锁 unwrap：" % len(bad), file=sys.stderr)
    for rel, line, snippet in bad[:30]:
        print("  %s:%d  %s" % (rel, line, snippet), file=sys.stderr)
    print("修复：改用 .lock().unwrap_or_else(|e| e.into_inner())", file=sys.stderr)
    sys.exit(1)
print("PASS: 无裸锁 unwrap")
