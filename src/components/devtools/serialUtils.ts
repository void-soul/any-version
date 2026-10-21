// 串口调试模块的纯工具函数与本地持久化：
// 与 React 无关，便于单测；编码/校验/行结束符的口径都集中在这里。

export type Encoding = "utf-8" | "gbk";
export type LineEnding = "none" | "lf" | "crlf" | "cr";
export type ChecksumAlgo = "none" | "crc16" | "xor" | "sum";
export type Direction = "rx" | "tx" | "sys" | "dev";
export type MatchType = "contains" | "prefix" | "exact" | "regex";

export interface PortInfo {
  name: string;
  description: string;
  vid?: number;
  pid?: number;
  serial_number?: string;
  manufacturer?: string;
}

export interface LogEntry {
  id: number;
  dir: Direction;
  text: string;
  hex?: string;
  bytes: number;
  time: string;
}

export interface SimRule {
  pattern: string;
  pattern_hex: boolean;
  match_type: MatchType;
  response: string;
  response_hex: boolean;
  append_newline: boolean;
  delay_ms: number;
}

export interface SerialPrefs {
  baud: number;
  dataBits: number;
  parity: "none" | "even" | "odd";
  stopBits: number;
  flow: "none" | "hardware" | "software";
  encoding: Encoding;
  lineEnding: LineEnding;
  hexView: boolean;
  showTime: boolean;
  autoScroll: boolean;
  checksum: ChecksumAlgo;
}

export const DEFAULT_PREFS: SerialPrefs = {
  baud: 115200,
  dataBits: 8,
  parity: "none",
  stopBits: 1,
  flow: "none",
  encoding: "utf-8",
  lineEnding: "lf",
  hexView: false,
  showTime: true,
  autoScroll: true,
  checksum: "none",
};

export const BAUD_PRESETS = [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];

const PREFS_KEY = "kira.serial.prefs";
const RULES_KEY = "kira.serial.simRules";
const HISTORY_KEY = "kira.serial.history";
const HISTORY_LIMIT = 30;

export function nowStamp(d = new Date()): string {
  return (
    d.toLocaleTimeString("zh-CN", { hour12: false }) +
    "." +
    String(d.getMilliseconds()).padStart(3, "0")
  );
}

export function emptySimRule(): SimRule {
  return {
    pattern: "",
    pattern_hex: false,
    match_type: "contains",
    response: "",
    response_hex: false,
    append_newline: true,
    delay_ms: 0,
  };
}

/** 把发送框内容切成「非空行」，兼容 CRLF / CR。 */
export function splitSendLines(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((l) => l.replace(/\r$/, ""))
    .filter((l) => l.length > 0);
}

/** base64 → 文本，按所选编码解码（GBK 需要浏览器自带 TextDecoder 支持）。 */
const decoderCache = new Map<string, TextDecoder>();
export function decodeBase64(b64: string, encoding: Encoding): string {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  let decoder = decoderCache.get(encoding);
  if (!decoder) {
    try {
      decoder = new TextDecoder(encoding === "gbk" ? "gbk" : "utf-8");
    } catch {
      decoder = new TextDecoder();
    }
    decoderCache.set(encoding, decoder);
  }
  return decoder.decode(bytes);
}

/** 解析 HEX 字符串（允许空格/逗号/冒号/短横分隔）；非法或奇数位返回 null。 */
export function parseHexBytes(text: string): number[] | null {
  const clean = text.replace(/[\s,:;-]/g, "");
  if (!/^[0-9a-fA-F]*$/.test(clean) || clean.length % 2 !== 0) return null;
  const out: number[] = [];
  for (let i = 0; i < clean.length; i += 2) out.push(parseInt(clean.slice(i, i + 2), 16));
  return out;
}

function crc16Modbus(bytes: number[]): number {
  let crc = 0xffff;
  for (const b of bytes) {
    crc ^= b;
    for (let i = 0; i < 8; i++) {
      crc = crc & 1 ? (crc >>> 1) ^ 0xa001 : crc >>> 1;
    }
  }
  return crc & 0xffff;
}

export function checksumBytes(bytes: number[], algo: ChecksumAlgo): number[] {
  switch (algo) {
    case "crc16": {
      const crc = crc16Modbus(bytes);
      // Modbus 约定低字节在前
      return [crc & 0xff, (crc >> 8) & 0xff];
    }
    case "xor":
      return [bytes.reduce((a, b) => a ^ b, 0)];
    case "sum":
      return [bytes.reduce((a, b) => a + b, 0) & 0xff];
    default:
      return [];
  }
}

const hex2 = (b: number) => b.toString(16).padStart(2, "0").toUpperCase();

/**
 * 按 HEX 解析内容并在末尾追加校验字节。
 * 返回规范化后的 HEX 文本与校验值；无法解析或未选择算法时返回 null。
 */
