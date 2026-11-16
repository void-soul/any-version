/**
 * X 书签导入的两条通道，选择规则集中在这里以便单测。
 *
 * - `cookie`：走 x.com 自己的 GraphQL（Cookie + 接口 ID），**免费**、能翻全量
 * - `official`：走 X 官方 API v2（OAuth 2.0），**按量计费**、只返回最近 800 条
 *
 * 两者都配置时优先免费通道：否则用户配了 OAuth 就会在不知情的情况下被计费，
 * 而结果反而更少。
 */
export type XImportChannel = "cookie" | "official" | "none";

export function pickXImportChannel(opts: {
  cookieReady: boolean;
  officialReady: boolean;
}): XImportChannel {
  if (opts.cookieReady) return "cookie";
  if (opts.officialReady) return "official";
  return "none";
}
