---
kind: configuration_system
name: AnyVersion 配置系统：声明式 JSON + Tauri 持久化 + 资源目录分层加载
category: configuration_system
scope:
    - '**'
source_files:
    - src-tauri/src/commands/config.rs
    - src-tauri/src/commands/project/registry.rs
    - src-tauri/src/commands/project/types.rs
    - src-tauri/tauri.conf.json
    - ai-tools/providers.json
    - ai-tools/mcp-config.json
    - projects/nodejs/config.json
---

## 1. 体系概览

本仓库采用「运行时用户配置（JSON 文件）+ 打包期资源清单（JSON 数据）+ Tauri 应用元信息」三层组合的配置模型，全部以 JSON 为统一序列化格式，由 Rust 后端在启动时按优先级加载、合并并缓存到进程内存。

- 运行时用户配置：`~/.any-version/config.json`（Windows 下 `USERPROFILE`/`HOME` 拼接 `.any-version`），保存版本目录、RSS 源、项目菜单开关等可编辑状态。
- 项目与 AI 工具定义：`projects/<id>/config.json` 与 `ai-tools/*/config.json`、`providers.json`、`mcp-config.json` 等，作为“零代码扩展”的声明式配置，通过 Tauri `bundle.resources` 打包进应用，运行期从资源目录或 exe 同层 `_up_/projects` 覆盖。
- Tauri 应用元配置：`src-tauri/tauri.conf.json`，描述构建、窗口、插件（updater）、资源注入等。

## 2. 核心文件与包

- 用户配置读写与迁移
  - `src-tauri/src/commands/config.rs`：`Config`/`BackupStore` 结构体、`get_base_dir()`、`load_config()/save_config()`、RSS 配置、存储路径迁移（versions_dir/links_dir）、备份文件 `backups.json`。
- 项目注册表（多语言 SDK 定义）
  - `src-tauri/src/commands/project/registry.rs`：扫描 `projects/` 子目录，聚合每个 `<id>/config.json` 及其拆分文件 `env_vars.json`、`find_rules.json`、`package_managers.json`、`remote_versions_config.json`；兼容旧版单文件 `projects.json`。
  - `src-tauri/src/commands/project/types.rs`：`ProjectDef`、`PackageManagerDef`、`EnvVarDef`、`FindRule`、`ConflictManagerDef` 等所有项目定义的 serde 类型。
- AI 工具与模型供应商
  - `ai-tools/providers.json`：OpenAI/Anthropic/Gemini 兼容端点列表。
  - `ai-tools/mcp-config.json`：各 CLI 工具的 MCP 配置文件路径与格式映射。
  - `ai-tools/<tool>/config.json`：单个 AI CLI 工具协议、命令模板、会话/技能目录等。
- Tauri 应用元配置
  - `src-tauri/tauri.conf.json`：构建脚本、前端输出目录、图标、打包资源（`../ai-tools/*.json`、`../projects/*/*.json`）、updater 插件。

## 3. 架构与约定

### 3.1 加载顺序与覆盖策略

| 层级 | 来源 | 说明 |
|---|---|---|
| 应用元配置 | `tauri.conf.json` | 构建期固定，决定资源注入与 updater 行为 |
| 内置定义 | 打包资源 `ai-tools/*.json`、`projects/*/*.json` | 随二进制分发，不可修改 |
| 用户覆盖 | exe 同层或 `_up_/projects`、当前工作目录、用户配置目录下的 `projects/` | 优先于内置，实现“零代码扩展” |
| 运行时用户配置 | `~/.any-version/config.json` | 首次启动自动生成默认值，后续由 UI 更新 |

`registry::load_registry()` 严格按以下顺序搜索 `projects/` 目录：
1. 打包后资源根目录（`res_dir`）
2. exe 所在目录及向上 5 层父目录
3. 当前工作目录
4. 用户配置目录（`~/.any-version`）

找到第一个非空列表即返回，否则回退到兼容模式读取 `projects.json`。

### 3.2 项目定义拆分约定

每个 SDK 对应一个目录 `projects/<id>/`，其中：
- `config.json`：主定义（`ProjectDef`）
- `env_vars.json` / `find_rules.json` / `package_managers.json` / `remote_versions_config.json`：可选拆分文件，**文件内容优先覆盖 config.json 内联同名字段**，便于大型项目维护。

### 3.3 用户配置结构

`Config` 包含：
- 存储路径：`versions_dir`、`links_dir`
- 托管清单：`managed_items`、`simple_managed_items`、`active_versions`
- 自定义安装/数据路径映射
- 项目菜单显示控制：`project_menu_configs`
- 项目委托设置：`project_delegations`（环境变量白名单、PATH 变量、是否创建符号链接等）
- RSS 订阅源与名称映射
- 首次运行标记 `has_run_before`

备份数据独立存放于 `backups.json`，并在启动时执行一次迁移（将旧的 `original_envs`/`original_paths` 迁入）。

### 3.4 配置变更与迁移

当通过 `update_config(versions_dir, links_dir)` 修改存储路径时：
- 自动移动已安装版本与链接目录
- 重建 Windows junction 指向
- 重写系统 PATH 环境变量中受控条目
- 根据 `EnvVarTier` 清理/写入注册表环境变量
- 通过 Tauri `emit("migrate-progress", ...)` 向前端推送进度事件

## 4. 开发者应遵循的规则

1. **新增 SDK 定义**：在 `projects/<new_id>/config.json` 中声明 `ProjectDef`，复杂数组字段拆入同名 `*.json` 文件；如需覆盖内置定义，在同级 `_up_/projects/<new_id>/` 放置相同结构。
2. **新增 AI 工具**：在 `ai-tools/<tool>/config.json` 中添加协议与命令模板，并在 `mcp-config.json` 中补充 MCP 配置文件映射。
3. **新增模型供应商**：在 `ai-tools/providers.json` 追加条目，保持 `openaiUrl`/`anthropicUrl`/`googleUrl` 三端点字段语义不变。
4. **修改用户配置结构**：仅在 `src-tauri/src/commands/config.rs` 中扩展 `Config` 结构体，并确保 `load_config()` 默认值与 `migrate_backups_from_config` 向后兼容。
5. **不要硬编码路径**：使用 `get_base_dir()` 获取用户配置根，避免直接写死 `C:\Users\...`。
6. **配置热更新**：对 `projects/` 与 `ai-tools/` 的改动无需重启——`registry::clear_registry_cache()` 会触发重新扫描。
7. **敏感信息**：API Key 等敏感字段通过 AI 工具 `configFile.write` 映射写入宿主 CLI 的配置文件（如 `~/.claude/settings.json`），不应持久化到 AnyVersion 自身的 `config.json`。
