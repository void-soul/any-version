import { describe, expect, it } from "vitest";
import { AT_FILE_LIMIT, matchProjectFiles } from "../atFiles";

const files = [
  "README.md",
  "src/App.tsx",
  "src/components/ai/ToolLauncher.tsx",
  "src/components/launcher/LauncherPanel.tsx",
  "src/components/launcher/types.ts",
  "src/utils/launcherStore.ts",
  "docs/launcher.md",
];

describe("matchProjectFiles", () => {
  it("空关键词与全空白关键词都不返回候选", () => {
    expect(matchProjectFiles(files, "")).toEqual([]);
    expect(matchProjectFiles(files, "   ")).toEqual([]);
  });

  it("大小写不敏感地按文件名匹配", () => {
    expect(matchProjectFiles(files, "TOOLLAUNCHER")).toEqual(["src/components/ai/ToolLauncher.tsx"]);
  });

  it("文件名前缀命中排在文件名包含与路径段命中之前", () => {
    const got = matchProjectFiles(files, "launcher");
    // 文件名以 launcher 开头的三条（docs/launcher.md、launcherStore.ts、LauncherPanel.tsx）
    // 全部排在「仅文件名包含」的 ToolLauncher.tsx 之前，后者又排在「按路径段命中」的
    // launcher/types.ts 之前
    const prefixHits = ["docs/launcher.md", "src/utils/launcherStore.ts", "src/components/launcher/LauncherPanel.tsx"];
    const lastPrefix = Math.max(...prefixHits.map((p) => got.indexOf(p)));
    expect(lastPrefix).toBeLessThan(got.indexOf("src/components/ai/ToolLauncher.tsx"));
    expect(got.indexOf("src/components/ai/ToolLauncher.tsx")).toBeLessThan(got.indexOf("src/components/launcher/types.ts"));
  });

  it("路径段前缀命中优先于路径中间恰好出现的子串", () => {
    const got = matchProjectFiles(files, "components/ai");
    expect(got[0]).toBe("src/components/ai/ToolLauncher.tsx");
  });

  it("同一档命中时短路径在前（通常更接近意图）", () => {
    const got = matchProjectFiles(files, "launcher");
    // 两条同属「文件名前缀」档，短的排前面
    expect(got.indexOf("src/utils/launcherStore.ts")).toBeLessThan(
      got.indexOf("src/components/launcher/LauncherPanel.tsx"),
    );
  });

  it("尊重条数上限，默认上限放宽到 12", () => {
    const many = Array.from({ length: 30 }, (_, i) => `src/f${i}.ts`);
    expect(matchProjectFiles(many, "f")).toHaveLength(AT_FILE_LIMIT);
    expect(matchProjectFiles(many, "f", 3)).toHaveLength(3);
    expect(matchProjectFiles(many, "f", 0)).toEqual([]);
  });

  it("没有命中时返回空数组而不是抛错", () => {
    expect(matchProjectFiles(files, "zzz-not-there")).toEqual([]);
  });

  it("不修改入参数组", () => {
    const input = [...files];
    matchProjectFiles(input, "launcher");
    expect(input).toEqual(files);
  });
});
