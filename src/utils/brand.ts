// ─── Kira 品牌统一出口 ───
// Kira：桌面开发助理，安静干活、随叫随到。
// 头像资源固定从 public/logo.png 读取（/logo.png）——想换头像，直接替换
// public 下的 logo.png 即可，代码无需改动，全 App 头像同步更新。

import { KIRA_QUOTES, kiraQuoteText } from "./kiraQuotes";

/// 头像固定路径（public/logo.png）。换资源只替换该文件。
export const VEX_AVATAR = "/logo.png";

/// 品牌名（默认渲染用；窗口标题等仍走各自配置）。
export const VEX_NAME = "Kira";

/// 常驻欢迎语：统一定义在 kiraQuotes.ts（Kira 统一语句库，励志名言）。
/// 这里只是向后兼容的别名，保证旧有 import 地址不破坏。
export const VEX_GREETINGS: string[] = [
  ...KIRA_QUOTES.map((q) => q.text),
];

/// 随 index 取一条轮换欢迎语（防越界）。来自 kiraQuotes 统一库。
export function greetingAt(index: number): string {
  return kiraQuoteText(index);
}

// ─── 签名赛博电子风主题（统一主色，可在设置里动态改） ───

/// 全 App 主强调色的默认签名色（赛博电光紫红）。
/// 用户可在设置里覆盖，保存到后端 module_theme_colors 的 `theme` 键。
export const VEX_CYBER_ACCENT = "#ff2d95";

/// 辅助青色（赛博双色调点缀）：部分既有的 cyan 描边/图标继续用它，
/// 与主色形成 cyberpunk 的经典「红-青」对撞。
export const VEX_CYBER_CYAN = "#22d3ee";

/// 全 App 主色在后端外观配置里占用的保留 key（存在 module_theme_colors["theme"]）。
/// 复用现有 set_module_theme_color / get_appearance_config，无需改 Rust。
export const VEX_THEME_STORE_KEY = "theme";

/// 预设主题色盘，供设置里一键挑选。
export const VEX_THEME_PRESETS: string[] = [
  "#ff2d95", // 电光紫红（默认）
  "#a855f7", // 霓虹紫
  "#22d3ee", // 电光青
  "#34d399", // 青绿
  "#f59e0b", // 琥珀
  "#ef4444", // 焰红
  "#3b82f6", // 磐石蓝
  "#f472b6", // 樱粉
];

/// 从后端外观配置里解析全 App 主色。
/// 取 module_theme_colors[VEX_THEME_STORE_KEY]，缺失/非法时回退默认签名色。
/// 供 App 主体注入 --module-accent、及悬浮窗读取主色使用。
export function resolveThemeAccent(moduleThemeColors?: Record<string, string>): string {
  return normalizeThemeAccent(moduleThemeColors?.[VEX_THEME_STORE_KEY]) ?? VEX_CYBER_ACCENT;
}

// ─── 主题色首帧恢复 ───
// 外观配置来自后端（invoke get_appearance_config），首次渲染时尚未到达，
// 于是首帧只能用默认色、随后跳变成用户主题色。这里把 accent 缓存到 localStorage，
// 并在 React 首次渲染前同步预置到 documentElement，消除这一帧闪烁
// （抄自 EchoBird b1b868b3 `restore saved palette before first paint`）。

/// 主题色的本地缓存 key。
export const VEX_THEME_CACHE_KEY = "vex_theme_accent";

/// 校验并规范化一个主色值：仅接受 6 位 hex（可带首尾空白），其余返回 null。
/// 后端配置值与本地缓存值共用，保证两处判定一致。
export function normalizeThemeAccent(
  value: string | null | undefined,
): string | null {
  if (!value) return null;
  const trimmed = value.trim();
  return /^#[0-9a-fA-F]{6}$/.test(trimmed) ? trimmed : null;
}

/// accent 对应的全部 CSS 变量。
/// App 主体注入与首帧预置共用同一份定义，避免两处变量漂移。
export function themeAccentVars(accent: string): Record<string, string> {
  return {
    "--module-accent": accent,
    "--module-accent-soft": `color-mix(in srgb, ${accent} 12%, transparent)`,
    "--module-accent-ring": `color-mix(in srgb, ${accent} 30%, transparent)`,
    "--module-accent-strong": `color-mix(in srgb, ${accent} 85%, white)`,
    "--neon": accent,
    "--cyan": VEX_CYBER_CYAN,
  };
}

/// 写入本地缓存（隐私模式/存储被禁用时静默失败，不影响主流程）。
export function cacheThemeAccent(accent: string): void {
  try {
    localStorage.setItem(VEX_THEME_CACHE_KEY, accent);
  } catch {
    /* private mode */
  }
}

/// 读取本地缓存的主色；无缓存或非法值返回 null。
export function readCachedThemeAccent(): string | null {
  try {
    return normalizeThemeAccent(localStorage.getItem(VEX_THEME_CACHE_KEY));
  } catch {
    return null;
  }
}

/// 在 React 首次渲染前把主色同步预置到 documentElement。
/// 未命中缓存时用默认签名色 —— 与后端返回默认值时结果一致，不会多一次跳变。
export function preapplyThemeAccent(): void {
  const accent = readCachedThemeAccent() ?? VEX_CYBER_ACCENT;
  const vars = themeAccentVars(accent);
  for (const [key, value] of Object.entries(vars)) {
    document.documentElement.style.setProperty(key, value);
  }
}