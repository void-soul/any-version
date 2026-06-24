import React from "react";
import { Search, RefreshCw } from "lucide-react";
import type { ProjectStatus, ProjectCategory } from "./types";
import { categoryLabel } from "./types";

const FILTERS: Array<{ key: ProjectCategory | "all"; label: string }> = [
  { key: "all", label: "全部" },
  { key: "language", label: "语言" },
  { key: "tool", label: "工具" },
  { key: "service", label: "服务" },
];

interface Props {
  projects: ProjectStatus[];
  selectedId: string | null;
  onSelect: (p: ProjectStatus) => void;
  search: string;
  onSearchChange: (v: string) => void;
  filter: ProjectCategory | "all";
  onFilterChange: (f: ProjectCategory | "all") => void;
  loading: boolean;
  onRefresh?: () => void;
}

export default function ProjectListPanel({
  projects, selectedId, onSelect, search, onSearchChange,
  filter, onFilterChange, loading, onRefresh,
}: Props) {
  const filtered = projects.filter((p) => {
    if (filter !== "all" && p.category !== filter) return false;
    if (search && !p.display_name.toLowerCase().includes(search.toLowerCase())) return false;
    return true;
  });

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div className="p-2.5 border-b border-white/5 space-y-1.5 bg-white/[0.02] flex-shrink-0">
        <div className="flex items-center gap-1.5">
          <div className="relative flex-1">
            <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3 h-3 text-slate-500" />
            <input
              type="text"
              placeholder="搜索..."
              value={search}
              onChange={(e) => onSearchChange(e.target.value)}
              className="w-full glass-input pl-7 pr-2 py-1 text-[11px]"
            />
          </div>
          {onRefresh && (
            <button onClick={onRefresh} disabled={loading}
              className="p-1 hover:bg-white/10 rounded text-slate-400 hover:text-slate-200 cursor-pointer flex-shrink-0"
              title="刷新">
              <RefreshCw className={`w-3 h-3 ${loading ? "animate-spin" : ""}`} />
            </button>
          )}
        </div>
        <div className="flex gap-1">
          {FILTERS.map((f) => (
            <button key={f.key} onClick={() => onFilterChange(f.key)}
              className={`flex-1 py-0.5 rounded text-[11px] font-semibold transition-all cursor-pointer ${
                filter === f.key ? "bg-blue-600 text-white" : "bg-white/5 text-slate-400 hover:text-slate-200"
              }`}>
              {f.label}
            </button>
          ))}
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        {loading ? (
          <div className="p-6 text-center text-slate-500 text-[11px]">加载中...</div>
        ) : filtered.length === 0 ? (
          <div className="p-6 text-center text-slate-500 text-[11px]">未找到匹配项目</div>
        ) : (
          filtered.map((p) => {
            const isSelected = selectedId === p.id;
            return (
              <div key={p.id} onClick={() => onSelect(p)}
                className={`relative p-2.5 flex items-center justify-between cursor-pointer transition-all border-b border-white/[0.03] ${
                  isSelected ? "bg-blue-600/10 border-l-2 border-l-blue-500"
                    : "hover:bg-white/[0.03] border-l-2 border-l-transparent"
                }`}>
                <div className="flex-1 min-w-0 flex items-center gap-2">
                  <span className={`flex-shrink-0 w-5 h-5 rounded-full flex items-center justify-center text-[10px] font-bold ${
                    p.managed 
                      ? p.is_simple_managed 
                        ? "bg-amber-500/15 text-amber-400" 
                        : "bg-emerald-500/15 text-emerald-400" 
                      : "bg-slate-500/10 text-slate-500"
                  }`}>
                    {p.managed ? "✓" : "✗"}
                  </span>
                  <div className="min-w-0 flex items-center gap-1">
                    <span className={`font-semibold text-[11px] truncate ${
                      p.managed 
                        ? p.is_simple_managed 
                          ? "text-amber-100/90" 
                          : "text-emerald-100" 
                        : "text-white"
                    }`}>
                      {p.display_name}
                    </span>
                    <span className={`flex-shrink-0 px-1 py-px rounded text-[7px] font-semibold border ${
                      p.category === "language" ? "bg-blue-500/10 text-blue-400 border-blue-500/20"
                        : p.category === "tool" ? "bg-amber-500/10 text-amber-400 border-amber-500/20"
                        : "bg-purple-500/10 text-purple-400 border-purple-500/20"
                    }`}>
                      {categoryLabel(p.category)}
                    </span>
                  </div>
                </div>
                <div className="flex-shrink-0 ml-2 relative z-10">
                  {p.installed ? (
                    p.active_version ? (
                      <span className="px-1.5 py-px rounded text-[9px] font-mono font-bold bg-emerald-500/15 text-emerald-400 border border-emerald-500/25">
                        v{p.active_version}
                      </span>
                    ) : (
                      <span className="px-1.5 py-px rounded text-[9px] font-semibold bg-emerald-500/10 text-emerald-400 border border-emerald-500/20">已安装</span>
                    )
                  ) : (
                    <span className="px-1.5 py-px rounded text-[9px] font-semibold bg-red-500/10 text-red-400 border border-red-500/20">未安装</span>
                  )}
                </div>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
