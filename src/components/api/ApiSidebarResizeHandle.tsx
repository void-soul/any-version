import type { RefObject } from "react";
import { useTranslation } from "react-i18next";
import { ChevronLeft } from "lucide-react";
import type { Dispatch, SetStateAction } from "react";

interface ApiSidebarResizeHandleProps {
  sbResizeRef: RefObject<{ moved: boolean; }>;
  sidebarW: number;
  setSidebarW: Dispatch<SetStateAction<number>>;
  setSidebarCollapsed: Dispatch<SetStateAction<boolean>>;
}

/** Pure ApiSidebarResizeHandle view; its owning panel retains side effects and state. */
export function ApiSidebarResizeHandle({ sbResizeRef, sidebarW, setSidebarW, setSidebarCollapsed }: ApiSidebarResizeHandleProps) {
  const { t } = useTranslation();
  return (
    <div
      className="absolute -right-1 top-0 z-10 flex h-full w-2.5 cursor-col-resize items-center justify-center hover:bg-white/[0.06]"
      title={t("api.resizeTip")}
      onMouseDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        sbResizeRef.current.moved = false;
        const startX = e.clientX; const startW = sidebarW;
        const onMove = (ev: MouseEvent) => { if (Math.abs(ev.clientX - startX) > 2) sbResizeRef.current.moved = true; setSidebarW(Math.min(460, Math.max(170, startW + (ev.clientX - startX)))); };
        const onUp = () => { window.removeEventListener("mousemove", onMove); window.removeEventListener("mouseup", onUp); };
        window.addEventListener("mousemove", onMove);
        window.addEventListener("mouseup", onUp);
      }}
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
