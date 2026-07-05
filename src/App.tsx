import React, { useState, useEffect } from "react";
import ProjectManager from "./components/ProjectManager";
import SystemTools from "./components/SystemTools";
import GlobalSettings from "./components/GlobalSettings";
import AiPanel from "./components/ai/AiPanel";
import RssReader from "./components/RssReader";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Wrench, Settings, X, Minus, Square, Rss, Cpu, Bot } from "lucide-react";
import "./App.css";

type PageId = "sdk" | "ai" | "news" | "tools" | "settings";

export default function App() {
  const [activePage, setActivePage] = useState<PageId>("news");
  const [defaultToolsTab, setDefaultToolsTab] = useState<"ports" | "backups" | "httpServer" | "imageBase64" | "pathEnv">("ports");
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);

  useEffect(() => {
    const initApp = async () => {
      try {
        await invoke("get_config");
      } catch (e) {
        console.error("Init error:", e);
      }
    };
    initApp();
  }, []);

  return (
    <div className="w-screen h-screen overflow-hidden bg-[#0d111d] text-slate-100 font-sans flex flex-col select-none">
      {/* top bar */}
      <div className="flex-shrink-0 h-11 flex items-center justify-between px-3 border-b border-white/5 bg-[#0e1220]/80 backdrop-blur-md z-50" data-tauri-drag-region>
        {/* Left: Logo + Name + Navigation Capsule */}
        <div className="flex items-center gap-2.5">
          <div className="flex items-center gap-2 pointer-events-none px-1 w-35" data-tauri-drag-region>
            <img src="/icon.png" className="w-5 h-5 object-contain" alt="logo" />
            <span className="text-[11px] font-bold text-white tracking-wide">AnyVersion</span>
          </div>


          <div className="flex items-center gap-0.5 bg-white/5 border border-white/5 rounded-lg p-0.5">
            {([
              { id: "news" as PageId, label: "资讯", icon: <Rss className="w-3 h-3" />, color: "bg-orange-600" },
              { id: "sdk" as PageId, label: "SDK", icon: <Cpu className="w-3 h-3" />, color: "bg-blue-600" },
              { id: "ai" as PageId, label: "AI", icon: <Bot className="w-3 h-3" />, color: "bg-violet-600" },
              { id: "tools" as PageId, label: "工具", icon: <Wrench className="w-3 h-3" />, color: "bg-emerald-600" },
              { id: "settings" as PageId, label: "设置", icon: <Settings className="w-3 h-3" />, color: "bg-red-600" },
            ]).map((item) => (
              <button
                key={item.id}
                onClick={() => setActivePage(item.id)}
                className={`px-3 py-1.5 rounded-md text-[10px] font-semibold flex items-center gap-1 transition-all cursor-pointer ${
                  activePage === item.id
                    ? `${item.color} text-white`
                    : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
                }`}
              >
                {item.icon}
                {item.label}
              </button>
            ))}
          </div>
        </div>

        {/* Draggable Middle Area */}
        <div className="flex-grow h-full" data-tauri-drag-region />

        {/* Right: Window Controls */}
        <div className="flex items-center gap-1">
          <button
            onClick={() => getCurrentWindow().minimize()}
            className="p-1.5 text-slate-400 hover:text-white hover:bg-white/5 rounded transition-all cursor-pointer"
            title="最小化"
          >
            <Minus className="w-3.5 h-3.5" />
          </button>
          <button
            onClick={() => getCurrentWindow().toggleMaximize()}
            className="p-1.5 text-slate-400 hover:text-white hover:bg-white/5 rounded transition-all cursor-pointer"
            title="还原/最大化"
          >
            <Square className="w-3.5 h-3.5" />
          </button>
          <button
            onClick={() => getCurrentWindow().close()}
            className="p-1.5 text-slate-400 hover:text-white hover:bg-red-500/80 rounded transition-all cursor-pointer"
            title="关闭"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>

      {/* content */}
      <div className="flex-grow flex flex-col min-h-0 relative">
        <div className={activePage === "sdk" ? "h-full w-full" : "hidden"}>
          <ProjectManager selectedId={selectedProjectId} onSelectId={setSelectedProjectId} />
        </div>
        <div className={activePage === "ai" ? "h-full w-full" : "hidden"}>
          <AiPanel />
        </div>
        <div className={activePage === "news" ? "h-full w-full flex flex-col" : "hidden"}>
          <RssReader />
        </div>
        <div className={activePage === "tools" ? "h-full w-full flex flex-col" : "hidden"}>
          <SystemTools defaultTab={defaultToolsTab} />
        </div>
        <div className={activePage === "settings" ? "h-full w-full flex flex-col overflow-y-auto" : "hidden"}>
          <GlobalSettings />
        </div>
      </div>
    </div>
  );
}
