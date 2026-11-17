import { describe, expect, it } from "vitest";
import { buildTree, descendants, normalizePath, rebasePath, splitPath } from "../groupTree";

const conn = (id: string, group: string) => ({ id, name: id, group });

describe("分组路径规范化", () => {
  it("统一分隔符并去掉多余斜杠", () => {
    expect(normalizePath("  a/b  ")).toBe("a/b");
    expect(normalizePath("/a//b/")).toBe("a/b");
    expect(normalizePath("a\\b")).toBe("a/b");
  });

  it("root 与空串都表示根（未分组）", () => {
    expect(normalizePath("")).toBe("");
    expect(normalizePath("root")).toBe("");
    expect(normalizePath("  ROOT ")).toBe("");
  });

  it("拆分出层级名称", () => {
    expect(splitPath("a/b/c")).toEqual(["a", "b", "c"]);
    expect(splitPath("")).toEqual([]);
  });
});

describe("文件夹树构建", () => {
  it("由连接路径推导出各级文件夹", () => {
    const tree = buildTree([conn("x", "prod/web"), conn("y", "prod/db"), conn("z", "")], []);
    expect(tree.folders.map((f) => f.path)).toEqual(["prod"]);
    const prod = tree.folders[0];
    expect(prod.folders.map((f) => f.path)).toEqual(["prod/db", "prod/web"]);
    expect(tree.items.map((i) => i.id)).toEqual(["z"]);
  });

  it("空文件夹也要出现（仅靠连接推导会丢）", () => {
    const tree = buildTree([], ["ops", "ops/backup"]);
    expect(tree.folders.map((f) => f.path)).toEqual(["ops"]);
    expect(tree.folders[0].folders.map((f) => f.path)).toEqual(["ops/backup"]);
    expect(tree.folders[0].folders[0].items).toEqual([]);
  });

  it("层级深度与显示名正确", () => {
    const tree = buildTree([], ["a/b/c"]);
    const a = tree.folders[0];
    const b = a.folders[0];
    const c = b.folders[0];
    expect([a.depth, b.depth, c.depth]).toEqual([0, 1, 2]);
    expect([a.name, b.name, c.name]).toEqual(["a", "b", "c"]);
  });
});

describe("子路径与前缀改写", () => {
  it("只承认真正的子级，不把 a/bc 当成 a/b 的后代", () => {
    const all = ["a", "a/b", "a/bc"];
    expect(descendants("a", all)).toEqual(["a", "a/b", "a/bc"]);
    expect(descendants("a/b", all)).toEqual(["a/b"]);
  });

  it("重命名文件夹时同步改写所有后代路径", () => {
    expect(rebasePath("a/b/c", "a", "x")).toBe("x/b/c");
    expect(rebasePath("a", "a", "x")).toBe("x");
    expect(rebasePath("ab", "a", "x")).toBe("ab");
    expect(rebasePath("a/b", "a/b", "")).toBe("");
  });

  it("移到根时把路径清空而不是留下前导斜杠", () => {
    expect(rebasePath("a/b", "a/b", "")).toBe("");
    expect(rebasePath("a/b/c", "a", "")).toBe("b/c");
  });
});
