// ════════════ 收藏时间：过滤预设换算与展示格式化 ════════════
//
// 后端存的是**本地时间字符串** `YYYY-MM-DDTHH:MM:SS`（与 created_at 同格式，
// 因此可以直接按字符串比较大小、也能拿来做 SQL 的 >= 过滤）。
// 这里只做两件事：
// ① 把「近 7 天 / 近 30 天 / 近一年」这类预设换算成该格式的起点（时区由前端定，
//    后端不猜时区）；
// ② 把时间串格式化成列表里显示的日期串，并在没有平台收藏时间时回退到入库时间。

/** 时间过滤预设。`all` = 不过滤。 */
export type SincePreset = "all" | "7d" | "30d" | "365d";

/** 排序方式：favorited=按收藏时间，created=按入库时间，updated=按最近更新。 */
export type FavoritesSort = "favorited" | "created" | "updated";

const DAY_MS = 24 * 60 * 60 * 1000;

/**
 * 时间预设 → 起点的本地时间字符串；`all`（或无法识别的值）返回 null，后端收到 null 即不过滤。
 *
 * `now` 可注入，便于断言而不依赖真实时间。
 */
export function sinceToLocalString(preset: SincePreset, now: Date = new Date()): string | null {
  const days = preset === "7d" ? 7 : preset === "30d" ? 30 : preset === "365d" ? 365 : 0;
  if (days === 0) return null;
  const target = new Date(now.getTime() - days * DAY_MS);
  const pad = (n: number) => String(n).padStart(2, "0");
  return (
    `${target.getFullYear()}-${pad(target.getMonth() + 1)}-${pad(target.getDate())}` +
    `T${pad(target.getHours())}:${pad(target.getMinutes())}:${pad(target.getSeconds())}`
  );
}

/** 取日期部分用于列表展示（`2021-03-04`）；空值或格式异常返回 null（调用方再决定回退）。 */
export function favoritedDateLabel(value: string | null | undefined): string | null {
  if (!value) return null;
  const date = String(value).slice(0, 10);
  return /^\d{4}-\d{2}-\d{2}$/.test(date) ? date : null;
}
