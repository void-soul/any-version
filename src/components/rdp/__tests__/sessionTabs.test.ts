import { describe, expect, it } from "vitest";
import {
  activateTab,
  closeTab,
  openTab,
  parseGroupPath,
  splitTabs,
  toggleSplit,
  type SessionTab,
  type TabsState,
} from "../sessionTabs";

function tab(id: string, title = id): SessionTab {
  return { id, connectionId: `c_${id}`, title, kind: "terminal" };
}

describe("会话标签", () => {
  it("打开新标签并置为激活", () => {
    const s = openTab({ tabs: [], activeId: null, splitId: null }, tab("a"));
    expect(s.tabs.map((t) => t.id)).toEqual(["a"]);
    expect(s.activeId).toBe("a");
    expect(s.splitId).toBeNull();
  });

  it("同一连接重复打开只激活已有标签，不新建会话", () => {
    let s = openTab({ tabs: [], activeId: null, splitId: null }, tab("a"));
    s = openTab(s, tab("b"));
    s = openTab(s, { ...tab("a2"), connectionId: "c_a", title: "另一个标题" });
    expect(s.tabs).toHaveLength(2);
    expect(s.activeId).toBe("a");
    expect(s.tabs.find((t) => t.id === "a")?.title).toBe("另一个标题");
  });

  it("同一连接的终端与文件管理是两个独立标签，互不顶掉", () => {
    let s = openTab({ tabs: [], activeId: null, splitId: null }, tab("t"));
    s = openTab(s, { id: "f", connectionId: "c_t", title: "c_t · files", kind: "sftp" });
    expect(s.tabs.map((t) => t.id)).toEqual(["t", "f"]);
    // 重复打开同类型才复用
    s = openTab(s, { id: "t2", connectionId: "c_t", title: "改名", kind: "terminal" });
    expect(s.tabs).toHaveLength(2);
    expect(s.activeId).toBe("t");
    expect(s.tabs[0].title).toBe("改名");
  });

  it("关闭激活标签后激活右邻，关闭末位则回到左邻", () => {
    let s: TabsState = { tabs: [], activeId: null, splitId: null };
    s = openTab(s, tab("a"));
    s = openTab(s, tab("b"));
    s = openTab(s, tab("c"));
    s = activateTab(s, "b");
    const afterFirst = closeTab(s, "b");
    expect(afterFirst.activeId).toBe("c");
    const afterSecond = closeTab(afterFirst, "c");
    expect(afterSecond.activeId).toBe("a");
  });

  it("关闭非激活标签不改变激活项", () => {
    let s: TabsState = { tabs: [], activeId: null, splitId: null };
    s = openTab(s, tab("a"));
    s = openTab(s, tab("b"));
    s = activateTab(s, "b");
    expect(closeTab(s, "a").activeId).toBe("b");
  });

  it("关掉最后一个标签后状态清空", () => {
    let s = openTab({ tabs: [], activeId: null, splitId: null }, tab("a"));
    s = closeTab(s, "a");
    expect(s.tabs).toEqual([]);
    expect(s.activeId).toBeNull();
    expect(s.splitId).toBeNull();
  });

  it("分屏把另一个标签放到右侧，且不与自己分屏", () => {
    let s: TabsState = { tabs: [], activeId: null, splitId: null };
    s = openTab(s, tab("a"));
    s = openTab(s, tab("b"));
    // 激活项留在左侧，右侧是「另一个」标签
    const split = toggleSplit(s);
    expect(split.splitId).toBe("a");
    expect(toggleSplit(split).splitId).toBeNull();
    expect(splitTabs(s).map((t) => t.id)).toEqual(["b"]);
    expect(toggleSplit({ ...s, activeId: "a" }).splitId).toBe("b");
    expect(splitTabs({ ...s, activeId: "a", splitId: "b" }).map((t) => t.id)).toEqual(["a", "b"]);
  });

  it("forceNew 允许同一主机开多个终端标签", () => {
    let s = openTab({ tabs: [], activeId: null, splitId: null }, tab("a"));
    s = openTab(
      s,
      { ...tab("a2"), connectionId: "c_a", title: "第二个终端" },
      true,
    );
    expect(s.tabs.map((t) => t.id)).toEqual(["a", "a2"]);
    expect(s.activeId).toBe("a2");
  });

  it("分屏对象被关闭时自动取消分屏", () => {
    let s: TabsState = { tabs: [], activeId: null, splitId: null };
    s = openTab(s, tab("a"));
    s = openTab(s, tab("b"));
    s = toggleSplit(s);
    expect(s.splitId).toBe("a");
    const closed = closeTab(s, "a");
    expect(closed.splitId).toBeNull();
  });
});

describe("分组路径", () => {
  it("空分组归到未分组", () => {
    expect(parseGroupPath("")).toEqual({ label: "", isUngrouped: true });
    expect(parseGroupPath("   ")).toEqual({ label: "", isUngrouped: true });
  });

  it("root 视为未分组（FinalShell 的 parent_id 默认值）", () => {
    expect(parseGroupPath("root")).toEqual({ label: "", isUngrouped: true });
  });

  it("普通分组保留原样并去空白", () => {
    expect(parseGroupPath(" 生产环境 ")).toEqual({ label: "生产环境", isUngrouped: false });
  });
});
