// 共享格式化工具：跨模块复用的纯函数（无业务语义，可安全抽到共享层）。
// 目标：替换各模块里手写的 formatSize/formatBytes/formatDate/千分位等重复实现。

/** 字节数 → 可读容量（B/KB/MB/GB/TB）。负值或空按 0 处理。 */
export function formatBytes(bytes: number | null | undefined, fractionDigits = 2): string {
  if (!bytes || bytes < 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  const s = v >= 100 || v % 1 === 0 ? String(Math.round(v)) : v.toFixed(fractionDigits);
  return `${s} ${units[i]}`;
}
// 常见别名（按各模块习惯调用）
export const formatSize = formatBytes;

/** 数字千分位分隔。 */
export function formatNumber(n: number | null | undefined): string {
  if (n == null || Number.isNaN(n)) return "0";
  return n.toLocaleString("en-US");
}

/** 文件大小 + 千分位字节（便于展示精确值）。 */
export function formatBytesPrecise(bytes: number | null | undefined): string {
  return `${formatNumber(bytes)} B`;
}

/** 毫秒数/秒数 → 时长（mm:ss / h:mm:ss）。 */
export function formatDuration(totalMs: number | null | undefined): string {
  const ms = Math.max(0, Math.floor(totalMs ?? 0));
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  const h = Math.floor(m / 60);
  const ss = String(s % 60).padStart(2, "0");
  const mm = String(m % 60).padStart(2, "0");
  if (h > 0) return `${h}:${mm}:${ss}`;
  return `${m}:${ss}`;
}

/** ISO/时间 → 本地日期时间（YYYY-MM-DD HH:mm）。容忍非法输入返回原串。 */
export function formatDateTime(v: string | number | Date | null | undefined): string {
  if (v == null || v === "") return "";
  const d = v instanceof Date ? v : new Date(v);
  if (Number.isNaN(d.getTime())) return String(v);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** 时间戳 → 本地日期（YYYY-MM-DD）。 */
export function formatDate(v: string | number | Date | null | undefined): string {
  if (v == null || v === "") return "";
  const d = v instanceof Date ? v : new Date(v);
  if (Number.isNaN(d.getTime())) return String(v);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/**
 * 相对时间：刚刚 / n 分钟前 / n 小时前 / 昨天 HH:mm / MM-DD / YYYY-MM-DD。
 * 兼容秒或毫秒时间戳（> 1e12 视为毫秒）。
 */
export function timeAgo(v: string | number | Date | null | undefined, now = Date.now()): string {
  if (v == null || v === "") return "";
  const d = v instanceof Date ? v : new Date(v);
  if (Number.isNaN(d.getTime())) return String(v);
  const t = typeof v === "number" && Math.abs(v) > 1e12 ? v : d.getTime();
  const diff = now - t;
  if (diff < 60_000) return "刚刚";
  const minutes = Math.floor(diff / 60_000);
  if (minutes < 60) return `${minutes} 分钟前`;
  const hours = Math.floor(diff / 3_600_000);
  if (hours < 24 && d.getDate() === new Date(now).getDate()) return `${hours} 小时前`;
  const pad = (n: number) => String(n).padStart(2, "0");
  const hm = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  const yesterday = new Date(now);
  yesterday.setDate(new Date(now).getDate() - 1);
  if (d.toDateString() === yesterday.toDateString()) return `昨天 ${hm}`;
  if (d.getFullYear() === new Date(now).getFullYear()) return `${d.getMonth() + 1}-${pad(d.getDate())}`;
  return formatDate(d);
}

/** 关键值截断：超长文本中间省略（codeblock/日志常用）。 */
export function truncateMiddle(s: string | null | undefined, maxLen = 60): string {
  if (!s) return "";
  if (s.length <= maxLen) return s;
  const half = Math.floor((maxLen - 1) / 2);
  return `${s.slice(0, half)}…${s.slice(s.length - (maxLen - 1 - half))}`;
}

/** 首尾空白清理并去空行（供输入框防呆）。 */
export function cleanWhitespace(s: string | null | undefined): string {
  return (s ?? "").replace(/\s+/g, " ").trim();
}

/** 补零 / padStart 简写。 */
export function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

/** 英文单词数 + 中文字数统计（编辑器状态栏用）。 */
export function countWords(text: string): { words: number; cjk: number; chars: number; nonWhitespace: number } {
  const chars = text.length;
  const nonWhitespace = text.replace(/\s/g, "").length;
  const cjk = (text.match(/[\u4e00-\u9fff\u3400-\u4dbf]/g) || []).length;
  const words = (text.replace(/[\u4e00-\u9fff\u3400-\u4dbf]/g, " ").match(/[A-Za-z0-9_]+(?:['’-][A-Za-z0-9_]+)*/g) || []).length;
  return { words, cjk, chars, nonWhitespace };
}