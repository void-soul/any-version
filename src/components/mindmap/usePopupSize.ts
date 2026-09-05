// ════════════ 思维导图悬浮窗高度管理（节点速记 / 贴纸速记共用） ════════════
// 两种模式：
// - 自动（默认）：内容变化时窗口高度贴合「自然内容高度」。
//   关键：内容容器是 flex-1，flex-1 子项（如贴纸输入框）的高度由当前窗口
//   高度决定——直接测量会把窗口高度当成内容高度，导致窗口永远停在初始
//   高度（贴纸窗 900px 的「默认太高」根因）。测量前临时把内容容器设为
//   flex:none + height:auto，测完还原，拿到真实自然高度。
// - 手动（动态调整）：用户拖底部把手（或窗口边缘）调整后进入手动模式，
//   停止自动贴合，高度记入 localStorage（窗口复用 hide/show 不丢）；
//   双击把手恢复自动模式。

import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";

const LS_PREFIX = "any_version_mm_pop_h_";

export interface PopupSizeOptions {
  /** 悬浮窗标识（localStorage 键区分：node / sticker） */
  kind: "node" | "sticker";
  /** 悬浮窗根容器（h-screen flex-col） */
  rootRef: React.RefObject<HTMLDivElement | null>;
  /** 内容滚动容器（flex-1；测量时临时取消拉伸） */
  contentRef: React.RefObject<HTMLDivElement | null>;
  /** 最小高度（px） */
  minH?: number;
}

export interface PopupSizeApi {
  /** 手动模式下的当前高度（自动模式为 null） */
  manualH: number | null;
  /** 底部高度拖拽把手的事件绑定 */
  gripHandlers: {
    onPointerDown: (e: React.PointerEvent) => void;
    onPointerMove: (e: React.PointerEvent) => void;
    onPointerUp: (e: React.PointerEvent) => void;
    onDoubleClick: () => void;
  };
}

export function usePopupSize(opts: PopupSizeOptions): PopupSizeApi {
  const { kind, rootRef, contentRef, minH = 260 } = opts;
  const lsKey = LS_PREFIX + kind;
  const [manualH, setManualH] = useState<number | null>(null);
  const manualRef = useRef<number | null>(null);
  // 自身 setSize 引发的 resize 事件标记（150ms 内忽略，避免误判为手动调整）
  const selfResizeRef = useRef(false);
  const dragRef = useRef<{ startY: number; startH: number; moved: boolean } | null>(null);

  // 挂载时恢复用户上次手动调整的高度
  useEffect(() => {
    try {
      const v = Number(localStorage.getItem(lsKey) ?? "");
      if (Number.isFinite(v) && v > 0) { manualRef.current = v; setManualH(v); }
    } catch { /* 忽略 */ }
  }, [lsKey]);

  const applySize = useCallback((h: number) => {
    try {
      const maxH = Math.max(minH, window.screen.availHeight - 40);
      const clamped = Math.round(Math.min(maxH, Math.max(minH, h)));
      if (Math.abs(clamped - window.innerHeight) < 4) return; // 已就位，不再设（防抖动循环）
      selfResizeRef.current = true;
      void getCurrentWindow()
        .setSize(new LogicalSize(window.innerWidth, clamped))
        .catch(() => { /* 浏览器预览等无 Tauri 环境静默降级 */ })
        .finally(() => { window.setTimeout(() => { selfResizeRef.current = false; }, 150); });
    } catch { /* 忽略 */ }
  }, [minH]);

  // 自动贴合：测量「根容器所有子项自然高度之和」（内容容器临时取消 flex 拉伸）
  const fitWindow = useCallback(() => {
    if (manualRef.current !== null) return;
    try {
      const root = rootRef.current;
      const el = contentRef.current;
      if (!root || !el) return;
      const prevFlex = el.style.flex;
      const prevHeight = el.style.height;
      el.style.flex = "none";
      el.style.height = "auto";
      let natural = 0;
      for (const k of Array.from(root.children) as HTMLElement[]) natural += k.getBoundingClientRect().height;
      el.style.flex = prevFlex;
      el.style.height = prevHeight;
      natural += 2; // 根容器上下边框
      const maxH = Math.max(minH, window.screen.availHeight - 40);
      const desired = Math.round(Math.min(maxH, Math.max(minH, natural)));
      applySize(desired);
    } catch { /* 忽略 */ }
  }, [rootRef, contentRef, minH, applySize]);

  // 内容尺寸变化（ResizeObserver）→ 自动模式贴合；手动模式不响应（不覆盖用户高度）
  useEffect(() => {
    const el = contentRef.current;
    if (!el) return;
    let raf = 0;
    const schedule = () => { cancelAnimationFrame(raf); raf = requestAnimationFrame(fitWindow); };
    const ro = new ResizeObserver(schedule);
    ro.observe(el);
    for (const k of el.children) ro.observe(k);
    schedule();
    return () => { ro.disconnect(); cancelAnimationFrame(raf); };
  }, [contentRef, fitWindow]);

  // 用户拖窗口边缘 resize（非本组件发起）→ 进入手动模式并记住高度
  useEffect(() => {
    let un: (() => void) | undefined;
    try {
      void getCurrentWindow().listen("tauri://resize", (e) => {
        if (selfResizeRef.current) return;
        const h = (e.payload as { height?: number })?.height;
        if (!h || h < 100) return;
        manualRef.current = h;
        setManualH(h);
        try { localStorage.setItem(lsKey, String(Math.round(h))); } catch { /* 忽略 */ }
      }).then((f) => { un = f; });
    } catch { /* 浏览器预览无 Tauri 窗口事件 */ }
    return () => { un?.(); };
  }, [lsKey]);

  const gripHandlers: PopupSizeApi["gripHandlers"] = {
    onPointerDown: (e) => {
      e.preventDefault();
      (e.target as HTMLElement).setPointerCapture?.(e.pointerId);
      dragRef.current = { startY: e.clientY, startH: window.innerHeight, moved: false };
    },
    onPointerMove: (e) => {
      const d = dragRef.current;
      if (!d) return;
      const dy = e.clientY - d.startY;
      if (!d.moved && Math.abs(dy) < 3) return;
      d.moved = true;
      const clamped = Math.max(minH, d.startH + dy);
      manualRef.current = clamped;
      setManualH(clamped);
      applySize(clamped);
    },
    onPointerUp: () => {
      const d = dragRef.current;
      dragRef.current = null;
      if (d?.moved && manualRef.current !== null) {
        try { localStorage.setItem(lsKey, String(Math.round(manualRef.current))); } catch { /* 忽略 */ }
      }
    },
    onDoubleClick: () => {
      // 双击把手：恢复自动贴合
      manualRef.current = null;
      setManualH(null);
      try { localStorage.removeItem(lsKey); } catch { /* 忽略 */ }
      fitWindow();
    },
  };

  return { manualH, gripHandlers };
}
