#!/usr/bin/env bash
# 列出参考仓自上次 Buddy 同步点以来、影响 Buddy 功能域的改动。
#
# 用法：
#   bash .agents/skills/cockpit-buddy-sync/scripts/relearn.sh              # 三个参考仓（默认）
#   bash .agents/skills/cockpit-buddy-sync/scripts/relearn.sh cockpit      # 只跑参考 A：cockpit-tools
#   bash .agents/skills/cockpit-buddy-sync/scripts/relearn.sh workdaddy    # 只跑参考 B：WorkDaddy
#   bash .agents/skills/cockpit-buddy-sync/scripts/relearn.sh 2api         # 只跑参考 C：workbuddy2api
#
# 路径解析：本机 bash 可能是 WSL（Linux）也可能是 Git Bash，故对 Windows 风格路径自动尝试
# 三种写法（E:/x、/e/x、/mnt/e/x）；也可用 COCKPIT_TOOLS_DIR / WORKDADDY_DIR /
# WORKBUDDY2API_DIR 直接覆盖。
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
  echo "== [A] cockpit-tools（辅参考，$ref）同步点 $pin → 当前 $head =="
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
  echo "下一步：对上面出现变化的文件，按 SKILL.md §2.2 找到我们的对应实现，对照 references/contracts.md 精读 diff。"
  echo "（辅参考只在主参考 WorkDaddy 未覆盖该域时才需要看；同域冲突以 WorkDaddy 为准，切换时序与合并模型除外。）"
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
  # WorkDaddy 1.2.76 起新增 Linux 平台件（*-linux*）与打包/发布件（build-*、install-*、win-launcher.js、
  # watchdog.js、platform.js、windows-native/、assets/）——这些与 Buddy 功能域无关，直接从监听范围排除，
  # 否则每次版本发布会刷出一大堆噪音 commit。
  local paths=(
    'scripts/*.js'
    'scripts/*.cmd'
    scripts/builtin
    schemas
    docs
    test
  )
  local excludes=(
    ':!scripts/*linux*'
    ':!scripts/*Linux*'
    ':!scripts/platform.js'
    ':!scripts/win-launcher.js'
    ':!scripts/watchdog.js'
    ':!scripts/build-*'
    ':!scripts/install-*'
    ':!scripts/uninstall-*'
    ':!scripts/Start-*'
    ':!scripts/Stop-*'
    ':!scripts/win/'
    ':!scripts/windows-native/'
    ':!scripts/assets/'
    ':!scripts/workbuddy-buddy-mark.svg'
    ':!docs/images/'
  )

  cd "$ref"
  head="$(git rev-parse --short=8 HEAD)"
  echo "== [B] WorkDaddy（主参考，$ref）同步点 $pin → 当前 $head =="
  grep -n "const DAEMON_VERSION\|const DAEMON_BUILD_ID" scripts/daemon.js | head -2
  if [ "$pin" = "$head" ]; then
    echo "（参考仓未变化；若用户仍报功能异常，走 references/pitfalls.md F/G 节，并跑 references/workdaddy.md §Z 的漂移校验锚点）"
    return 0
  fi
  echo
  echo "== 相关 commit =="
  git log --oneline "$pin..HEAD" -- "${paths[@]}" "${excludes[@]}"
  echo
  echo "== 文件级差异 =="
  git diff --stat "$pin..HEAD" -- "${paths[@]}" "${excludes[@]}"
  echo
  echo "下一步：按 SKILL.md §2.1 找到我们的对应实现；WorkDaddy 属异架构，先做 §3 步 3 的"
  echo "可移植性三分类（纯逻辑 / 官方接口 / 注入依赖），再对照 references/workdaddy.md 精读 diff。"
  echo "注意 §0.1 硬规则：WorkDaddy 只作用于 WorkBuddy 路径，勿据此改 CodeBuddy CN 行为。"
}

# ─── 参考 C：workbuddy2api（我们自己的 fork，跟进上游协议变化） ───

