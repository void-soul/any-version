// 国际化进度扫描：统计各 TS/TSX 文件里的中文字符串密度，按「性价比」排序，
// 给出后续提取翻译的优先级。
//
// 用法：
//   node scripts/scan-i18n-density.mjs            # 全部文件，按优先级排序
//   node scripts/scan-i18n-density.mjs --top 20   # 只看前 20 名
//   node scripts/scan-i18n-density.mjs src/components/mindmap  # 只看某目录
//
// 判据说明：
//   - 中文行数：包含 CJK 字符的代码行（粗粒度，同一行可能有多个字符串）
//   - 中文串数：按中文连续片段估算（"你好" 与 "你好世界" 都算 1 处，粗粒度）
//   - 优先级 = 中文串数 / 总行数（密度越高，单次提取性价比越高）
//   - 已国际化：检测文件是否 import 了 react-i18next（含 useTranslation/t 调用）
import { readdirSync, statSync, readFileSync } from "node:fs";
import { join, extname, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const CJK = /[\u4e00-\u9fff]/;
// 匹配引号内的中文片段（单/双引号，含模板串里的中文）
const CJK_SEG = /["'`][^"'`\n]*[\u4e00-\u9fff][^"'`\n]*["'`]/g;

const ROOT = join(fileURLToPath(new URL(".", import.meta.url)), "..");
const args = process.argv.slice(2);
let topN = Infinity;
let onlyDir = null;
let excludeTests = false;
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--top") {
    topN = parseInt(args[i + 1], 10) || Infinity;
    i++; // 跳过数值参数，避免被当成目录
  } else if (args[i] === "--exclude-tests") {
    excludeTests = true;
  } else if (!args[i].startsWith("--")) {
    onlyDir = args[i];
  }
}

/** 递归收集目录下的 .ts/.tsx 文件 */
function collectFiles(dir, out = []) {
  for (const name of readdirSync(dir)) {
    if (name === "node_modules" || name === ".git" || name === "dist" || name === "target") continue;
    const full = join(dir, name);
    const st = statSync(full);
    if (st.isDirectory()) out = collectFiles(full, out);
    else if (extname(full) === ".ts" || extname(full) === ".tsx") out.push(full);
  }
  return out;
}

// Windows 下路径用反斜杠、参数常用正斜杠：统一转正斜杠，按「相对仓库根」的前缀匹配。
const norm = (p) => p.replace(/\\/g, "/");
const dirPrefix = onlyDir ? norm(onlyDir).replace(/\/+$/, "") + "/" : null;
const files = collectFiles(ROOT).filter((f) => {
  const rel = norm(relative(ROOT, f));
  if (excludeTests && (/__tests__|\/test(?:s)?\/|(?:\.|_)test\.tsx?$/.test(rel))) return false;
  if (!dirPrefix) return true;
  return rel.startsWith(dirPrefix) || rel === dirPrefix.slice(0, -1);
});

const rows = files.map((file) => {
  const content = readFileSync(file, "utf8");
  const lines = content.split(/\r?\n/);
  const cjkLines = lines.filter((l) => CJK.test(l)).length;
  const segments = content.match(CJK_SEG)?.length ?? 0;
  const totalLines = lines.length;
  const i18nReady =
    /react-i18next|useTranslation|from ["']\.\.?\/.*i18n/.test(content) || /\bt\(["']/.test(content);
  return {
    file: norm(relative(ROOT, file)),
    totalLines,
    cjkLines,
    segments,
    density: totalLines ? segments / totalLines : 0,
    i18nReady,
  };
});

rows.sort((a, b) => b.density - a.density || b.segments - a.segments);

const shown = rows.slice(0, topN);
console.log("文件".padEnd(58) + "行数".padStart(6) + "中文行".padStart(7) + "中文串".padStart(7) + "密度".padStart(7) + " 已国际化");
console.log("-".repeat(95));
for (const r of shown) {
  console.log(
    r.file.padEnd(58) +
      String(r.totalLines).padStart(6) +
      String(r.cjkLines).padStart(7) +
      String(r.segments).padStart(7) +
      r.density.toFixed(3).padStart(7) +
      (r.i18nReady ? "  ✅" : "  —")
  );
}

const pending = rows.filter((r) => !r.i18nReady);
const done = rows.filter((r) => r.i18nReady);
const totalSeg = rows.reduce((s, r) => s + r.segments, 0);
const pendingSeg = pending.reduce((s, r) => s + r.segments, 0);
console.log("-".repeat(95));
console.log(`合计: ${rows.length} 个文件, 中文串约 ${totalSeg} 处 | 已国际化 ${done.length} 个 (${done.reduce((s, r) => s + r.segments, 0)} 处), 待提取 ${pending.length} 个 (${pendingSeg} 处)`);
console.log("\n优先级建议: 密度高 + 行数大的文件先做（单位工作量翻译面最大）。");
console.log("剔除建议: 密度高但行数小的文件（如工具函数）可后置，交给低频场景。");
