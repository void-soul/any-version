import React, { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ProjectStatus, ProjectCategory } from "./project/types";
import ProjectListPanel from "./project/ProjectListPanel";
import ProjectDetailPanel from "./project/ProjectDetailPanel";

export default function ProjectManager() {
  const [projects, setProjects] = useState<ProjectStatus[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState<ProjectCategory | "all">("all");

  const fetchProjects = useCallback(async (silent = false) => {
    if (!silent) setLoading(true);
    try {
      const list = await invoke<ProjectStatus[]>("project_list");
      setProjects(list);
    } catch (e) {
      console.error("Failed to load projects:", e);
    } finally {
      if (!silent) setLoading(false);
    }
  }, []);

  // 增量更新单个项目（不重新加载整个列表）
  const updateProject = useCallback(async (id: string) => {
    try {
      const updated = await invoke<ProjectStatus>("project_status", { id });
      setProjects(prev => prev.map(p => p.id === id ? updated : p));
    } catch (e) {
      console.error("Failed to update project:", e);
    }
  }, []);

  useEffect(() => { fetchProjects(); }, []);

  const selectedProject = projects.find((p) => p.id === selectedId) ?? null;

  return (
    <div className="h-full select-none px-3 py-2 flex flex-col">
      <div className="flex-1 min-h-0 grid grid-cols-12 gap-2">
        <div className="col-span-4 h-full rounded-xl border border-white/5 overflow-hidden bg-white/[0.01]">
          <ProjectListPanel
            projects={projects}
            selectedId={selectedId}
            onSelect={(p) => setSelectedId(p.id)}
            search={search}
            onSearchChange={setSearch}
            filter={filter}
            onFilterChange={setFilter}
            loading={loading}
            onRefresh={() => fetchProjects()}
          />
        </div>
        <div className="col-span-8 h-full rounded-xl border border-white/5 overflow-hidden bg-white/[0.01]">
          <ProjectDetailPanel
            project={selectedProject}
            onRefresh={async () => { await fetchProjects(); }}
            onProjectUpdate={updateProject}
          />
        </div>
      </div>
    </div>
  );
}
