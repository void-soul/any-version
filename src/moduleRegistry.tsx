import React, { lazy, Suspense, ComponentType } from "react";
import {
  Rocket,
  Rss,
  Cpu,
  Bot,
  CalendarCheck,
  Boxes,
  Waypoints,
  ShieldCheck,
  Clipboard,
  Settings,
  Network,
  Database,
  Server,
  Video,
  Image,
  ListOrdered,
  ScrollText,
  Braces,
  BookOpen,
  KeyRound,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";

/**
 * 统一模块注册表：any-version 的所有模块都是「平级」的，
 * 只不过有的显示在顶栏（toolbar），有的收进「更多」（overflow）。
 * 用户可自由控制每个模块的归类（顶栏/更多）与启用/禁用。
 *
 * 每个模块声明：id、label、icon、color、component（懒加载）。
 * 「设置」模块为特殊模块（不可禁用、始终在顶栏），其余模块完全平等。
 */

export interface ModuleDef {
  id: string;
  label: string;
  icon: LucideIcon;
  color: string;
  /** 默认是否显示在顶栏（false = 默认收进「更多」） */
  defaultToolbar: boolean;
  /** 懒加载组件 */
  Component: ComponentType<any>;
  /** 是否特殊模块（不可禁用、不可移入「更多」） */
  pinned?: boolean;
  /** 组件加载时的 loading 占位 */
  loading?: React.ReactNode;
}

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

// —— 模块清单（全部平级） ——
export const MODULES: ModuleDef[] = [
  {
    id: "launcher",
    label: "启动",
    icon: Rocket,
    color: "#8b5cf6",
    defaultToolbar: true,
    Component: lazyLoad(() => import("./components/launcher/LauncherPanel")),
  },
  {
    id: "news",
    label: "资讯",
    icon: Rss,
    color: "#ea580c",
    defaultToolbar: true,
    Component: lazyLoad(() => import("./components/RssReader")),
  },
  {
    id: "sdk",
    label: "SDK",
    icon: Cpu,
    color: "#2563eb",
    defaultToolbar: true,
    Component: lazyLoad(() => import("./components/ProjectManager")),
  },
  {
    id: "ai",
    label: "AI",
    icon: Bot,
    color: "#7c3aed",
    defaultToolbar: true,
    Component: lazyLoad(() => import("./components/ai/AiPanel")),
  },
  {
    id: "tasks",
    label: "任务",
    icon: CalendarCheck,
    color: "#f59e0b",
    defaultToolbar: true,
    Component: lazyLoad(() => import("./components/tasks/TaskPanel")),
  },
  {
    id: "node",
    label: "服务",
    icon: Boxes,
    color: "#0891b2",
    defaultToolbar: true,
    Component: lazyLoad(() => import("./components/node/NodeManagerPanel")),
  },
  {
    id: "mihomo",
    label: "代理",
    icon: Waypoints,
    color: "#4f46e5",
    defaultToolbar: true,
    Component: lazyLoad(() => import("./components/SystemTools/Mihomo")),
  },
  {
    id: "cert",
    label: "证书",
    icon: ShieldCheck,
    color: "#0d9488",
    defaultToolbar: true,
    Component: lazyLoad(() => import("./components/SystemTools/CertManager")),
  },
  {
    id: "clipboard",
    label: "剪贴板",
    icon: Clipboard,
    color: "#0ea5e9",
    defaultToolbar: true,
    Component: lazyLoad(() => import("./components/ClipboardPanel")),
  },
  {
    id: "otp",
    label: "OTP",
    icon: KeyRound,
    color: "#f59e0b",
    defaultToolbar: false,
    Component: lazyLoad(() => import("./components/OtpPanel")),
  },
  {
    id: "settings",
    label: "设置",
    icon: Settings,
    color: "#dc2626",
    defaultToolbar: true,
    pinned: true, // 设置模块始终在顶栏，不可禁用
    Component: lazyLoad(() => import("./components/GlobalSettings")),
  },
  // —— 以下默认收进「更多」的子模块 ——
  {
    id: "ports",
    label: "端口排查",
    icon: Network,
    color: "#0ea5e9",
    defaultToolbar: false,
    Component: lazyLoad(() => import("./components/PortScanner")),
  },
  {
    id: "backups",
    label: "环境备份",
    icon: Database,
    color: "#8b5cf6",
    defaultToolbar: false,
    Component: lazyLoad(() => import("./components/EnvBackupManager")),
  },
  {
    id: "httpServer",
    label: "HTTP 服务",
    icon: Server,
    color: "#0891b2",
    defaultToolbar: false,
    Component: lazyLoad(() => import("./components/HttpServer")),
  },
  {
    id: "rtspServer",
    label: "RTSP 服务",
    icon: Video,
    color: "#4f46e5",
    defaultToolbar: false,
    Component: lazyLoad(() => import("./components/SystemTools/RtspServer")),
  },
  {
    id: "imageBase64",
    label: "图片 Base64",
    icon: Image,
    color: "#0d9488",
    defaultToolbar: false,
    Component: lazyLoad(() => import("./components/ImageBase64")),
  },
  {
    id: "pathEnv",
    label: "PATH 变量",
    icon: ListOrdered,
    color: "#f59e0b",
    defaultToolbar: false,
    Component: lazyLoad(() => import("./components/PathEnvManager")),
  },
  {
    id: "logViewer",
    label: "日志查看",
    icon: ScrollText,
    color: "#059669",
    defaultToolbar: false,
    Component: lazyLoad(() => import("./components/LogViewer")),
  },
  {
    id: "jsonBrowser",
    label: "JSON 浏览",
    icon: Braces,
    color: "#dc2626",
    defaultToolbar: false,
    Component: lazyLoad(() => import("./components/SystemTools/JsonBrowser")),
  },
  {
    id: "markdownReader",
    label: "Markdown 阅读",
    icon: BookOpen,
    color: "#ea580c",
    defaultToolbar: false,
    Component: lazyLoad(() => import("./components/SystemTools/MarkdownReader")),
  },
];

/** 按 id 取模块定义 */
export const MODULE_MAP: Record<string, ModuleDef> = Object.fromEntries(
  MODULES.map((m) => [m.id, m])
);

/** 默认模块顺序（未自定义时） */
export const DEFAULT_MODULE_ORDER: string[] = MODULES.map((m) => m.id);

/** 默认顶栏模块 id 列表（未自定义时） */
export const DEFAULT_TOOLBAR: string[] = MODULES.filter((m) => m.defaultToolbar).map((m) => m.id);

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
  const enabled = MODULES.filter((m) => !disabled.includes(m.id));
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
