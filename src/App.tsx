import React, { useState, useEffect } from "react";
import ProjectManager from "./components/ProjectManager";
import SystemTools from "./components/SystemTools";
import GlobalSettings from "./components/GlobalSettings";
import AiPanel from "./components/ai/AiPanel";
import TaskPanel from "./components/tasks/TaskPanel";
import TaskReminderToast from "./components/tasks/TaskReminderToast";
import RssReader from "./components/RssReader";
import NodeManagerPanel from "./components/node/NodeManagerPanel";
import Mihomo from "./components/SystemTools/Mihomo";
import CertManager from "./components/SystemTools/CertManager";
import LauncherPanel from "./components/launcher/LauncherPanel";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Wrench, Settings, X, Minus, Square, Rss, Cpu, Bot, CalendarCheck, Download, AlertTriangle, CheckCircle2, Loader2, Boxes, Waypoints, ShieldCheck, Rocket } from "lucide-react";
import { convertFileSrc } from "@tauri-apps/api/core";
import "./App.css";

type PageId = "launcher" | "sdk" | "ai" | "tasks" | "node" | "mihomo" | "cert" | "news" | "tools" | "settings";

// 顶级模块默认主题色（可在全局设置里自定义覆盖）
const MODULE_DEFAULTS: Record<string, { label: string; color: string; dark?: boolean }> = {
  launcher: { label: "启动", color: "#8b5cf6" }, // purple
  news: { label: "资讯", color: "#ea580c" }, // orange
  sdk: { label: "SDK", color: "#2563eb" }, // blue
  ai: { label: "AI", color: "#7c3aed" }, // violet
  tasks: { label: "任务", color: "#f59e0b" }, // amber
  node: { label: "服务", color: "#0891b2" }, // cyan
  mihomo: { label: "代理", color: "#4f46e5" }, // indigo
  cert: { label: "证书", color: "#0d9488" }, // teal
  tools: { label: "更多", color: "#059669" }, // emerald
  settings: { label: "设置", color: "#dc2626" }, // red
};

const MODULE_ORDER: PageId[] = ["launcher", "news", "sdk", "ai", "tasks", "node", "mihomo", "cert", "tools", "settings"];

// 自定义字体 @font-face 的全局 CSS 注入
function buildFontFaceCss(customFontPath: string): string {
  if (!customFontPath) return "";
  try {
    const src = convertFileSrc(customFontPath);
    return `@font-face{font-family:'AppCustomFont';src:url('${src}') format('woff2'),url('${src}') format('woff'),url('${src}') format('truetype'),url('${src}') format('opentype');font-display:swap;}`;
  } catch {
    return "";
  }
}

