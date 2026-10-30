
// ─── 类型（camelCase，与后端 / Flutter 端一致） ───

export interface PickyBookmark {
  id: string;
  title: string;
  description?: string | null;
  url?: string | null;
  imageUrl?: string | null;
  faviconUrl?: string | null;
  createdAt: string;
  updatedAt: string;
  refined: boolean;
  metaFetched: boolean;
  /** 页面正文（纯文本）：PC 端与专用 APP 通过 S3 同步此字段；为空表示尚未抓取 */
  content?: string | null;
  [key: string]: unknown;
}


export interface PickyComment {
  id: string;
  bookmarkId: string;
  content: string;
  createdAt: string;
  updatedAt: string;
  parentId?: string | null;
}


export interface PickyTag {
  id: string;
  name: string;
  color: string;
  createdAt: string;
}


export interface PickyState {
  bookmarks: PickyBookmark[];
  comments: PickyComment[];
  tags: PickyTag[];
  bookmarkTags: Record<string, string[]>;
}


export interface PickySyncConfig {
  endpoint?: string | null;
  region: string;
  accessKeyId: string;
  secretAccessKey: string;
  bucketName: string;
  prefix?: string | null;
  enabled: boolean;
  lastSyncAt?: string | null;
  addressingStyle: string;
  tlsVerify: boolean;
  timeoutSeconds: number;
  concurrentReqs: number;
}


// ─── 工具 ───

export function fmtTime(iso?: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  return d.toLocaleString("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}


export function hostOf(url?: string | null): string {
  if (!url) return "";
  try {
    return new URL(url).hostname;
  } catch {
    return url;
  }
}


// ─── 面板 ───

export type Tab = "all" | "active" | "archived";
