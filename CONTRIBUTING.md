# 贡献指南

## 环境要求

- Windows（Tauri 2 + WebView2）、Node ≥ 20.19、pnpm ≥ 10（CI 用 11）、Rust stable
- `cargo-audit`（CI 通过 `taiki-e/install-action@cargo-audit` 自动安装）

## 全新机器一键起步

```bash
pnpm install --frozen-lockfile
pnpm test
pnpm tauri dev
```

不需要手工创建 junction、shim、PATH 或 bucket；`scripts/bundle-resources.mjs` 会在
`start` / `build` 前自动把 `ai-tools/`、`projects/`、`node-projects/` 复制到
`src-tauri/_up_/`，CI 的 `bundle resources` 步骤同理。

## 提交前必跑（与 CI 一致）

```bash
pnpm check:architecture
pnpm check:contracts
pnpm check:catches
pnpm check:locks
pnpm check:rustfmt
pnpm exec tsc --noEmit
pnpm test
pnpm test:coverage
pnpm audit --prod --audit-level=high
cargo check --locked --manifest-path src-tauri/Cargo.toml
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo test --locked --manifest-path crates/cert-deployment/Cargo.toml
cargo audit --file src-tauri/Cargo.lock
```

约定：

- **契约**：Rust serde 字段 / Tauri 命令参数改名必须同步 `src/` 的 TS 类型与调用点，
  `check:contracts` 与 `ipcContract.test.ts` 会拦住漏改。
- **失败可观测**：禁止新增 `.catch(() => {})` / `catch {}`；写路径使用
  `src/utils/invokeOrReport.ts` 的 `reportInvokeError`，确可忽略时用
  `ignoreInvokeError(e, 理由)`。
- **锁**：生产 Rust 代码禁止 `.lock().unwrap()`，统一
  `.lock().unwrap_or_else(|e| e.into_inner())`。
- **格式**：新增/改动的 Rust 文件必须通过 `rustfmt`；存量未格式化文件在
  `scripts/rustfmt-baseline.txt` 白名单中，白名单**只允许收缩**。
- **时间/随机数**：存储用 UTC（`src-tauri/src/commands/foundation/time.rs`），
  鉴权凭据用 `foundation/secure_random.rs`，不得回退到时间戳等可预测值。
- **数据库**：schema 变更走 `foundation/migrate.rs` 的 `user_version` 步骤，
  必须包在事务里并提供失败回滚。
- **测试**：新功能补单测；共享全局状态的测试要显式串行化（见
  `commands/tasks/tests.rs` 的 `DB_TEST_LOCK`）。
- **提交**：不自动提交；提交信息关联 QA ID（见项目 `qa-log` 技能约定）。

## 架构决策

关键取舍记录在 [docs/adr/](docs/adr/)，改动前先读，避免重复讨论已有结论。
