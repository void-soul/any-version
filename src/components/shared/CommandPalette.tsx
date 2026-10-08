// 全局命令面板（Cmd+K / Ctrl+K）：聚合「模块导航 + 全局命令」，一处直达。
//
// 为什么不用 SharedModal：全 app 约定「弹框不允许 Esc / 点遮罩关闭」，而命令面板
// 恰好相反——Esc 关闭、点遮罩关闭是它的基本手感。所以这里自己 portal 到 body，
// 只借用 `.modal-mask` 这个标记类：main.tsx 见到它就不会把 Esc 当成「隐藏窗口」。
import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { Search } from "lucide-react";

import type { PaletteItem } from "../../utils/paletteCommands";

/**
 * 顶栏按钮的开启入口。
 * 与 toast / vexSay 同一套路：模块级函数 + 组件内部订阅，
 * 调用方不必为了开一个面板去抬升状态到 App。
 */
let notifyOpen: (() => void) | null = null;

export function openCommandPalette(): void {
  notifyOpen?.();
}

export default function CommandPalette({ items }: { items: PaletteItem[] }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    notifyOpen = () => {
      setQuery("");
      setActive(0);
      setOpen(true);
    };
    return () => {
      notifyOpen = null;
    };
  }, []);

  // 全局唤起：Cmd+K（macOS）/ Ctrl+K（Windows）。输入框里也要能唤起。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setQuery("");
        setActive(0);
        setOpen((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return items;
    return items.filter(
      (it) =>
        it.label.toLowerCase().includes(q) ||
        (it.keywords ?? []).some((k) => k.toLowerCase().includes(q)) ||
        (it.hint ?? "").toLowerCase().includes(q),
    );
  }, [items, query]);

  // 换关键词 / 换数据源都回到第一条，避免停在一条已经不存在的候选上
  useEffect(() => {
    setActive(0);
  }, [query, items]);

  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  // 键盘操作：↑↓ 选择、Enter 执行、Esc 关闭（Esc 走这里而不是 main.tsx 的「隐藏窗口」）
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        setActive((i) => Math.min(i + 1, filtered.length - 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setActive((i) => Math.max(i - 1, 0));
      } else if (e.key === "Enter") {
        e.preventDefault();
        const item = filtered[active];
        if (item) {
          setOpen(false);
          item.run();
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, filtered, active]);

  // 选中项滚动进视野（键盘往下走时不然会看不见高亮）
  useEffect(() => {
    if (!open) return;
    listRef.current
      ?.querySelector<HTMLElement>(`[data-idx="${active}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [active, open]);

  if (!open) return null;

  return createPortal(
    <div
      className="modal-mask fixed inset-0 z-[400] flex items-start justify-center bg-black/55 pt-[12vh] backdrop-blur-sm"
      onClick={() => setOpen(false)}
    >
      <div
        role="dialog"
        aria-modal="true"
        className="flex w-[560px] max-w-[92vw] flex-col overflow-hidden rounded-panel border border-white/10 bg-slate-900/95 shadow-2xl shadow-black/60"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2 border-b border-white/10 px-3.5 py-3">
          <Search className="h-4 w-4 flex-shrink-0 text-slate-500" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label={t("palette.placeholder")}
            placeholder={t("palette.placeholder")}
            spellCheck={false}
            className="flex-1 bg-transparent text-body text-slate-100 placeholder-slate-500 outline-none"
          />
          <span className="flex-shrink-0 rounded border border-white/10 bg-white/5 px-1.5 py-0.5 text-micro text-slate-500">
            Esc
          </span>
        </div>

        <div ref={listRef} className="max-h-[52vh] overflow-y-auto py-1">
          {filtered.length === 0 ? (
            <p className="px-4 py-6 text-center text-caption text-slate-500">
              {t("palette.noMatch")}
            </p>
          ) : (
            filtered.map((it, i) => {
              const Icon = it.icon;
              return (
                <div
                  key={it.id}
                  data-idx={i}
                  onMouseEnter={() => setActive(i)}
                  onClick={() => {
                    setOpen(false);
                    it.run();
                  }}
                  className={`flex cursor-pointer items-center gap-2.5 px-3.5 py-2 transition-colors ${
                    i === active
                      ? "bg-[var(--module-accent-soft)] text-white"
                      : "text-slate-300 hover:bg-white/5"
                  }`}
                >
                  {Icon ? <Icon className="h-3.5 w-3.5 flex-shrink-0" /> : null}
                  <span className="min-w-0 flex-1 truncate text-body">{it.label}</span>
                  {it.hint ? (
                    <span className="flex-shrink-0 text-micro text-slate-600">{it.hint}</span>
                  ) : null}
                </div>
              );
            })
          )}
        </div>

        <div className="border-t border-white/10 px-3.5 py-2 text-micro text-slate-600">
          {t("palette.hint")}
        </div>
      </div>
    </div>,
    document.body,
  );
}
