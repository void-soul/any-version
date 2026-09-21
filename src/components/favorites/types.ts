// 收藏 / 星标聚合模块的类型定义（与 src-tauri/commands/favorites 对应）

/** 一条收藏条目 */
export interface FavoriteRow {
  id: number;
  source: string; // github | bilibili | zhihu
  external_id: string;
  url: string;
  title: string;
  subtitle?: string | null;
  description?: string | null;
  status: string; // ok | gone | redirect | unknown
  checked_at?: string | null;
  ai_locked: boolean;
  ai_model?: string | null;
  created_at: string;
  updated_at: string;
  /** 多标签：一个条目可以同时属于多个分类 */
  tags: string[];
}

export interface FavoriteStats {
  total: number;
  unclassified: number;
  gone: number;
  by_source: [string, number][];
  tags: [string, number][];
}

export interface ImportResult {
  login: string;
  fetched: number;
  added: number;
  updated: number;
  skipped: number;
  cancelled: boolean;
}

export interface ClassifyResult {
  classified: number;
  tagsWritten: number;
  model: string;
  remaining: number;
}

export interface CheckResult {
  checked: number;
  gone: number;
  redirect: number;
  unknown: number;
  aborted: boolean;
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
