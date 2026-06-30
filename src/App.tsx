import React, { useState, useEffect } from "react";
import ProjectManager from "./components/ProjectManager";
import SystemTools from "./components/SystemTools";
import GlobalSettings from "./components/GlobalSettings";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Wrench, Settings, X, Minus, Square, Rss } from "lucide-react";
import "./App.css";

export default function App() {
  const [activeTab, setActiveTab] = useState<"projects" | "tools" | "settings">("projects");
  const [defaultToolsTab, setDefaultToolsTab] = useState<"ports" | "backups" | "httpServer" | "imageBase64" | "pathEnv" | "rss">("ports");
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);

  useEffect(() => {
    const initApp = async () => {
      try { 
        await invoke("get_config"); 
        
        // Check if first launch for RSS feed
        const rssConfig = await invoke<{ rss_sources: string[]; is_first_launch: boolean }>("get_rss_config");
        if (rssConfig.is_first_launch) {
          setActiveTab("projects");
          setSelectedProjectId(null);
        }
      } catch (e) { 
        console.error("Init error:", e); 
      }
    };
    initApp();
  }, []);

  return (
    <div className="w-screen h-screen overflow-hidden bg-[#0d111d] text-slate-100 font-sans flex flex-col select-none">
      {/* top bar */}
      <div className="flex-shrink-0 h-9 flex items-center justify-between px-4 border-b border-white/5 bg-[#0e1220]/80 backdrop-blur-md z-50" data-tauri-drag-region>
        {/* Left: Software Name */}
        <div className="flex items-center gap-2 pointer-events-none" data-tauri-drag-region>
          <img src="/icon.png" className="w-4.5 h-4.5 object-contain" alt="logo" />
          <span className="text-[11px] font-bold text-white tracking-wide">AnyVersion</span>
        </div>

        {/* Draggable Middle Area */}
        <div className="flex-grow h-full" data-tauri-drag-region />

        {/* Right Section: Capsule + Window Controls */}
        <div className="flex items-center gap-3">
          {/* Tools & Settings Capsule */}
          <div className="flex items-center gap-0.5 bg-white/5 border border-white/5 rounded-lg p-0.5">
            <button
              onClick={() => {
                setActiveTab("projects");
                setSelectedProjectId(null);
              }}
              className={`px-3 py-1 rounded-md text-[10px] font-semibold flex items-center gap-1 transition-all cursor-pointer ${
                activeTab === "projects" && selectedProjectId === null
                  ? "bg-blue-600 text-white"
                  : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
              }`}
            >
              <Rss className="w-3 h-3" />
              资讯
            </button>
            <button
              onClick={() => setActiveTab(activeTab === "tools" ? "projects" : "tools")}
              className={`px-3 py-1 rounded-md text-[10px] font-semibold flex items-center gap-1 transition-all cursor-pointer ${
                activeTab === "tools" ? "bg-blue-600 text-white" : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
              }`}
            >
              <Wrench className="w-3 h-3" />
              系统工具
            </button>
            <button
              onClick={() => setActiveTab(activeTab === "settings" ? "projects" : "settings")}
              className={`px-3 py-1 rounded-md text-[10px] font-semibold flex items-center gap-1 transition-all cursor-pointer ${
                activeTab === "settings" ? "bg-blue-600 text-white" : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
              }`}
            >
              <Settings className="w-3 h-3" />
              设置
            </button>
          </div>

          {/* Divider */}
          <div className="w-px h-4 bg-white/10" />

          {/* Window Controls */}
          <div className="flex items-center gap-1">
            <button
              onClick={() => getCurrentWindow().minimize()}
              className="p-1 text-slate-400 hover:text-white hover:bg-white/5 rounded transition-all cursor-pointer"
              title="最小化"
            >
              <Minus className="w-3.5 h-3.5" />
            </button>
            <button
              onClick={() => getCurrentWindow().toggleMaximize()}
              className="p-1 text-slate-400 hover:text-white hover:bg-white/5 rounded transition-all cursor-pointer"
              title="还原/最大化"
            >
              <Square className="w-3.5 h-3.5" />
            </button>
            <button
              onClick={() => getCurrentWindow().close()}
              className="p-1 text-slate-400 hover:text-white hover:bg-red-500/80 rounded transition-all cursor-pointer"
              title="关闭"
            >
              <X className="w-3.5 h-3.5" />
            </button>
          </div>
        </div>
      </div>

      {/* content */}
      <div className="flex-grow flex flex-col min-h-0 relative">
        <div className={activeTab === "projects" ? "h-full w-full" : "hidden"}>
          <ProjectManager selectedId={selectedProjectId} onSelectId={setSelectedProjectId} />
        </div>
        <div className={activeTab === "tools" ? "h-full w-full flex flex-col" : "hidden"}>
          <div className="flex justify-end px-4 pt-2 flex-shrink-0">
            <button onClick={() => setActiveTab("projects")} className="p-1 hover:bg-white/10 rounded text-slate-400 hover:text-slate-200 cursor-pointer" title="返回">
              <X className="w-4 h-4" />
            </button>
          </div>
          <SystemTools defaultTab={defaultToolsTab} />
        </div>
        <div className={activeTab === "settings" ? "h-full w-full flex flex-col overflow-y-auto" : "hidden"}>
          <div className="flex justify-end px-4 pt-2 flex-shrink-0">
            <button onClick={() => setActiveTab("projects")} className="p-1 hover:bg-white/10 rounded text-slate-400 hover:text-slate-200 cursor-pointer" title="返回">
              <X className="w-4 h-4" />
            </button>
          </div>
          <GlobalSettings />
        </div>
      </div>
    </div>
  );
}
