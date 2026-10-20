#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""回滚 CodeBuddy CN 会话 junction 实验（2026-10-20）。

背景：把测试账号 13111114601（uid 5b9effcf）的四类会话目录改成 junction 后，
切号合并把它"物化"成了真实目录（`replace_dir_atomic` 的 rename 替换）。
本脚本把该账号恢复原状，并清理实验残留。

做的事（只碰 uid 5b9effcf，绝不动当前账号 1eacae31 的目录）：
  1. 检查 CodeBuddy CN 是否已退出（在跑就拒绝执行）
  2. 备份会话库 codebuddy-sessions.vscdb
  3. 删除物化出来的 history / check-point / file-tree / plan-task
  4. 把 *.orig-test-20261020 改回原名
  5. 删除实验目录 D:\\any-versions\\buddy\\junction-test
  6. 把会话库里 userId=5b9effcf 的记录改回 1eacae31（这些记录本来就是主账号的）
  7. 校验并写日志

用法：
    python rollback-junction-test.py             # 真正执行
    python rollback-junction-test.py --dry-run   # 只打印计划，不改任何文件
    python rollback-junction-test.py --no-db     # 不改会话库 userId

执行完请回面板切回主账号 15235398496（切号流程会再做一次 remap 兜底）。
"""

import argparse
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import time

try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

UID_B = "5b9effcf-2ea0-4447-8f05-ad7164c9f34e"          # 被测账号 13111114601
UID_MAIN = "1eacae31-92db-4f15-8c46-19edd7ca5276"       # 主账号 15235398496（只读，不碰它的目录）
ORIG_SUFFIX = ".orig-test-20261020"
KINDS = ["history", "check-point", "file-tree", "plan-task"]
TEST_ROOT = r"D:\any-versions\buddy\junction-test"
EXPECTED_HISTORY_SESSIONS = 365  # 回滚前实测：B 原数据的 history 会话数

ACC = os.path.join(
    os.environ["LOCALAPPDATA"], "CodeBuddyExtension", "Data",
    UID_B, "CodeBuddyIDE", UID_B,
)
CN_DB = os.path.join(os.environ["APPDATA"], "CodeBuddy CN", "codebuddy-sessions.vscdb")
LOG_PATH = os.path.join(
    r"D:\any-versions\buddy",
    "rollback-junction-test-%s.log" % time.strftime("%Y%m%d-%H%M%S"),
)

_lines = []


def log(msg=""):
    print(msg)
    _lines.append(str(msg))


def count_sessions(root):
    total = 0
    if not os.path.isdir(root):
        return 0
    for name in os.listdir(root):
        ws = os.path.join(root, name)
        if not os.path.isdir(ws) or os.path.islink(ws):
            continue
        for item in os.listdir(ws):
            path = os.path.join(ws, item)
            if item != "index.json" and os.path.isdir(path) and not os.path.islink(path):
                total += 1
    return total


def client_running():
    try:
        out = subprocess.run(
            ["tasklist"], capture_output=True, text=True, encoding="mbcs", errors="replace"
        ).stdout
    except Exception:
        return []
    hits = []
    for line in out.splitlines():
        low = line.lower()
        if "codebuddy" in low:
            hits.append(line.strip())
    return hits


FILE_ATTRIBUTE_REPARSE_POINT = 0x400


def is_reparse(path):
    """junction / symlink / mount point 都算。不要只信 os.path.islink ——
    Windows 上 junction 的 islink 在不同 Python 版本下结果不一致，必须看属性位。"""
    try:
        st = os.lstat(path)
    except OSError:
        return False
    if os.path.islink(path):
        return True
    return bool(getattr(st, "st_file_attributes", 0) & FILE_ATTRIBUTE_REPARSE_POINT)


def force_rmtree(path):
    """删除目录树；遇到 junction/symlink 只删链接本身，绝不下钻删除目标。
    不用 shutil.rmtree：它在遇到 reparse point 时直接抛
    "Cannot call rmtree on a symbolic link"。"""
    if is_reparse(path):
        try:
            os.rmdir(path)
        except OSError:
            os.unlink(path)
        return
    with os.scandir(path) as entries:
        for entry in entries:
            if entry.is_dir(follow_symlinks=False):
                if is_reparse(entry.path):
                    try:
                        os.rmdir(entry.path)
                    except OSError:
                        os.unlink(entry.path)
                else:
                    force_rmtree(entry.path)
            else:
                os.unlink(entry.path)
    os.rmdir(path)


def remove_path(path, dry):
    """删除目录或 junction 链接（链接只删链接本身）。"""
    if not os.path.lexists(path):
        return False
    if is_reparse(path):
        log("    删除链接: %s" % path)
        if not dry:
            try:
                os.rmdir(path)
            except OSError:
                os.unlink(path)
        return True
    log("    删除目录: %s" % path)
    if not dry:
        force_rmtree(path)
    return True


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--no-db", action="store_true")
    args = parser.parse_args()
    dry = args.dry_run

    log("=== 回滚 CodeBuddy CN junction 实验 ===")
    log("模式: %s" % ("DRY-RUN（不改任何东西）" if dry else "真实执行"))
    log("账号目录: %s" % ACC)

    if not os.path.isdir(ACC):
        log("!! 账号目录不存在，脚本中止：%s" % ACC)
        return 2

    if not dry:
        running = client_running()
        if running:
            log("!! 检测到 CodeBuddy 相关进程，请先完全退出（含托盘）：")
            for line in running:
                log("   " + line)
            return 3
        log("客户端已退出 ✓")

    # 1) 备份会话库
    if os.path.exists(CN_DB) and not dry:
        backup = CN_DB + ".rollback-" + time.strftime("%Y%m%d-%H%M%S") + ".bak"
        shutil.copy2(CN_DB, backup)
        log("\n[1] 会话库已备份: %s" % backup)
    else:
        log("\n[1] %s 会话库备份" % ("将备份" if not dry else "跳过"))

    # 2) 目录回滚
    log("\n[2] 回滚账号目录")
    for kind in KINDS:
        acc = os.path.join(ACC, kind)
        orig = os.path.join(ACC, kind + ORIG_SUFFIX)
        log("  - %s" % kind)
        if not os.path.lexists(orig):
            if os.path.isdir(acc) and count_sessions(acc) > 0:
                log("    已回滚（%s 有数据且无备份目录），跳过" % kind)
            else:
                log("    !! 既无 %s 也无 %s，跳过" % (kind, kind + ORIG_SUFFIX))
            continue
        orig_sessions = count_sessions(orig)
        if kind == "history" and orig_sessions < 300:
            log("    !! 备份目录会话数异常（%d），拒绝还原以免覆盖成空目录" % orig_sessions)
            log("    !! 请把日志发给我，不要继续操作")
            return 5
        if os.path.lexists(acc):
            remove_path(acc, dry)
        log("    还原: %s -> %s（%d 条会话）" % (kind + ORIG_SUFFIX, kind, orig_sessions))
        if not dry:
            os.rename(orig, acc)

    # 3) 删除实验目录
    log("\n[3] 清理实验目录 %s" % TEST_ROOT)
    if os.path.lexists(TEST_ROOT):
        remove_path(TEST_ROOT, dry)
    else:
        log("    不存在，跳过")

    # 4) 修正会话库 userId
    log("\n[4] 修正会话库 userId")
    if args.no_db:
        log("    --no-db：跳过（切号时会自动 remap）")
    elif not os.path.exists(CN_DB):
        log("    !! 找不到会话库，跳过: %s" % CN_DB)
    else:
        conn = sqlite3.connect(CN_DB)
        try:
            rows = conn.execute(
                "SELECT key, value FROM ItemTable WHERE key LIKE 'session:%'"
            ).fetchall()
            updates = []
            for key, value in rows:
                text = value.decode("utf-8", "replace") if isinstance(value, bytes) else value
                if not text:
                    continue
                try:
                    data = json.loads(text)
                except Exception:
                    continue
                if data.get("userId") == UID_B:
                    data["userId"] = UID_MAIN
                    updates.append((json.dumps(data, ensure_ascii=False), key))
            log("    待改记录: %d 条（%s -> %s）" % (len(updates), UID_B[:8], UID_MAIN[:8]))
            if updates and not dry:
                conn.executemany("UPDATE ItemTable SET value=? WHERE key=?", updates)
                conn.commit()
                log("    已提交 ✓")
        finally:
            conn.close()

    # 5) 校验
    if dry:
        log("\n[5] 校验：DRY-RUN 不执行（真实结果需实际运行后检查）")
        log("\nDRY-RUN 结束：以上是计划，没有改动任何文件。")
        return 0

    log("\n[5] 校验")
    hist = os.path.join(ACC, "history")
    sessions = count_sessions(hist)
    checks = []
    checks.append(("history 不是链接", not os.path.islink(hist)))
    checks.append(("history 会话数 ≈ %d（实际 %d）" % (EXPECTED_HISTORY_SESSIONS, sessions),
                   abs(sessions - EXPECTED_HISTORY_SESSIONS) <= 2))
    leftovers = [k for k in KINDS if os.path.lexists(os.path.join(ACC, k + ORIG_SUFFIX))]
    checks.append(("无 .orig-test 残留", not leftovers))
    checks.append(("实验目录已清", not os.path.lexists(TEST_ROOT)))
    if not args.no_db and os.path.exists(CN_DB):
        conn = sqlite3.connect(CN_DB)
        try:
            n_b = 0
            for (value,) in conn.execute(
                "SELECT value FROM ItemTable WHERE key LIKE 'session:%'"
            ):
                text = value.decode("utf-8", "replace") if isinstance(value, bytes) else value
                try:
                    if json.loads(text or "{}").get("userId") == UID_B:
                        n_b += 1
                except Exception:
                    pass
        finally:
            conn.close()
        checks.append(("会话库无 %s 归属（剩 %d）" % (UID_B[:8], n_b), n_b == 0))

    ok = True
    for label, passed in checks:
        log("    [%s] %s" % ("OK" if passed else "!!", label))
        ok = ok and passed

    log("\n%s" % ("全部校验通过 ✓" if ok else "有校验未通过，请把本日志发给我"))
    log("\n下一步：打开 any-version 面板，切回主账号 15235398496。")
    log("（切号流程会再做一次 userId remap 兜底，然后正常启动客户端。）")
    try:
        with open(LOG_PATH, "w", encoding="utf-8") as fh:
            fh.write("\n".join(_lines))
        log("日志: %s" % LOG_PATH)
    except OSError as exc:
        log("日志写入失败（不影响结果）: %s" % exc)
    return 0 if ok else 4


if __name__ == "__main__":
    sys.exit(main())
