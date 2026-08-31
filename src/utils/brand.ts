// ─── Kira 品牌统一出口 ───
// Kira：暖心的桌面伙伴，安静陪着、有求必应。
// 头像资源固定从 public/logo.png 读取（/logo.png）——想换头像，直接替换
// public 下的 logo.png 即可，代码无需改动，全 App 头像同步更新。

import { KIRA_QUOTES, kiraQuoteText, timeGreeting as kiraTimeGreeting } from "./kiraQuotes";

/// 头像固定路径（public/logo.png）。换资源只替换该文件。
export const VEX_AVATAR = "/logo.png";

/// 品牌名（默认渲染用；窗口标题等仍走各自配置）。
export const VEX_NAME = "Kira";

/// 角色人设（用于介绍/提示文案）。
export const VEX_PERSONA = "暖心的桌面伙伴：安静陪着，有求必应，不闹腾";

/// 常驻欢迎语：统一定义在 kiraQuotes.ts（Kira 统一语句库，励志名言）。
/// 这里只是向后兼容的别名，保证旧有 import 地址不破坏。
export const VEX_GREETINGS: string[] = [
  ...KIRA_QUOTES.map((q) => q.text),
];

/// 随 index 取一条轮换欢迎语（防越界）。来自 kiraQuotes 统一库。
export function greetingAt(index: number): string {
  return kiraQuoteText(index);
}

/// 按时段返回开场白（对话式，拟真人）：早上/上午/中午/下午/晚上/深夜。
/// 供启动欢迎 toast 与时间感知问候使用。统一来自 kiraQuotes。
export function timeGreeting(d: Date = new Date()): string {
  return kiraTimeGreeting(d);
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
  const c = moduleThemeColors?.[VEX_THEME_STORE_KEY];
  if (!c || !c.trim()) return VEX_CYBER_ACCENT;
  // 仅接受合法的 hex 颜色，避免坏值污染 --module-accent。
  return /^#[0-9a-fA-F]{6}$/.test(c.trim()) ? c.trim() : VEX_CYBER_ACCENT;
}