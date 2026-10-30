import { lazy, Suspense, type ComponentType } from "react";
import { MODULE_CATALOG, DEFAULT_MODULE_ORDER, DEFAULT_TOOLBAR, type ModuleMeta } from "./moduleCatalog";
export { moduleLabel, DEFAULT_MODULE_ORDER, DEFAULT_TOOLBAR } from "./moduleCatalog";
export interface ModuleDef extends ModuleMeta { Component: ComponentType<any>; }

// 统一的懒加载包装，加载时显示简洁占位。
function lazyLoad(fn: () => Promise<{ default: ComponentType<any> }>): ComponentType<any> {
  const Lazy = lazy(fn);
  return (props: any) => (
    <Suspense
      fallback={
        <div className="h-full w-full flex items-center justify-center text-slate-500 text-sm">
          加载中…
        </div>
      }
    >
      <Lazy {...props} />
    </Suspense>
  );
}

const PAGE_LOADERS: Record<string, ComponentType<any>> = {
  launcher: lazyLoad(() => import("./components/launcher/LauncherPanel")),
  news: lazyLoad(() => import("./components/RssReader")),
  sdk: lazyLoad(() => import("./components/ProjectManager")),
  ai: lazyLoad(() => import("./components/ai/AiPanel")),
  mindmap: lazyLoad(() => import("./components/mindmap/MindmapPanel")),
  api: lazyLoad(() => import("./components/api/ApiPanel")),
  node: lazyLoad(() => import("./components/node/NodeManagerPanel")),
  mihomo: lazyLoad(() => import("./components/SystemTools/Mihomo")),
  cert: lazyLoad(() => import("./components/SystemTools/CertManager")),
  clipboard: lazyLoad(() => import("./components/ClipboardPanel")),
  otp: lazyLoad(() => import("./components/OtpPanel")),
  translate: lazyLoad(() => import("./components/TranslatePanel")),
  picky: lazyLoad(() => import("./components/PickyPanel")),
  buddy: lazyLoad(() => import("./components/buddy/BuddyPanel")),
  settings: lazyLoad(() => import("./components/GlobalSettings")),
  network: lazyLoad(() => import("./components/LocalNetworkPanel")),
  httpServer: lazyLoad(() => import("./components/HttpServer")),
  rtspServer: lazyLoad(() => import("./components/SystemTools/RtspServer")),
  imageBase64: lazyLoad(() => import("./components/ImageBase64")),
  env: lazyLoad(() => import("./components/EnvManager")),
  logViewer: lazyLoad(() => import("./components/LogViewer")),
  jsonBrowser: lazyLoad(() => import("./components/SystemTools/JsonBrowser")),
  markdownReader: lazyLoad(() => import("./components/SystemTools/MarkdownReader")),
  serial: lazyLoad(() => import("./components/devtools/SerialMonitor")),
  wsDebugger: lazyLoad(() => import("./components/devtools/WsDebugger")),
  music: lazyLoad(() => import("./components/music/MusicPanel")),
  favorites: lazyLoad(() => import("./components/favorites/FavoritesPanel")),
  suno: lazyLoad(() => import("./components/suno/SunoPanel")),
  tasks: lazyLoad(() => import("./components/tasks/TaskCalendarPanel")),
  dbdesigner: lazyLoad(() => import("./components/dbdesigner/DbDesignerPanel")),
};

export const MODULES: ModuleDef[] = MODULE_CATALOG.map((meta) => ({ ...meta, Component: PAGE_LOADERS[meta.id] }));
export const MODULE_MAP: Record<string, ModuleDef> = Object.fromEntries(MODULES.map((m) => [m.id, m]));

/**
 * 根据布局配置计算「顶栏模块」与「更多模块」。
 * @param order   用户自定义顺序（可能为空 = 默认顺序）
 * @param toolbar 顶栏模块 id 列表（为空 = 默认顶栏）
 * @param disabled 禁用模块 id 列表
 */
export function resolveModuleLayout(
  order: string[],
  toolbar: string[],
  disabled: string[]
): { toolbarModules: ModuleDef[]; moreModules: ModuleDef[]; allEnabled: ModuleDef[] } {
  const enabled = MODULES.filter((m) => m.pinned || !disabled.includes(m.id));
  const effectiveToolbar =
    toolbar.length > 0 ? toolbar : DEFAULT_TOOLBAR.filter((id) => !disabled.includes(id));

  // 顺序：先按用户顺序（含未出现在 order 里的默认追加），再过滤禁用。
  const effectiveOrder = order.length > 0 ? order : DEFAULT_MODULE_ORDER;
  const sorted = [...enabled].sort((a, b) => {
    const ia = effectiveOrder.indexOf(a.id);
    const ib = effectiveOrder.indexOf(b.id);
    const ra = ia === -1 ? Number.MAX_SAFE_INTEGER : ia;
    const rb = ib === -1 ? Number.MAX_SAFE_INTEGER : ib;
    return ra - rb;
  });

  const toolbarSet = new Set(effectiveToolbar);
  const toolbarModules: ModuleDef[] = [];
  const moreModules: ModuleDef[] = [];
  for (const m of sorted) {
    if (m.pinned) {
      // 特殊模块（设置）始终顶栏
      toolbarModules.push(m);
    } else if (toolbarSet.has(m.id)) {
      toolbarModules.push(m);
    } else {
      moreModules.push(m);
    }
  }
  return { toolbarModules, moreModules, allEnabled: sorted };
}
