/** SSH 终端外观设置：纯逻辑 + localStorage 持久化，便于单测。 */
export interface TerminalPrefs {
  fontSize: number;
  fontFamily: string;
  theme: "dark" | "light";
}

export const FONT_FAMILIES = [
  { value: "Consolas, 'Cascadia Mono', monospace", label: "Consolas" },
  { value: "'JetBrains Mono', 'Fira Code', Consolas, monospace", label: "JetBrains Mono" },
  { value: "'Courier New', monospace", label: "Courier New" },
] as const;

export const DEFAULT_PREFS: TerminalPrefs = {
  fontSize: 13,
  fontFamily: FONT_FAMILIES[0].value,
  theme: "dark",
};

const MIN_SIZE = 8;
const MAX_SIZE = 32;
const STORAGE_KEY = "av.ssh.terminal.prefs";
/** 设置变更后广播，已打开的终端会即时套用，不必重连。 */
export const PREFS_EVENT = "av-terminal-prefs";

export function normalizePrefs(raw: unknown): TerminalPrefs {
  if (typeof raw !== "object" || raw === null) return { ...DEFAULT_PREFS };
  const o = raw as Record<string, unknown>;

  let fontSize = DEFAULT_PREFS.fontSize;
  if (typeof o.fontSize === "number" && Number.isFinite(o.fontSize)) {
    fontSize = Math.min(MAX_SIZE, Math.max(MIN_SIZE, Math.round(o.fontSize)));
  }

  const family = FONT_FAMILIES.find((f) => f.value === o.fontFamily);
  const theme = o.theme === "light" ? "light" : "dark";

  return {
    fontSize,
    fontFamily: family ? family.value : DEFAULT_PREFS.fontFamily,
    theme,
  };
}

export function loadPrefs(): TerminalPrefs {
  try {
    return normalizePrefs(JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null"));
  } catch {
    return { ...DEFAULT_PREFS };
  }
}

export function savePrefs(prefs: TerminalPrefs): TerminalPrefs {
  const next = normalizePrefs(prefs);
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    // 隐私模式/配额满：仅本次生效，不阻断用户
  }
  window.dispatchEvent(new Event(PREFS_EVENT));
  return next;
}

export function xtermTheme(theme: "dark" | "light") {
  return theme === "light"
    ? {
        background: "#f8fafc",
        foreground: "#1e293b",
        cursor: "#0284c7",
        selectionBackground: "#bae6fd",
      }
    : {
        background: "#0b0f16",
        foreground: "#e2e8f0",
        cursor: "#38bdf8",
        selectionBackground: "#1e3a5f",
      };
}
