import { describe, expect, it } from "vitest";

/**
 * AUDIT-050 单一真源守护：防止新代码重新散落出「同一规则的另一份实现」。
 * 用 Vite 的 raw glob 直接读源码原文，不依赖 node:fs / @types/node。
 */
const TS = import.meta.glob<string>("/src/**/*.{ts,tsx}", {
    query: "?raw",
    import: "default",
    eager: true,
});
const RS = import.meta.glob<string>("/src-tauri/src/**/*.rs", {
    query: "?raw",
    import: "default",
    eager: true,
});

/** 去掉注释行，避免把「说明文字里的示例代码」当成真实代码。 */
const withoutComments = (text: string) =>
    text
        .split("\n")
        .map((l) => {
            const t = l.trim();
            return t.startsWith("//") || t.startsWith("*") || t.startsWith("/*") ? "" : l;
        })
        .join("\n");

const prodTs = (path: string) =>
    path.endsWith(".d.ts") || path.includes("/__tests__/")
        ? ""
        : withoutComments(TS[path] ?? "");
const allTs = () => Object.keys(TS).map(prodTs).join("\n");
const allRs = () => Object.values(RS).join("\n");

describe("横切能力单一真源", () => {
    it("静默失败必须用统一上报封装，不出现裸空 catch", () => {
        // 与 scripts/check-silent-catch.mjs 保持同一口径：逐行匹配，注释行已剔除。
        const offenders = allTs()
            .split("\n")
            .filter((line) => /\.catch\(\(\)\s*=>\s*\{\s*\}\)|\bcatch\s*\{\s*\}/.test(line));
        expect(offenders).toEqual([]);
        expect(allTs()).toContain("reportInvokeError");
        expect(allTs()).toContain("ignoreInvokeError");
    });

    it("失败上报封装只有一份实现", () => {
        const defs = Object.keys(TS).filter((p) => /\bexport function reportInvokeError\b/.test(TS[p] ?? ""));
        expect(defs).toEqual(["/src/utils/invokeOrReport.ts"]);
    });

    it("树渲染定义处必须带深度上限或环检测", () => {
        // 只检查**定义**（`const renderXxxNodes =`），调用方不需要重复声明上限。
        const offenders = Object.keys(TS).filter((p) => {
            const s = prodTs(p);
            if (!/\b(?:const|function)\s+render(?:Folder)?Nodes\b/.test(s)) return false;
            return !/MAX_TREE_DEPTH|flattenTree|maxDepth|pathIds/.test(s);
        });
        expect(offenders).toEqual([]);
    });

    it("Rust 端统一时间/随机/常量真源存在且被引用", () => {
        const text = allRs();
        for (const path of [
            "/src-tauri/src/commands/foundation/time.rs",
            "/src-tauri/src/commands/foundation/secure_random.rs",
            "/src-tauri/src/commands/foundation/defaults.rs",
            "/src-tauri/src/commands/foundation/migrate.rs",
        ]) {
            expect(RS[path], `缺少 ${path}`).toBeDefined();
        }
        expect(text).toContain("foundation::defaults::ports");
        expect(text).toContain("foundation::secure_random");
        expect(text).toContain("foundation::migrate");
    });

    it("生产 Rust 代码不使用裸锁 unwrap", () => {
        const offenders = Object.entries(RS).filter(([p, s]) => {
            if (p.includes("/tests/") || p.endsWith("tests.rs")) return false;
            const beforeTests = s.split("#[cfg(test)]")[0];
            return /\.(lock|read|write)\(\)\s*\.unwrap\(\)/.test(beforeTests);
        });
        expect(offenders.map(([p]) => p)).toEqual([]);
    });
});
