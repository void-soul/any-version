/** 多会话标签的纯状态机，便于单测；组件只负责渲染与调用这些函数。 */
export type SessionKind = "terminal" | "sftp" | "forward" | "process" | "net";

export interface SessionTab {
  /** 会话唯一 id（每个标签一条独立后端会话）。 */
  id: string;
  /** 连接 id：同一连接重复打开时复用已有标签，避免重复建连。 */
  connectionId: string;
  title: string;
  kind: SessionKind;
}

export interface TabsState {
  tabs: SessionTab[];
  activeId: string | null;
  /** 左侧主视图 + 右侧分屏；null 表示单屏。 */
  splitId: string | null;
}

/**
 * 打开标签。去重键是 `connectionId + kind`：
 * 同一连接可以同时开「终端」和「文件管理」，但同类型重复打开只激活已有标签。
 */
export function openTab(state: TabsState, tab: SessionTab, forceNew = false): TabsState {
  // forceNew：同一主机允许开多个终端（FinalShell 的常见用法）；
  // 文件管理/转发仍复用，避免重复建连。
  const existing = forceNew
    ? undefined
    : state.tabs.find(
        (t) => t.connectionId === tab.connectionId && t.kind === tab.kind,
      );
  if (existing) {
    return {
      ...state,
      tabs: state.tabs.map((t) => (t.id === existing.id ? { ...t, title: tab.title } : t)),
      activeId: existing.id,
    };
  }
  return { ...state, tabs: [...state.tabs, tab], activeId: tab.id };
}

export function activateTab(state: TabsState, id: string): TabsState {
  if (!state.tabs.some((t) => t.id === id)) return state;
  return { ...state, activeId: id };
}

/** 关闭标签：优先激活右邻，没有右邻则取左邻；关掉分屏目标时自动取消分屏。 */
export function closeTab(state: TabsState, id: string): TabsState {
  const index = state.tabs.findIndex((t) => t.id === id);
  if (index < 0) return state;
  const tabs = state.tabs.filter((t) => t.id !== id);
  if (tabs.length === 0) return { tabs: [], activeId: null, splitId: null };
  const activeId = state.activeId === id
    ? tabs[Math.min(index, tabs.length - 1)].id
    : state.activeId;
  const splitId = state.splitId === id ? null : state.splitId;
  return { tabs, activeId, splitId };
}

/** 切换分屏：把「非激活的另一个标签」放到右侧；只有一个标签时不生效。 */
export function toggleSplit(state: TabsState): TabsState {
  if (state.splitId) return { ...state, splitId: null };
  const other = state.tabs.find((t) => t.id !== state.activeId);
  return other ? { ...state, splitId: other.id } : state;
}

/** 实际要渲染的标签（单屏 1 个、分屏 2 个）。 */
export function splitTabs(state: TabsState): SessionTab[] {
  const left = state.tabs.find((t) => t.id === state.activeId);
  const right = state.tabs.find((t) => t.id === state.splitId);
  return [left, right].filter((t): t is SessionTab => Boolean(t));
}

export interface GroupPath {
  label: string;
  isUngrouped: boolean;
}

/** FinalShell 的 `parent_id` 默认是 "root"，语义等同未分组。 */
export function parseGroupPath(raw: string): GroupPath {
  const label = raw.trim();
  if (label === "" || label === "root") return { label: "", isUngrouped: true };
  return { label, isUngrouped: false };
}
