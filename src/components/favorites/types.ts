// 收藏 / 星标聚合模块的类型定义（与 src-tauri/commands/favorites 对应）

/** 一条收藏条目（后端 `FavoriteRow` 用 `rename_all = "camelCase"` 序列化） */
export interface FavoriteRow {
  id: number;
  source: string; // github | bilibili | zhihu
  externalId: string;
  url: string;
  title: string;
  subtitle?: string | null;
  description?: string | null;
  status: string; // ok | gone | redirect | unknown
  checkedAt?: string | null;
  aiLocked: boolean;
  aiModel?: string | null;
  createdAt: string;
  updatedAt: string;
  /** 多标签：一个条目可以同时属于多个分类 */
  tags: string[];
}

export interface FavoriteStats {
  total: number;
  unclassified: number;
  gone: number;
  /** 各来源条数（后端 camelCase 序列化，故不是 by_source） */
  bySource: [string, number][];
  tags: [string, number][];
}

export interface ImportResult {
  login: string;
  fetched: number;
  added: number;
  updated: number;
  skipped: number;
  cancelled: boolean;
  /** 读取失败的收藏夹（`标题（原因）`）；其余收藏夹照常导入 */
  failed?: string[];
}

export interface ClassifyResult {
  classified: number;
  tagsWritten: number;
  model: string;
  remaining: number;
  /** 用户中途点了停止（已归类的部分保留） */
  cancelled: boolean;
}

export interface CheckResult {
  checked: number;
  gone: number;
  redirect: number;
  unknown: number;
  /** 因限流提前中断 */
  aborted: boolean;
  /** 用户点了停止 */
  cancelled: boolean;
}

/** 后端实时进度事件（favorites-progress）：导入 / 归类 / 失效检测共用一个载荷 */
export interface FavoritesProgress {
  stage: "import" | "classify" | "check";
  source?: string | null;
  folder?: string | null;
  message?: string | null;
  fetched?: number | null;
  added?: number | null;
  updated?: number | null;
  skipped?: number | null;
  classified?: number | null;
  tagsWritten?: number | null;
  remaining?: number | null;
  /** 知乎专用：当前收藏夹已抓取条数 / 服务端报告的总数（Paging.Totals） */
  folderFetched?: number | null;
  folderTotal?: number | null;
  /** 失效检测专用：已探测条数 / 本轮待探测总数 */
  checked?: number | null;
  checkTotal?: number | null;
  done: boolean;
}

/** 条目正文缓存（知乎收藏内容 / GitHub README） */
export interface CachedContent {
  text: string;
  /** 来源标注：知乎是收藏夹名，GitHub 是 README 文件名 */
  label?: string | null;
  fetchedAt: string;
}

/** 凭证健康状态（后端 fav_credential_status） */
export interface CredentialStatus {
  source: string;
  configured: boolean;
  /** ok | expired | unknown */
  status: string;
  checkedAt?: string | null;
  /** 从 Cookie 里解析出的过期时间（unix 秒）；解析不出为 null */
  expiresAt?: number | null;
  updatedAt?: string | null;
}

/** 凭证是否「即将过期」（默认 3 天内）。返回剩余天数用于文案。 */
export function expiringInDays(expiresAt?: number | null, within = 3): number | null {
  if (!expiresAt) return null;
  const leftMs = expiresAt * 1000 - Date.now();
  if (leftMs <= 0) return 0;
  const days = leftMs / 86400000;
  return days <= within ? Math.ceil(days) : null;
}

export const SOURCE_LABELS: Record<string, string> = {
  github: "GitHub",
  bilibili: "B站",
  zhihu: "知乎",
};

/** 状态徽标样式：失效最醒目，未探测最弱 */
export function statusBadge(status: string): { text: string; className: string } | null {
  switch (status) {
    case "gone":
      return { text: "favorites.statusGone", className: "bg-rose-500/15 text-rose-300" };
    case "redirect":
      return { text: "favorites.statusRedirect", className: "bg-amber-500/15 text-amber-300" };
    case "unknown":
      return { text: "favorites.statusUnknown", className: "bg-slate-500/15 text-slate-400" };
    default:
      return null;
  }
}
