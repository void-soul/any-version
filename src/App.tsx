import { useState, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { X, Minus, Square, Download, AlertTriangle, Loader2, FolderOpen, ChevronDown, Settings } from "lucide-react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { MODULES, MODULE_MAP, resolveModuleLayout } from "./moduleRegistry";
import VexAvatar from "./components/VexAvatar";
import { VEX_CYBER_CYAN, resolveThemeAccent } from "./utils/brand";
import { kiraQuoteLine } from "./utils/kiraQuotes";
import { vexSay, onVexSay, type VexSayKind } from "./utils/vexSay";
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

  // 冷启动闪屏：Kira 赛博 Logo + 进度条，短暂铺满后淡出，替代白屏
  const [booting, setBooting] = useState(true);
  useEffect(() => {
    const t = window.setTimeout(() => setBooting(false), 1500);
    return () => window.clearTimeout(t);
  }, []);

  // 初次见面：三步引导卡（localStorage 记忆，只看一次）
  const [introOpen, setIntroOpen] = useState(false);
  const [introStep, setIntroStep] = useState(0);
  useEffect(() => {
    try {
      if (localStorage.getItem("vex_intro_seen") === "1") return;
      const t = window.setTimeout(() => setIntroOpen(true), 900);
      return () => window.clearTimeout(t);
    } catch {
      /* localStorage 不可用则跳过引导 */
    }
  }, []);
  const finishIntro = () => {
    try {
      localStorage.setItem("vex_intro_seen", "1");
    } catch {
      /* ignore */
    }
    setIntroOpen(false);
  };

  // vex 事件伴随语 toast（vexSay 全局总线，自动收起）
  const [vexToast, setVexToast] = useState<{ msg: string; kind: VexSayKind; id: number } | null>(null);
  useEffect(() => {
    let tid: ReturnType<typeof setTimeout> | null = null;
    const off = onVexSay((msg, kind) => {
      setVexToast({ msg, kind, id: Date.now() });
      if (tid) window.clearTimeout(tid);
      tid = window.setTimeout(() => setVexToast(null), 4200);
    });
    return () => {
      off();
      if (tid) window.clearTimeout(tid);
    };
  }, []);
  // 数据目录（全局路径 data_dir）：首次启动下载运行组件前可先选择存储位置
  const [binDataDir, setBinDataDir] = useState("");
  const [binOldDataDir, setBinOldDataDir] = useState("");
  const [binMigrating, setBinMigrating] = useState(false);

  // 外观：模块主题色 + 全局字体 + 模块顺序 + 模块布局（顶栏/禁用）+ 背景底图纹理
  const [appearance, setAppearance] = useState<{
    moduleThemeColors: Record<string, string>;
    globalFont: string;
    customFontPath: string;
    moduleOrder: string[];
    toolbarModules: string[];
    disabledModules: string[];
    backgroundTexture: string;
  }>({
    moduleThemeColors: {},
    globalFont: "",
    customFontPath: "",
    moduleOrder: [],
    toolbarModules: [],
    disabledModules: [],
    backgroundTexture: "",
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
      // 把统一函数库(kQuotes)：把 Kira 语录推给托盘（悬停提示 + 问候菜单共同取这一句）
      try {
        await invoke("set_tray_quote", { text: kiraQuoteLine() });
      } catch (e) {
        console.error("推送托盘语录失败", e);
      }
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
          backgroundTexture: string;
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
            backgroundTexture: string;
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
      vexSay("运行组件都备齐了，随时能用。✨", "success");
    } catch (e) {
      setBinError(typeof e === "string" ? e : String(e));
      vexSay("唔…下载这步卡住了，我帮你看看是哪出的问题", "error");
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

  // 全 App 主强调色：优先读后端配置里的主题色（module_theme_colors["theme"]），
  // 未设置时回退默认签名色。各模块内部用 --module-accent 系列变量联动处同步跟随。
  const activeModuleColor = resolveThemeAccent(appearance.moduleThemeColors);
  const moduleThemeVars = {
    "--module-accent": activeModuleColor,
    "--module-accent-soft": `color-mix(in srgb, ${activeModuleColor} 12%, transparent)`,
    "--module-accent-ring": `color-mix(in srgb, ${activeModuleColor} 30%, transparent)`,
    "--module-accent-strong": `color-mix(in srgb, ${activeModuleColor} 85%, white)`,
    "--neon": activeModuleColor,
    "--cyan": VEX_CYBER_CYAN,
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

  // 全局背景底图纹理 class（由全局设置决定；空=默认网格）。
  const bgTextureClass = appearance.backgroundTexture
    ? `app-bg-${appearance.backgroundTexture}`
    : "cyber-grid";

  // 若当前激活模块被禁用，回退到「启动」模块
  useEffect(() => {
    if (appearance.disabledModules.includes(activePage)) {
      switchPage("launcher");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [appearance.disabledModules]);

  return (
    <div
      className={`w-screen h-screen overflow-hidden bg-[#0d111d] text-slate-100 flex flex-col ${bgTextureClass}`}
      style={{ fontFamily: effectiveFontFamily }}
    >
      {fontFaceCss && <style>{fontFaceCss}</style>}

      {/* 冷启动闪屏：Kira 赛博 Logo + 进度，替代白屏 */}
      {booting && (
        <div className="fixed inset-0 z-[10000] flex flex-col items-center justify-center gap-6 bg-[#0b101b] cyber-grid">
          <div className="relative">
            <span className="absolute -inset-3 rounded-full blur-xl opacity-60" style={{ background: `radial-gradient(circle, ${activeModuleColor}55, transparent 70%)` }} />
            <VexAvatar size={92} className="relative" />
          </div>
          <div className="text-center">
            <div className="text-xl font-black tracking-[0.35em] text-white">
              K<span className="text-[var(--module-accent)]">i</span>ra
            </div>
            <div className="mt-1 text-[10px] tracking-[0.3em] text-slate-500">暖心的桌面伙伴</div>
          </div>
          <div className="h-1 w-48 overflow-hidden rounded-full bg-white/10">
            <div
              className="h-full rounded-full animate-[vexbusybar_1.1s_ease-in-out_infinite]"
              style={{ background: `linear-gradient(90deg, ${activeModuleColor}, ${VEX_CYBER_CYAN})` }}
            />
          </div>
        </div>
      )}

      {/* Kira 事件伴随语 toast（成功/报错统一人设） */}
      {vexToast && (
        <div
          key={vexToast.id}
          className="fixed left-1/2 top-14 z-[210] -translate-x-1/2 animate-in fade-in slide-in-from-top-3 duration-300"
        >
          <div
            className={`vex-neon-edge flex items-center gap-2.5 rounded-2xl px-4 py-2.5 backdrop-blur-md ${
              vexToast.kind === "error"
                ? "vex-toast-pulse bg-[#1a1016]/90"
                : vexToast.kind === "success"
                  ? "vex-toast-light bg-[#0f1a16]/90"
                  : "bg-[#12151f]/90"
            }`}
            style={{
              boxShadow:
                vexToast.kind === "error"
                  ? "0 0 14px rgba(244,63,94,0.35), 0 0 34px rgba(244,63,94,0.20), 0 10px 26px rgba(0,0,0,0.5)"
                  : vexToast.kind === "success"
                    ? "0 0 12px rgba(52,211,153,0.30), 0 0 30px rgba(52,211,153,0.18), 0 10px 26px rgba(0,0,0,0.5)"
                    : "0 0 12px color-mix(in srgb, var(--module-accent) 30%, transparent), 0 0 30px color-mix(in srgb, var(--module-accent) 16%, transparent), 0 10px 26px rgba(0,0,0,0.5)",
            }}
          >
            <VexAvatar size={26} />
            <span className="text-[11px] text-slate-200">{vexToast.msg}</span>
          </div>
        </div>
      )}

      {/* 初次见面：三步引导卡 */}
      {introOpen && (
        <div className="fixed inset-0 z-[300] flex items-center justify-center bg-black/70 backdrop-blur-sm">
          <div className="cyber-border w-[380px] max-w-[92vw] rounded-2xl p-6 shadow-2xl shadow-black/60">
            <div className="flex items-center gap-3">
              <VexAvatar size={46} />
              <div>
                <div className="text-sm font-black text-white">hi，我是 Kira</div>
                <div className="text-[10px] text-slate-400">暖心的桌面伙伴</div>
              </div>
            </div>
            <div className="mt-4 min-h-[72px] text-[12px] leading-relaxed text-slate-300">
              {introStep === 0 && (
                <>我会一直住在这台电脑里：顶栏、落地页、悬浮窗还有托盘都看得见我。平时不用管我，需要时喊一声就行。</>
              )}
              {introStep === 1 && (
                <>后台的事忙完了我会第一时间跟你报个信；要是卡住了，也会帮你盯着。想找某个功能？按下它的快捷键就呼出来了。</>
              )}
              {introStep === 2 && (
                <>不用想着一次全记住。挑个最想先试的模块，点开看看，我在旁边等着你。</>
              )}
            </div>
            <div className="mt-5 flex items-center justify-between">
              <div className="flex gap-1">
                {[0, 1, 2].map((i) => (
                  <span key={i} className={`h-1 w-4 rounded-full ${i === introStep ? "bg-[var(--module-accent)]" : "bg-white/15"}`} />
                ))}
              </div>
              <div className="flex gap-2">
                {introStep < 2 ? (
                  <>
                    <button onClick={finishIntro} className="px-3 py-1.5 rounded-lg text-[11px] text-slate-400 hover:text-white transition cursor-pointer">跳过</button>
                    <button onClick={() => setIntroStep((s) => s + 1)} className="px-4 py-1.5 rounded-lg text-[11px] font-semibold text-white transition cursor-pointer" style={{ background: activeModuleColor }}>下一步 →</button>
                  </>
                ) : (
                  <button onClick={finishIntro} className="px-5 py-1.5 rounded-lg text-[11px] font-semibold text-white transition cursor-pointer" style={{ background: `linear-gradient(90deg, ${activeModuleColor}, ${VEX_CYBER_CYAN})` }}>开始吧</button>
                )}
              </div>
            </div>
          </div>
        </div>
      )}

      {/* 微扫描线质感层 */}
      <div className="cyber-scanline fixed inset-0 z-[9998]" />

      {/* content 区环境光晕（霓虹氛围光，叠在底图之上，随主题色同步） */}
      <div className="vex-neon-ambient fixed inset-0 z-0" />

      {/* top bar */}
      <div className="relative flex-shrink-0 h-11 flex items-center justify-between px-3 border-b border-white/5 bg-[#0e1220]/80 backdrop-blur-md z-50" data-tauri-drag-region>
        {/* 顶栏底部霓虹辉光细线 */}
        <div className="vex-neon-line absolute bottom-0 left-0 right-0 h-px" />
        {/* Left: Logo + Name + Navigation Capsule */}
        <div className="flex items-center gap-2.5">
          <div className="flex items-center gap-2 pointer-events-none px-1 w-35" data-tauri-drag-region>
            <VexAvatar size={22} glow={activeModuleColor} className="vex-neon-breathe" />
            <span className="vex-neon-text text-[11px] font-black tracking-wide">Kira</span>
          </div>


          <div className="relative flex items-center gap-0.5 bg-white/5 border border-white/5 rounded-lg p-0.5">
            {toolbarModules.filter((m) => m.id !== "settings").map((m) => {
              const effectiveColor = activeModuleColor;
              const isActive = activePage === m.id;
              const Icon = m.icon;
              return (
                <button
                  key={m.id}
                  onClick={() => switchPage(m.id)}
                  className={`px-3 py-1.5 rounded-md text-[10px] font-semibold flex items-center gap-1 transition-all cursor-pointer ${
                    isActive
                      ? "vex-neon-ring text-white"
                      : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
                  }`}
                  style={isActive ? { backgroundColor: effectiveColor, "--neon": effectiveColor } as React.CSSProperties : undefined}
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
                      ? "vex-neon-ring text-white"
                      : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
                  }`}
                  style={
                    moreModules.some((m) => m.id === activePage)
                      ? { backgroundColor: activeModuleColor, "--neon": activeModuleColor } as React.CSSProperties
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
                ? "vex-neon-ring text-white"
                : "vex-neon-hover text-slate-400 hover:text-white hover:bg-white/5"
            }`}
            style={activePage === "settings" ? { backgroundColor: activeModuleColor, "--neon": activeModuleColor } as React.CSSProperties : undefined}
            title="设置"
          >
            <SettingsIcon className="w-3.5 h-3.5" />
          </button>
          <div className="w-px h-4 bg-white/10 mx-0.5" />
          <button
            onClick={() => getCurrentWindow().minimize()}
            className="vex-neon-hover p-1.5 text-slate-400 hover:text-white hover:bg-white/5 rounded transition-all cursor-pointer"
            title="最小化"
          >
            <Minus className="w-3.5 h-3.5" />
          </button>
          <button
            onClick={() => getCurrentWindow().toggleMaximize()}
            className="vex-neon-hover p-1.5 text-slate-400 hover:text-white hover:bg-white/5 rounded transition-all cursor-pointer"
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
          // 内存优化：默认只渲染当前激活模块，切走即卸载（对应组件内的事件监听/定时器随
          // unmount 释放；后台服务如 RTSP/HTTP/剪贴板/全局快捷键/mihomo 都由 Rust 后端承载，
          // 卸载前端面板不影响其运行）。settings/launcher 高频切换，保留常驻避免重新拉数据。
          const keepAlive = m.id === "settings" || m.id === "launcher";
          if (!isActive && !keepAlive) return null;
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
                    placeholder="e.g. D:\Kira"
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
