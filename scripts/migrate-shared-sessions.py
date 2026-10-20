#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""把 CodeBuddy CN 的多账号会话副本迁移成「一份共享目录 + 每个账号 junction」。

背景（2026-10-20 实测）：
  Data/<uid>/CodeBuddyIDE/<uid>/{history,check-point,file-tree,plan-task}/<hash>/<id>/
  会话合并会把同一条会话复制进每个切过的账号，本机 23 个账号 ≈ 85 GB，独有会话仅 99 条。
  客户端能正常在 junction 上读写（Data 本身就在 symlink 上），卡点是我们自己的合并代码
  会把链接物化 —— 已在 `session_transfer/codebuddy.rs` 加了共享模式短路。

做的事：
  1. 选当前账号的 6 类目录，整体 rename 到 <store>/shared-sessions/codebuddy-cn/（同盘瞬间）
  2. 其他账号：把「共享目录里没有的」会话复制过去并合并 index.json
  3. 删除其他账号的原目录，建 junction 指向共享目录
  4. 校验：每个账号的 junction 可读、会话数、释放空间

用法：
    python migrate-shared-sessions.py               # dry-run（默认，只统计）
    python migrate-shared-sessions.py --apply       # 真正执行
    python migrate-shared-sessions.py --apply --keep-backup   # 不删原目录，改名为 .migrated-bak

前置条件：
  - CodeBuddy CN 必须完全退出
  - 先跑过 rollback-junction-test.py（不能有 *.orig-test-* 残留）
  - any-version 已更新到含「共享模式短路」的版本
