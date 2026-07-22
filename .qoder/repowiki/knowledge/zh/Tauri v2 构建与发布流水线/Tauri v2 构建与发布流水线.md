---
kind: build_system
name: Tauri v2 构建与发布流水线
category: build_system
scope:
    - '**'
source_files:
    - package.json
    - src-tauri/Cargo.toml
    - src-tauri/tauri.conf.json
    - src-tauri/build.rs
    - scripts/bump-version.js
    - .github/workflows/ci.yml
    - .github/workflows/package.yml
    - .github/workflows/release.yml
---

本项目采用 pnpm + Vite (前端) + Tauri v2 (Rust 后端) 的混合构建体系，通过 GitHub Actions 完成 CI、打包与 Release 全流程。

### 1. 构建工具链与入口
- 前端：package.json 中 dev/build/preview 脚本基于 Vite 7 + TypeScript 5.8，产物输出到 dist/；tauri.conf.json 配置 beforeDevCommand: npm run dev、beforeBuildCommand: npm run build、frontendDist: ../dist，使 Tauri 在开发时代理到 http://localhost:9999，生产时直接内嵌静态资源。
- 后端：src-tauri/Cargo.toml 以 crate-type = ["staticlib","cdylib","rlib"] 暴露为 Tauri 插件库，依赖 tauri v2 及 opener/dialog/updater/autostart/process 等官方插件；build.rs 仅调用 tauri_build::build() 生成能力清单。
- 版本同步：scripts/bump-version.js 统一修改 package.json、src-tauri/Cargo.toml、src-tauri/tauri.conf.json 三处的 version 字段，是唯一的版本号来源。

### 2. 打包与分发
- 目标平台：tauri.conf.json 中 bundle.targets = "all"，当前实际只支持 Windows（代码依赖 winreg），CI 与 Release 均运行在 windows-latest。
- 安装包格式：同时产出 .msi（WiX）和 .exe（NSIS）；Release 流水线显式 choco install nsis 以满足 NSIS 工具链需求。
- 自动更新：启用 tauri-plugin-updater，endpoints 指向 GitHub Releases 的 latest.json，签名密钥通过 TAURI_SIGNING_PRIVATE_KEY / TAURI_SIGNING_PRIVATE_KEY_PASSWORD 注入，由 tauri-action 生成带 .sig 的更新清单。
- 资源嵌入：bundle.resources 将 ai-tools/*.json、projects/*/*.json 打入安装包，供运行时读取。

### 3. CI 流水线（.github/workflows）
- ci.yml：双 Job。Ubuntu 上执行 pnpm install --frozen-lockfile + pnpm run build（类型检查+Vite 打包）；Windows 上固定 Rust 1.93，缓存 cargo registry/git/target，软门限 cargo fmt --check 与 cargo clippy，硬门限 cargo test。Dependabot PR 跳过 Rust Job。
- package.yml：手动触发，接收 version 输入，调用 bump-version.js 提交变更并推送 v<ver> tag。
- release.yml：监听 tag 或手动指定 tag。先创建/复用 draft release，再在 Windows runner 上安装 Node 22 + pnpm 11 + Rust 1.93 + NSIS，调用 tauri-apps/tauri-action@v0 上传产物至同一 release（通过 releaseId 避免竞态），最后用 gh api 重命名资产为 AnyVersion_<VER>_x64-setup.exe/.msi。

### 4. 开发者约定
- 版本号：一律通过 pnpm bump <x.y.z> 或 node scripts/bump-version.js <x.y.z> 同步三处版本，禁止手工改。
- Rust 工具链：CI 与 Release 锁定 1.93，本地也建议安装该版本以避免 clippy/fmt 差异。
- 平台约束：项目仅面向 Windows，跨平台编译不会通过；新增平台需同步修改 ci.yml、release.yml 的 runner 与 bundle targets。
- 依赖缓存：CI 使用 actions/cache 缓存 ~/.cargo/registry、~/.cargo/git、src-tauri/target，以及 swatinem/rust-cache 缓存 target，加速构建。
- 安全：updater 公钥已内联于 tauri.conf.json，私钥通过 GitHub Secrets 注入，禁止在仓库中存放私钥。