export function applyChecksum(
  hexText: string,
  algo: ChecksumAlgo
): { text: string; checksumHex: string } | null {
  if (algo === "none") return null;
  const bytes = parseHexBytes(hexText);
  if (!bytes || bytes.length === 0) return null;
  const checksum = checksumBytes(bytes, algo);
  return {
    text: [...bytes, ...checksum].map(hex2).join(" "),
    checksumHex: checksum.map(hex2).join(" "),
  };
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export function portMeta(p: PortInfo): string {
  const parts: string[] = [];
  if (p.vid !== undefined && p.pid !== undefined) {
    parts.push(`VID ${p.vid.toString(16).toUpperCase().padStart(4, "0")}:PID ${p.pid
      .toString(16)
      .toUpperCase()
      .padStart(4, "0")}`);
  }
  if (p.serial_number) parts.push(`SN ${p.serial_number}`);
  if (p.manufacturer) parts.push(p.manufacturer);
  return parts.join(" · ");
}

const DIR_TAG: Record<Direction, string> = { rx: "RX", tx: "TX", dev: "DEV", sys: "SYS" };

export function buildExportText(logs: LogEntry[], hexView: boolean, showTime: boolean): string {
  return logs
    .map((e) => {
      const body = hexView && e.hex !== undefined ? e.hex : e.text;
      const time = showTime ? `[${e.time}] ` : "";
      return `${time}${DIR_TAG[e.dir]} ${body}`;
    })
    .join("\n");
}

export function pushHistory(list: string[], text: string): string[] {
  const t = text.trim();
  if (!t) return list;
  return [t, ...list.filter((x) => x !== t)].slice(0, HISTORY_LIMIT);
}

function clampInt(v: unknown, min: number, max: number, fallback: number): number {
  const n = Number(v);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(max, Math.max(min, Math.round(n)));
}

export function loadPrefs(): SerialPrefs {
  try {
    const raw = localStorage.getItem(PREFS_KEY);
    if (!raw) return { ...DEFAULT_PREFS };
    const p = JSON.parse(raw) as Partial<SerialPrefs>;
    return {
      baud: clampInt(p.baud, 50, 12_000_000, DEFAULT_PREFS.baud),
      dataBits: [5, 6, 7, 8].includes(Number(p.dataBits)) ? Number(p.dataBits) : 8,
      parity: p.parity === "even" || p.parity === "odd" ? p.parity : "none",
      stopBits: p.stopBits === 2 ? 2 : 1,
      flow: p.flow === "hardware" || p.flow === "software" ? p.flow : "none",
      encoding: p.encoding === "gbk" ? "gbk" : "utf-8",
      lineEnding:
        p.lineEnding === "lf" || p.lineEnding === "crlf" || p.lineEnding === "cr"
          ? p.lineEnding
          : "none",
      hexView: p.hexView === true,
      showTime: p.showTime !== false,
      autoScroll: p.autoScroll !== false,
      checksum:
        p.checksum === "crc16" || p.checksum === "xor" || p.checksum === "sum"
          ? p.checksum
          : "none",
    };
  } catch {
    return { ...DEFAULT_PREFS };
  }
}

export function savePrefs(prefs: SerialPrefs): void {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(prefs));
  } catch {
    /* 隐私模式 / 配额满时静默失败 */
  }
}

export function loadSimRules(): SimRule[] | null {
  try {
    const raw = localStorage.getItem(RULES_KEY);
    if (!raw) return null;
    return normalizeRules(JSON.parse(raw));
  } catch {
    return null;
  }
}

export function saveSimRules(rules: SimRule[]): void {
  try {
    localStorage.setItem(RULES_KEY, JSON.stringify(rules));
  } catch {
    /* 忽略 */
  }
}

export function loadHistory(): string[] {
  try {
    const raw = localStorage.getItem(HISTORY_KEY);
    if (!raw) return [];
    const list = JSON.parse(raw);
    return Array.isArray(list) ? list.filter((x): x is string => typeof x === "string").slice(0, HISTORY_LIMIT) : [];
  } catch {
    return [];
  }
}

export function saveHistory(list: string[]): void {
  try {
    localStorage.setItem(HISTORY_KEY, JSON.stringify(list));
  } catch {
    /* 忽略 */
  }
}

/** 校验并规范化外部导入的规则 JSON。 */
export function normalizeRules(input: unknown): SimRule[] {
  if (!Array.isArray(input)) throw new Error("规则文件应为 JSON 数组");
  if (input.length === 0) throw new Error("规则文件为空");
  const matchTypes: MatchType[] = ["contains", "prefix", "exact", "regex"];
  return input.map((raw, i) => {
    if (!raw || typeof raw !== "object") throw new Error(`第 ${i + 1} 条规则格式不正确`);
    const r = raw as Record<string, unknown>;
    const str = (v: unknown) => (typeof v === "string" ? v : "");
    const mt = str(r.match_type) as MatchType;
    return {
      pattern: str(r.pattern),
      pattern_hex: r.pattern_hex === true,
      match_type: matchTypes.includes(mt) ? mt : "contains",
      response: str(r.response),
      response_hex: r.response_hex === true,
      append_newline: r.append_newline === true,
      delay_ms: clampInt(r.delay_ms, 0, 10_000, 0),
    };
  });
}
