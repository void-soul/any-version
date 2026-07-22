---
kind: error_handling
name: Rust/Tauri 命令层错误处理策略
category: error_handling
scope:
    - '**'
source_files:
    - src-tauri/src/lib.rs
    - src-tauri/src/commands/mod.rs
    - src-tauri/src/commands/cache.rs
    - src-tauri/src/commands/config.rs
    - src-tauri/src/commands/http_server.rs
    - src-tauri/Cargo.toml
---

## 1. 采用的错误处理体系

本仓库在 Rust 后端（Tauri v2）中采用**轻量、统一的字符串化错误模式**，未引入 `anyhow`/`thiserror` 等第三方错误库。核心约定如下：

- **Tauri 命令返回类型统一为 `Result<T, String>`**：所有通过 `#[tauri::command]` 暴露给前端的函数均以 `String` 作为错误分支，错误信息直接携带人类可读的中文描述（如 `"创建 junction 失败: stdout=... stderr=..."`）。
- **内部 I/O 操作使用 `std::io::Result` + `map_err(|e| e.to_string())?`**：对 `fs::write`、`tokio::fs::File::open` 等调用进行链式转换，将底层 OS 错误转为 `String` 后继续向上传播。
- **非关键路径静默吞错**：大量使用 `let _ = ...` 或 `if let Ok(...) = ...` 忽略可恢复的错误（如清理环境变量、写入调试日志），避免启动阶段因次要错误中断。
- **HTTP 内嵌服务器以硬编码 HTTP 状态码响应错误**：`http_server.rs` 在文件读取失败时直接返回 `500 Internal Server Error` HTML 体，不向上抛出异常。
- **进程级 panic 仅用于不可恢复的构建期错误**：`lib.rs` 中仅在 Tauri Builder 构建失败时使用 `.expect("error while building tauri application")`；运行时未发现 `panic!` 调用。

## 2. 关键文件与位置

| 文件 | 职责 |
|---|---|
| `src-tauri/src/lib.rs` | Tauri 应用入口、插件注册、窗口事件处理、全局 PATH 同步与环境变量清理 |
| `src-tauri/src/commands/mod.rs` | 命令模块聚合，无自定义错误类型定义 |
| `src-tauri/src/commands/cache.rs` | 缓存迁移/复制/junction 创建，集中展示 `Result<(), String>` + `map_err` 用法 |
| `src-tauri/src/commands/config.rs` | 配置读写、备份存储、迁移结果结构体 `MigrateResult { errors: Vec<String> }` |
| `src-tauri/src/commands/http_server.rs` | 内嵌 HTTP 服务器，以 HTTP 状态码表达错误 |
| `src-tauri/Cargo.toml` | 依赖声明，确认未引入 `anyhow`/`thiserror` |

## 3. 架构与约定

- **命令边界即错误边界**：每个 `#[tauri::command] fn` 都是前端可触达的最小错误单元，错误信息沿 IPC 直达 React UI，由前端决定如何呈现（弹窗、Toast、列表项高亮等）。
- **批量操作的“部分成功”语义**：通过 `MigrateResult` 这类结构体收集多个子步骤的错误到 `errors: Vec<String>`，允许前端分步展示而非整体失败。
- **调试输出双通道**：除 `Result` 返回值外，还通过 `eprintln!` + 写入 `skill-debug.log` 的方式记录详细上下文，弥补 `String` 错误信息不够丰富的缺陷。
- **无中间件/拦截器**：Tauri 未注册全局 invoke handler 拦截器，错误处理分散在每个命令实现中。

## 4. 开发者应遵循的规则

1. **对外暴露的命令一律返回 `Result<T, String>`**，错误消息使用中文并包含必要上下文（路径、参数、stdout/stderr）。
2. **I/O 错误使用 `map_err(|e| e.to_string())?` 转换**，不要自行构造复杂错误类型。
3. **可恢复的非关键错误用 `let _ = ...` 忽略**，确保主流程不被副作用阻塞。
4. **批量操作使用结构体收集错误**（参考 `MigrateResult`），避免一次性全部失败。
5. **不在运行时使用 `panic!`**，仅在构建期不可恢复场景使用 `expect!`。
6. **需要更丰富错误上下文时优先扩展 `String` 消息**，而非引入新的错误类型——保持与现有 Tauri 命令签名兼容。
