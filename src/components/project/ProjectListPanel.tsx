import React from "react";
import {
  Search,
  Layers,
  Wrench,
  Server,
  CheckCircle,
  AlertTriangle,
  ShieldCheck,
} from "lucide-react";
import type { ProjectStatus, ProjectCategory } from "./types";
import { categoryLabel } from "./types";

// ── 筛选标签配置 ──
const FILTERS: Array<{ key: ProjectCategory | "all"; label: string }> = [
  { key: "all", label: "全部" },
  { key: "language", label: "语言" },
  { key: "tool", label: "工具" },
  { key: "service", label: "服务" },
];

const categoryIcon: Record<ProjectCategory, React.ReactNode> = {
  language: <Layers className="w-3 h-3" />,
  tool: <Wrench className="w-3 h-3" />,
  service: <Server className="w-3 h-3" />,
};

// ── Props ──
interface Props {
  projects: ProjectStatus[];
  selectedId: string | null;
  onSelect: (p: ProjectStatus) => void;
  search: string;
  onSearchChange: (v: string) => void;
  filter: ProjectCategory | "all";
  onFilterChange: (f: ProjectCategory | "all") => void;
  loading: boolean;
}

export default function ProjectListPanel({
  projects,
  selectedId,
  onSelect,
  search,
  onSearchChange,
  filter,
  onFilterChange,
  loading,
}: Props) {
  // 过滤逻辑
  const filtered = projects.filter((p) => {
    if (filter !== "all" && p.category !== filter) return false;
    if (search && !p.display_name.toLowerCase().includes(search.toLowerCase())) return false;
    return true;
  });

  return (
    <div className="glass-panel rounded-2xl border border-white/5 overflow-hidden flex flex-col h-full">
      {/* 搜索框 */}
      <div className="p-3 border-b border-white/5 space-y-2.5 bg-white/3 flex-shrink-0">
        <div className="relative">
          <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-slate-500" />
          <input
            type="text"
            placeholder="搜索项目名称..."
            value={search}
            onChange={(e) => onSearchChange(e.target.value)}
            className="w-full glass-input pl-8 pr-3 py-1.5 text-xs"
          />
        </div>

        {/* 类型筛选 */}
        <div className="flex gap-1.5">
          {FILTERS.map((f) => (
            <button
              key={f.key}
              onClick={() => onFilterChange(f.key)}
              className={`flex-1 py-1 rounded-lg text-[10px] font-semibold transition-all cursor-pointer ${
                filter === f.key
                  ? "bg-blue-600 text-white shadow-md"
                  : "bg-white/5 text-slate-400 hover:text-slate-200 hover:bg-white/10"
              }`}
            >
              {f.label}
            </button>
          ))}
        </div>
      </div>

      {/* 项目列表 */}
      <div className="flex-1 overflow-y-auto divide-y divide-white/5">
        {loading ? (
          <div className="p-8 text-center text-slate-500 text-xs">正在加载项目列表...</div>
        ) : filtered.length === 0 ? (
          <div className="p-8 text-center text-slate-500 text-xs">未找到匹配的项目</div>
        ) : (
          filtered.map((p) => {
            const isSelected = selectedId === p.id;
            return (
              <div
                key={p.id}
                onClick={() => onSelect(p)}
                className={`p-3.5 flex items-center gap-3 hover:bg-white/2 cursor-pointer transition-all ${
                  isSelected ? "bg-blue-600/5 border-l-2 border-blue-500" : ""
                }`}
              >
                {/* 图标 */}
                <div className="w-8 h-8 rounded-lg bg-white/5 border border-white/10 flex items-center justify-center text-sm flex-shrink-0">
                  {p.icon ? (
                    <span>{p.icon}</span>
                  ) : (
                    categoryIcon[p.category] ?? <Layers className="w-3.5 h-3.5 text-slate-400" />
                  )}
                </div>

                {/* 信息 */}
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-1.5">
                    <h4 className="font-semibold text-white text-xs truncate">{p.display_name}</h4>
                    {p.managed && (
                      <span className="flex-shrink-0 px-1.5 py-0.5 rounded text-[8px] bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-bold flex items-center gap-0.5">
                        <ShieldCheck className="w-2.5 h-2.5" />
                        已托管
                      </span>
                    )}
                  </div>
                  <div className="flex items-center gap-1.5 mt-0.5">
                    <span
                      className={`px-1.5 py-0.5 rounded text-[8px] font-semibold border ${
                        p.category === "language"
                          ? "bg-blue-500/10 text-blue-400 border-blue-500/20"
                          : p.category === "tool"
                          ? "bg-amber-500/10 text-amber-400 border-amber-500/20"
                          : "bg-purple-500/10 text-purple-400 border-purple-500/20"
                      }`}
                    >
                      {categoryLabel(p.category)}
                    </span>
                    {p.install_source && (
                      <span className="text-[8px] text-slate-500">{p.install_source}</span>
                    )}
                  </div>
                </div>

                {/* 状态 */}
                <div className="text-right flex-shrink-0">
                  {p.installed ? (
                    <div className="flex items-center gap-1">
                      <CheckCircle className="w-3 h-3 text-emerald-400" />
                      {p.active_version ? (
                        <span className="text-[10px] bg-blue-500/10 text-blue-400 border border-blue-500/20 px-1.5 py-0.5 rounded font-mono font-medium">
                          v{p.active_version}
                        </span>
                      ) : (
                        <span className="text-[10px] text-slate-400">已安装</span>
                      )}
                    </div>
                  ) : (
                    <span className="text-[10px] text-slate-500 flex items-center gap-1">
                      <AlertTriangle className="w-3 h-3" />
                      未安装
                    </span>
                  )}
                  {p.installed_versions.length > 0 && (
                    <p className="text-[9px] text-slate-500 mt-0.5">
                      {p.installed_versions.length} 个版本
                    </p>
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
