import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openPath } from "@tauri-apps/plugin-opener";

/**
 * 「项目」是技能与 MCP 共用的概念（都是「装给某个项目」），后端也共用同一个
 * `projects.json`。这里收敛成一份状态：**两个页面必须用同一个 hook**。
 *
 * 之前各管各的，后果很具体：在技能页新加一个项目，MCP 页不会刷新（要切页或手动刷），
 * 反过来也一样；两边项目列表的 UI 也已经开始分叉（MCP 侧多了勾选框、行距不同）。
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

export function useProjects(options?: { withCounts?: boolean }) {
  const withCounts = options?.withCounts ?? false;
  const [projects, setProjects] = useState<SkillProject[]>([]);
  const [selProject, setSelProjectRaw] = useState("");
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [counts, setCounts] = useState<Record<string, ProjectSkillCount>>({});
  const [loading, setLoading] = useState(true);

  const reloadCounts = useCallback(async () => {
    if (!withCounts) return;
    try {
      const list = await invoke<ProjectSkillCount[]>("list_project_skill_counts");
      const map: Record<string, ProjectSkillCount> = {};
      for (const c of list) map[c.projectId] = c;
      setCounts(map);
    } catch (e) {
      console.error("读取项目技能数失败:", e);
    }
  }, [withCounts]);

  const reload = useCallback(async () => {
    try {
      const list = await invoke<SkillProject[]>("list_projects");
      setProjects(list);
      // 选中项兜底：项目被删后不能继续停在它上面
      setSelProjectRaw((cur) =>
        cur && list.some((p) => p.id === cur) ? cur : list[0]?.id ?? "",
      );
      // 勾选里若有已不存在的项目要清掉，否则批量安装会带着幽灵 id 去调命令
      setPicked((prev) => {
        const next = new Set([...prev].filter((id) => list.some((p) => p.id === id)));
        return next.size === prev.size ? prev : next;
      });
      void reloadCounts();
    } catch (e) {
      console.error("加载项目列表失败:", e);
    } finally {
      setLoading(false);
    }
  }, [reloadCounts]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const togglePick = useCallback((id: string) => {
    setPicked((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const openProjectDir = useCallback(async (path: string) => {
    await openPath(path);
  }, []);

  const removeProject = useCallback(
    async (id: string) => {
      await invoke("remove_project", { id });
      await reload();
    },
    [reload],
  );

  const allPicked = useMemo(
    () => projects.length > 0 && picked.size === projects.length,
    [projects.length, picked.size],
  );

  return {
    projects,
    selProject,
    setSelProject: setSelProjectRaw,
    picked,
    togglePick,
    setPicked,
    allPicked,
    counts,
    loading,
    reload,
    reloadCounts,
    openProjectDir,
    removeProject,
  };
}
