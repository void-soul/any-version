import React, { useState, useEffect } from "react";
import ProjectManager from "./components/ProjectManager";
import SystemTools from "./components/SystemTools";
import GlobalSettings from "./components/GlobalSettings";
import AiPanel from "./components/ai/AiPanel";
import TaskPanel from "./components/tasks/TaskPanel";
import TaskReminderToast from "./components/tasks/TaskReminderToast";
import RssReader from "./components/RssReader";
import NodeManagerPanel from "./components/node/NodeManagerPanel";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Wrench, Settings, X, Minus, Square, Rss, Cpu, Bot, CalendarCheck, Download, AlertTriangle, CheckCircle2, Loader2, Boxes } from "lucide-react";
import "./App.css";

type PageId = "sdk" | "ai" | "tasks" | "news" | "tools" | "settings" | "node";

export default function App() {
  const [activePage, setActivePage] = useState<PageId>("news");
  const [defaultToolsTab, setDefaultToolsTab] = useState<"ports" | "backups" | "httpServer" | "imageBase64" | "pathEnv">("ports");
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  const [showReminder, setShowReminder] = useState(false);

  // 运行组件（ffmpeg/lego/mediamtx/mihomo）首次启动检测
  const [binAssets, setBinAssets] = useState<{
    missing: string[];
    allPresent: boolean;
    installDir: string;
  } | null>(null);
  const [binDownloading, setBinDownloading] = useState(false);
  const [binProgress, setBinProgress] = useState<{
    downloaded: number;
    total: number;
    speed: string;
    phase: string;
  }>({ downloaded: 0, total: 0, speed: "", phase: "" });
  const [binError, setBinError] = useState<string | null>(null);

  // 懒挂载：仅渲染至少被访问过一次的页面，避免启动时全部组件同时初始化
  const [mountedPages, setMountedPages] = useState<Set<PageId>>(new Set(["news"]));
  const switchPage = (page: PageId) => {
    setActivePage(page);
    setMountedPages((prev) => {
      if (prev.has(page)) return prev;
      const next = new Set(prev);
      next.add(page);
      return next;
    });
  };

  useEffect(() => {
    const initApp = async () => {
      try {
        await invoke("get_config");
      } catch (e) {
        console.error("Init error:", e);
      }
      // 启动后检测运行组件是否齐全（ffmpeg/lego/mediamtx/mihomo）
      try {
        const status = await invoke<{
          missing: string[];
          allPresent: boolean;
          installDir: string;
        }>("check_bin_assets");
        if (!status.allPresent) {
          setBinAssets(status);
        }
      } catch (e) {
        console.error("Check bin assets error:", e);
      }
      // 启动后延迟弹出今日待办提醒一次（等窗口稳定）
      setTimeout(() => setShowReminder(true), 900);
    };
    initApp();
  }, []);

  // 下载运行组件
  const downloadBinAssets = async () => {
    setBinDownloading(true);
    setBinError(null);
    setBinProgress({ downloaded: 0, total: 0, speed: "", phase: "connecting" });
    const unlisten = await listen<{
      downloaded: number;
      total: number;
      speedStr: string;
      phase: string;
    }>("bin-assets-progress", (e) => {
      setBinProgress({
        downloaded: e.payload.downloaded,
        total: e.payload.total,
        speed: e.payload.speedStr,
        phase: e.payload.phase,
      });
    });
    try {
      await invoke("download_bin_assets");
      setBinAssets(null); // 关闭模态
    } catch (e) {
      setBinError(typeof e === "string" ? e : String(e));
    } finally {
      unlisten();
      setBinDownloading(false);
    }
  };

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
              { id: "tasks" as PageId, label: "任务", icon: <CalendarCheck className="w-3 h-3" />, color: "bg-amber-500 !text-slate-900" },
              { id: "node" as PageId, label: "服务", icon: <Boxes className="w-3 h-3" />, color: "bg-cyan-600" },
              { id: "tools" as PageId, label: "更多", icon: <Wrench className="w-3 h-3" />, color: "bg-emerald-600" },
              { id: "settings" as PageId, label: "设置", icon: <Settings className="w-3 h-3" />, color: "bg-red-600" },
            ]).map((item) => (
              <button
                key={item.id}
                onClick={() => switchPage(item.id)}
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
        {mountedPages.has("sdk") && (
          <div className={activePage === "sdk" ? "h-full w-full" : "hidden"}>
            <ProjectManager selectedId={selectedProjectId} onSelectId={setSelectedProjectId} />
          </div>
        )}
        {mountedPages.has("ai") && (
          <div className={activePage === "ai" ? "h-full w-full" : "hidden"}>
            <AiPanel />
          </div>
        )}
        {mountedPages.has("tasks") && (
          <div className={activePage === "tasks" ? "h-full w-full" : "hidden"}>
            <TaskPanel />
          </div>
        )}
        {mountedPages.has("node") && (
          <div className={activePage === "node" ? "h-full w-full" : "hidden"}>
            <NodeManagerPanel />
          </div>
        )}
        {mountedPages.has("news") && (
          <div className={activePage === "news" ? "h-full w-full flex flex-col" : "hidden"}>
            <RssReader />
          </div>
        )}
        {mountedPages.has("tools") && (
          <div className={activePage === "tools" ? "h-full w-full flex flex-col" : "hidden"}>
            <SystemTools defaultTab={defaultToolsTab} />
          </div>
        )}
        {mountedPages.has("settings") && (
          <div className={activePage === "settings" ? "h-full w-full flex flex-col overflow-y-auto" : "hidden"}>
            <GlobalSettings />
          </div>
        )}

        {/* 启动后今日待办提醒（右下角自定义弹窗） */}
        {showReminder && (
          <TaskReminderToast
            onClose={() => setShowReminder(false)}
            onOpenTasks={() => {
              setShowReminder(false);
              switchPage("tasks");
            }}
          />
        )}

        {/* 运行组件下载（首次启动缺失时全屏阻塞，不下载无法使用） */}
        {binAssets && (
          <div className="fixed inset-0 z-[10000] flex items-center justify-center bg-black/70 backdrop-blur-sm">
            <div className="w-[420px] max-w-[92vw] rounded-2xl border border-white/10 bg-slate-900/95 shadow-2xl shadow-black/60 p-6">
              <div className="flex items-center gap-2.5 mb-3">
                <AlertTriangle className="w-5 h-5 text-amber-400 flex-shrink-0" />
                <h2 className="text-[15px] font-bold text-white">需要下载运行组件</h2>
              </div>
              <p className="text-[12px] text-slate-400 leading-relaxed mb-3">
                以下运行组件缺失，应用部分功能（代理 / 媒体流 / 证书等）依赖它们。
                请下载后继续使用（约 209&nbsp;MB，解压至 <code className="text-[10px] text-amber-300/90 break-all">{binAssets.installDir}</code>）。
              </p>
              <div className="flex flex-wrap gap-1.5 mb-4">
                {binAssets.missing.map((m) => (
                  <span
                    key={m}
                    className="px-2 py-0.5 rounded-md bg-amber-500/10 border border-amber-500/20 text-[10px] text-amber-300 font-mono"
                  >
                    {m}
                  </span>
                ))}
              </div>

              {binProgress.phase === "extracting" && (
                <p className="text-[11px] text-sky-300 mb-2 flex items-center gap-1.5">
                  <Loader2 className="w-3 h-3 animate-spin" /> 正在解压…
                </p>
              )}

              {binProgress.total > 0 && (
                <div className="mb-3">
                  <div className="h-2 rounded-full bg-white/10 overflow-hidden">
                    <div
                      className="h-full rounded-full bg-gradient-to-r from-amber-400 to-orange-500 transition-all duration-150"
                      style={{
                        width: `${binProgress.total > 0 ? Math.min(100, (binProgress.downloaded / binProgress.total) * 100) : 0}%`,
                      }}
                    />
                  </div>
                  <div className="flex justify-between mt-1 text-[10px] text-slate-500 font-mono">
                    <span>
                      {(binProgress.downloaded / 1024 / 1024).toFixed(1)} / {(binProgress.total / 1024 / 1024).toFixed(1)} MB
                    </span>
                    <span>{binProgress.speed}</span>
                  </div>
                </div>
              )}

              {binError && (
                <p className="text-[11px] text-red-400 mb-3 break-all">下载失败：{binError}</p>
              )}

              <button
                onClick={downloadBinAssets}
                disabled={binDownloading}
                className="w-full flex items-center justify-center gap-2 py-2.5 rounded-xl bg-amber-500 hover:bg-amber-400 disabled:opacity-50 disabled:cursor-not-allowed text-[13px] font-bold text-slate-900 transition-all"
              >
                {binDownloading ? (
                  <>
                    <Loader2 className="w-4 h-4 animate-spin" /> 下载中…
                  </>
                ) : (
                  <>
                    <Download className="w-4 h-4" /> 开始下载（约 209 MB）
                  </>
                )}
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
