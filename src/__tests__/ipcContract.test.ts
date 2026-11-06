import { describe, expect, it } from "vitest";

/**
 * IPC 契约的往返 smoke 测试（纯静态 + 无 Tauri 运行时、无 node 类型依赖）。
 *
 * 项目里 `invoke("cmd", {...})` 是运行时字符串契约：Rust 侧命令改名/未注册时
 * `tsc` 与单元测试都发现不了，只表现为「点了没反应」。这里在 vitest 里把
 * 「Rust 定义 / Rust 注册 / TS 调用」三向差集做成断言，保证 `pnpm test`
 * 本身就能拦住这类破坏，而不必依赖外部脚本或真机 Tauri。
 *
 * 用 Vite 的 import.meta.glob 读源码原文：不引入 node:fs/@types/node，
 * 全新机器按 lockfile 装完依赖即可直接运行。
 */
const RUST_FILES = import.meta.glob<string>("/src-tauri/src/**/*.rs", {
  query: "?raw",
  import: "default",
  eager: true,
});
const WEB_FILES = import.meta.glob<string>("/src/**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
});

// 已知例外（有意为之，不是漏接线）。新增条目必须写明理由，且应随整改逐步清空。
const INVOKE_TEMPLATE_PLACEHOLDERS = new Set([
  "command_name", // UI 里用于展示/拼接命令名的模板占位符，不是真实 invoke
]);
const DEFINED_BUT_UNREGISTERED_ALLOWLIST = new Set<string>();

function sourceOf(files: Record<string, string>, key: string): string {
  const value = files[key];
  if (typeof value !== "string") throw new Error(`缺少源码文件 ${key}`);
  return value;
}

function registeredCommands(): Set<string> {
  const source = sourceOf(RUST_FILES, "/src-tauri/src/app_commands.rs");
  const start = source.indexOf("generate_handler![");
  const end = source.indexOf("]", start);
  if (start < 0 || end < 0) throw new Error("找不到 generate_handler![ ... ] 注册块");
  const body = source.slice(start, end);
  const names = new Set<string>();
  // 注册项两类：commands::<module>::<fn> 与 tray::<fn>（托盘命令在 app_commands.rs:214-215）。
  for (const match of body.matchAll(/\b(?:commands|tray)::([A-Za-z0-9_:]+)\s*,/g)) {
    names.add(match[1].split("::").pop()!);
  }
  return names;
}

function definedCommands(): Set<string> {
  const names = new Set<string>();
  for (const source of Object.values(RUST_FILES)) {
    // 允许 #[tauri::command] 与 pub fn 之间夹 /// 文档注释（api_delete_module 即如此）。
    for (const match of source.matchAll(
      /#\[tauri::command[^\]]*\]\s*(?:\/\/\/[^\n]*\n\s*)*(?:pub\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)\s*\(/g,
    )) {
      names.add(match[1]);
    }
  }
  return names;
}

function invokedCommands(): Set<string> {
  const names = new Set<string>();
  for (const [file, source] of Object.entries(WEB_FILES)) {
    if (file.includes("/__tests__/") || file.endsWith(".d.ts")) continue;
    for (const match of source.matchAll(/\binvoke(?:<[^>]*>)?\s*\(\s*["']([A-Za-z0-9_]+)["']/g)) {
      names.add(match[1]);
    }
  }
  return names;
}

describe("IPC 命令契约（Rust 定义 / 注册 / WebView 调用 三向一致）", () => {
  it("注册块可读且规模合理", () => {
    expect(registeredCommands().size).toBeGreaterThan(700);
  });

  it("前端调用的每个命令都已注册（漏注册 = 点击无反应）", () => {
    const registered = registeredCommands();
    const missing = [...invokedCommands()].filter(
      (name) => !registered.has(name) && !INVOKE_TEMPLATE_PLACEHOLDERS.has(name),
    );
    expect(missing.sort()).toEqual([]);
  });

  it("已定义的 tauri command 不应长期停留在未注册状态", () => {
    const registered = registeredCommands();
    const dead = [...definedCommands()].filter(
      (name) => !registered.has(name) && !DEFINED_BUT_UNREGISTERED_ALLOWLIST.has(name),
    );
    expect(dead.sort()).toEqual([]);
  });

  it("白名单只允许收缩：已删除的死命令不应继续留在例外表", () => {
    const defined = definedCommands();
    const stale = [...DEFINED_BUT_UNREGISTERED_ALLOWLIST].filter((name) => !defined.has(name));
    expect(stale.sort()).toEqual([]);
  });
});
