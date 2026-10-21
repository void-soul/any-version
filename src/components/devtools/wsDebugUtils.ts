// 网络调试器（WebSocket / SSE / TCP / UDP）纯工具层。
// 只放可独立单测的逻辑：事件解析、过滤、导出、目标/偏好持久化。
import { parseHexBytes, type LineEnding } from "./serialUtils";

export type Proto = "ws" | "sse" | "tcp" | "udp";
export type Dir = "rx" | "tx" | "sys" | "event" | "open" | "close";

export interface SseMeta {
  id?: string;
  event?: string;
  data?: string;
  retry?: number;
}

export interface LogEntry {
  id: number;
  dir: Dir;
  text: string;
  hex?: string;
  bytes: number;
  time: string;
  meta?: SseMeta;
}

export interface WsPrefs {
  showTime: boolean;
  autoScroll: boolean;
  hexView: boolean;
  autoReconnect: boolean;
  lineEnding: LineEnding;
}

export type TargetMap = Record<Proto, string[]>;

export const MAX_LOGS = 2000;
export const MAX_PENDING = 5000;
export const MAX_TARGETS = 8;
export const MAX_SEND_HISTORY = 20;

const PREFS_KEY = "kira.wsdebug.prefs";
const TARGETS_KEY = "kira.wsdebug.targets";
const HISTORY_KEY = "kira.wsdebug.history";

export const PROTOS: Proto[] = ["ws", "sse", "tcp", "udp"];

export const DEFAULT_PREFS: WsPrefs = {
  showTime: true,
  autoScroll: true,
  hexView: false,
  autoReconnect: false,
  lineEnding: "none",
};

export function emptyTargets(): TargetMap {
  return { ws: [], sse: [], tcp: [], udp: [] };
}

/**
 * 解析「每行一条 Key: Value」的请求头。
 * 空行忽略；没有冒号或键为空的行走不通，直接丢弃（不把整段文本塞进 header）。
 */
export function parseHeadersText(text: string): string[][] | null {
  const pairs: string[][] = [];
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    const idx = line.indexOf(":");
    if (idx <= 0) continue;
    const name = line.slice(0, idx).trim();
    const value = line.slice(idx + 1).trim();
    if (!name || /\s/.test(name)) continue;
    pairs.push([name, value]);
  }
  return pairs.length > 0 ? pairs : null;
}

/**
 * 解析一段 SSE 事件块（后端按空行切好的原始文本）。
 * 多行 data 按 SSE 规范用 \n 拼接；注释行（以 : 开头）与未知字段忽略。
 */
export function parseSseBlock(raw: string): SseMeta {
  const meta: SseMeta = {};
  const data: string[] = [];
  for (const rawLine of raw.split(/\r?\n/)) {
    if (!rawLine || rawLine.startsWith(":")) continue;
    const idx = rawLine.indexOf(":");
    const field = idx === -1 ? rawLine : rawLine.slice(0, idx);
    let value = idx === -1 ? "" : rawLine.slice(idx + 1);
    if (value.startsWith(" ")) value = value.slice(1);
    switch (field) {
      case "event":
        meta.event = value;
        break;
      case "id":
        meta.id = value;
        break;
      case "retry": {
        const n = Number(value);
        if (Number.isFinite(n)) meta.retry = n;
        break;
      }
      case "data":
        data.push(value);
        break;
      default:
        break;
    }
  }
  if (data.length > 0) meta.data = data.join("\n");
  return meta;
}

/** 把一次成功连接的目标记到对应协议的历史里（去重、最近的在前、上限 8 条）。 */
export function pushTarget(map: TargetMap, proto: Proto, value: string): TargetMap {
  const v = value.trim();
  if (!v) return map;
  return {
    ...map,
    [proto]: [v, ...map[proto].filter((x) => x !== v)].slice(0, MAX_TARGETS),
  };
}

function readJson<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return fallback;
    return JSON.parse(raw) as T;
  } catch {
    return fallback;
  }
}

