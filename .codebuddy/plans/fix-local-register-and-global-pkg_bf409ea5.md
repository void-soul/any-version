---
name: fix-local-register-and-global-pkg
overview: 修复注册本地版本的 OS Error 5 问题（改为用 junction 代替复制）、删除未实现的"注册本地版本"勾选功能、修复全局包（yarn 等）无法执行的问题（PATH 中缺少 bin 目录）
todos:
  - id: fix-register-local-junction
    content: 重写 project_register_local_inner，改用 junction 指向本地目录（不再复制）
    status: completed
  - id: delete-register-local-checked
    content: 删除前端 registerLocalChecked 相关 UI 和 state
    status: completed
    dependencies:
      - fix-register-local-junction
  - id: fix-nodejs-bin-paths
    content: 修复 get_bin_paths 和 get_sdk_bin_paths 对 nodejs 的 PATH 处理，添加 bin 子目录
    status: completed
    dependencies:
      - fix-register-local-junction
  - id: test-and-verify
    content: 测试验证：注册本地版本、全局包执行、取消托管还原
    status: completed
    dependencies:
      - fix-nodejs-bin-paths
---

## 产品概述

修复 AnyVersion 工具中"托管项目"功能的 3 个 bug。

## 核心功能

1. **修复注册本地版本时的"拒绝访问 os 5"错误**：将复制整个目录的方式改为创建 junction 指向本地目录，避免文件占用问题
2. **删除未工作的"将本地已安装版本也注册到 AnyVersion"功能**：按用户建议，删除该 checkbox，让用户托管后手动添加
3. **修复 nodejs 全局包无法执行的问题**：在 `get_bin_paths` 中为 nodejs 添加 `bin` 子目录到 PATH

## 技术栈

- 后端：Rust + Tauri 2.x
- 前端：React + TypeScript
- 核心机制：Windows junction（目录联接）用于版本切换

## 实现方案

### 问题 3：注册本地版本改用 junction（核心修复）

**当前问题**：`versions.rs` 第 487 行 `copy_dir_all(src, &dest_dir)` 复制整个目录，若源目录有文件被占用（如 node.exe 正在运行），Windows 返回 OS Error 5（拒绝访问）。

**修复方案**：

- 不再复制目录，直接在 `versions_dir\{id}\{version}` 位置创建 junction 指向本地目录
- 复用已有的 `crate::commands::cache::create_junction` 函数（`cache.rs` 第 68-97 行）
- 修改 `project_register_local_inner` 函数（`versions.rs` 第 471-500 行）

**关键代码修改**：

```rust
// versions.rs 第 471-500 行，重写 project_register_local_inner
pub fn project_register_local_inner(id: &str, local_path: &str) -> Result<(), String> {
    let config = load_config();
    let src = Path::new(local_path);
    if !src.exists() {
        return Err("本地路径不存在".to_string());
    }

    // 自动识别版本号
    let effective_version = detect_version_from_path(id, src)
        .ok_or_else(|| "无法自动识别版本号，请手动指定版本号".to_string())?;

    let dest_dir = Path::new(&config.versions_dir).join(id).join(&effective_version);
    if dest_dir.exists() {
        // 已存在：尝试移除（可能是旧 junction）
        let _ = fs::remove_dir(&dest_dir);
        if dest_dir.exists() {
            return Err(format!("版本 {} 已存在，无需重复添加", effective_version));
        }
    }

    // 创建 junction 指向本地目录（不再复制）
    let versions_id_dir = Path::new(&config.versions_dir).join(id);
    fs::create_dir_all(&versions_id_dir).map_err(|e| e.to_string())?;
    crate::commands::cache::create_junction(&dest_dir, src)?;

    // 首次安装时自动创建 link_dir 的 junction
    let junction_path = Path::new(&config.links_dir).join(id);
    if !junction_path.exists() {
        let _ = crate::commands::cache::create_junction(&junction_path, &dest_dir);
    }

    let link_str = junction_path.to_string_lossy().to_string();
    let dest_str = dest_dir.to_string_lossy().to_string();
    let _ = crate::commands::env::configure_sdk_env_vars(id, &link_str, &dest_str);

    Ok(())
}
```

---

### 问题 4：删除"将本地已安装版本也注册"功能

**当前问题**：前端有 `registerLocalChecked` state（第 64 行），但后端没有对应的批量注册命令，功能未实现。

