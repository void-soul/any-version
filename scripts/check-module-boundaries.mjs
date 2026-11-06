import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// 存量 ≥500 行文件白名单（AUDIT-049）。只允许收缩：
// - 白名单内文件不得比登记时更大；
// - 白名单外的新增文件必须 < 500 行。
const baseline = new Map(
    require("./large-file-baseline.json").files.map((f) => [f.file, f.lines]),
);
const NEW_FILE_LIMIT = 500;
const failures = [];
const extensions = new Set([".rs", ".ts", ".tsx", ".js", ".jsx"]);
const files = [];

async function visit(relative) {
  for (const entry of await readdir(path.join(root, relative), { withFileTypes: true })) {
    const name = `${relative}/${entry.name}`;
    if (entry.isDirectory()) await visit(name);
    else if (entry.isFile() && extensions.has(path.extname(name))) files.push(name);
  }
}

for (const directory of ["src", "src-tauri/src", "crates/cert-deployment/src"]) {
  await visit(directory);
}

for (const file of files) {
  const text = await readFile(path.join(root, file), "utf8");
  const lines = text ? text.split(/\r?\n/).length - Number(text.endsWith("\n")) : 0;
  const code = text.replace(/^\s*\/\/.*$/gm, "");
  const baselineLines = baseline.get(file);
  if (baselineLines !== undefined) {
    // 存量文件：允许保持原样或变小，禁止继续膨胀。
    if (lines > baselineLines) {
      failures.push(`${file}: ${lines} lines (存量基线 ${baselineLines}，不得继续增长)`);
    }
    if (lines < baselineLines) {
      // 提醒维护者重新生成白名单，让阈值随拆分逐步收紧。
      console.log(`[info] ${file}: ${lines} < ${baselineLines}，可运行 node scripts/gen-large-file-baseline.mjs 收缩白名单`);
    }
  } else if (lines >= NEW_FILE_LIMIT) {
    // 新文件（含新建或此前 <500 行后膨胀的文件）：硬上限 500 行。
    failures.push(`${file}: 新增文件 ${lines} lines (limit: fewer than ${NEW_FILE_LIMIT})`);
  }

  // Check source imports, including type imports. This is a narrow layer guard,
  // not a claim to infer runtime dependencies or the whole application graph.
  if (file.startsWith("src-tauri/src/proxy/")) {
    for (const match of code.matchAll(/^\s*(?:pub(?:\([^)]*\))?\s+)?use\s+crate::commands::([^;]+);/gm)) {
      if (!match[1].startsWith("foundation::")) failures.push(`${file}: proxy imports application commands`);
    }
  }
  if (file === "src-tauri/src/commands/foundation.rs" || file.startsWith("src-tauri/src/commands/foundation/")) {
    if (/^\s*(?:pub(?:\([^)]*\))?\s+)?use\s+(?:crate::commands|super)::(?:config|project|storage_migration)\b/m.test(code)) {
      failures.push(`${file}: foundation imports a business or application service`);
    }
  }
  if (file === "src-tauri/src/commands/utils.rs" || file === "src-tauri/src/commands/config.rs" || file.startsWith("src-tauri/src/commands/config/")) {
    if (/(?:crate::commands::|super::)(?:project|storage_migration|storage_commands)(?:::|\s*[;,{])/.test(code)) {
      failures.push(`${file}: generic configuration or utilities depend on SDK/application migration`);
    }
  }
  if (file === "src-tauri/src/commands/project/registry.rs" && /utils::(?:dev_repo_root|get_resource_dir)\b/.test(code)) {
    failures.push(`${file}: SDK registry locates resources through generic utilities instead of foundation`);
  }
  if (file === "src/moduleCatalog.ts" && /(?:from\s*|import\s*\()\s*["'][^"']*moduleRegistry/.test(code)) {
    failures.push(`${file}: module metadata imports page composition`);
  }
  if (file.startsWith("src/components/shared/ai/") || file.startsWith("src/components/shared/markdown/") || file === "src/components/shared/MonacoEditor.tsx") {
    for (const match of code.matchAll(/(?:from\s*|import\s*\()\s*["'](\.[^"']+)["']/g)) {
      const target = path.posix.normalize(path.posix.join(path.posix.dirname(file), match[1]));
      if (target.startsWith("src/components/") && !target.startsWith("src/components/shared/")) {
        failures.push(`${file}: shared capability imports business UI ${target}`);
      }
    }
  }
}

if (failures.length) {
  console.error(failures.join("\n"));
  process.exitCode = 1;
} else {
  console.log(`Architecture checks passed (${files.length} source files).`);
}
