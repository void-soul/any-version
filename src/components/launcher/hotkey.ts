/**
 * 启动器快捷键工具。
 *
 * 存储格式与后端 `parse_hotkey`（windows.rs）保持一致：
 *   "Ctrl+Shift+L"、"Alt+Space"、"F2"、"Win+V"
 *
 * 前端只负责「录制 → 规范字符串」和「按键事件 → 同样格式」的比对，
 * 真正注册由后端完成（全局快捷键走 RegisterHotKey，窗口内快捷键走 keydown）。
 */

const MODIFIER_ORDER = ["Ctrl", "Alt", "Shift", "Win"] as const;

const KEY_ALIASES: Record<string, string> = {
  " ": "Space",
  space: "Space",
  spacebar: "Space",
  escape: "Esc",
  esc: "Esc",
  arrowup: "Up",
  arrowdown: "Down",
  arrowleft: "Left",
  arrowright: "Right",
  return: "Enter",
  enter: "Enter",
  tab: "Tab",
  backspace: "Backspace",
  delete: "Delete",
  del: "Delete",
  insert: "Insert",
  home: "Home",
  end: "End",
  pageup: "PageUp",
  pagedown: "PageDown",
  "`": "`",
};

const NAMED_KEYS = new Set([
  "Space", "Esc", "Up", "Down", "Left", "Right", "Enter", "Tab",
  "Backspace", "Delete", "Insert", "Home", "End", "PageUp", "PageDown", "`",
]);

/** 把单个按键名（KeyboardEvent.key）规范化为后端可识别的名字；无法识别返回 null。 */
function normalizeKey(raw: string): string | null {
  if (!raw) return null;
  const lower = raw.toLowerCase();
  if (KEY_ALIASES[lower]) return KEY_ALIASES[lower];
  if (/^f([1-9]|1[0-2])$/.test(lower)) return lower.toUpperCase();
  if (raw.length === 1) {
    // 后端只支持 ASCII 字母/数字（以及反引号）
    if (/^[a-z0-9]$/i.test(raw)) return raw.toUpperCase();
    if (raw === "`" || raw === "~") return "`";
    return null;
  }
  return null;
}

/** 从键盘事件生成规范化的组合键字符串；仅有修饰键时返回 null。 */
export function comboFromKeyboardEvent(e: KeyboardEvent | React.KeyboardEvent): string | null {
  const key = normalizeKey(e.key);
  if (!key) return null;
  if (key === "Backspace" || key === "Delete" || key === "Esc") {
    // 单独按这些键通常用于清除，不当作快捷键
    if (!e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) return null;
  }
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Win");
  return [...mods, key].join("+");
}

/** 把任意写法（大小写/顺序不同）规范化为统一的比较字符串。 */
export function normalizeCombo(combo: string | null | undefined): string {
  if (!combo) return "";
  const parts = combo.split("+").map((p) => p.trim()).filter(Boolean);
  if (parts.length === 0) return "";
  const mods = new Set<string>();
  let key: string | null = null;
  for (const part of parts) {
    const lower = part.toLowerCase();
    if (lower === "ctrl" || lower === "control") mods.add("Ctrl");
    else if (lower === "alt") mods.add("Alt");
    else if (lower === "shift") mods.add("Shift");
    else if (lower === "win" || lower === "meta" || lower === "super") mods.add("Win");
    else key = normalizeKey(part);
  }
  if (!key) return "";
  const ordered = MODIFIER_ORDER.filter((m) => mods.has(m));
  return [...ordered, key].join("+");
}

/** 组合键是否包含 Ctrl / Alt / Win（Shift 单独不算，因为它会改变输入字符）。 */
export function isGlobalSafeCombo(combo: string): boolean {
  const parts = combo.split("+").map((p) => p.trim().toLowerCase());
  return parts.includes("ctrl") || parts.includes("alt") || parts.includes("win") || parts.includes("meta");
}

/** 该组合键是否是后端支持的字母/数字/F 键等（用于提示无效输入）。 */
export function isValidCombo(combo: string): boolean {
  const norm = normalizeCombo(combo);
  if (!norm) return false;
  const key = norm.split("+").pop() || "";
  return NAMED_KEYS.has(key) || /^[A-Z0-9]$/.test(key) || /^F([1-9]|1[0-2])$/.test(key);
}
