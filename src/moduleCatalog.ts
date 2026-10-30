import {
  Rocket,
  Rss,
  Cpu,
  Bot,
  Brain,
  Boxes,
  Waypoints,
  ShieldCheck,
  Clipboard,
  Settings,
  Activity,
  Server,
  Video,
  Image,
  ListOrdered,
  ScrollText,
  Braces,
  BookOpen,
  KeyRound,
  Languages,
  Bookmark,
  FlaskConical,
  Usb,
  Cable,
  UserRoundCog,
  Music,
  Music2,
  Star,
  CalendarDays,
  Database,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import i18n from "./i18n/index";

/** Pure navigation metadata. It never imports business pages or their loaders. */
export interface ModuleMeta {
  id: string;
  label: string;
  icon: LucideIcon;
  color: string;
  defaultToolbar: boolean;
  pinned?: boolean;
}

export const MODULE_CATALOG: ModuleMeta[] = [
  {
    id: "launcher",
    label: "启动",
    icon: Rocket,
    color: "#8b5cf6",
    defaultToolbar: true,
  },
  {
    id: "news",
    label: "资讯",
    icon: Rss,
    color: "#ea580c",
    defaultToolbar: true,
  },
  {
    id: "sdk",
    label: "SDK",
    icon: Cpu,
    color: "#2563eb",
    defaultToolbar: true,
  },
  {
    id: "ai",
    label: "AI",
    icon: Bot,
    color: "#7c3aed",
    defaultToolbar: true,
  },
  {
    id: "mindmap",
    label: "思维导图",
    icon: Brain,
    color: "#f59e0b",
    defaultToolbar: true,
  },
  {
    id: "api",
    label: "API",
    icon: FlaskConical,
    color: "#06b6d4",
    defaultToolbar: true,
  },

  {
    id: "node",
    label: "服务",
    icon: Boxes,
    color: "#0891b2",
    defaultToolbar: true,
  },
  {
    id: "mihomo",
    label: "代理",
    icon: Waypoints,
    color: "#4f46e5",
    defaultToolbar: true,
  },
  {
    id: "cert",
    label: "证书",
    icon: ShieldCheck,
    color: "#0d9488",
    defaultToolbar: true,
  },
  {
    id: "clipboard",
    label: "剪贴板",
    icon: Clipboard,
    color: "#0ea5e9",
    defaultToolbar: true,
  },
  {
    id: "otp",
    label: "OTP",
    icon: KeyRound,
    color: "#f59e0b",
    defaultToolbar: false,
  },
  {
    id: "translate",
    label: "翻译",
    icon: Languages,
    color: "#10b981",
    defaultToolbar: true,
  },
  {
    id: "picky",
    label: "Picky",
    icon: Bookmark,
    color: "#e11d48",
    defaultToolbar: false,
  },
  {
    id: "buddy",
    label: "Buddy",
    icon: UserRoundCog,
    color: "#0d9488",
    defaultToolbar: false,
  },
  {
    id: "settings",
    label: "设置",
    icon: Settings,
    color: "#dc2626",
    defaultToolbar: true,
    pinned: true, // 设置模块始终在顶栏，不可禁用
  },
  // —— 以下默认收进「更多」的子模块 ——
  {
    id: "network",
    label: "本地网络",
    icon: Activity,
    color: "#0ea5e9",
    defaultToolbar: false,
  },
  {
    id: "httpServer",
    label: "HTTP 服务",
    icon: Server,
    color: "#0891b2",
    defaultToolbar: false,
  },
  {
    id: "rtspServer",
    label: "RTSP 服务",
    icon: Video,
    color: "#4f46e5",
    defaultToolbar: false,
  },
  {
    id: "imageBase64",
    label: "图片 Base64",
    icon: Image,
    color: "#0d9488",
    defaultToolbar: false,
  },
  {
    id: "env",
    label: "环境变量",
    icon: ListOrdered,
    color: "#f59e0b",
    defaultToolbar: false,
  },
  {
    id: "logViewer",
    label: "日志查看",
    icon: ScrollText,
    color: "#059669",
    defaultToolbar: false,
  },
  {
    id: "jsonBrowser",
    label: "JSON 浏览",
    icon: Braces,
    color: "#dc2626",
    defaultToolbar: false,
  },
  {
    id: "markdownReader",
    label: "Markdown",
    icon: BookOpen,
    color: "#ea580c",
    defaultToolbar: false,
  },
  // —— 开发者工具（默认收进「更多」） ——
  {
    id: "serial",
    label: "串口调试",
    icon: Usb,
    color: "#14b8a6",
    defaultToolbar: false,
  },
  {
    id: "wsDebugger",
    label: "网络调试器",
    icon: Cable,
    color: "#6366f1",
    defaultToolbar: false,
  },
  // —— 音乐播放器（纯本地，默认收进「更多」） ——
  {
    id: "music",
    label: "音乐播放器",
    icon: Music,
    color: "#8b5cf6",
    defaultToolbar: false,
  },
  // —— 收藏 / 星标聚合（只读导入 + AI 归类）——
  {
    id: "favorites",
    label: "收藏",
    icon: Star,
    color: "#f59e0b",
    defaultToolbar: false,
  },
  // —— Suno 音频下载（解析用户主页 → 下载 m4a → 转 mp3）——
  {
    id: "suno",
    label: "Suno 下载",
    icon: Music2,
    color: "#ec4899",
    defaultToolbar: false,
  },
  // —— 任务计划（全屏日历；独立库 tasks.db，与思维导图解耦）——
  {
    id: "tasks",
    label: "任务计划",
    icon: CalendarDays,
    color: "#f59e0b",
    defaultToolbar: false,
  },
  // —— 数据库设计器（画表/视图 → 导出建表 SQL；纯文件 .dbdesign.json）——
  {
    id: "dbdesigner",
    label: "数据库设计",
    icon: Database,
    color: "#0ea5e9",
    defaultToolbar: false,
  },
];

/** 按 id 取模块定义 */
export const MODULE_CATALOG_MAP: Record<string, ModuleMeta> = Object.fromEntries(
  MODULE_CATALOG.map((m) => [m.id, m])
);

/**
 * 模块显示名：优先取 i18n 翻译（modules.<id>），翻译缺失时回退注册表里的 label。
 * 避免在导航/设置等高频处硬编码，切换语言后模块名即时更新。
 */
export function moduleLabel(id: string): string {
  const key = `modules.${id}`;
  try {
    const translated = i18n.t(key);
    if (translated && translated !== key) return translated;
  } catch {
    /* 忽略：回退注册表 label */
  }
  return MODULE_CATALOG_MAP[id]?.label ?? id;
}

/** 默认模块顺序（未自定义时） */
export const DEFAULT_MODULE_ORDER: string[] = MODULE_CATALOG.map((m) => m.id);

/** 默认顶栏模块 id 列表（未自定义时） */
export const DEFAULT_TOOLBAR: string[] = MODULE_CATALOG.filter((m) => m.defaultToolbar).map((m) => m.id);
