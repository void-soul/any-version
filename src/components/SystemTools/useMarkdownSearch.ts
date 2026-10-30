import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { collectTextRanges, escapeRegExp, type Tab } from "./MarkdownReader.model";
import type { RefObject } from "react";

/** Find/replace and preview highlighting share one bounded search state. */
export function useMarkdownSearch({ active, mode, taRef, setContent, flashNotice, previewVisible, contentRef }: {
  mode: "view" | "edit";
  active: Tab | null;
  taRef: RefObject<HTMLTextAreaElement | null>;
  setContent: (path: string, next: string, pushUndo?: boolean) => void;
  flashNotice: (msg: string) => void;
  previewVisible: boolean;
  contentRef: RefObject<HTMLDivElement | null>;
}) {
  const { t } = useTranslation();
  // 查找与替换状态
  const [findOpen, setFindOpen] = useState(false);

  const [findQuery, setFindQuery] = useState("");

  const [findReplace, setFindReplace] = useState("");

  const [findIndex, setFindIndex] = useState(0);

  const findInputRef = useRef<HTMLInputElement>(null);

  /** 查找匹配偏移（源码层），最多 500 处 */
  const findMatches = useMemo(() => {
    if (!findOpen || !findQuery || !active?.content) return [];
    const hay = active.content.toLowerCase();
    const needle = findQuery.toLowerCase();
    const out: number[] = [];
    let from = hay.indexOf(needle);
    while (from !== -1 && out.length < 500) {
      out.push(from);
      from = hay.indexOf(needle, from + Math.max(needle.length, 1));
    }
    return out;
  }, [findOpen, findQuery, active?.content]);

  // 匹配数变化时把当前索引收敛到有效范围
  useEffect(() => {
    if (findIndex >= findMatches.length) setFindIndex(0);
  }, [findMatches.length, findIndex]);

  /** 跳动到第 delta 个匹配（相对当前位置；-1 表示尚未定位） */
  const jumpToMatch = useCallback((delta: number) => {
    const total = findMatches.length;
    if (!total || !active) return;
    let next: number;
    if (findIndex < 0) next = delta >= 0 ? 0 : total - 1;
    else next = ((findIndex + delta) % total + total) % total;
    setFindIndex(next);
    if (mode !== "edit") return;
    const ta = taRef.current;
    if (!ta) return;
    const off = findMatches[next];
    ta.focus();
    ta.setSelectionRange(off, off + findQuery.length);
    const lh = Number.parseFloat(getComputedStyle(ta).lineHeight) || 24;
    const line = ta.value.slice(0, off).split("\n").length - 1;
    ta.scrollTop = Math.max(0, line * lh - ta.clientHeight / 2 + lh);
  }, [active, findIndex, findMatches, findQuery, mode]);

  /** 替换当前匹配 */
  const replaceCurrent = useCallback(() => {
    if (!active || !findMatches.length) return;
    const idx = findIndex < 0 ? 0 : Math.min(findIndex, findMatches.length - 1);
    const off = findMatches[idx];
    setContent(active.path, active.content.slice(0, off) + findReplace + active.content.slice(off + findQuery.length));
  }, [active, findMatches, findIndex, findQuery, findReplace, setContent]);

  /** 全部替换 */
  const replaceAll = useCallback(() => {
    if (!active || !findQuery) return;
    const count = findMatches.length;
    if (!count) return;
    const re = new RegExp(escapeRegExp(findQuery), "gi");
    setContent(active.path, active.content.replace(re, () => findReplace));
    flashNotice(t("mdreader.replacedCount", { n: count }));
  }, [active, findQuery, findMatches.length, findReplace, setContent, flashNotice, t]);

  // 预览态查找高亮（CSS Custom Highlight API；不支持时只做定位滚动）
  useEffect(() => {
    const api = (window as unknown as { CSS?: { highlights?: Map<string, unknown> }; Highlight?: new (...r: Range[]) => unknown });
    try { api.CSS?.highlights?.delete("md-find"); } catch { /* 忽略 */ }
    if (!findOpen || !findQuery || !previewVisible || !active || active.loading) return;
    const ranges = collectTextRanges(contentRef.current, findQuery);
    if (ranges.length && api.CSS?.highlights && api.Highlight) {
      try { api.CSS.highlights.set("md-find", new api.Highlight(...ranges)); } catch { /* 忽略 */ }
    }
    const cur = findIndex >= 0 ? ranges[Math.min(findIndex, ranges.length - 1)] : undefined;
    cur?.startContainer.parentElement?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [findOpen, findQuery, findIndex, active?.content, active?.loading, previewVisible]);
  return { findOpen, setFindOpen, findQuery, setFindQuery, findReplace, setFindReplace, findIndex, setFindIndex, findInputRef, findMatches, jumpToMatch, replaceCurrent, replaceAll };
}