relearn_2api() {
  local ref pin head
  if ! ref="$(find_ref "${WORKBUDDY2API_DIR:-}" 'E:/pro/other-sdk/buddy/workbuddy2api')"; then
    echo "== [C] workbuddy2api：未找到 fork =="
    echo "已尝试 E:/pro/other-sdk/buddy/workbuddy2api（含 /e/、/mnt/e 变体）；可用 WORKBUDDY2API_DIR=<路径> 覆盖。"
    return 0
  fi
  pin="$(sed -n '1s/.*\[\([0-9a-f]*\)\].*/\1/p' "$SKILL_DIR/sync-point.workbuddy2api.txt")"

  # 只监听协议真源：转换器 / 凭据 / at-rest 加密。
  # 排除 .deps（vendored 依赖）、日志、测试夹具 —— 它们随依赖升级刷噪音。
  local paths=(
    'core/*.py'
    'README.md'
  )
  local excludes=(
    ':!.deps/*'
    ':!*.log'
  )

  cd "$ref"
  head="$(git rev-parse --short=8 HEAD)"
  # 官方是 origin/upstream（fork 才是我们的），见 references/workbuddy2api.md §C.0
  if ! git rev-parse --verify --quiet origin/main >/dev/null; then
    git fetch origin --quiet 2>/dev/null || true
  fi
  local official official_head
  official="$(git rev-parse --short=8 origin/main 2>/dev/null || echo '?')"
  official_head="$(git rev-parse --short=8 HEAD)"
  echo "== [C] workbuddy2api（参考 C，$ref）=="
  echo "   同步点 $pin → 本地 HEAD $head；官方 origin/main = $official"
  echo "   remote: $(git remote -v | awk '{print $1}' | sort -u | tr '\n' ' ')"
  echo
  # 只在真的有新增提交时才展示 diff：两点式 `git diff HEAD..origin/main` 在无新增时
  # 仍会打印**我们自己改动的反向差异**，会被误读成"官方改了这些"。
  local new_commits
  new_commits="$(git log --oneline HEAD..origin/main -- "${paths[@]}" "${excludes[@]}" 2>/dev/null)"
  echo "== 官方新增、我们还没有的提交（要跟进的） =="
  if [ -z "$new_commits" ]; then
    echo "  （无）"
  else
    echo "$new_commits"
    echo
    echo "== 上述提交的文件级差异 =="
    git diff --stat HEAD..origin/main -- "${paths[@]}" "${excludes[@]}" 2>/dev/null || true
  fi
  echo
  echo "== 我们领先官方的提交（推 fork 前看这个） =="
  git log --oneline origin/main..HEAD -- "${paths[@]}" "${excludes[@]}" 2>/dev/null || true
  echo
  if [ "$pin" = "$head" ] && [ -z "$(git log --oneline HEAD..origin/main 2>/dev/null)" ]; then
    echo "（无新增可跟；若用户报 2API 异常，走 references/pitfalls.md I 节检查单 ——"
    echo "  该域最常见的原因是上游契约变了，而非本地代码漂移。）"
    return 0
  fi
  echo "下一步：读 core/converter.py 等的 diff → 对照 references/workbuddy2api.md §C.1"
  echo "Python↔Rust 对照表 → 判断 commands/buddy/twoapi/ 是否要跟 → 跑 §5 验证协议。"
  echo "注意 §C.2：上游只收流式、鉴权头是 WorkBuddy 特有、模型目录不在 /v2/models。"
}

case "$WHICH" in
  cockpit|a|A) relearn_cockpit ;;
  workdaddy|b|B) relearn_workdaddy ;;
  2api|c|C) relearn_2api ;;
  all)
    relearn_cockpit; echo; echo "────────────────────────────────────────────"; echo
    relearn_workdaddy; echo; echo "────────────────────────────────────────────"; echo
    relearn_2api
    ;;
  *) echo "用法: $0 [cockpit|workdaddy|2api|all]" >&2; exit 2 ;;
esac
