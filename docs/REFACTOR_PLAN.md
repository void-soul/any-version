# AnyVersion 代码审计与重构计划

## 一、审计发现

### 1. Rust 后端硬编码问题

**`versions.rs` — 最严重的硬编码集中区**

| 函数 | 问题 | 影响 |
|---|---|---|
| `get_download_url()` | 42+ 行 match 硬编码所有下载 URL | 新增语言需改 Rust 代码重新编译 |
| `project_list_remote_versions()` | 15+ 分支硬编码所有远程版本 API | 新增语言需改 Rust 代码 |
| `project_install_version()` | `if id == "python"` / `if id == "mysql"` 硬编码后置逻辑 | 新增语言需改 Rust 代码 |

**`projects.json` 中已有但未使用的字段：**
- `download_url_template` — 0 个项目有值（全部空）
- `remote_versions_url` — 0 个项目有值（全部空）
- `cache_default_path` — 0 个项目有值

**代码重复：**
- `get_bin_paths()` 在 `scanner.rs` 和 `commands.rs` 中完全重复
- `configure_sdk_env_vars()` 中的特殊路径逻辑与 `commands.rs` 中重复

### 2. 前端结构问题

**侧边栏结构：**
- 「环境备份还原」是独立入口，应整合为「系统工具」的子标签
- 「全局路径设置」名称不直观，应改为「设置」

**缺失功能：**
- 版本检查（GitHub Release 检测升级）
- 路径修改时的已有安装迁移

---

## 二、重构计划

### Phase 1: 填充 projects.json 数据（消除 Rust 硬编码的前提）

为每个项目补充：
```json
{
  "download_url_template": "https://go.dev/dl/go{version}.windows-amd64.zip",
  "download_file_ext": "zip",
  "remote_versions_api": {
    "type": "json_array",
    "url": "https://go.dev/dl/?mode=json&include=all",
    "version_field": "version",
    "version_transform": "trim_prefix:go",
    "filter": "stable==true",
    "max_count": 100
  }
}
```

### Phase 2: Rust 后端重构

1. `get_download_url()` → 读取 `download_url_template`，用 `{version}` 占位符替换
2. `project_list_remote_versions()` → 读取 `remote_versions_api` 配置，通用 HTTP+JSON 解析
3. `get_bin_paths()` → 去重，统一到 scanner 模块
4. `project_install_version()` 后置逻辑 → JSON 中增加 `post_install` 配置

### Phase 3: 前端重构

1. **侧边栏整合：** 环境备份还原 → 系统工具子标签
2. **设置页重构：** 改名「设置」，增加版本检查、路径迁移
3. **组件提取：** 公共 UI 模式抽取为可复用组件

---

## 三、建议执行顺序

1. ✅ Phase 1: 填充 projects.json（纯数据，不改代码）
2. ✅ Phase 2: Rust 重构（消除硬编码）
3. ✅ Phase 3: 前端重构（侧边栏 + 设置页 + 组件优化）

每完成一个 Phase 可独立验证，降低风险。
