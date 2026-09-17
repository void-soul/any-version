#!/usr/bin/env bash
# 列出参考仓自上次 Buddy 同步点以来、影响 Buddy 功能域的改动。
#
# 用法：
#   bash .agents/skills/cockpit-buddy-sync/scripts/relearn.sh              # 两个参考仓（默认）
#   bash .agents/skills/cockpit-buddy-sync/scripts/relearn.sh cockpit      # 只跑参考 A：cockpit-tools
#   bash .agents/skills/cockpit-buddy-sync/scripts/relearn.sh workdaddy    # 只跑参考 B：WorkDaddy
#
# 路径解析：本机 bash 可能是 WSL（Linux）也可能是 Git Bash，故对 Windows 风格路径自动尝试
# 三种写法（E:/x、/e/x、/mnt/e/x）；也可用 COCKPIT_TOOLS_DIR / WORKDADDY_DIR 直接覆盖。
set -euo pipefail

SKILL_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WHICH="${1:-all}"

# 把 Windows 风格路径展开成三种候选写法
path_candidates() {
  local win="$1" drive rest
  drive="$(printf '%s' "$win" | cut -c1 | tr 'A-Z' 'a-z')"
  rest="$(printf '%s' "$win" | cut -c3-)"
  printf '%s\n' "$win" "/$drive$rest" "/mnt/$drive$rest"
}

# $1 = 环境变量覆盖值（可为空）；$2 = Windows 风格默认路径
find_ref() {
  local override="$1" win="$2" cand
  if [ -n "$override" ]; then
    if [ -d "$override" ]; then printf '%s' "$override"; return 0; fi
    echo "指定的目录不存在: $override" >&2
    return 1
  fi
  while IFS= read -r cand; do
    if [ -d "$cand" ]; then printf '%s' "$cand"; return 0; fi
  done < <(path_candidates "$win")
  return 1
}

# ─── 参考 A：cockpit-tools（Rust/Tauri，同架构，可直接移植） ───

relearn_cockpit() {
  local ref pin head
  if ! ref="$(find_ref "${COCKPIT_TOOLS_DIR:-}" 'E:/pro/other-sdk/buddy/cockpit-tools')"; then
    echo "== [A] cockpit-tools：未找到参考仓 =="
    echo "已尝试 E:/pro/other-sdk/buddy/cockpit-tools（含 /e/、/mnt/e 变体）；可用 COCKPIT_TOOLS_DIR=<路径> 覆盖。"
    return 0
  fi
  pin="$(sed -n '1s/.*\[\([0-9a-f]*\)\].*/\1/p' "$SKILL_DIR/sync-point.txt")"

  # 只监听 Tauri 应用实际编译的 src-tauri/src 路径。
  # 注意：crates/cockpit-core 下的 workbuddy_account/oauth/instance 等同名文件是 cockpit-cli 用的副本，
  # src-tauri 并不编译它，改动这些副本不代表 Buddy 行为变化，勿加入监听。
  local paths=(
    # 账号库 / 互导 / 当前账号态 / 注入
    src-tauri/src/modules/provider_current_state.rs
    src-tauri/src/modules/vscode_inject.rs
    src-tauri/src/modules/workbuddy_account.rs
    src-tauri/src/modules/codebuddy_cn_account.rs
    src-tauri/src/modules/workbuddy_auto_checkin.rs
    # 会话：列表 / 合并
    src-tauri/src/modules/codebuddy_session.rs
    src-tauri/src/modules/codebuddy_session_list.rs
    src-tauri/src/modules/codebuddy_session_transfer.rs
    src-tauri/src/modules/workbuddy_session_transfer.rs
    # OAuth / 用量 / 签到（模块 + 命令层）
    src-tauri/src/modules/workbuddy_oauth.rs
    src-tauri/src/modules/codebuddy_cn_oauth.rs
    src-tauri/src/commands/workbuddy.rs
    src-tauri/src/commands/codebuddy_cn.rs
    # 实例：关闭/启动/注入时序（切换流程）
    src-tauri/src/modules/workbuddy_instance.rs
    src-tauri/src/modules/codebuddy_cn_instance.rs
    src-tauri/src/commands/workbuddy_instance.rs
    src-tauri/src/commands/codebuddy_cn_instance.rs
  )

  cd "$ref"
  head="$(git rev-parse --short=8 HEAD)"
  echo "== [A] cockpit-tools（$ref）同步点 $pin → 当前 $head =="
  if [ "$pin" = "$head" ]; then
    echo "（参考仓未变化；若用户仍报功能异常，走 references/pitfalls.md 检查单）"
    return 0
  fi
  echo
  echo "== 相关 commit =="
  git log --oneline "$pin..HEAD" -- "${paths[@]}"
  echo
  echo "== 文件级差异 =="
  git diff --stat "$pin..HEAD" -- "${paths[@]}"
  echo
  echo "下一步：对上面出现变化的文件，按 SKILL.md §2.1 找到我们的对应实现，对照 references/contracts.md 精读 diff。"
}

# ─── 参考 B：WorkDaddy（Node.js + CDP 注入，异架构，只借语义） ───

relearn_workdaddy() {
  local ref pin head
  if ! ref="$(find_ref "${WORKDADDY_DIR:-}" 'E:/pro/other-sdk/buddy/WorkDaddy')"; then
    echo "== [B] WorkDaddy：未找到参考仓 =="
    echo "已尝试 E:/pro/other-sdk/buddy/WorkDaddy（含 /e/、/mnt/e 变体）；可用 WORKDADDY_DIR=<路径> 覆盖。"
    return 0
  fi
  pin="$(sed -n '1s/.*\[\([0-9a-f]*\)\].*/\1/p' "$SKILL_DIR/sync-point.workdaddy.txt")"

  # 监听真源（scripts/）+ 行为规范（test/）+ 任务包 schema + 设计文档。
  # 打包/发布/平台流程文件（build-*、install-*、win-launcher.js、watchdog.js、macos-*）与 Buddy 功能域无关，不监听。
  local paths=(
    'scripts/*.js'
    scripts/builtin
    schemas
    docs
    test
  )

  cd "$ref"
  head="$(git rev-parse --short=8 HEAD)"
  echo "== [B] WorkDaddy（$ref）同步点 $pin → 当前 $head =="
  grep -n "const DAEMON_VERSION\|const DAEMON_BUILD_ID" scripts/daemon.js | head -2
  if [ "$pin" = "$head" ]; then
    echo "（参考仓未变化；若用户仍报功能异常，走 references/pitfalls.md F/G 节，并跑 references/workdaddy.md §Z 的漂移校验锚点）"
    return 0
  fi
  echo
  echo "== 相关 commit =="
  git log --oneline "$pin..HEAD" -- "${paths[@]}"
  echo
  echo "== 文件级差异 =="
  git diff --stat "$pin..HEAD" -- "${paths[@]}"
  echo
  echo "下一步：按 SKILL.md §2.2 找到我们的对应实现；WorkDaddy 属异架构，先做 §3 步 3 的"
  echo "可移植性三分类（纯逻辑 / 官方接口 / 注入依赖），再对照 references/workdaddy.md 精读 diff。"
}

case "$WHICH" in
  cockpit|a|A) relearn_cockpit ;;
  workdaddy|b|B) relearn_workdaddy ;;
  all) relearn_cockpit; echo; echo "────────────────────────────────────────────"; echo; relearn_workdaddy ;;
  *) echo "用法: $0 [cockpit|workdaddy|all]" >&2; exit 2 ;;
esac
