// 列表虚拟化（无第三方依赖）：只渲染视口内的若干行，其余用上下占位高度撑起滚动条。
//
// 为什么自己做：项目里没有 react-window / react-virtuoso，而收藏列表动辄几千条
// （GitHub star + 书签 + B站 + 知乎），全量 DOM 会明显卡顿。
//
// 行高不固定（描述行数不同、展开正文、标签换行），所以：
// - 先按 `estimateHeight` 估高，保证首屏就能算对滚动高度；
// - 每行渲染后量一次真实高度，缓存到 `heights` 里，滚回去时位置不会跳。
// 纯计算部分（offsets / 二分 / 区间）单独导出，便于单测。

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

export interface VirtualRange {
  /** 首个渲染行的下标（含） */
  start: number;
  /** 最后一个渲染行的下标（不含） */
  end: number;
  /** 上方占位高度 */
  topPad: number;
  /** 下方占位高度 */
  bottomPad: number;
}

/** 每行的起始偏移表，长度 = count + 1（末位是总高度）。 */
export function buildOffsets(
  count: number,
  estimate: number,
  heights: Map<string, number>,
  keyOf: (index: number) => string,
): number[] {
  const offsets = new Array<number>(count + 1);
  offsets[0] = 0;
  for (let i = 0; i < count; i += 1) {
    const measured = heights.get(keyOf(i));
    const h = measured != null && measured > 0 ? measured : estimate;
    offsets[i + 1] = offsets[i] + h;
  }
  return offsets;
}

/** 二分：找出 `scrollTop` 落在哪一行（返回该行下标）。 */
export function findIndexAtOffset(offsets: number[], top: number): number {
  const last = offsets.length - 2; // 最后一个有效行下标
  if (last < 0) return 0;
  if (top <= 0) return 0;
  let lo = 0;
  let hi = last;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1; // 靠右取中，避免 lo 卡住不动
    if (offsets[mid] <= top) lo = mid;
    else hi = mid - 1;
  }
  return Math.min(lo, last);
}

/** 当前应该渲染的区间（含 overscan 缓冲，避免快速滚动时露白）。 */
export function computeRange(
  offsets: number[],
  count: number,
  scrollTop: number,
  viewport: number,
  overscan = 4,
): VirtualRange {
  if (count <= 0) return { start: 0, end: 0, topPad: 0, bottomPad: 0 };
  const start = Math.max(0, findIndexAtOffset(offsets, scrollTop) - overscan);
  const lastVisible = findIndexAtOffset(offsets, scrollTop + Math.max(viewport, 1));
  const end = Math.min(count, Math.max(lastVisible + 1 + overscan, start + 1));
  return {
    start,
    end,
    topPad: offsets[start],
    bottomPad: offsets[count] - offsets[end],
  };
}

export interface UseVirtualListOptions<T> {
  items: T[];
  /** 行 key（用作高度缓存的键，必须稳定） */
  getKey: (item: T, index: number) => string;
  /** 未测量前的估计行高 */
  estimateHeight?: number;
  overscan?: number;
}

export interface UseVirtualListResult<T> {
  scrollRef: React.RefObject<HTMLDivElement | null>;
  onScroll: () => void;
  range: VirtualRange;
  totalHeight: number;
  /** 每行的起始偏移（行用 `transform: translateY(offsets[index])` 定位） */
  offsets: number[];
  visible: { item: T; index: number; key: string }[];
  /** 行元素的 ref（交给渲染函数挂到行根节点上，用于量高度） */
  measureRef: (key: string) => (el: HTMLElement | null) => void;
}

export function useVirtualList<T>({
  items,
  getKey,
  estimateHeight = 72,
  overscan = 4,
}: UseVirtualListOptions<T>): UseVirtualListResult<T> {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const rafRef = useRef<number | null>(null);
  const elsRef = useRef(new Map<string, HTMLElement>());
  // getKey 每次渲染都可能是新函数：放 ref 里，避免把 offsets 的 memo 打穿
  const getKeyRef = useRef(getKey);
  getKeyRef.current = getKey;

  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(600);
  const [heights, setHeights] = useState<Map<string, number>>(() => new Map());

  const offsets = useMemo(
    () => buildOffsets(items.length, estimateHeight, heights, (i) => getKeyRef.current(items[i], i)),
    [items, heights, estimateHeight],
  );
  const range = useMemo(
    () => computeRange(offsets, items.length, scrollTop, viewport, overscan),
    [offsets, items.length, scrollTop, viewport, overscan],
  );

  // 滚动用一个 rAF 节流：滚动事件远密于帧
  const onScroll = useCallback(() => {
    if (rafRef.current != null) return;
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = null;
      const el = scrollRef.current;
      if (!el) return;
      setScrollTop(el.scrollTop);
      setViewport(el.clientHeight);
    });
  }, []);

  // 容器尺寸变化（窗口缩放 / 左右分栏拖动）也要重算可见条数
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    setViewport(el.clientHeight);
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => setViewport(el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const measureRef = useCallback(
    (key: string) => (el: HTMLElement | null) => {
      if (!el) {
        elsRef.current.delete(key);
        return;
      }
      elsRef.current.set(key, el);
    },
    [],
  );

  // 每次渲染后校正可见行的高度：展开正文 / 标签换行 / 内容异步到达都会改变行高，
  // 不校正的话滚动条长度与实际内容不一致（滚到底会跳）。
  useEffect(() => {
    let changed = false;
    const next = new Map(heights);
    for (const [key, el] of elsRef.current) {
      const h = el.offsetHeight;
      if (h > 0 && Math.abs((next.get(key) ?? -1) - h) >= 1) {
        next.set(key, h);
        changed = true;
      }
    }
    if (changed) setHeights(next);
  });

  const visible = useMemo(() => {
    const out: { item: T; index: number; key: string }[] = [];
    for (let i = range.start; i < range.end && i < items.length; i += 1) {
      out.push({ item: items[i], index: i, key: getKeyRef.current(items[i], i) });
    }
    return out;
  }, [items, range.start, range.end]);

  return {
    scrollRef,
    onScroll,
    range,
    totalHeight: offsets[items.length] ?? 0,
    offsets,
    visible,
    measureRef,
  };
}