"""

import argparse
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import time
import unicodedata

try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

STORE_DEFAULT = r"D:\any-versions\buddy"
ORIG_MARK = ".orig-test-"
KINDS = ["history", "check-point", "file-tree", "plan-task", "genie-cache", "connectors"]
FILE_ATTRIBUTE_REPARSE_POINT = 0x400

_lines = []

GB = 1073741824.0
PROGRESS_INTERVAL = 0.3   # 进度行刷新间隔（秒）
PROGRESS_EVERY = 64       # 每扫这么多个文件才检查一次时间，避免 time() 拖慢扫描


def log(msg=""):
    print(msg, flush=True)
    _lines.append(str(msg))


def fmt_int(n):
    return "{:,}".format(n)


def disp_width(text):
    """中文按 2 列算，用于把进度行覆盖干净。"""
    return sum(2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1 for ch in text)


class Progress:
    """长时间扫描/复制/删除时在同一行刷新进度，避免看起来像卡死。

    只写 stdout，不写日志文件；非终端（重定向到文件）时退化成结束打一行。
    """

    def __init__(self, label):
        self.label = label
        self.files = 0
        self.bytes = 0
        self.started = time.time()
        self.interactive = sys.stdout.isatty()
        self._last = 0.0
        self._since = 0
        self._width = 0
        if self.interactive:
            self._render()

    def bump(self, size):
        self.files += 1
        self.bytes += size
        if not self.interactive:
            return
        self._since += 1
        if self._since < PROGRESS_EVERY:
            return
        self._since = 0
        now = time.time()
        if now - self._last >= PROGRESS_INTERVAL:
            self._last = now
            self._render()

    def _text(self):
        elapsed = time.time() - self.started
        rate = self.files / elapsed if elapsed > 0.2 else 0.0
        return "  %s  %s 个文件  %.2f GB  %.1fs%s" % (
            self.label,
            fmt_int(self.files),
            self.bytes / GB,
            elapsed,
            ("  %s/s" % fmt_int(int(rate))) if rate else "",
        )

    def _render(self):
        text = self._text()
        width = disp_width(text)
        if width < self._width:
            text += " " * (self._width - width)
            width = self._width
        self._width = width
        sys.stdout.write("\r" + text)
        sys.stdout.flush()

    def done(self, extra=""):
        text = self._text() + extra
        if self.interactive:
            width = disp_width(text)
            if width < self._width:
                text += " " * (self._width - width)
            sys.stdout.write("\r" + text + "\n")
            sys.stdout.flush()
            self._width = 0
        else:
            print(text, flush=True)


def is_reparse(path):
    try:
        st = os.lstat(path)
    except OSError:
        return False
    if os.path.islink(path):
        return True
    return bool(getattr(st, "st_file_attributes", 0) & FILE_ATTRIBUTE_REPARSE_POINT)


def dir_size(path, label=""):
    """统计目录大小。

    用 scandir 而不是 walk+getsize：Windows 上 stat 信息来自目录枚举缓存，
    不再每个文件建一次句柄，几十万文件时快数倍；label 非空时在同一行刷进度。
    """
    prog = Progress(label) if label else None
    total = 0
    stack = [path]
    while stack:
        cur = stack.pop()
        try:
            it = os.scandir(cur)
        except OSError:
            continue
        with it:
            for entry in it:
                try:
                    st = entry.stat(follow_symlinks=False)
                except OSError:
                    continue
                if getattr(st, "st_file_attributes", 0) & FILE_ATTRIBUTE_REPARSE_POINT:
                    continue
                if entry.is_dir(follow_symlinks=False):
                    stack.append(entry.path)
                else:
                    total += st.st_size
                    if prog:
                        prog.bump(st.st_size)
    if prog:
        prog.done()
    return total


def client_running():
    try:
        out = subprocess.run(
            ["tasklist"], capture_output=True, text=True, encoding="mbcs", errors="replace"
        ).stdout
    except Exception:
        return []
    return [ln.strip() for ln in out.splitlines() if "codebuddy" in ln.lower()]


def sessions_of(root):
    """{workspace_name: set(session_id)}"""
    result = {}
    if not os.path.isdir(root):
        return result
    for ws in os.listdir(root):
        wsp = os.path.join(root, ws)
        if not os.path.isdir(wsp) or is_reparse(wsp):
            continue
        ids = set()
        for name in os.listdir(wsp):
            p = os.path.join(wsp, name)
            if name != "index.json" and os.path.isdir(p) and not is_reparse(p):
                ids.add(name)
        result[ws] = ids
    return result


def load_json(path, fallback):
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except Exception:
        return fallback


def write_json_atomic(path, data):
    tmp = path + ".kira-tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(data, fh, ensure_ascii=False, indent=2)
    os.replace(tmp, path)


def merge_index(shared_ws, src_ws):
    """把源 workspace 的 index.json 条目并进共享，返回新增条数。"""
    shared_path = os.path.join(shared_ws, "index.json")
    src_path = os.path.join(src_ws, "index.json")
    src = load_json(src_path, None)
    if not isinstance(src, dict):
        return 0
    shared = load_json(shared_path, None)
    if not isinstance(shared, dict):
        shared = {"conversations": [], "current": ""}
    have = {c.get("id") for c in shared.get("conversations") or []}
    added = 0
    for conv in src.get("conversations") or []:
        cid = conv.get("id")
        if cid and cid not in have:
            shared.setdefault("conversations", []).append(conv)
            have.add(cid)
            added += 1
    if added:
        write_json_atomic(shared_path, shared)
    return added


def copy_session(src, dst, prog=None):
    """复制单个会话目录，逐文件推进度（copy2 保留时间戳，与 copytree 行为一致）。"""

    def _copy(s, d):
        shutil.copy2(s, d)
        size = 0
        try:
            size = os.path.getsize(d)
        except OSError:
            pass
        if prog:
            prog.bump(size)

    shutil.copytree(src, dst, copy_function=_copy)


def account_uids(store):
    accounts_dir = os.path.join(store, "buddy_codebuddy_cn_accounts")
    uids = []
    if not os.path.isdir(accounts_dir):
        return uids
    for name in os.listdir(accounts_dir):
        if not name.endswith(".json"):
            continue
        data = load_json(os.path.join(accounts_dir, name), None)
        if isinstance(data, dict) and data.get("uid"):
            uids.append((data["uid"], data.get("email") or ""))
    return uids


def current_uid(store):
    data = load_json(os.path.join(store, "current_accounts.json"), {})
    acc_id = (data.get("current_accounts") or {}).get("codebuddy-cn")
    if not acc_id:
        return None
    path = os.path.join(store, "buddy_codebuddy_cn_accounts", acc_id + ".json")
    acc = load_json(path, None)
    return acc.get("uid") if isinstance(acc, dict) else None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--store", default=STORE_DEFAULT)
    parser.add_argument("--ext", default=os.path.join(
        os.environ["LOCALAPPDATA"], "CodeBuddyExtension", "Data"))
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--keep-backup", action="store_true")
    args = parser.parse_args()
    apply = args.apply
    store = args.store
    ext = args.ext
    default_ext = os.path.join(os.environ["LOCALAPPDATA"], "CodeBuddyExtension", "Data")
    test_mode = os.path.normcase(ext) != os.path.normcase(default_ext)
    shared_root = os.path.join(store, "shared-sessions", "codebuddy-cn")

    t_main = time.time()
    log("=== CodeBuddy CN 会话共享迁移 ===")
    log("模式: %s" % ("APPLY（会删除原副本）" if apply else "DRY-RUN（只统计）"))
    log("数据根: %s" % store)
    log("共享根: %s" % shared_root)

    if not os.path.isdir(ext):
        log("!! 找不到 %s" % ext)
        return 2

    if not apply or test_mode:
        log("\n[0] 跳过进程检查（%s）" % ("dry-run" if not apply else "测试模式"))
    else:
        running = client_running()
        if running:
            log("!! CodeBuddy 仍在运行，请完全退出：")
            for line in running[:5]:
                log("   " + line)
            return 3

    uids = account_uids(store)
    if not uids:
        log("!! 账号库为空: %s" % store)
        return 2
    main_uid = current_uid(store)
    log("账号数: %d，当前账号: %s" % (len(uids), (main_uid or "?")[:8]))

    # 前置检查：不能有未回滚的实验残留
    leftovers = []
    for uid, _email in uids:
        acc = os.path.join(ext, uid, "CodeBuddyIDE", uid)
        if not os.path.isdir(acc):
            continue
        for kind in KINDS:
            if os.path.isdir(os.path.join(acc, kind + ORIG_MARK + "20261020")):
                leftovers.append(uid)
                break
    if leftovers:
        if apply:
            log("!! 检测到未回滚的实验目录（*.orig-test-20261020），请先运行 rollback-junction-test")
            for uid in leftovers:
                log("   " + uid)
            return 4
        log("[警告] 检测到未回滚的实验目录（dry-run 不阻止，真实执行会拒绝）：")
        for uid in leftovers:
            log("   " + uid)

    # 已迁移检测
    already = [uid for uid, _ in uids
               if is_reparse(os.path.join(ext, uid, "CodeBuddyIDE", uid, "history"))]
    if already and not apply:
        log("\n[提示] 已有 %d 个账号是 junction，可能是部分迁移" % len(already))

    # 每个非 junction 账号的会话数
    stats = {}
    for uid, _email in uids:
        root = os.path.join(ext, uid, "CodeBuddyIDE", uid, "history")
        if is_reparse(root):
            continue
        s = sessions_of(root)
        stats[uid] = sum(len(v) for v in s.values())

    # 共享根是否已有数据（断点续跑）
    shared_existing = {}
    if os.path.isdir(shared_root):
        for kind in KINDS:
            shared_existing[kind] = sessions_of(os.path.join(shared_root, kind))
    shared_count = sum(len(v) for m in shared_existing.values() for v in m.values())

    # 底本：共享根已有数据就不再移动任何账号；否则选一个非 junction 的账号（优先当前账号）
    move_uid = None
    if shared_count == 0:
        candidates = {uid: n for uid, n in stats.items() if n > 0}
        if not candidates:
            log("!! 没有可用的底本账号（全是 junction 且共享根为空）")
            return 5
        move_uid = main_uid if main_uid in candidates else max(candidates, key=candidates.get)

    if move_uid:
        log("\n[1] 底本账号: %s（%d 条会话，整体 move 到共享根）"
            % (move_uid[:8], stats.get(move_uid, 0)))
    else:
        log("\n[1] 共享根已有 %d 条会话，不再移动底本" % shared_count)

    # 每个 kind 的"已存在"集合 = 共享根 + 底本账号
    base_sets = {}
    for kind in KINDS:
        ids = set()
        for ws_ids in (shared_existing.get(kind) or {}).values():
            ids |= ws_ids
        if move_uid:
            root = os.path.join(ext, move_uid, "CodeBuddyIDE", move_uid, kind)
            for ws_ids in sessions_of(root).values():
                ids |= ws_ids
        base_sets[kind] = ids

    log("\n开始扫描各账号会话占用（大量小文件，可能需要几分钟；下面每行会实时刷新进度）")
    scan_t0 = time.time()
    plan = []
    if move_uid:
        plan.append(("move", move_uid, "", 0, 0))
    todo = [(i, uid) for i, (uid, _e) in enumerate(uids, 1)
            if uid != move_uid and os.path.isdir(os.path.join(ext, uid, "CodeBuddyIDE", uid))]
    for i, uid in todo:
        acc = os.path.join(ext, uid, "CodeBuddyIDE", uid)
        uniq = 0
        reclaim = 0
        for kind in KINDS:
            src = os.path.join(acc, kind)
            if not os.path.isdir(src) or is_reparse(src):
                continue
            s = sessions_of(src)
            for _ws, ids in s.items():
                uniq += len(ids - base_sets[kind])
            reclaim += dir_size(src, "[%d/%d] %s/%s" % (i, len(uids), uid[:8], kind))
        plan.append(("migrate", uid, "", uniq, reclaim))
        log("  已统计 %d/%d  %s  独有 %d 条，%.1f GB"
            % (i, len(uids), uid[:8], uniq, reclaim / GB))
    log("  扫描耗时 %.1fs" % (time.time() - scan_t0))

    total_reclaim = sum(p[4] for p in plan if p[0] == "migrate")
    total_uniq = sum(p[3] for p in plan if p[0] == "migrate")
    log("\n[2] 迁移计划")
    for item in plan:
        if item[0] == "move":
            log("  底本 %s: 4 类目录整体移动" % item[1][:8])
        else:
            _t, uid, _k, uniq, reclaim = item
            log("  账号 %s: 独有会话 %d，可释放 %.1f GB"
                % (uid[:8], uniq, reclaim / 1073741824.0))
    log("  合计独有: %d 条，可释放 %.1f GB" % (total_uniq, total_reclaim / 1073741824.0))

    if not apply:
        log("\nDRY-RUN 结束：没有改动任何文件。确认无误后加 --apply 执行。")
        log("总耗时 %.1fs" % (time.time() - t_main))
        return 0

    # ── 真正执行 ──
    log("\n[3] 创建共享根")
    os.makedirs(shared_root, exist_ok=True)

    log("\n[4] 移动底本目录到共享根")
    base_root = os.path.join(ext, move_uid, "CodeBuddyIDE", move_uid) if move_uid else None
    for kind in (KINDS if move_uid else []):
        src = os.path.join(base_root, kind)
        dst = os.path.join(shared_root, kind)
        if not os.path.isdir(src) or is_reparse(src):
            continue
        if os.path.isdir(dst) and os.listdir(dst):
            # 共享根已有内容（断点续跑）：逐 workspace 合并
            log("  %s: 共享根已有数据，改为合并" % kind)
            prog = Progress("[合并底本 %s]" % kind)
            for ws, ids in sessions_of(src).items():
                d = os.path.join(dst, ws)
                os.makedirs(d, exist_ok=True)
                for cid in ids:
                    s = os.path.join(src, ws, cid)
                    t = os.path.join(d, cid)
                    if not os.path.exists(t):
                        copy_session(s, t, prog)
                merge_index(d, os.path.join(src, ws))
            prog.done()
            force_rmtree(src, "[删除底本副本 %s]" % kind)
        else:
            if os.path.isdir(dst) and not os.listdir(dst):
                os.rmdir(dst)
            os.rename(src, dst)
            log("  %s -> %s" % (kind, dst))

    log("\n[5] 合并其他账号的独有会话并替换为 junction")
    for uid, _email in uids:
        acc = os.path.join(ext, uid, "CodeBuddyIDE", uid)
        if not os.path.isdir(acc):
            continue
        for kind in KINDS:
            src = os.path.join(acc, kind)
            dst = os.path.join(shared_root, kind)
            if os.path.isdir(src) and is_reparse(src):
                continue
            if not os.path.isdir(src):
                continue
            copied = 0
            label = "%s/%s" % (uid[:8], kind)
            prog = Progress("[复制独有 %s]" % label)
            for ws, ids in sessions_of(src).items():
                d = os.path.join(dst, ws)
                os.makedirs(d, exist_ok=True)
                for cid in ids:
                    s = os.path.join(src, ws, cid)
                    t = os.path.join(d, cid)
                    if not os.path.exists(t):
                        copy_session(s, t, prog)
                        copied += 1
                merge_index(d, os.path.join(src, ws))
            if copied:
                prog.done()
            if args.keep_backup:
                bak = src + ".migrated-bak"
                if os.path.exists(bak):
                    force_rmtree(bak, "[删除旧备份 %s]" % label)
                os.rename(src, bak)
            else:
                force_rmtree(src, "[删除原副本 %s]" % label)
            log("  %s/%s: 补充 %d 条，原目录已移除" % (uid[:8], kind, copied))

    log("\n[6] 建立 junction")
    for uid, _email in uids:
        acc = os.path.join(ext, uid, "CodeBuddyIDE", uid)
        if not os.path.isdir(acc):
            continue
        for kind in KINDS:
            link = os.path.join(acc, kind)
            target = os.path.join(shared_root, kind)
            if is_reparse(link):
                try:
                    same = os.path.realpath(link).lower() == os.path.realpath(target).lower()
                except OSError:
                    same = False
                if same:
                    continue
                # 指向别处（实验遗留 / 旧共享根）：删链接后重建
                log("  重建链接: %s" % os.path.relpath(link, ext))
                force_rmtree(link)
            if os.path.exists(link):
                force_rmtree(link)
            os.makedirs(target, exist_ok=True)
            r = subprocess.run(
                ["cmd", "/c", "mklink", "/J", link, target],
                capture_output=True, text=True, encoding="mbcs", errors="replace",
            )
            if r.returncode != 0:
                log("  !! %s 建链接失败: %s" % (link, (r.stderr or r.stdout).strip()))
    log("  junction 完成")

    # 校验
    log("\n[7] 校验")
    ok = True
    for uid, _email in uids:
        acc = os.path.join(ext, uid, "CodeBuddyIDE", uid)
        if not os.path.isdir(acc):
            continue
        for kind in KINDS:
            link = os.path.join(acc, kind)
            if not is_reparse(link):
                log("  !! %s/%s 不是 junction" % (uid[:8], kind))
                ok = False
    shared_sessions = sum(len(v) for v in sessions_of(os.path.join(shared_root, "history")).values())
    log("  共享 history 会话数: %d" % shared_sessions)
    log("  共享根大小: %.2f GB"
        % (dir_size(shared_root, "[校验] 共享根") / GB))

    log("\n%s" % ("迁移完成 ✓" if ok else "有校验失败，请把日志发给我"))
    log("总耗时 %.1fs" % (time.time() - t_main))
    log("接下来：启动 CodeBuddy CN 确认会话列表正常；再切一次号，日志里应出现")
    log("  [Buddy CN Transfer] ... 跳过复制（只 remap userId=…）")

    log_path = os.path.join(store, "migrate-shared-%s.log" % time.strftime("%Y%m%d-%H%M%S"))
    try:
        with open(log_path, "w", encoding="utf-8") as fh:
            fh.write("\n".join(_lines))
        log("日志: %s" % log_path)
    except OSError as exc:
        log("日志写入失败: %s" % exc)
    return 0 if ok else 5


def force_rmtree(path, label=""):
    prog = Progress(label) if label else None
    _force_rmtree(path, prog)
    if prog:
        prog.done()


def _force_rmtree(path, prog):
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
                    _force_rmtree(entry.path, prog)
            else:
                size = 0
                try:
                    size = entry.stat(follow_symlinks=False).st_size
                except OSError:
                    pass
                os.unlink(entry.path)
                if prog:
                    prog.bump(size)
    os.rmdir(path)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("\n已中断（Ctrl+C）。尚未执行的操作不会发生；若已建出共享根，重跑会自动改为合并续做。",
              flush=True)
        sys.exit(130)
