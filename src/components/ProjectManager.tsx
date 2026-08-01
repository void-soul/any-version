import React, { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ProjectStatus, ProjectCategory } from "./project/types";
import ProjectListPanel from "./project/ProjectListPanel";
import ProjectDetailPanel from "./project/ProjectDetailPanel";

interface ProjectManagerProps {
  selectedId: string | null;
  onSelectId: (id: string | null) => void;
}

export default function ProjectManager({ selectedId, onSelectId }: ProjectManagerProps) {
  const [projects, setProjects] = useState<ProjectStatus[]>([]);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState<ProjectCategory | "all">("all");

  // 跟踪已懒加载完整状态的项目 ID（避免重复请求 / 无限循环）
  const enrichedIds = useRef<Set<string>>(new Set());
  // 标记当前列表是否为快速加载（跳过了缓存大小计算）
  const isFastLoaded = useRef(false);

  // 左右分栏拖拽
  const [leftWidth, setLeftWidth] = useState(300);
  const [dragging, setDragging] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  const handleResizeStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setDragging(true);
  }, []);

  useEffect(() => {
    if (!dragging) return;

    const handleMouseMove = (e: MouseEvent) => {
      if (!containerRef.current) return;
      const rect = containerRef.current.getBoundingClientRect();
      const x = e.clientX - rect.left;
      const minW = 200;
      const maxW = rect.width * 0.7;
      setLeftWidth(Math.min(maxW, Math.max(minW, x)));
    };

    const handleMouseUp = () => setDragging(false);

    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleMouseUp);
    return () => {
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
    };
  }, [dragging]);

  const fetchProjects = useCallback(async (silent = false, fast = false) => {
    if (!silent) setLoading(true);
    try {
      const cmd = fast ? "project_list_fast" : "project_list";
      const list = await invoke<ProjectStatus[]>(cmd);
      setProjects(list);
      isFastLoaded.current = fast;
      if (!fast) {
        // 完整加载后所有项目已有完整数据，清空 enriched 集合
        enrichedIds.current = new Set(list.map(p => p.id));
      } else {
        enrichedIds.current = new Set();
      }
    } catch (e) {
      console.error("Failed to load projects:", e);
    } finally {
      if (!silent) setLoading(false);
    }
  }, []);

  // 增量更新单个项目（不重新加载整个列表，包含缓存大小等完整信息）
  const updateProject = useCallback(async (id: string) => {
    try {
      const updated = await invoke<ProjectStatus>("project_status", { id });
      setProjects(prev => prev.map(p => p.id === id ? updated : p));
    } catch (e) {
      console.error("Failed to update project:", e);
    }
  }, []);

  // 初次加载使用快速模式（跳过缓存大小计算），提升列表加载速度
  useEffect(() => { fetchProjects(false, true); }, []);

  // 选中项目时懒加载完整状态（包括缓存大小），仅快速加载模式下触发
  useEffect(() => {
    if (!selectedId || !isFastLoaded.current) return;
    if (enrichedIds.current.has(selectedId)) return;
    enrichedIds.current.add(selectedId);
    updateProject(selectedId);
  }, [selectedId, updateProject]);

  // 自动选中第一个项目
  useEffect(() => {
    if (projects.length > 0 && !selectedId) {
      onSelectId(projects[0].id);
    }
  }, [projects, selectedId, onSelectId]);

  const selectedProject = projects.find((p) => p.id === selectedId) ?? null;

  return (
    <div className="h-full select-none px-3 py-2 flex flex-col">
      <div
        ref={containerRef}
        className={`flex-1 min-h-0 flex gap-0 ${dragging ? "cursor-col-resize" : ""}`}
      >
        {/* 左侧面板 */}
        <div
          className="h-full rounded-xl border border-white/5 overflow-hidden bg-white/[0.01] flex-shrink-0"
          style={{ width: leftWidth }}
        >
          <ProjectListPanel
            projects={projects}
            selectedId={selectedId}
            onSelect={(p) => onSelectId(p.id)}
            search={search}
            onSearchChange={setSearch}
            filter={filter}
            onFilterChange={setFilter}
            loading={loading}
            onRefresh={() => fetchProjects(false, false)}
          />
        </div>

        {/* 拖拽分隔条 */}
        <div
          className={`w-2 h-full flex-shrink-0 cursor-col-resize group flex items-center justify-center ${
            dragging ? "bg-blue-500/20" : ""
          }`}
          onMouseDown={handleResizeStart}
        >
          <div
            className={`w-[3px] h-10 rounded-full transition-colors ${
              dragging ? "bg-blue-400" : "bg-white/10 group-hover:bg-blue-400/60"
            }`}
          />
        </div>

        {/* 右侧面板 */}
        <div className="flex-1 h-full min-w-[200px] rounded-xl border border-white/5 overflow-hidden bg-white/[0.01]">
          <ProjectDetailPanel
            project={selectedProject}
            onRefresh={async () => { await fetchProjects(false, false); }}
            onProjectUpdate={updateProject}
          />
        </div>
      </div>
    </div>
  );
}
