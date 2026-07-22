---
kind: dependency_management
name: pnpm + Cargo 双包管理器与锁定文件策略
category: dependency_management
scope:
    - '**'
source_files:
    - package.json
    - pnpm-workspace.yaml
    - pnpm-lock.yaml
    - .npmrc
    - src-tauri/Cargo.toml
    - src-tauri/Cargo.lock
---

本仓库采用 pnpm workspace（单根）+ Cargo 的双包管理器方案，分别管理前端（React/Vite/Tauri-frontend）和后端（Tauri Rust）依赖，并通过各自的锁定文件保证构建可复现。

## 1. 使用的系统与工具
- Node.js 生态：使用 pnpm 作为包管理器，配合 pnpm-workspace.yaml 声明工作区；通过 .npmrc 控制日志级别。
- Rust 生态：使用 Cargo 作为包管理器与构建系统，所有 crate 版本在 Cargo.toml 中声明，精确解析结果固化到 Cargo.lock。
- 无 vendoring：未启用任何语言级的 vendor 目录（node_modules/.pnpm 由 pnpm 自动管理，src-tauri/target 为构建产物而非源码级 vendoring）。

## 2. 关键文件
- package.json：前端依赖与脚本入口（start/dev/build/tauri/bump）。
- pnpm-workspace.yaml：声明当前目录为唯一 workspace，并禁用 esbuild 的 allowBuilds。
- pnpm-lock.yaml：pnpm v9 lockfile，记录每个包的精确版本、peer 依赖与 integrity。
- src-tauri/Cargo.toml：Rust 包清单，集中声明 Tauri 插件、网络/压缩/数据库等依赖及 feature。
- src-tauri/Cargo.lock：Cargo 生成的完整依赖树与 checksum，用于 CI 一致性校验。
- .npmrc：仅设置 loglevel=error，未配置私有源或镜像。

## 3. 架构与约定
- 单一应用根：workspace 仅包含 '.'，不存在子包拆分，所有依赖集中在根 package.json。
- 版本范围策略：
  - Node 侧大量使用 ^major 宽泛范围（如 @tauri-apps/api: ^2、react: ^19.1.0），由 lockfile 锁定实际版本。
  - Rust 侧对核心库使用 version = "X" 语义化主版本约束（如 tauri = { version = "2" }、serde = { version = "1.0", features = ["derive"] }），部分工具库使用精确小版本（如 winreg = "0.52"、walkdir = "2.5"）。
- Tauri 前后端依赖对齐：前端 @tauri-apps/* 插件与后端 tauri-plugin-* crate 保持同 major 版本族，确保 IPC 能力一致。
- 构建期依赖隔离：tauri-build 放在 [build-dependencies]，运行时依赖放入 [dependencies]，符合 Tauri v2 推荐结构。
- 无私有注册表/代理配置：.npmrc 与 Cargo.toml 均未出现 registry=、source= 或 .cargo/config.toml，默认走 npmjs.org 与 crates.io。

## 4. 开发者应遵循的规则
- 新增依赖时：
  - Node 侧通过 pnpm add [-D] <pkg> 修改 package.json 并更新 pnpm-lock.yaml，禁止手动编辑 lockfile。
  - Rust 侧通过 cargo add <crate> 修改 Cargo.toml 并更新 Cargo.lock。
- 版本升级：优先使用 pnpm up / cargo update 后提交锁文件，避免引入未经验证的变动。
- CI 一致性：CI 步骤需先安装 pnpm 与 Cargo，再执行 pnpm install 与 cargo build，利用 lockfile 保证环境可复现。
- 不引入 vendor 目录：不要将 node_modules 或 target 纳入版本控制；如需离线构建，应在 CI 缓存层处理。
- Tauri 插件同步：新增 @tauri-apps/plugin-* 时需在后端 Cargo.toml 中添加对应 tauri-plugin-* crate，并保持主版本号一致。