// ════════════ `@` 文件引用候选的匹配与排序 ════════════
//
// 原实现是「整条路径含关键词 → 取前 8 条」，两个问题让它看起来「匹配不到文件」：
// ① 只认子串，且排序只看路径字母序 —— 敲 `launcher` 时 `src/.../ToolLauncher.tsx`
//    可能排在第 9 条之后，正好被截掉；敲文件名片段比敲路径前缀更符合直觉，却更吃亏；
// ② 上限 8 条，同一片段命中十几个文件时只能看到前几个。
//
// 这里按「命中位置越靠前、越像目标文件」排序：文件名前缀 > 文件名包含 >
// 路径段前缀 > 路径包含；同级再按路径短者优先（短路径通常更接近用户意图）。

/** 候选条数上限：比原 8 条放宽，同时仍保证弹层不喧宾夺主。 */
export const AT_FILE_LIMIT = 12;

/** 命中等级，越小越靠前；未命中返回 null。 */
function hitRank(path: string, query: string): number | null {
  const lower = path.toLowerCase();
  const name = lower.slice(lower.lastIndexOf("/") + 1);
  if (name.startsWith(query)) return 0;
  if (name.includes(query)) return 1;
  // 路径段前缀：查询串紧跟某个 `/` 出现（`ai/tool`、`components/launcher` 这种
  // 「按目录敲」的用法），比「关键词恰好落在路径中间」更可能是有意为之
  if (lower.startsWith(query) || lower.includes(`/${query}`)) return 2;
  if (lower.includes(query)) return 3;
  return null;
}

/**
 * 从候选文件中挑出与 `query` 匹配的路径，按相关度排序后取前 `limit` 条。
 *
 * `query` 为空或全空白时返回空数组（未输入关键词时不该弹候选）。
 * 函数不修改入参数组。
 */
export function matchProjectFiles(
  files: readonly string[],
  query: string,
  limit: number = AT_FILE_LIMIT,
): string[] {
  const q = query.trim().toLowerCase();
  if (!q || limit <= 0) return [];
  const scored: { path: string; rank: number }[] = [];
  for (const path of files) {
    const rank = hitRank(path, q);
    if (rank !== null) scored.push({ path, rank });
  }
  scored.sort(
    (a, b) => a.rank - b.rank || a.path.length - b.path.length || a.path.localeCompare(b.path),
  );
  return scored.slice(0, limit).map((s) => s.path);
}
