import React, { useState, useEffect } from "react";
import ProjectManager from "./components/ProjectManager";
import SystemTools from "./components/SystemTools";
import GlobalSettings from "./components/GlobalSettings";
import { invoke } from "@tauri-apps/api/core";
import { Wrench, Settings, X } from "lucide-react";
import "./App.css";

export default function App() {
  const [activeTab, setActiveTab] = useState<"projects" | "tools" | "settings">("projects");

  useEffect(() => {
    const initApp = async () => {
      try { await invoke("get_config"); } catch (e) { console.error("Init error:", e); }
    };
    initApp();
  }, []);

  return (
    <div className="w-screen h-screen overflow-hidden bg-[#0d111d] text-slate-100 font-sans flex flex-col">
      {/* top bar */}
      <div className="flex-shrink-0 flex items-center justify-between px-4 py-1.5 border-b border-white/5 bg-white/[0.02] z-50">
        <div className="flex items-center gap-2">
          <div className="w-5 h-5 rounded bg-blue-600 flex items-center justify-center">
            <svg className="w-3 h-3 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
            </svg>
          </div>
          <span className="text-[11px] font-semibold text-white tracking-wide">AnyVersion</span>
        </div>
        <div className="flex items-center gap-0.5 bg-white/5 border border-white/5 rounded-lg p-0.5">
          <button
            onClick={() => setActiveTab(activeTab === "tools" ? "projects" : "tools")}
            className={`px-2.5 py-1 rounded-md text-[10px] font-semibold flex items-center gap-1 transition-all cursor-pointer ${
              activeTab === "tools" ? "bg-blue-600 text-white" : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
            }`}
          >
            <Wrench className="w-3 h-3" />
            系统工具
          </button>
          <button
            onClick={() => setActiveTab(activeTab === "settings" ? "projects" : "settings")}
            className={`px-2.5 py-1 rounded-md text-[10px] font-semibold flex items-center gap-1 transition-all cursor-pointer ${
              activeTab === "settings" ? "bg-blue-600 text-white" : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
            }`}
          >
            <Settings className="w-3 h-3" />
            设置
          </button>
        </div>
      </div>

      {/* content */}
      <div className="flex-1 min-h-0 relative">
        <div className={activeTab === "projects" ? "h-full w-full" : "hidden"}>
          <ProjectManager />
        </div>
        <div className={activeTab === "tools" ? "h-full w-full flex flex-col" : "hidden"}>
          <div className="flex justify-end px-4 pt-2 flex-shrink-0">
            <button onClick={() => setActiveTab("projects")} className="p-1 hover:bg-white/10 rounded text-slate-400 hover:text-slate-200 cursor-pointer" title="返回">
              <X className="w-4 h-4" />
            </button>
          </div>
          <SystemTools />
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
