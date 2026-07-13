# 协作功能（CollabRoom）改造任务记录

> 最后更新：2026-07-13
> 目标：把协作派发从「一次性 cmd /c 批处理」升级为「流式 + 会话绑定 + 事件推送」，去掉前端 1.5s 轮询。

## 已完成（P0→P1→P2）

### 后端
- `src-tauri/src/commands/ai/collab.rs`
  - `collab_send_message` 增加 `app: tauri::AppHandle` 参数。
  - `dispatch_to_tool` 重写为**流式派发**：spawn 子进程后逐行读 stdout；`runner=="stream-json"` 时解析 `content_block_delta.delta.text` → `emit("collab:delta")`；`result` → 最终文本 + 捕获 `session_id`。
  - 新增 Tauri 事件：`collab:delta`（增量文本）、`collab:msg-updated`（收尾 done/error）。
  - **会话绑定**：`tool_sessions["{room}::{tool}"]` 存工具原生 session id；有 id → `dispatch_resume_cmd`（claude `--resume {session_id}`），不再开新会话；无 id → `dispatch_cmd`，首轮结束写回真实 id。
  - 取消/超时：`DISPATCH_STATE`（OnceLock+Mutex<HashMap<msg_id, DispatchCtrl>>），`collab_cancel_dispatch` 置 cancel + `taskkill /T` 杀树；超 1800s 自动终止。
  - 新增 `collab_reset_session(room_id, tool_id)` 清续聊绑定。
- `src-tauri/src/commands/ai_registry.rs`：`ToolConfig` 新增 `dispatch_resume_cmd`、`runner` 字段。
- `src-tauri/src/lib.rs`：注册 `collab_cancel_dispatch`、`collab_reset_session`。

### 配置
- `ai-tools/claude-code/config.json`：`dispatchCmd` → `claude -p --output-format stream-json --verbose --input-file {prompt_file}`；新增 `dispatchResumeCmd`（带 `{session_id}`）、`"runner": "stream-json"`。

### 前端
- `src/components/ai/types.ts`：新增 `CollabDeltaPayload`、`CollabMsgUpdatedPayload`。
- `src/components/ai/CollabRoom.tsx`：
  - 删除 `poll()`，改为 `listen("collab:delta" / "collab:msg-updated")` 增量渲染。
  - 发送中按钮变「⏹ 停止」（调 `collab_cancel_dispatch`）；新增「⟳ 重置会话」（调 `collab_reset_session`）。
  - `busy` 由首个非 running 的 `msg-updated` 事件解除；`running` 且有内容时实时显示流式文本。
  - `send` 加 `busy` 防护防二次派发。

## 未完成任务

### [编译] cargo check 通过
- 状态：进行中。已修复 `reader` 缺 `mut` 错误（2026-07-13）。
- 待办：在本机跑 `cargo check`，消除所有编译错误/警告（警告：`DispatchCtrl.room_id` 已删，注意无其他 unused）。

### [P0 实测] claude stream-json 字段名确认（关键阻塞点）
- 需在有 claude 的环境实测：
  ```
  echo "hi" > t.txt && claude -p --output-format stream-json --verbose --input-file t.txt
  ```
- 确认：① 增量文本是否在 `content_block_delta.delta.text`；② 最终 `session_id` 出现在哪个事件（现 `parse_stream_json` 取任意事件的 `session_id` 字段兜底）。
- 若有差异 → 改 `parse_stream_json`（collab.rs）。

### [P3] opencode / codex 的更优集成 ✅ 已完成（需真机验证）
后端已支持三种 runner（`dispatch_to_tool` 按 `runner` 字段分发解析器）：
- `stream-json`（claude）：`content_block_delta.delta.text` → delta；`result` → 收尾 + 捕获 session_id。配置 `--input-file {prompt_file}`。
- `codex-json`（codex）：`codex exec --json`。会话 id 取 `thread.started.thread_id`；助手文本取 `item.completed(item_type=assistant_message).text`。`turn.failed` → 错误收尾。续聊：`codex exec --json --full-auto resume {session_id} "{prompt}"`。`promptMode: "arg"`（提示词内联 `{prompt}`，已转义 `"` 与 `%`）。
- `opencode-json`（opencode）：`opencode run --format json`。容错解析：从 `content`/`text`/`message.content`(字符串或数组)/`delta` 提取文本；`session_id` 取 `session_id`/`session.id`/`id`；`done|result|completed|turn.completed|session.completed` → 收尾。续聊：`opencode run --format json -s {session_id} "{prompt}"`。`promptMode: "arg"`。

`ToolConfig` 新增 `prompt_mode`（`file`/`stdin`/`arg`），后端支持：
- `file` → 模板用 `{prompt_file}`（已加引号路径），如 claude。
- `stdin` → 子进程 stdin 喂临时文件，模板无占位（codex 若验证支持可读 stdin 可切此模式，免转义/长度限制）。
- `arg` → 模板用 `{prompt}` 内联，经 `escape_cmd_arg` 转义。

⚠️ 真机验证点：
1. **codex**：`codex exec --json` 事件字段名（`thread_id`/`item.completed` 结构）是否与解析一致；`resume {session_id}` 是否接受后续提示词参数。
2. **opencode**：`--format json` 的真实事件结构（目前为容错解析，可能需按实际 schema 调整 `parse_opencode_json`）。
3. 两工具 `arg` 模式下，超长提示词（含大文件附件）可能触及 Windows `cmd` 命令行长度上限（~8191 字符）。如需更稳可改 `promptMode: "stdin"`（codex 已确认支持 stdin；opencode 待确认）。

### [验证] 端到端流式 + 续聊真机验证
- 编译通过后，在本机用真实 claude 跑一次：首轮新建会话 → 看 `session_id` 是否写回 `collab.json`；第二轮是否自动 `--resume` 续聊。
- 验证停止按钮（cancel + taskkill 杀树）在 Windows 下确实终止子进程。

## 风险 / 待定
1. claude 各版本 stream-json 字段名/事件结构可能不同 → P0 实测。
2. Windows 杀进程树依赖 `taskkill /F /T /PID`，若工具自身又 spawn 子进程可能残留 → 考虑 job object。
3. opencode daemon 协议细节未确认（P3 前）。
4. codex 续聊弱，标为已知限制。
