// ─── Kira 统一语句库 ───
// 所有「Kira 说出口的话」都从这里取：欢迎语、托盘悬停提示、托盘问候菜单，
// 避免文案散落在组件/后端多处。内容为励志名言名句，旨在陪你一起把事情做成。
//
// 注意：托盘 tooltip / 托盘问候菜单由后端渲染，无法直接 import 本模块。
// 前端在应用启动时用 set_tray_quote 把 kiraQuote() 推给后端，后端据此更新。

/// 一句励志名言（text=名句正文，source=出处/作者，可选）。
export interface KiraQuote {
  text: string;
  source?: string;
}

export const KIRA_QUOTES: KiraQuote[] = [
  { text: "天行健，君子以自强不息。", source: "《周易》" },
  { text: "路漫漫其修远兮，吾将上下而求索。", source: "屈原" },
  { text: "千淘万漉虽辛苦，吹尽狂沙始到金。", source: "刘禹锡" },
  { text: "宝剑锋从磨砺出，梅花香自苦寒来。", source: "《警世贤文》" },
  { text: "穷且益坚，不坠青云之志。", source: "王勃" },
  { text: "长风破浪会有时，直挂云帆济沧海。", source: "李白" },
  { text: "锲而不舍，金石可镂。", source: "荀子《劝学》" },
  { text: "千里之行，始于足下。", source: "老子" },
  { text: "不积跬步，无以至千里。", source: "荀子《劝学》" },
  { text: "世上无难事，只要肯登攀。", source: "毛泽东" },
  { text: "博观而约取，厚积而薄发。", source: "苏轼" },
  { text: "志之所趋，无远弗届。", source: "《格言联璧》" },
  { text: "星光不问赶路人，时光不负有心人。" },
  { text: "越努力，越幸运。" },
  { text: "今天的苦果，是昨天的伏笔；当下的付出，是明日的花开。" },
];

/// 轮换取一句励志名言。index 省略时按当前时间轮换，保证每次调用都换一句。
export function kiraQuote(index?: number): KiraQuote {
  const i =
    index === undefined
      ? Math.floor(Date.now() / 8000) // 每 8 秒换一句，与欢迎语节奏一致
      : Math.abs(index);
  return KIRA_QUOTES[i % KIRA_QUOTES.length];
}

/// 同 kiraQuote，但返回纯文字的简短版（名句正文，不带出处）。
export function kiraQuoteText(index?: number): string {
  return kiraQuote(index).text;
}

/// 名句 + 出处：`正文 —— 出处`；无出处则只回正文。供展示更「名言感」的场合。
export function kiraQuoteLine(index?: number): string {
  const q = kiraQuote(index);
  return q.source ? `${q.text} —— ${q.source}` : q.text;
}

/// 按时段返回开场白（对话式，拟真人）：早上/上午/中午/下午/晚上/深夜。
/// 保留时间感知的开场，再接一句励志名言，让 Kira 既应景又打气。
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