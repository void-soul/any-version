// 生成「存量 ≥500 行文件」白名单，供 check-module-boundaries.mjs 使用（AUDIT-049）。
//
// 白名单默认**只允许收缩**：拆分大文件后直接重新生成即可。
// 若任何存量文件比旧基线更长（新功能确实需要），直接运行会报错，必须显式给理由：
//   node scripts/gen-large-file-baseline.mjs --approve-growth="2026-11-06 约束往返补齐"
// 批准记录会写进 JSON 的 approvals 数组，便于事后审计「为什么放宽过一次」。
import { readdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const out = path.join(root, "scripts/large-file-baseline.json");
const extensions = new Set([".rs", ".ts", ".tsx", ".js", ".jsx"]);
const approveArg = process.argv.find((a) => a.startsWith("--approve-growth="));
const approveReason = approveArg ? approveArg.slice("--approve-growth=".length).trim() : "";
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

const rows = [];
for (const file of files) {
    const text = await readFile(path.join(root, file), "utf8");
    const lines = text ? text.split(/\r?\n/).length - Number(text.endsWith("\n")) : 0;
    if (lines >= 500) rows.push({ file, lines });
}
rows.sort((a, b) => b.lines - a.lines);

let previous = { approvals: [], files: [] };
try {
    previous = JSON.parse(await readFile(out, "utf8"));
} catch {
    // 第一次生成没有旧基线
}
const oldLines = new Map((previous.files || []).map((f) => [f.file, f.lines]));
const growth = rows.filter((r) => oldLines.has(r.file) && r.lines > oldLines.get(r.file));
if (growth.length && !approveReason) {
    console.error("以下存量文件比基线更长，不能直接重新生成白名单：");
    for (const g of growth) {
        console.error(`  ${g.file}: ${oldLines.get(g.file)} -> ${g.lines}`);
    }
    console.error("要么拆分这些文件，要么显式批准增长：");
    console.error('  node scripts/gen-large-file-baseline.mjs --approve-growth="日期 + 理由"');
    process.exit(1);
}

const approvals = [...(previous.approvals || [])];
if (growth.length) {
    approvals.push({
        date: new Date().toISOString().slice(0, 10),
        reason: approveReason,
        growth: growth.map((g) => ({ file: g.file, from: oldLines.get(g.file), to: g.lines })),
    });
}

await writeFile(
    out,
    JSON.stringify({
        note: "存量 ≥500 行文件白名单，只允许收缩；新增文件必须 < 500 行（AUDIT-049）",
        generatedFrom: "scripts/gen-large-file-baseline.mjs",
        approvals,
        files: rows,
    }, null, 2) + "\n",
    "utf8",
);
console.log(`baseline: ${rows.length} files >= 500 lines -> scripts/large-file-baseline.json`);
if (growth.length) console.log(`approved growth: ${growth.length} files (${approveReason}); recorded in approvals`);
