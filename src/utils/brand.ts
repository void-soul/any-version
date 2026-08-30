// ─── Vex 品牌统一出口 ───
// vex：暖心的桌面伙伴，安静陪着、有求必应。
// 头像资源固定从 public/logo.png 读取（/logo.png）——想换头像，直接替换
// public 下的 logo.png 即可，代码无需改动，全 App 头像同步更新。

/// 头像固定路径（public/logo.png）。换资源只替换该文件。
export const VEX_AVATAR = "/logo.png";

/// 品牌名（默认渲染用；窗口标题等仍走各自配置）。
export const VEX_NAME = "vex";

/// 角色人设（用于介绍/提示文案）。
export const VEX_PERSONA = "暖心的桌面伙伴：安静陪着，有求必应，不闹腾";

/// 常驻欢迎语：随时间轮换，赋予程序「生命力」。
/// 语气拿捏：平和、自然，像熟识的朋友随口说一句，不喊口号。
/// 覆盖几种生活场景：闲着、忙、累了歇口气、想找事做。
export const VEX_GREETINGS: string[] = [
  "我在呢，有什么想弄的，随时说一声。",
  "今天也慢慢来，我从旁边陪着。",
  "忙归忙，记得歇一歇。",
  "有需要就喊我，我一直都在。",
  "想到什么，随手告诉我一声就行。",
  "又被我逮到你开工了，加油。",
  "别急，我陪你把事情一件件理顺。",
  "弄累了就停一停，不赶这一会儿。",
  "这活儿交给我，你去喝口水。",
  "拿不定主意就先放着，不急。",
  "今天想干点啥？随便挑，我跟着你。",
  "歇够了就说一声，咱们接着来。",
  "慢慢来，事情总做得完的。",
  "我在旁边看着，需要搭手就喊我。",
];

/// 随 index 取一条轮换欢迎语（防越界）。
export function greetingAt(index: number): string {
  if (VEX_GREETINGS.length === 0) return "";
  return VEX_GREETINGS[Math.abs(index) % VEX_GREETINGS.length];
}

/// 按时段返回开场白（对话式，拟真人）：早上/上午/中午/下午/晚上/深夜。
/// 供启动欢迎 toast 与时间感知问候使用。语气平和自然，不说教不喊口号。
export function timeGreeting(d: Date = new Date()): string {
  const h = d.getHours();
  if (h < 5) return "夜深了，别熬太狠，早点歇。";
  if (h < 9) return "早呀，新的一天，慢慢来。";
  if (h < 12) return "上午好呀，精神不错嘛。";
  if (h < 14) return "中午好，记得吃口热乎的。";
  if (h < 18) return "下午好，这会儿做事刚刚好。";
  if (h < 22) return "晚上好，忙了一天辛苦了。";
  return "夜猫子，该睡了哦。";
}

// ─── 签名赛博电子风主题（统一主色，不再每模块单独设色） ───

/// 主强调色（赛博电光紫红）：全 App 的 --module-accent 统一用它，
/// 激活态/按钮/聚焦边框/高亮全部随它走。
export const VEX_CYBER_ACCENT = "#ff2d95";

/// 辅助青色（赛博双色调点缀）：部分既有的 cyan 描边/图标继续用它，
/// 与紫红形成 cyberpunk 的经典「红-青」对撞。
export const VEX_CYBER_CYAN = "#22d3ee";