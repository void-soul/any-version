import { AppWindow, Archive, Code2, FileText, Folder, Image, Layers, Music, Video } from "lucide-react";
export interface IndexProgress {
  drive: string;
  scanned: number;
  inserted: number;
  current: string;
  elapsedMs: number;
  done: boolean;
  canceled: boolean;
  error: string | null;
}


/** 结果类型预设：exts 为空表示不限后缀；kind=dir 只出文件夹。 */
export const FILTER_PRESETS = [
  { id: "all", kind: "all", exts: [] as string[], labelKey: "fileSearch.filterAll", icon: Layers },
  { id: "dir", kind: "dir", exts: [] as string[], labelKey: "fileSearch.filterDir", icon: Folder },
  {
    id: "image",
    kind: "file",
    exts: ["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "ico", "heic", "avif", "tif", "tiff", "psd"],
    labelKey: "fileSearch.filterImage",
    icon: Image,
  },
  {
    id: "video",
    kind: "file",
    exts: ["mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "ts", "rmvb"],
    labelKey: "fileSearch.filterVideo",
    icon: Video,
  },
  {
    id: "audio",
    kind: "file",
    exts: ["mp3", "flac", "wav", "aac", "ogg", "m4a", "wma", "ape", "opus"],
    labelKey: "fileSearch.filterAudio",
    icon: Music,
  },
  {
    id: "doc",
    kind: "file",
    exts: ["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "csv", "rtf", "epub", "one"],
    labelKey: "fileSearch.filterDoc",
    icon: FileText,
  },
  {
    id: "archive",
    kind: "file",
    exts: ["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "iso", "cab"],
    labelKey: "fileSearch.filterArchive",
    icon: Archive,
  },
  {
    id: "code",
    kind: "file",
    exts: ["rs", "ts", "tsx", "js", "jsx", "py", "go", "java", "kt", "c", "cpp", "h", "hpp", "cs", "rb", "php", "html", "css", "scss", "json", "toml", "yaml", "yml", "xml", "sh", "ps1", "sql"],
    labelKey: "fileSearch.filterCode",
    icon: Code2,
  },
  {
    id: "exec",
    kind: "file",
    exts: ["exe", "msi", "bat", "cmd", "com", "dll", "sys", "lnk"],
    labelKey: "fileSearch.filterExec",
    icon: AppWindow,
  },
];


/** 读取本地视图偏好（失败时回退默认值）。 */
export function readPref(key: string, fallback: string): string {
  try {
    return localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}


export function writePref(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* 隐私模式等场景忽略 */
  }
}


export function formatSize(bytes: number): string {
  if (!bytes || bytes < 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}


export function formatTime(sec: number): string {
  if (!sec) return "";
  return new Date(sec * 1000).toLocaleString();
}


/** 目录部分（结果行显示所在位置）。 */
export function dirOf(path: string, name: string): string {
  const cut = path.length - name.length;
  return cut > 0 ? path.slice(0, cut) : path;
}


/** 关键词高亮：取输入里最后一个词。
 *  前面的词可能是盘符/目录（如 `e pro gld 1234`），高亮它们没有意义。
 *  选中行的背景就是主题色，高亮文字必须换成对比色，否则会和背景同色看不清。 */
export function highlight(text: string, query: string, selected: boolean) {
  const tokens = query.trim().split(/\s+/);
  const keyword = tokens[tokens.length - 1] ?? "";
  if (!keyword) return text;
  const idx = text.toLowerCase().indexOf(keyword.toLowerCase());
  if (idx < 0) return text;
  return (
    <>
      {text.slice(0, idx)}
      <span
        className={`rounded-[3px] px-0.5 font-semibold ${
          selected
            ? "bg-black/25 text-white"
            : "bg-[var(--module-accent-soft)] text-[var(--module-accent)]"
        }`}
      >
        {text.slice(idx, idx + keyword.length)}
      </span>
      {text.slice(idx + keyword.length)}
    </>
  );
}