export default function App() {
  // 应用启动默认进入「启动」（Launcher）模块
  const [activePage, setActivePage] = useState<PageId>("launcher");
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

  // 外观：每个顶级模块的主题色 + 全局字体
  const [appearance, setAppearance] = useState<{
    moduleThemeColors: Record<string, string>;
    globalFont: string;
    customFontPath: string;
  }>({ moduleThemeColors: {}, globalFont: "", customFontPath: "" });

  // 懒挂载：仅渲染至少被访问过一次的页面，避免启动时全部组件同时初始化
  const [mountedPages, setMountedPages] = useState<Set<PageId>>(new Set(["launcher"]));
  const switchPage = (page: PageId) => {
    setActivePage(page);
    setMountedPages((prev) => {
      if (prev.has(page)) return prev;
      const next = new Set(prev);
      next.add(page);
      return next;
    });
  };

  // 修复 Windows 上 WebView2 失去/重新获得焦点后键盘无法输入的问题：
  // 窗口重新获得焦点时显式聚焦 webview 内容（Alt-Tab 回来等场景）。
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    const setup = async () => {
      try {
        const win = getCurrentWindow();
        unlisten = await win.onFocusChanged(({ payload: focused }) => {
          if (focused) {
            // 延迟到焦点路由稳定后再聚焦，避免被 Windows 后续的焦点恢复覆盖
            setTimeout(() => {
              getCurrentWebview().setFocus().catch(() => {});
            }, 30);
          }
        });
      } catch (e) {
        console.error("注册窗口焦点监听失败:", e);
      }
    };
    setup();
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  useEffect(() => {
    const initApp = async () => {
      try {
        await invoke("get_config");
      } catch (e) {
        console.error("Init error:", e);
      }
      // 加载外观配置（模块主题色 + 全局字体）
      try {
        const ap = await invoke<{
          moduleThemeColors: Record<string, string>;
          globalFont: string;
          customFontPath: string;
        }>("get_appearance_config");
        setAppearance(ap);
      } catch (e) {
        console.error("Load appearance error:", e);
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

      // 监听全局快捷键唤起主窗口：切到「启动」模块
      listen("launcher-toggle", () => {
        switchPage("launcher");
      });
      // 监听外观变更（全局设置里修改模块主题色/字体后实时生效）
      listen("appearance-updated", async () => {
        try {
          const ap = await invoke<{
            moduleThemeColors: Record<string, string>;
            globalFont: string;
            customFontPath: string;
          }>("get_appearance_config");
          setAppearance(ap);
        } catch (e) {
          console.error("刷新外观失败", e);
        }
      });
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

  const effectiveFontFamily = appearance.customFontPath
    ? "'AppCustomFont', system-ui, sans-serif"
    : appearance.globalFont
      ? `${appearance.globalFont}, system-ui, sans-serif`
      : undefined;
  const fontFaceCss = buildFontFaceCss(appearance.customFontPath);

  return (
    <div
      className="w-screen h-screen overflow-hidden bg-[#0d111d] text-slate-100 flex flex-col"
      style={{ fontFamily: effectiveFontFamily }}
    >
      {fontFaceCss && <style>{fontFaceCss}</style>}
      {/* top bar */}
      <div className="flex-shrink-0 h-11 flex items-center justify-between px-3 border-b border-white/5 bg-[#0e1220]/80 backdrop-blur-md z-50" data-tauri-drag-region>
        {/* Left: Logo + Name + Navigation Capsule */}
        <div className="flex items-center gap-2.5">
          <div className="flex items-center gap-2 pointer-events-none px-1 w-35" data-tauri-drag-region>
            <img src="/icon.png" className="w-5 h-5 object-contain" alt="logo" />
            <span className="text-[11px] font-bold text-white tracking-wide">AnyVersion</span>
          </div>


          <div className="flex items-center gap-0.5 bg-white/5 border border-white/5 rounded-lg p-0.5">
            {MODULE_ORDER.map((id) => {
              const cfg = MODULE_DEFAULTS[id];
              const effectiveColor = appearance.moduleThemeColors[id] || cfg.color;
              const isActive = activePage === id;
              return (
                <button
                  key={id}
                  onClick={() => switchPage(id)}
                  className={`px-3 py-1.5 rounded-md text-[10px] font-semibold flex items-center gap-1 transition-all cursor-pointer ${
                    isActive
                      ? "text-white shadow-sm"
                      : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
                  }`}
                  style={isActive ? { backgroundColor: effectiveColor } : undefined}
                  title={`${cfg.label} (可在全局设置调整主题色)`}
                >
                  {id === "launcher" && <Rocket className="w-3 h-3" />}
                  {id === "news" && <Rss className="w-3 h-3" />}
                  {id === "sdk" && <Cpu className="w-3 h-3" />}
                  {id === "ai" && <Bot className="w-3 h-3" />}
                  {id === "tasks" && <CalendarCheck className="w-3 h-3" />}
                  {id === "node" && <Boxes className="w-3 h-3" />}
                  {id === "mihomo" && <Waypoints className="w-3 h-3" />}
                  {id === "cert" && <ShieldCheck className="w-3 h-3" />}
                  {id === "tools" && <Wrench className="w-3 h-3" />}
                  {id === "settings" && <Settings className="w-3 h-3" />}
                  {cfg.label}
                </button>
              );
            })}
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
        {mountedPages.has("launcher") && (
          <div className={activePage === "launcher" ? "h-full w-full flex flex-col" : "hidden"}>
            <LauncherPanel />
          </div>
        )}
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
        {mountedPages.has("mihomo") && (
          <div className={activePage === "mihomo" ? "h-full w-full flex flex-col" : "hidden"}>
            <Mihomo />
          </div>
        )}
        {mountedPages.has("cert") && (
          <div className={activePage === "cert" ? "h-full w-full flex flex-col" : "hidden"}>
            <CertManager />
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
