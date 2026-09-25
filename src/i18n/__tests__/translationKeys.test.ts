import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * i18n key 完整性回归。
 *
 * 真实事故：新增词条时用错了锚点（`buddy.conflictWorkspace`），整批 key 落进了
 * buddy 段，而代码里取的是 `toollaunch.*` —— 界面直接把 key 显示出来了。
 * 这个测试直接对着**代码里真实用到的 key** 校验，比人工比对 JSON 可靠：
 * 只要某个 `t("a.b")` 在任一份语言包里不存在，就会在这里炸掉。
 */

const SRC_ROOT = join(process.cwd(), "src");
const LOCALES = {
  zh: JSON.parse(readFileSync(join(SRC_ROOT, "i18n/locales/zh/translation.json"), "utf-8")),
  en: JSON.parse(readFileSync(join(SRC_ROOT, "i18n/locales/en/translation.json"), "utf-8")),
};

function walk(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) {
      walk(full, out);
    } else if (/\.tsx?$/.test(name)) {
      out.push(full);
    }
  }
  return out;
}

/** 去掉注释：文档里的用法示例（如 `t("xxx.title")`）不是真实调用，不该参与校验。 */
function stripComments(text: string): string {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/(^|\s)\/\/[^\n]*/g, "$1");
}

/** 收集代码里静态写死的 i18n key（动态拼接的 `${...}` 跳过，无法静态校验）。 */
function collectKeys(): string[] {
  const keys = new Set<string>();
  for (const file of walk(SRC_ROOT)) {
    const text = stripComments(readFileSync(file, "utf-8"));
    // t("a.b") / t('a.b')；含 ${ 或 + 的动态 key 不匹配
    for (const m of text.matchAll(/\bt\(\s*["']([A-Za-z0-9_]+(?:\.[A-Za-z0-9_]+)+)["']/g)) {
      keys.add(m[1]);
    }
  }
  return [...keys].sort();
}

function hasKey(tree: unknown, key: string): boolean {
  let node: unknown = tree;
  for (const part of key.split(".")) {
    if (!node || typeof node !== "object") return false;
    node = (node as Record<string, unknown>)[part];
  }
  return node !== undefined;
}

describe("translation keys", () => {
  const keys = collectKeys();

  it("代码里确实取到了一批静态 key（防止正则失效导致空跑）", () => {
    expect(keys.length).toBeGreaterThan(100);
  });

  it("每个用到的 key 在中英文包里都存在（缺一个就会把 key 显示给用户）", () => {
    const missingZh = keys.filter((k) => !hasKey(LOCALES.zh, k));
    const missingEn = keys.filter((k) => !hasKey(LOCALES.en, k));
    expect(missingZh, `中文包缺失：${missingZh.slice(0, 10).join(", ")}`).toEqual([]);
    expect(missingEn, `英文包缺失：${missingEn.slice(0, 10).join(", ")}`).toEqual([]);
  });
});
