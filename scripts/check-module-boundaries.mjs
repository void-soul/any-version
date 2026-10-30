import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
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

for (const directory of ["src", "src-tauri/src", "cert-receiver/src", "crates/cert-deployment/src"]) {
  await visit(directory);
}

for (const file of files) {
  const text = await readFile(path.join(root, file), "utf8");
  const lines = text ? text.split(/\r?\n/).length - Number(text.endsWith("\n")) : 0;
  const code = text.replace(/^\s*\/\/.*$/gm, "");
  if (lines >= 1000) failures.push(`${file}: ${lines} lines (limit: fewer than 1000)`);

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
