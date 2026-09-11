#!/usr/bin/env bash
# 列出 cockpit-tools 自上次 Buddy 同步点以来、影响 Buddy 功能域的改动。
# 用法：bash .agents/skills/cockpit-buddy-sync/scripts/relearn.sh
set -euo pipefail

SKILL_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PIN="$(sed -n '1s/.*\[\([0-9a-f]*\)].*/\1/p' "$SKILL_DIR/sync-point.txt")"
REF="${COCKPIT_TOOLS_DIR:-E:/pro/other-sdk/cockpit-tools}"

# 只监听 Tauri 应用实际编译的 src-tauri/src 路径。
# 注意：crates/cockpit-core 下的 workbuddy_account/oauth/instance 等同名文件是 cockpit-cli 用的副本，
# src-tauri 并不编译它，改动这些副本不代表 Buddy 行为变化，勿加入监听。
PATHS=(
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

cd "$REF"
HEAD="$(git rev-parse --short=8 HEAD)"
echo "== cockpit-tools 同步点 $PIN → 当前 $HEAD =="
if [ "$PIN" = "$HEAD" ]; then
  echo "（参考仓未变化；若用户仍报功能异常，走 references/pitfalls.md 检查单）"
  exit 0
fi
echo "\n== 相关 commit =="
git log --oneline "$PIN..HEAD" -- "${PATHS[@]}"
echo "\n== 文件级差异 =="
git diff --stat "$PIN..HEAD" -- "${PATHS[@]}"
echo "\n下一步：对上面出现变化的文件，按 SKILL.md §2 找到我们的对应实现，对照 references/contracts.md 精读 diff。"