**修复方案**：

1. 删除前端 `ProjectDetailPanel.tsx` 中的 `registerLocalChecked` state
2. 删除托管预览对话框中与 `registerLocalChecked` 相关的 checkbox UI
3. 删除 `EMPTY_UI` 中的 `registerLocalChecked: false` 初始化
4. 清理相关 patch 调用

**修改文件**：`src/components/project/ProjectDetailPanel.tsx`

---

### 问题 6：修复 nodejs 全局包 PATH 问题

**当前问题**：

- `NPM_CONFIG_PREFIX` 被设置为 `link_dir`（`commands.rs` 第 142 行）
- npm `-g` 安装的 bin 会在 `<prefix>\` 下（npm 行为）
- 但 `get_bin_paths` 对 nodejs 只返回 `[link_dir]`，没有加 `\bin\`
- 导致 yarn 等全局包安装后无法执行

**修复方案**：修改 `scanner.rs` 第 473 行，为 nodejs 添加 `bin` 子目录：

```rust
// scanner.rs 第 473-475 行修改
"nodejs" => {
    // npm -g 的 bin 在 <prefix> 下，但部分包可能在 <prefix>\bin 下
    vec![link_dir.to_string(), format!("{}\\bin", link_dir)]
}
"bun" | "yarn" | "pnpm" | "nginx" | "redis" => {
    vec![link_dir.to_string()]
}
```

**同时检查 `env.rs` 第 507-532 行的 `get_sdk_bin_paths` 函数**（硬编码 fallback），也需要同步修改：

```rust
// env.rs 第 524 行修改
"nodejs" => {
    vec![link_dir.to_string(), format!("{}\\bin", link_dir)]
}
```

---

## 架构设计

### 系统架构图

```mermaid
graph TD
    A[用户点击"注册本地版本"] --> B[project_register_local_inner]
    B --> C{检查本地路径}
    C -->|存在| D[识别版本号]
    C -->|不存在| E[返回错误]
    D --> F[创建 junction 指向本地目录]
    F --> G[创建 link_dir junction]
    G --> H[配置环境变量]
    H --> I[完成]
```

### 数据流

1. 用户指定本地路径 → 前端调用 `project_register_local`
2. 后端识别版本号 → 在 `versions_dir\{id}\` 下创建版本目录的 junction
3. 创建 `links_dir\{id}` 的 junction 指向当前版本
4. 配置环境变量（`NODE_PATH` 等）
5. 更新 PATH（`get_bin_paths` 返回的路径列表）

## 目录结构

```
e:\pro\my\any-version\
├── src-tauri\src\commands\project\
│   ├── versions.rs          [修改] 重写 project_register_local_inner，改用 junction
│   └── scanner.rs           [修改] 修复 get_bin_paths 对 nodejs 的 PATH 处理
├── src-tauri\src\commands\
│   └── env.rs                [修改] 修复 get_sdk_bin_paths 对 nodejs 的处理
└── src\components\project\
    └── ProjectDetailPanel.tsx [修改] 删除 registerLocalChecked 相关代码
```

## 关键代码结构

### 修改后的 `project_register_local_inner` 函数签名（接口级）

```rust
/// 注册本地版本（创建 junction 指向本地目录）
/// - 不再复制目录，避免文件占用问题（OS Error 5）
/// - 在 versions_dir\{id}\{version} 创建 junction 指向 local_path
pub fn project_register_local_inner(id: &str, local_path: &str) -> Result<(), String>;
```

### 修改后的 `get_bin_paths` 对 nodejs 的返回

```rust
/// nodejs 的 bin 路径：
/// - link_dir 本身（node.exe 在此）
/// - link_dir\bin（部分 npm 全局包的 bin 可能在此）
"nodejs" => vec![link_dir.to_string(), format!("{}\\bin", link_dir)]
```

## Agent Extensions

### SubAgent

- **code-explorer**
- Purpose: 深度探索代码库，确认所有需要修改的位置
- Expected outcome: 找到所有与 `registerLocalChecked`、`copy_dir_all`、nodejs `bin_paths` 相关的代码位置

- **bmad-dev**
- Purpose: 实施功能修复，基于 PRD 和架构设计进行开发
- Expected outcome: 完成 3 个问题的代码修复，包括 Rust 后端和 TypeScript 前端