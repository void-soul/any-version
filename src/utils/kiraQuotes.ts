// ─── Kira 统一语句库 ───
// 所有「Kira 说出口的话」都从这里取：欢迎语、托盘悬停提示、托盘问候菜单，
// 避免文案散落在组件/后端多处。内容为励志名言名句，旨在陪你一起把事情做成。
//
// 注意：托盘 tooltip / 托盘问候菜单由后端渲染，无法直接 import 本模块。
// 前端在应用启动时用 set_tray_quote 把 kiraQuote() 推给后端，后端据此更新。

export interface KiraQuote {
  text: string;
  source?: string;
  /** 英文版（词典）：en.text=正文，en.source=出处/作者。为空时借用源字段过滤。 */
  en?: { text: string; source?: string };
}

export const KIRA_QUOTES: KiraQuote[] = [
  { text: "天行健，君子以自强不息。", source: "《周易》", en: { text: "As heaven maintains vigor through movements, a gentleman should constantly strive for self-improvement.", source: "I Ching" } },
  { text: "路漫漫其修远兮，吾将上下而求索。", source: "屈原", en: { text: "The road ahead is long and far; I will search high and low.", source: "Qu Yuan" } },
  { text: "千淘万漉虽辛苦，吹尽狂沙始到金。", source: "刘禹锡", en: { text: "After a thousand siftings and washings, the dross is blown away to reveal the gold.", source: "Liu Yuxi" } },
  { text: "宝剑锋从磨砺出，梅花香自苦寒来。", source: "《警世贤文》", en: { text: "The edge of a sword comes from sharpening; the fragrance of plum blossoms comes from the bitter cold.", source: "Jing Shi Xian Wen" } },
  { text: "穷且益坚，不坠青云之志。", source: "王勃", en: { text: "Though poor, one's will grows firmer and never falls from lofty ambition.", source: "Wang Bo" } },
  { text: "长风破浪会有时，直挂云帆济沧海。", source: "李白", en: { text: "Ride the wind and break the waves in due time; hoist the sail and cross the vast seas.", source: "Li Bai" } },
  { text: "锲而不舍，金石可镂。", source: "荀子《劝学》", en: { text: "With persistent effort, even metal and stone can be engraved.", source: "Xunzi — Encouraging Learning" } },
  { text: "千里之行，始于足下。", source: "老子", en: { text: "A journey of a thousand miles begins with a single step.", source: "Laozi" } },
  { text: "不积跬步，无以至千里。", source: "荀子《劝学》", en: { text: "Without accumulating small steps, one cannot reach a thousand miles.", source: "Xunzi — Encouraging Learning" } },
  { text: "世上无难事，只要肯登攀。", source: "毛泽东", en: { text: "Nothing in the world is difficult for one who sets their mind to it.", source: "Mao Zedong" } },
  { text: "博观而约取，厚积而薄发。", source: "苏轼", en: { text: "Read widely and choose carefully; accumulate deeply and release sparingly.", source: "Su Shi" } },
  { text: "志之所趋，无远弗届。", source: "《格言联璧》", en: { text: "Where the will points, no distance is too far.", source: "Ge Yan Lian Bi" } },
  { text: "星光不问赶路人，时光不负有心人。", en: { text: "Stars do not ask the traveler; time does not fail the devoted." } },
  { text: "越努力，越幸运。", en: { text: "The harder you try, the luckier you get." } },
  { text: "今天的苦果，是昨天的伏笔；当下的付出，是明日的花开。", en: { text: "Today's bitter fruit is yesterday's seed; today's effort is tomorrow's blossom." } },
];

/// 当前界面语言（由 i18n 初始化时写入；不依赖 i18n 模块避免循环引用）。
let uiLang: string = "zh";

/** 供 i18n 初始化/切换时同步当前语言（"zh" | "en"）。 */
export function setUiLanguage(lang: string): void {
  uiLang = lang?.startsWith("en") ? "en" : "zh";
}

/** 取一条名句：英文界面返回英译（en 字段），否则返回中文原文。 */
export function kiraQuote(index?: number): KiraQuote {
  const i =
    index === undefined
      ? Math.floor(Date.now() / 8000) // 每 8 秒换一句，与欢迎语节奏一致
      : Math.abs(index);
  const q = KIRA_QUOTES[i % KIRA_QUOTES.length];
  if (uiLang === "en" && q.en?.text) {
    return q.en.source ? { text: q.en.text, source: q.en.source, en: q.en } : { text: q.en.text, en: q.en };
  }
  return q;
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