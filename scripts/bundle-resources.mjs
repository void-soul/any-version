// 打包前把仓库根的 ai-tools / projects / projects.json 资源拷贝进
// src-tauri/_up_，再用 `tauri.conf.json` 的 `bundle.resources` 以
// 「src-tauri 内部相对路径」（_up_/...）声明它们。
//
// 为什么这样做：
// 之前 resources 用 `../ai-tools/*.json` 这种跨目录(`../`) + glob 写法，
// Tauri 会把它们映射进安装包的 `_up_` 目录。但本地与 CI 对 `../` 的解析
// 基准不一致（CI 的 runner 工作目录 / tauri-action 调用方式差异），导致
// CI 构建出的安装包里缺失 `_up_`，程序启动后找不到 ai-tools / projects 配置。
// 改为在打包前显式把资源拷贝进 src-tauri/_up_，并用无 `../` 的相对路径声明，
// 可彻底消除本地/CI 差异，且保留 `_up_` 目录结构（现有 Rust 查找逻辑零改动）。

import { cpSync, mkdirSync, existsSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(__dirname, "..");
const destRoot = join(repoRoot, "src-tauri", "_up_");

// 清空旧的 _up_，避免残留过期文件
if (existsSync(destRoot)) {
  rmSync(destRoot, { recursive: true, force: true });
}
mkdirSync(join(destRoot, "ai-tools"), { recursive: true });
mkdirSync(join(destRoot, "projects"), { recursive: true });
mkdirSync(join(destRoot, "node-projects"), { recursive: true });
mkdirSync(join(destRoot, "page-agent-bridge"), { recursive: true });

function copyIfExists(srcRel, destRel) {
  const src = join(repoRoot, srcRel);
  const dest = join(destRoot, destRel);
  if (!existsSync(src)) {
    console.warn(`[bundle-resources] 跳过（源不存在）: ${srcRel}`);
    return;
  }
  mkdirSync(dirname(dest), { recursive: true });
  cpSync(src, dest, { recursive: true });
  console.log(`[bundle-resources] ${srcRel} -> _up_/${destRel}`);
}

copyIfExists("ai-tools", "ai-tools");
copyIfExists("projects", "projects");
copyIfExists("node-projects", "node-projects");

// 构建 page-agent 浏览器桥并复制到应用资源，生产环境由 Rust 从 resource_dir 读取。
const bridgeDir = join(repoRoot, "page-agent-bridge");
if (existsSync(join(bridgeDir, "build.mjs"))) {
  console.log("[bundle-resources] 构建 page-agent-bridge …");
  const { execFileSync } = await import("node:child_process");
  // 在 Windows 的某些 Node 启动环境中，直接 spawn npm.cmd 会返回 EINVAL；
  // 使用当前 Node 执行 npm-cli.js，避免依赖 shell/.cmd 解释器。
  const npmCli = process.env.npm_execpath || join(dirname(process.execPath), "node_modules", "npm", "bin", "npm-cli.js");
  execFileSync(process.execPath, [npmCli, "run", "build"], {
    cwd: bridgeDir,
    stdio: "inherit",
  });
  copyIfExists("page-agent-bridge/dist/page-agent-bridge.iife.js", "page-agent-bridge/page-agent-bridge.iife.js");
}

console.log("[bundle-resources] 完成：资源已就绪于 src-tauri/_up_");
