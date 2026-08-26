import { useState, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { X, Minus, Square, Download, AlertTriangle, Loader2, FolderOpen, ChevronDown, Settings } from "lucide-react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { MODULES, MODULE_MAP, resolveModuleLayout } from "./moduleRegistry";
import "./App.css";

// 模块 id 即字符串（所有模块平级）。
export type PageId = string;

// 向后兼容导出：模块默认外观（label/color），供 GlobalSettings 等使用。
export const MODULE_DEFAULTS: Record<string, { label: string; color: string }> =
  Object.fromEntries(MODULES.map((m) => [m.id, { label: m.label, color: m.color }]));

// 向后兼容导出：默认模块顺序（全部平级模块）。
export const MODULE_ORDER: PageId[] = MODULES.map((m) => m.id);

// 自定义字体 @font-face 的全局 CSS 注入
function buildFontFaceCss(customFontPath: string): string {
  if (!customFontPath) return "";
  try {
    const src = convertFileSrc(customFontPath);
    // 按文件扩展名只声明一个正确的 format()，避免同一 URL 挂多个不匹配的 format
    // 导致 Chromium 整条跳过、字体静默回退到系统默认（导入后「没变化」的根因）。
    const ext = customFontPath.split(".").pop()?.toLowerCase() || "";
    const format = ext === "woff2" ? "woff2" : ext === "woff" ? "woff" : ext === "otf" ? "opentype" : "truetype";
    return `@font-face{font-family:'AppCustomFont';src:url('${src}') format('${format}');font-display:swap;}`;
  } catch {
    return "";
  }
}

export default function App() {
  // 应用启动默认进入「启动」（Launcher）模块
  const [activePage, setActivePage] = useState<PageId>("launcher");
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);

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
  // 数据目录（全局路径 data_dir）：首次启动下载运行组件前可先选择存储位置
  const [binDataDir, setBinDataDir] = useState("");
  const [binOldDataDir, setBinOldDataDir] = useState("");
  const [binMigrating, setBinMigrating] = useState(false);

  // 外观：模块主题色 + 全局字体 + 模块顺序 + 模块布局（顶栏/禁用）
  const [appearance, setAppearance] = useState<{
    moduleThemeColors: Record<string, string>;
    globalFont: string;
    customFontPath: string;
    moduleOrder: string[];
    toolbarModules: string[];
    disabledModules: string[];
  }>({
    moduleThemeColors: {},
    globalFont: "",
    customFontPath: "",
    moduleOrder: [],
    toolbarModules: [],
    disabledModules: [],
  });

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

  useEffect(() => {
    const initApp = async () => {
      // 读取当前数据目录（全局路径），供首次启动下载运行组件时选择/迁移
      try {
        const config = await invoke<{ data_dir?: string }>("get_config");
        const dir = config.data_dir || (await invoke<string>("get_data_dir_cmd").catch(() => ""));
        setBinDataDir(dir);
        setBinOldDataDir(dir);
      } catch (e) {
        console.error("Init error:", e);
      }
      // 加载外观配置（模块主题色 + 全局字体）
      try {
        const ap = await invoke<{
          moduleThemeColors: Record<string, string>;
          globalFont: string;
          customFontPath: string;
          moduleOrder: string[];
          toolbarModules: string[];
          disabledModules: string[];
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
      // 监听全局快捷键唤起主窗口：切到「启动」模块
      listen("launcher-toggle", () => {
        switchPage("launcher");
      });
      // 监听模块专属快捷键唤起：直接切到对应模块（来自后端 launcher-open-module 事件，载荷为 moduleId）
      listen<string>("launcher-open-module", (event) => {
        const m = event.payload;
        if (m && MODULE_MAP[m] && !appearance.disabledModules.includes(m)) {
          switchPage(m);
        }
      });
      // 监听外观变更（全局设置里修改模块主题色/字体后实时生效）
      listen("appearance-updated", async () => {
        try {
          const ap = await invoke<{
            moduleThemeColors: Record<string, string>;
            globalFont: string;
            customFontPath: string;
            moduleOrder: string[];
            toolbarModules: string[];
            disabledModules: string[];
          }>("get_appearance_config");
          setAppearance(ap);
        } catch (e) {
          console.error("刷新外观失败", e);
        }
      });
    };
    initApp();
  }, []);

  // 当前激活模块变化时上报后端，供模块专属热键做「显示/隐藏」切换判定
  useEffect(() => {
    emit("launcher-active-page", activePage).catch(() => {});
  }, [activePage]);

  // 路径归一化（去掉首尾空白与末尾斜杠），用于判断数据目录是否变化
  const normalizePath = (s: string) => s.trim().replace(/[\\/]+$/, "");

  // 浏览选择数据目录
  const handleBrowseBinDataDir = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, title: "选择数据目录" });
      if (selected) setBinDataDir(selected as string);
    } catch {
      alert("文件夹选择器不可用，请手动输入路径。");
    }
  };

  // 下载运行组件
  const downloadBinAssets = async () => {
    setBinDownloading(true);
    setBinError(null);
    setBinProgress({ downloaded: 0, total: 0, speed: "", phase: "connecting" });
    // 若用户修改了数据目录：先迁移全部数据到新位置，再下载（组件将落到新目录/bin）
    if (binDataDir.trim() && normalizePath(binDataDir) !== normalizePath(binOldDataDir)) {
      setBinMigrating(true);
      try {
        await invoke("update_config", { dataDir: binDataDir.trim() });
        setBinOldDataDir(binDataDir.trim());
      } catch (e) {
        setBinError(`数据目录迁移失败：${typeof e === "string" ? e : String(e)}`);
        setBinDownloading(false);
        setBinMigrating(false);
        return;
      } finally {
        setBinMigrating(false);
      }
    }
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

  // 运行组件实际安装目录（随数据目录输入实时变化）
  const effectiveBinDir = (binDataDir.trim() || binOldDataDir || "…").replace(/[\\/]+$/, "");

  const effectiveFontFamily = appearance.customFontPath
    ? "'AppCustomFont', system-ui, sans-serif"
    : appearance.globalFont
      ? `${appearance.globalFont}, system-ui, sans-serif`
      : undefined;
  const fontFaceCss = buildFontFaceCss(appearance.customFontPath);

  // 当前激活模块的主题色：注入内容区，供各模块内部用 --module-accent 系列变量联动
  const activeModuleColor =
    appearance.moduleThemeColors[activePage] || MODULE_DEFAULTS[activePage]?.color || "#8b5cf6";
  const moduleThemeVars = {
    "--module-accent": activeModuleColor,
    "--module-accent-soft": `color-mix(in srgb, ${activeModuleColor} 12%, transparent)`,
    "--module-accent-ring": `color-mix(in srgb, ${activeModuleColor} 30%, transparent)`,
    "--module-accent-strong": `color-mix(in srgb, ${activeModuleColor} 85%, white)`,
  } as React.CSSProperties;

  // 计算模块布局：顶栏模块 / 更多模块 / 全部启用模块。
  const { toolbarModules, moreModules, allEnabled } = useMemo(
    () =>
      resolveModuleLayout(
        appearance.moduleOrder,
        appearance.toolbarModules,
        appearance.disabledModules
      ),
    [appearance.moduleOrder, appearance.toolbarModules, appearance.disabledModules]
  );

  // 「设置」模块图标（用于窗口右上角只显示图标的设置按钮）
  const SettingsIcon = MODULE_MAP["settings"]?.icon ?? Settings;

  // 「更多」下拉菜单开合
  const [moreOpen, setMoreOpen] = useState(false);

  // 若当前激活模块被禁用，回退到「启动」模块
  useEffect(() => {
    if (appearance.disabledModules.includes(activePage)) {
      switchPage("launcher");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [appearance.disabledModules]);

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


          <div className="relative flex items-center gap-0.5 bg-white/5 border border-white/5 rounded-lg p-0.5">
            {toolbarModules.filter((m) => m.id !== "settings").map((m) => {
              const effectiveColor = appearance.moduleThemeColors[m.id] || m.color;
              const isActive = activePage === m.id;
              const Icon = m.icon;
              return (
                <button
                  key={m.id}
                  onClick={() => switchPage(m.id)}
                  className={`px-3 py-1.5 rounded-md text-[10px] font-semibold flex items-center gap-1 transition-all cursor-pointer ${
                    isActive
                      ? "text-white shadow-sm"
                      : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
                  }`}
                  style={isActive ? { backgroundColor: effectiveColor } : undefined}
                  title={`${m.label} (可在全局设置调整主题色)`}
                >
                  <Icon className="w-3 h-3" />
                  {m.label}
                </button>
              );
            })}

            {/* 「更多」下拉：收纳未置顶的模块 */}
            {moreModules.length > 0 && (
              <div className="relative">
                <button
                  onClick={() => setMoreOpen((v) => !v)}
                  className={`px-3 py-1.5 rounded-md text-[10px] font-semibold flex items-center gap-1 transition-all cursor-pointer ${
                    moreModules.some((m) => m.id === activePage)
                      ? "text-white shadow-sm"
                      : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
                  }`}
                  style={
                    moreModules.some((m) => m.id === activePage)
                      ? { backgroundColor: "#059669" }
                      : undefined
                  }
                  title="更多模块"
                >
                  <span className="w-3 h-3 flex items-center justify-center">⋯</span>
                  更多
                  <ChevronDown className={`w-2.5 h-2.5 transition-transform ${moreOpen ? "rotate-180" : ""}`} />
                </button>
                {moreOpen && (
                  <>
                    <div className="fixed inset-0 z-40" onClick={() => setMoreOpen(false)} />
                    <div className="absolute top-full right-0 mt-1.5 z-50 min-w-[160px] rounded-lg border border-white/10 bg-[#151a2a] shadow-2xl shadow-black/60 p-1">
                      {moreModules.map((m) => {
                        const Icon = m.icon;
                        return (
                          <button
                            key={m.id}
                            onClick={() => {
                              switchPage(m.id);
                              setMoreOpen(false);
                            }}
                            className={`w-full px-3 py-2 rounded-md text-[11px] font-medium flex items-center gap-2 transition-all cursor-pointer text-left ${
                              activePage === m.id
                                ? "bg-white/10 text-white"
                                : "text-slate-300 hover:bg-white/5"
                            }`}
                          >
                            <Icon className="w-3.5 h-3.5" style={{ color: m.color }} />
                            {m.label}
                          </button>
                        );
                      })}
                    </div>
                  </>
                )}
              </div>
            )}
          </div>
        </div>

        {/* Draggable Middle Area */}
        <div className="flex-grow h-full" data-tauri-drag-region />

        {/* Right: Settings + Window Controls */}
        <div className="flex items-center gap-1">
          <button
            onClick={() => switchPage("settings")}
            className={`p-1.5 rounded transition-all cursor-pointer ${
              activePage === "settings"
                ? "text-white"
                : "text-slate-400 hover:text-white hover:bg-white/5"
            }`}
            style={activePage === "settings" ? { backgroundColor: appearance.moduleThemeColors["settings"] || "#dc2626" } : undefined}
            title="设置"
          >
            <SettingsIcon className="w-3.5 h-3.5" />
          </button>
          <div className="w-px h-4 bg-white/10 mx-0.5" />
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
      <div className="flex-grow flex flex-col min-h-0 relative" style={moduleThemeVars}>
        {allEnabled.map((m) => {
          if (!mountedPages.has(m.id)) return null;
          const Comp = m.Component;
          const isActive = activePage === m.id;
          // 特殊模块的额外 props
          const extraProps: Record<string, unknown> =
            m.id === "sdk" ? { selectedId: selectedProjectId, onSelectId: setSelectedProjectId } : {};
          const containerClass =
            m.id === "settings"
              ? "h-full w-full flex flex-col overflow-y-auto"
              : "h-full w-full flex flex-col";
          return (
            <div key={m.id} className={isActive ? containerClass : "hidden"}>
              <Comp {...extraProps} />
            </div>
          );
        })}

        {/* 运行组件下载（首次启动缺失时全屏阻塞，不下载无法使用） */}
        {binAssets && (
          <div className="fixed inset-0 z-[10000] flex items-center justify-center bg-black/70 backdrop-blur-sm">
            <div className="w-[440px] max-w-[94vw] max-h-[92vh] overflow-y-auto rounded-2xl border border-white/10 bg-slate-900/95 shadow-2xl shadow-black/60 p-6">
              <div className="flex items-center gap-2.5 mb-3">
                <AlertTriangle className="w-5 h-5 text-amber-400 flex-shrink-0" />
                <h2 className="text-[15px] font-bold text-white">需要下载运行组件</h2>
              </div>
              <p className="text-[12px] text-slate-400 leading-relaxed mb-3">
                以下运行组件缺失，应用部分功能（代理 / 媒体流 / 证书等）依赖它们。
                请下载后继续使用（约 209&nbsp;MB，解压至{" "}
                <code className="text-[10px] text-amber-300/90 break-all">
                  {effectiveBinDir}\bin
                </code>
                ）。
              </p>
              <div className="flex flex-wrap gap-1.5 mb-3">
                {binAssets.missing.map((m) => (
                  <span
                    key={m}
                    className="px-2 py-0.5 rounded-md bg-amber-500/10 border border-amber-500/20 text-[10px] text-amber-300 font-mono"
                  >
                    {m}
                  </span>
                ))}
              </div>

              {/* 数据目录（全局路径）选择：下载前可先确定存储位置 */}
              <div className="mb-4 rounded-xl border border-white/10 bg-white/[0.03] p-3">
                <div className="flex items-center gap-1.5 mb-2">
                  <FolderOpen className="w-3.5 h-3.5 text-sky-400 flex-shrink-0" />
                  <span className="text-[11px] font-semibold text-slate-300">
                    数据目录（全局路径 data_dir）
                  </span>
                  {binDataDir.trim() && normalizePath(binDataDir) !== normalizePath(binOldDataDir) && (
                    <span className="ml-auto text-[9px] text-amber-300 bg-amber-500/10 border border-amber-500/20 rounded px-1.5 py-0.5 flex-shrink-0">
                      已修改，下载前将迁移
                    </span>
                  )}
                </div>
                <div className="flex items-center gap-2">
                  <input
                    type="text"
                    value={binDataDir}
                    disabled={binDownloading}
                    onChange={(e) => setBinDataDir(e.target.value)}
                    className="flex-1 min-w-0 glass-input px-3 py-2 text-[11px] font-mono bg-black/30 border border-white/10 rounded-lg focus:outline-none focus:border-sky-400/50 disabled:opacity-50"
                    placeholder="e.g. D:\AnyVersion"
                  />
                  <button
                    onClick={handleBrowseBinDataDir}
                    disabled={binDownloading}
                    className="p-2 bg-white/5 hover:bg-white/10 text-slate-400 hover:text-slate-200 rounded-lg border border-white/5 cursor-pointer transition-all flex-shrink-0 disabled:opacity-50"
                    title="选择文件夹"
                  >
                    <FolderOpen className="w-3.5 h-3.5" />
                  </button>
                </div>
                <p className="text-[9px] text-slate-500 mt-1.5 leading-relaxed">
                  所有可变数据（SDK、运行组件、Node 服务、证书、数据库）都存储在此目录下
                  {binOldDataDir ? (
                    <>
                      ，当前默认{" "}
                      <code className="text-slate-400 break-all">{binOldDataDir}</code>
                    </>
                  ) : null}
                  。
                </p>
                <div className="mt-1.5 text-[9px] font-mono text-slate-500 space-y-0.5 leading-relaxed break-all">
                  <div>
                    <span className="text-amber-400">运行组件</span>{" "}
                    {effectiveBinDir}\bin
                  </div>
                  <div>
                    <span className="text-cyan-400">SDK</span> {effectiveBinDir}\sdk
                  </div>
                  <div>
                    <span className="text-emerald-400">Node 服务</span>{" "}
                    {effectiveBinDir}\node-projects
                  </div>
                  <div>
                    <span className="text-teal-400">证书</span> {effectiveBinDir}\certs
                  </div>
                </div>
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
                  binMigrating ? (
                    <>
                      <Loader2 className="w-4 h-4 animate-spin" /> 正在迁移数据…
                    </>
                  ) : (
                    <>
                      <Loader2 className="w-4 h-4 animate-spin" /> 下载中…
                    </>
                  )
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