function writeJson(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* 隐私模式 / 配额满时静默失败 */
  }
}

export function loadPrefs(): WsPrefs {
  const p = readJson<Partial<WsPrefs>>(PREFS_KEY, {});
  return {
    showTime: p.showTime !== false,
    autoScroll: p.autoScroll !== false,
    hexView: p.hexView === true,
    autoReconnect: p.autoReconnect === true,
    lineEnding:
      p.lineEnding === "lf" || p.lineEnding === "crlf" || p.lineEnding === "cr"
        ? p.lineEnding
        : "none",
  };
}

export function savePrefs(prefs: WsPrefs): void {
  writeJson(PREFS_KEY, prefs);
}

export function loadTargets(): TargetMap {
  const raw = readJson<Partial<TargetMap>>(TARGETS_KEY, {});
  const out = emptyTargets();
  for (const proto of PROTOS) {
    const list = raw[proto];
    if (Array.isArray(list)) {
      out[proto] = list
        .filter((x): x is string => typeof x === "string" && x.trim().length > 0)
        .slice(0, MAX_TARGETS);
    }
  }
  return out;
}

export function saveTargets(map: TargetMap): void {
  writeJson(TARGETS_KEY, map);
}

export function loadSendHistory(): string[] {
  const raw = readJson<unknown>(HISTORY_KEY, []);
  return Array.isArray(raw)
    ? raw.filter((x): x is string => typeof x === "string").slice(0, MAX_SEND_HISTORY)
    : [];
}

export function saveSendHistory(list: string[]): void {
  writeJson(HISTORY_KEY, list.slice(0, MAX_SEND_HISTORY));
}

/** 日志过滤：文本、HEX、SSE 事件元信息都参与匹配，大小写不敏感。 */
export function filterLogs(logs: LogEntry[], query: string): LogEntry[] {
  const q = query.trim().toLowerCase();
  if (!q) return logs;
  return logs.filter((e) => {
    const meta = e.meta
      ? `${e.meta.event ?? ""} ${e.meta.id ?? ""} ${e.meta.data ?? ""}`
      : "";
    return (
      e.text.toLowerCase().includes(q) ||
      (e.hex ?? "").toLowerCase().includes(q) ||
      meta.toLowerCase().includes(q)
    );
  });
}

const DIR_TAG: Record<Dir, string> = {
  rx: "RX",
  tx: "TX",
  sys: "SYS",
  event: "EVENT",
  open: "OPEN",
  close: "CLOSE",
};

export function buildExportText(
  logs: LogEntry[],
  hexView: boolean,
  showTime: boolean
): string {
  return logs
    .map((e) => {
      const body = hexView && e.hex !== undefined ? e.hex : e.text;
      const time = showTime ? `[${e.time}] ` : "";
      const tags = [
        e.meta?.event ? `event=${e.meta.event}` : "",
        e.meta?.id ? `id=${e.meta.id}` : "",
      ]
        .filter(Boolean)
        .join(" ");
      const meta = tags ? ` {${tags}}` : "";
      return `${time}${DIR_TAG[e.dir]}${meta} ${body}`;
    })
    .join("\n");
}

/** 连接时长：HH:MM:SS（超过 24h 继续累计小时）。 */
export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const two = (n: number) => String(n).padStart(2, "0");
  return `${two(h)}:${two(m)}:${two(s)}`;
}

/** 只给 TCP/UDP 文本发送追加行结束符；WS/SSE 不追加。 */
export function withLineEnding(text: string, le: LineEnding): string {
  switch (le) {
    case "lf":
      return `${text}\n`;
    case "crlf":
      return `${text}\r\n`;
    case "cr":
      return `${text}\r`;
    default:
      return text;
  }
}

/** HEX 规范化：合法返回大写空格分隔文本，非法返回 null（用于发送前校验与展示）。 */
export function normalizeHex(text: string): string | null {
  const bytes = parseHexBytes(text);
  if (!bytes) return null;
  return bytes.map((b) => b.toString(16).padStart(2, "0").toUpperCase()).join(" ");
}
