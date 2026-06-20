import React from "react";
import { 
  HeartPulse, 
  Layers, 
  Database, 
  Trash2, 
  Globe, 
  Wrench, 
  Box, 
  Settings,
  ShieldCheck
} from "lucide-react";

interface SidebarProps {
  activeTab: string;
  setActiveTab: (tab: string) => void;
}

export default function Sidebar({ activeTab, setActiveTab }: SidebarProps) {
  const menuItems = [
    { id: "diagnostics", label: "集成环境体检", icon: HeartPulse },
    { id: "sdks", label: "SDK 版本管理", icon: Layers },
    { id: "services", label: "本地服务管理", icon: Database },
    { id: "caches", label: "开发缓存清理", icon: Trash2 },
    { id: "mirrors", label: "国内镜像配置", icon: Globe },
    { id: "packages", label: "全局包管理", icon: Box },
    { id: "tools", label: "系统实用工具", icon: Wrench },
    { id: "settings", label: "全局路径设置", icon: Settings },
  ];

  return (
    <aside className="w-64 glass-sidebar h-screen flex flex-col select-none">
      {/* Brand Header */}
      <div className="p-6 border-b border-white/5 flex items-center gap-3">
        <div className="w-9 h-9 rounded-lg bg-blue-600 flex items-center justify-center text-white shadow-lg shadow-blue-500/20">
          <ShieldCheck className="w-5 h-5" />
        </div>
        <div>
          <h1 className="font-semibold text-white tracking-wide text-sm">AnyVersion</h1>
          <p className="text-[10px] text-slate-400">开发者工作站 v1.0</p>
        </div>
      </div>

      {/* Navigation menu */}
      <nav className="flex-1 px-4 py-6 space-y-1.5 overflow-y-auto">
        {menuItems.map((item) => {
          const Icon = item.icon;
          const isActive = activeTab === item.id;
          return (
            <button
              key={item.id}
              onClick={() => setActiveTab(item.id)}
              className={`w-full flex items-center gap-3.5 px-4 py-3 rounded-xl text-xs font-medium transition-all duration-200 cursor-pointer ${
                isActive
                  ? "bg-blue-600/90 text-white shadow-lg shadow-blue-500/10 border-l-[3px] border-blue-400"
                  : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
              }`}
            >
              <Icon className={`w-4 h-4 transition-transform duration-200 ${isActive ? "scale-110" : ""}`} />
              <span>{item.label}</span>
            </button>
          );
        })}
      </nav>

      {/* Footer info */}
      <div className="p-6 border-t border-white/5 space-y-1 text-center">
        <p className="text-[10px] text-slate-500">Windows 工作站</p>
        <p className="text-[9px] text-slate-600 leading-relaxed">任何人，零基础，也能一步到位搭建开发环境。一切操作透明可见。</p>
      </div>
    </aside>
  );
}
