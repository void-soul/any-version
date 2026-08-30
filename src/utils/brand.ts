// ─── Vex 品牌统一出口 ───
// vex：二次元女性角色，活力四射、个性张扬。
// 头像资源固定从 public/logo.png 读取（/logo.png）——想换头像，直接替换
// public 下的 logo.png 即可，代码无需改动，全 App 头像同步更新。

/// 头像固定路径（public/logo.png）。换资源只替换该文件。
export const VEX_AVATAR = "/logo.png";

/// 品牌名（默认渲染用；窗口标题等仍走各自配置）。
export const VEX_NAME = "vex";

/// 角色人设（用于介绍/提示文案）。
export const VEX_PERSONA = "二次元元气少女，活力四射，个性张扬";

/// 常驻欢迎语：随时间轮换，赋予程序「生命力」。
export const VEX_GREETINGS: string[] = [
  "嗨！我是 vex，今天也要活力满满地冲鸭！",
  "vex 已就位，有什么想整的？交给本元气少女就对了！",
  "嘿嘿，要不要让 vex 帮你把事情一把子搞定呀？",
  "元气能量，注入！vex 随时待命～",
  "别愁，有本姑娘在，什么活儿都能整出花来！",
  "又见面啦～今天也想干点大事呢！",
  "vex 在线营业中，随时候场！",
];

/// 随 index 取一条轮换欢迎语（防越界）。
export function greetingAt(index: number): string {
  if (VEX_GREETINGS.length === 0) return "";
  return VEX_GREETINGS[Math.abs(index) % VEX_GREETINGS.length];
}

// ─── 签名赛博电子风主题（统一主色，不再每模块单独设色） ───

/// 主强调色（赛博电光紫红）：全 App 的 --module-accent 统一用它，
/// 激活态/按钮/聚焦边框/高亮全部随它走。
export const VEX_CYBER_ACCENT = "#ff2d95";

/// 辅助青色（赛博双色调点缀）：部分既有的 cyan 描边/图标继续用它，
/// 与紫红形成 cyberpunk 的经典「红-青」对撞。
export const VEX_CYBER_CYAN = "#22d3ee";