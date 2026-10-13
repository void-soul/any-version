import { useEffect, useMemo, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openPath } from "@tauri-apps/plugin-opener";

/**
 * 「项目」是技能与 MCP 共用的概念（都是「装给某个项目」），后端也共用同一个
 * `projects.json`。这里收敛成**一份模块级状态**：技能页与 MCP 页读的是同一个快照。
 *
 * ## 为什么必须是模块级单例，而不是各页各拿一份 state
 *
 * 之前这个文件导出的是一个普通 hook，内部是纯 `useState`——所谓「共用」只是共用代码，
 * 每个组件各拿各的实例副本。而 `AiPanel` 用 `mountedTabs` 让两个页面 keep-alive 常驻、
 * 互不卸载，于是：在技能页删掉一个项目，MCP 页手里的数组永远不变（后端 `projects.json`
 * 其实已经改了，**只有前端不同步**），而且 MCP 页连刷新按钮都救不回来。
 *
 * `useSyncExternalStore` 在这里比另引一个状态库更合适：本仓库没有 zustand，
 * 而「谁是唯一数据源」这件事用模块级快照表达最直白——不存在两份，也就不存在同步问题。
 */
export interface SkillProject {
  id: string;
  path: string;
  name: string;
  addedAt: string;
}

/** 某项目的技能计数（后端 list_project_skill_counts；只有技能侧用） */
export interface ProjectSkillCount {
  projectId: string;
  total: number;
  linked: number;
  local: number;
  broken: number;
}

interface ProjectsState {
  projects: SkillProject[];
  selProject: string;
  picked: Set<string>;
  counts: Record<string, ProjectSkillCount>;
  loading: boolean;
}

const EMPTY_COUNTS: Record<string, ProjectSkillCount> = {};

let state: ProjectsState = {
  projects: [],
  selProject: "",
  picked: new Set(),
  counts: EMPTY_COUNTS,
  loading: true,
};

const listeners = new Set<() => void>();

function setState(patch: Partial<ProjectsState>) {
  // 整体换新对象：useSyncExternalStore 靠引用比较判断要不要重渲染，
  // 原地改字段的话 React 看不到变化（这正是「点了没反应」的典型成因）。
  state = { ...state, ...patch };
  for (const l of listeners) l();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

const getSnapshot = () => state;

/** 正在进行的加载（用于去重并发调用，两个页面同时挂载时只发一次 IPC） */
let inFlight: Promise<void> | null = null;

/** 技能计数：一次扫完所有项目，比逐个 `list_project_skills` 便宜得多 */
async function loadCounts(): Promise<Record<string, ProjectSkillCount>> {
  try {
    const list = await invoke<ProjectSkillCount[]>("list_project_skill_counts");
    const map: Record<string, ProjectSkillCount> = {};
    for (const c of list) map[c.projectId] = c;
    return map;
  } catch (e) {
    console.error("读取项目技能数失败:", e);
    return state.counts;
  }
}

/**
 * 从后端重读项目清单与计数。
 *
 * 并发调用会合并成同一次请求：两个页面同时挂载时都调它，
 * 不去重就会发两遍 IPC、把盘扫两遍（而两页拿到的是同一份数据）。
 */
export async function reloadProjects(): Promise<void> {
  if (inFlight) return inFlight;
  inFlight = (async () => {
    try {
      const list = await invoke<SkillProject[]>("list_projects");
      const counts = await loadCounts();
      setState({
        projects: list,
        counts,
        // 选中项兜底：项目被删后不能继续停在它上面
        selProject:
          state.selProject && list.some((p) => p.id === state.selProject)
            ? state.selProject
            : list[0]?.id ?? "",
        // 勾选里若有已不存在的项目要清掉，否则批量安装会带着幽灵 id 去调命令
        picked: new Set([...state.picked].filter((id) => list.some((p) => p.id === id))),
      });
    } catch (e) {
      console.error("加载项目列表失败:", e);
    } finally {
      setState({ loading: false });
    }
  })();
  try {
    await inFlight;
  } finally {
    inFlight = null;
  }
}

/** 只重算技能计数（装/删技能后用，不必重读项目清单） */
export async function reloadProjectCounts(): Promise<void> {
  setState({ counts: await loadCounts() });
}

export const setSelProject = (id: string) => setState({ selProject: id });

/**
 * 设置勾选集合。**同时接受值与 updater 函数**——它顶替的是 React 的 state setter，
 * 而三个调用点里有一半用的是 `(prev) => next` 这种形式。
 * 去掉 updater 支持的话，那些地方要改成先读 `picked` 再算，在事件回调里读到旧值。
 */
export const setPicked = (next: Set<string> | ((prev: Set<string>) => Set<string>)) =>
  setState({ picked: typeof next === "function" ? next(state.picked) : next });

export function togglePick(id: string) {
  const next = new Set(state.picked);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  setState({ picked: next });
}

/**
 * 删除项目（后端已同步落盘，这里再 reload 让**两个页面**同时看到结果）。
 *
 * 注意：删的只是「项目清单里的一项」，不会去动项目目录里的 `.agents/skills`、
 * `.mcp.json` 或已安装的技能——那些是用户文件，后端刻意不做连带删除。
 */
export async function removeProject(id: string): Promise<void> {
  await invoke("remove_project", { id });
  await reloadProjects();
}

export async function openProjectDir(path: string): Promise<void> {
  await openPath(path);
}

/**
 * 读共享的项目状态。参数已去掉 `withCounts`：计数现在跟着清单一起加载，
 * 「技能页要计数、MCP 页不要」的分叉已经没有意义（那个 IPC 只数目录条目，很便宜）。
 */
export function useProjects() {
  const snap = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  // 首次挂载时拉一次。两个页面同时挂载也只发一次 IPC（reloadProjects 内部去重）。
  useEffect(() => {
    void reloadProjects();
  }, []);

  const allPicked = useMemo(
    () => snap.projects.length > 0 && snap.picked.size === snap.projects.length,
    [snap.projects.length, snap.picked.size],
  );

  return {
    projects: snap.projects,
    selProject: snap.selProject,
    setSelProject,
    picked: snap.picked,
    togglePick,
    setPicked,
    allPicked,
    counts: snap.counts,
    loading: snap.loading,
    reload: reloadProjects,
    reloadCounts: reloadProjectCounts,
    openProjectDir,
    removeProject,
  };
}

/** 供不方便用 hook 的地方（如事件回调里）直接读当前快照 */
export const getProjectsSnapshot = getSnapshot;
