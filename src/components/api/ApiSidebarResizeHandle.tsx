import { useEffect, useRef } from "react";
import type { Dispatch, RefObject, SetStateAction } from "react";
import { useTranslation } from "react-i18next";
import { ChevronLeft } from "lucide-react";

interface ApiSidebarResizeHandleProps {
  sbResizeRef: RefObject<{ moved: boolean }>;
  sidebarW: number;
  setSidebarW: Dispatch<SetStateAction<number>>;
  setSidebarCollapsed: Dispatch<SetStateAction<boolean>>;
}

/** Pure ApiSidebarResizeHandle view; its owning panel retains side effects and state. */
export function ApiSidebarResizeHandle({ sbResizeRef, sidebarW, setSidebarW, setSidebarCollapsed }: ApiSidebarResizeHandleProps) {
  const { t } = useTranslation();
  const dragRef = useRef<{ pointerId: number; startX: number; startW: number } | null>(null);
  const elementRef = useRef<HTMLDivElement | null>(null);

  // 组件在拖拽中被卸载（切换模块）时，显式释放 pointer capture 并清空状态。
  // 旧实现把 mousemove/mouseup 挂在 window 上、只在 onUp 移除，窗口外松手或
  // 卸载都会永久留下监听器并持续 setState（AUDIT-012）。
  useEffect(() => () => {
    const el = elementRef.current;
    const drag = dragRef.current;
    if (el && drag && el.hasPointerCapture?.(drag.pointerId)) {
      el.releasePointerCapture(drag.pointerId);
    }
    dragRef.current = null;
  }, []);

  const finishDrag = (pointerId: number, el: HTMLDivElement) => {
    if (dragRef.current?.pointerId !== pointerId) return;
    if (el.hasPointerCapture?.(pointerId)) el.releasePointerCapture(pointerId);
    dragRef.current = null;
  };

  return (
    <div
      ref={elementRef}
      className="absolute -right-1 top-0 z-10 flex h-full w-2.5 cursor-col-resize items-center justify-center hover:bg-white/[0.06] touch-none"
      title={t("api.resizeTip")}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        sbResizeRef.current.moved = false;
        dragRef.current = { pointerId: e.pointerId, startX: e.clientX, startW: sidebarW };
        e.currentTarget.setPointerCapture(e.pointerId);
      }}
      onPointerMove={(e) => {
        const drag = dragRef.current;
        if (!drag || drag.pointerId !== e.pointerId) return;
        if (Math.abs(e.clientX - drag.startX) > 2) sbResizeRef.current.moved = true;
        setSidebarW(Math.min(460, Math.max(170, drag.startW + (e.clientX - drag.startX))));
      }}
      onPointerUp={(e) => finishDrag(e.pointerId, e.currentTarget)}
      onPointerCancel={(e) => finishDrag(e.pointerId, e.currentTarget)}
      onLostPointerCapture={() => { dragRef.current = null; }}
    >
      <button
        type="button"
        className="flex h-6 w-2.5 items-center justify-center rounded-l bg-slate-800/80 text-slate-400 opacity-0 transition group-hover/sb:opacity-100 pointer-events-none group-hover/sb:pointer-events-auto hover:text-white cursor-pointer"
        title={t("api.collapseSidebar")}
        onClick={(e) => { e.stopPropagation(); if (!sbResizeRef.current.moved) setSidebarCollapsed(true); }}
      >
        <ChevronLeft className="h-3 w-3" />
      </button>
    </div>
  );
}
