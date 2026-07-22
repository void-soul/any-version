---
kind: external_dependency
name: Tauri 桌面应用框架
slug: tauri
category: external_dependency
category_hints:
    - vendor_identity
scope:
    - '**'
---

本项目基于 Tauri 2 构建桌面 GUI，Rust 后端通过 #[tauri::command] 暴露给前端。打包后资源目录与 exe 同目录及向上 5 层、当前工作目录、用户配置目录（~/.any-version）中查找 ai-tools 注册表；Tauri 打包时 `../ai-tools` 的 `..` 会被映射为 `_up_` 前缀布局。所有 AI 工具启动、技能管理、用量统计等能力均通过 Tauri command 暴露。