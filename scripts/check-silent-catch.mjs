// AUDIT-016 静态闸：禁止新增静默失败的 catch。
// 允许：显式调用 ignoreInvokeError(e, "理由") 的路径（卸载清理/预览降级/能力探测）。
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const SRC = join(ROOT, "src");
const EMPTY_ARROW = /\.catch\(\(\)\s*=>\s*\{\s*\}\)/g;
const EMPTY_BLOCK = /\bcatch\s*\{\s*\}/g;

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) {
      if (name !== "__tests__" && name !== "node_modules") walk(full, out);
    } else if (/\.(ts|tsx)$/.test(name)) {
      out.push(full);
    }
  }
  return out;
}

const bad = [];
for (const file of walk(SRC)) {
  const text = readFileSync(file, "utf8");
  // 去掉注释行后再匹配，避免把「说明文字」当代码（含 JSDoc 的 `*` 行）。
  const lines = text.split("\n").map((l) => {
    const t = l.trim();
    return t.startsWith("//") || t.startsWith("*") || t.startsWith("/*") ? "" : l;
  });
  lines.forEach((line, i) => {
    if (EMPTY_ARROW.test(line) || EMPTY_BLOCK.test(line)) {
      bad.push(`${relative(ROOT, file)}:${i + 1}  ${line.trim().slice(0, 120)}`);
    }
    EMPTY_ARROW.lastIndex = 0;
    EMPTY_BLOCK.lastIndex = 0;
  });
}

if (bad.length) {
  console.error(`FAIL: 发现 ${bad.length} 处静默 catch（AUDIT-016）：`);
  for (const line of bad.slice(0, 30)) console.error("  " + line);
  console.error("修复：改用 reportInvokeError(e, context)；确可忽略时用 ignoreInvokeError(e, 理由)。");
  process.exit(1);
}
console.log("PASS: 无静默 catch");
