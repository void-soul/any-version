---
kind: logging_system
name: Rust 后端无结构化日志系统，仅使用 eprintln! 与文件直写
category: logging_system
scope:
    - '**'
source_files:
    - src-tauri/Cargo.toml
    - src-tauri/src/lib.rs
    - src-tauri/src/main.rs
    - src-tauri/src/commands/cache.rs
---

本仓库的 Rust 后端（src-tauri）未引入任何第三方日志框架（Cargo.toml 中不存在 tracing、env_logger、slog、fern、log4rs 等依赖），也未在 lib.rs / main.rs 中进行全局 logger 初始化。整体日志策略如下：

1. **调试输出**：大量业务代码直接使用 `eprintln!` 打印带模块前缀的文本行（如 `[collab]`、`[skill]`、`[registry]` 等），用于开发期在 Windows 控制台窗口查看；由于 Tauri 默认以 `windows_subsystem = "windows"` 运行，打包后这些 stderr 不可见。
2. **持久化调试日志**：通过 `commands::cache::skill_debug_log` 将技能相关的关键操作（创建 junction、错误信息）追加写入应用数据目录下的 `skill-debug.log`，时间戳格式为 `%Y-%m-%d %H:%M:%S%.3f`，供打包运行时排查问题。
3. **进程/服务级 log_dir**：项目类型定义中包含 `log_dir: Option<String>` / `log_dir: String` 字段，并在 service 命令中支持 `{log_dir}` 模板替换，但这是面向被托管子进程的外部日志目录，并非 AnyVersion 自身进程的日志输出。
4. **前端侧**：React/Vite 前端未发现专门的日志收集或上报逻辑，主要依赖 Rust 侧通过 Tauri event（如 `migrate-progress`、`clean-cache-progress`）向前端推送进度事件，而非结构化日志通道。

**开发者应遵循的规则**：
- 新增调试信息优先复用 `skill_debug_log` 写入 `skill-debug.log`，避免直接 `println!/eprintln!` 导致打包后丢失。
- 不要自行引入新的日志 crate，保持与现有 `eprintln! + skill-debug.log` 模式一致。
- 对需要跨进程/子进程输出的场景，沿用 `log_dir` 配置项由调用方决定落盘位置，不要在 AnyVersion 主进程内再建一套日志路由。