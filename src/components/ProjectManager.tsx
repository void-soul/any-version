import React, { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw, FolderKanban } from "lucide-react";
import type { ProjectStatus, ProjectCategory } from "./project/types";
import ProjectListPanel from "./project/ProjectListPanel";
import ProjectDetailPanel from "./project/ProjectDetailPanel";

export default function ProjectManager() {
  const [projects, setProjects] = useState<ProjectStatus[]>([]);
  const [selectedProject, setSelectedProject] = useState<ProjectStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState<ProjectCategory | "all">("all");

  const fetchProjects = useCallback(async () => {
    setLoading(true);
    try {
      const list = await invoke<ProjectStatus[]>("project_list");
      setProjects(list);
      // 同步更新选中项
      if (selectedProject) {
        const updated = list.find((p) => p.id === selectedProject.id);
        if (updated) setSelectedProject(updated);
      }
    } catch (e) {
      console.error("Failed to load projects:", e);
    } finally {
      setLoading(false);
    }
  }, [selectedProject]);

  useEffect(() => {
    fetchProjects();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const handleSelect = (p: ProjectStatus) => {
    setSelectedProject(p);
  };

  return (
    <div className="flex-1 p-8 overflow-y-auto space-y-6 h-screen select-none flex flex-col">
      {/* 头部 */}
      <div className="flex items-center justify-between flex-shrink-0">
        <div className="flex items-center gap-3">
          <div className="w-10 h-10 rounded-xl bg-blue-600/20 border border-blue-500/20 flex items-center justify-center">
            <FolderKanban className="w-5 h-5 text-blue-400" />
          </div>
          <div>
            <h2 className="text-xl font-semibold text-white tracking-wide">项目管理</h2>
            <p className="text-xs text-slate-400 mt-0.5">
              统一管理开发语言、工具和本地服务的版本、环境变量与配置。所有操作透明可见。
            </p>
          </div>
        </div>
        <button
          onClick={fetchProjects}
          disabled={loading}
          className="flex items-center gap-2 px-3.5 py-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-xl text-xs border border-white/5 transition-all cursor-pointer"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${loading ? "animate-spin" : ""}`} />
          刷新列表
        </button>
      </div>

      {/* 主体：左右两栏 */}
      <div className="grid grid-cols-1 lg:grid-cols-12 gap-6 flex-1 min-h-0">
        {/* 左侧：项目列表 */}
        <div className="lg:col-span-4 h-[calc(100vh-180px)]">
          <ProjectListPanel
            projects={projects}
            selectedId={selectedProject?.id ?? null}
            onSelect={handleSelect}
            search={search}
            onSearchChange={setSearch}
            filter={filter}
            onFilterChange={setFilter}
            loading={loading}
          />
        </div>

        {/* 右侧：项目详情 */}
        <div className="lg:col-span-8 h-[calc(100vh-180px)]">
          <ProjectDetailPanel
            project={selectedProject}
            onRefresh={fetchProjects}
          />
        </div>
      </div>
    </div>
  );
}
