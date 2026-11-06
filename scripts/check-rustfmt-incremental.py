"""增量 rustfmt 闸门：存量未格式化文件走白名单，任何新增的未格式化文件都阻断。

用法：
    python scripts/check-rustfmt-incremental.py              # CI / 本地检查
    python scripts/check-rustfmt-incremental.py --update-baseline  # 维护白名单

设计约束：
- 仓库当前有 165 个存量 Rust 文件未通过 `cargo fmt --check`。一次性全库格式化
  会产生几百个文件的无关 diff，并污染审计整改的改动面。
- 因此改为「基线差集」：只要 `cargo fmt --check` 报出的文件集合不比基线多，
  就认为通过；任何新文件或被改动后不再符合格式的文件会立刻让 CI 失败。
- 白名单只允许减少：若某文件已被格式化修复，用 --update-baseline 收缩基线。
"""
import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASELINE = ROOT / "scripts" / "rustfmt-baseline.txt"
MANIFESTS = [
    ROOT / "src-tauri" / "Cargo.toml",
    ROOT / "crates" / "cert-deployment" / "Cargo.toml",
]

# "Diff in \\?\E:\path\to\file.rs:34:" / "Diff in /path/to/file.rs:34:"
DIFF_RE = re.compile(r"^Diff in (.+?\.(?:rs)):\d+:", re.M)


def relative(path):
    raw = path.strip()
    if raw.startswith("\\\\?\\"):
        raw = raw[4:]
    try:
        return Path(raw).resolve().relative_to(ROOT).as_posix()
    except ValueError:
        return Path(raw).as_posix()


def collect_unformatted():
    found = set()
    for manifest in MANIFESTS:
        if not manifest.exists():
            continue
        proc = subprocess.run(
            ["cargo", "fmt", "--manifest-path", str(manifest), "--", "--check"],
            cwd=str(ROOT), capture_output=True, text=True, encoding="utf-8", errors="replace",
        )
        output = (proc.stdout or "") + (proc.stderr or "")
        for match in DIFF_RE.finditer(output):
            found.add(relative(match.group(1)))
        # 非零退出但没有 Diff 行 = rustfmt 本身报错（语法错误 / 组件缺失），必须失败
        if proc.returncode != 0 and not DIFF_RE.search(output):
            print("rustfmt 执行失败（%s）：\n%s" % (manifest.name, output[:2000]), file=sys.stderr)
            return None
    return found


def read_baseline():
    if not BASELINE.exists():
        return set()
    return {
        line.strip() for line in BASELINE.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.startswith("#")
    }


def write_baseline(files):
    lines = [
        "# 存量未格式化 Rust 文件白名单（由 check-rustfmt-incremental.py --update-baseline 生成）",
        "# 只允许收缩：修复格式后请重新生成，不要把新文件加进来。",
    ] + sorted(files)
    BASELINE.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--update-baseline", action="store_true", help="重新生成存量白名单")
    args = parser.parse_args()

    current = collect_unformatted()
    if current is None:
        return 2

    if args.update_baseline:
        write_baseline(current)
        print("baseline updated: %d files -> %s" % (len(current), BASELINE.relative_to(ROOT)))
        return 0

    baseline = read_baseline()
    new_bad = current - baseline
    fixed = baseline - current
    print("rustfmt: 存量白名单 %d 个，当前未格式化 %d 个" % (len(baseline), len(current)))
    if fixed:
        print("  已修复（可收缩基线）: %d 个" % len(fixed))
    if new_bad:
        print("  FAIL: 以下文件未通过 rustfmt，且不在存量白名单中：", file=sys.stderr)
        for path in sorted(new_bad):
            print("    " + path, file=sys.stderr)
        print("  修复方式：cargo fmt --manifest-path <Cargo.toml>", file=sys.stderr)
        return 1
    print("  PASS (incremental)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
