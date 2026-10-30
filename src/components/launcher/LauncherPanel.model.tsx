import { closestCorners, pointerWithin } from "@dnd-kit/core";
import type { CollisionDetection } from "@dnd-kit/core";
import { Classification, Item, WebSearchSource } from "./types";
/**
 * 自定义碰撞检测：
 * - 优先指针命中（pointerWithin），若多个容器重叠（子分组嵌套在外层直挂区内），
 *   取 rect 面积最小的「最内层」容器 —— 否则 closestCorners 在距离都为 0 时
 *   按注册顺序取外层容器，导致空子分组永远收不到拖入的项目。
 * - 指针不落在任何 droppable 上（卡片间隙/边距）时，回退 closestCorners 保持排序手感。
 */
export const customCollisionDetection: CollisionDetection = (args) => {
  const pointer = pointerWithin(args);
  if (pointer.length > 0) {
    const innermost = [...pointer].sort((a, b) => {
      const ra = args.droppableRects.get(a.id);
      const rb = args.droppableRects.get(b.id);
      const sa = ra ? ra.width * ra.height : 0;
      const sb = rb ? rb.width * rb.height : 0;
      return sa - sb;
    })[0];
    return innermost ? [innermost] : [];
  }
  return closestCorners(args);
};


/** 卡片渲染所需的视图参数（由 LauncherPanel 里的 view 派生） */
export type ItemView = {
  iconSize: number;
  showName: boolean;
  iconBg: boolean;
  itemFontSize: number;
  itemRadius: number;
  itemBorder: boolean;
  densityCard: string;
  cardBorderClass: string;
  itemNameStyle: React.CSSProperties;
};


export function cardClass(
  item: Item,
  view: ItemView,
  checkResults: Record<number, { exists: boolean }>,
  extra = "",
): string {
  const missing = checkResults[item.id] && !checkResults[item.id].exists;
  return [
    "group relative rounded-card border cursor-pointer min-w-0 flex items-center",
    view.densityCard,
    view.cardBorderClass,
    missing
      ? "border-red-500/70 bg-red-500/10 hover:border-red-400 shadow-lg shadow-red-500/20"
      : "border-white/5 hover:border-[var(--module-accent-ring)] bg-white/[0.02] hover:bg-[var(--module-accent-soft)]",
    "active:scale-95",
    extra,
  ].join(" ");
}


/// 迁移目标里「新建一个分类」的哨兵值（分类 id 都是正整数，不会撞）。
export const NEW_CATEGORY_VALUE = "__new__";


/** 后端默认搜索引擎的兜底副本（设置里缺失时使用）。 */
export const DEFAULT_WEB_SOURCES: WebSearchSource[] = [
  { id: "1", keyword: "bd", name: "百度", url: "https://www.baidu.com/s?wd=%s", description: "百度搜索" },
  { id: "2", keyword: "gg", name: "Google", url: "https://www.google.com/search?q=%s", description: "Google 搜索" },
  { id: "3", keyword: "gh", name: "GitHub", url: "https://github.com/search?q=%s", description: "GitHub 搜索" },
  { id: "4", keyword: "bing", name: "必应", url: "https://cn.bing.com/search?q=%s", description: "微软必应搜索" },
  { id: "5", keyword: "bili", name: "哔哩哔哩", url: "https://search.bilibili.com/all?keyword=%s", description: "B站视频搜索" },
];


/** 判断输入是否像一个可直达的网址（带协议，或包含域名点号且不含空格）。 */
export function looksLikeUrl(input: string): boolean {
  const s = input.trim();
  if (!s || /\s/.test(s)) return false;
  if (/^https?:\/\//i.test(s)) return true;
  // 要求最后一段是至少 2 位的字母顶级域，避免把「3.5」这类数字误判成网址
  return /^[\w-]+(\.[\w-]+)*\.[a-z]{2,}(:\d+)?([/?#].*)?$/i.test(s);
}


/** 补全协议（用于直达网址）。 */
export function normalizeUrl(input: string): string {
  const s = input.trim();
  return /^[a-z][a-z0-9+.-]*:\/\//i.test(s) ? s : `https://${s}`;
}


/** 找到某个分类最顶层的祖先 id（分类快捷键指向子分类时，先切到其顶级分类）。 */
export function topAncestorId(list: Classification[], id: number): number {
  let cursor = list.find((c) => c.id === id);
  let guard = 0;
  while (cursor?.parentId != null && guard < 100) {
    const parent = list.find((c) => c.id === cursor!.parentId);
    if (!parent) break;
    cursor = parent;
    guard++;
  }
  return cursor?.id ?? id;
}


/** 把分类树摊平成一维（默认迁移目标候选用）。 */
export function flattenCategories(list: Classification[]): Classification[] {
  const out: Classification[] = [];
  const walk = (nodes: Classification[]) => {
    for (const n of nodes) {
      out.push(n);
      if (n.childList?.length) walk(n.childList);
    }
  };
  walk(list);
  return out;
}

export type SearchQuickAction = {
    key: string;
    title: string;
    subtitle: string;
    url: string;
    kind: "url" | "engine";
  };
