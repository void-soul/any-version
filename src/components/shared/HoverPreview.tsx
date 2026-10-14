import { useEffect, useRef, useState, type ReactNode } from "react";

/**
 * 跟随鼠标的只读预览气泡。
 *
 * 从思维导图的节点预览抽出来共用（原先那段逻辑内嵌在 MindmapPanel 里，
 * 数据库设计的表列表要同样的交互，只能整段复制 —— 复制后两边就会各自漂移）。
 *
 * 内建了两处**容易被漏掉**的交互细节，抽出来就是为了让它们只写一次：
 * - **250ms 关闭延迟**：鼠标从触发点移向气泡的路上气泡不能消失，否则永远点不进去；
 * - **移入气泡本体时不关**：长内容要能滚动看完。靠 `hoveringRef` 守卫，
 *   定时器到点时先看鼠标在不在气泡里。
 *
 * 必须用 `fixed`：触发点可能在 `overflow-y-auto` 的列表里，`absolute` 会被裁剪，
 * 而且列表滚动时坐标会漂。
 */
export interface HoverPreviewState {
  /** 鼠标 clientX/clientY */
  x: number;
  y: number;
  /** 气泡标题 */
  title: string;
  /** 标题左侧色点与标题色（不传则不渲染色点） */
  accent?: string;
  /** 正文 */
  content: ReactNode;
}

export function useHoverPreview() {
  const [preview, setPreview] = useState<HoverPreviewState | null>(null);
  const hoveringRef = useRef(false);
  const closeTimer = useRef<number | null>(null);

  const cancelClose = () => {
    if (closeTimer.current) {
      window.clearTimeout(closeTimer.current);
      closeTimer.current = null;
    }
  };

  /** 显示/移动到某个位置 */
  const show = (next: HoverPreviewState) => {
    cancelClose();
    setPreview(next);
  };

  /** 请求关闭（延迟 250ms；鼠标已移入气泡本体则取消） */
  const scheduleHide = () => {
    cancelClose();
    closeTimer.current = window.setTimeout(() => {
      if (!hoveringRef.current) setPreview(null);
    }, 250);
  };

  /** 立刻关闭（列表滚动、侧栏收起等场景） */
  const hideNow = () => {
    cancelClose();
    hoveringRef.current = false;
    setPreview(null);
  };

  // 卸载时清掉定时器，否则会对已卸载组件 setState
  useEffect(() => cancelClose, []);

  return {
    preview,
    show,
    scheduleHide,
    hideNow,
    /** 挂在气泡本体的 onMouseEnter / onMouseLeave 上 */
    hoverGuard: {
      onMouseEnter: () => {
        hoveringRef.current = true;
        cancelClose();
      },
      onMouseLeave: () => {
        hoveringRef.current = false;
        setPreview(null);
      },
    },
  };
}

/** 气泡本体。位置由 `useHoverPreview` 的调用方给的 `x/y` 决定。 */
export function HoverPreview({
  preview,
  width = 350,
  maxHeight = 300,
  hoverGuard,
}: {
  preview: HoverPreviewState;
  width?: number;
  maxHeight?: number;
  hoverGuard: { onMouseEnter: () => void; onMouseLeave: () => void };
}) {
  const margin = 14;
  const left = Math.min(preview.x + margin, window.innerWidth - width - 30);
  const top = Math.min(preview.y + margin, window.innerHeight - maxHeight - 40);
  return (
    <div
      className="fixed z-[300]"
      style={{ left: Math.max(left, 8), top: Math.max(top, 8) }}
      onMouseEnter={hoverGuard.onMouseEnter}
      onMouseLeave={hoverGuard.onMouseLeave}
      // 浮层里滚动不该带动背后的画布/列表
      onWheel={(e) => e.stopPropagation()}
    >
      <div
        className="rounded-card border border-white/10 bg-surface-panel shadow-2xl"
        style={{ width }}
      >
        <div className="flex items-center gap-1.5 border-b border-white/10 px-3 py-2">
          {preview.accent && (
            <span
              className="h-2 w-2 shrink-0 rounded-full"
              style={{ backgroundColor: preview.accent, boxShadow: `0 0 6px ${preview.accent}` }}
            />
          )}
          <span className="min-w-0 truncate text-caption font-semibold" style={{ color: preview.accent }}>
            {preview.title}
          </span>
        </div>
        <div className="overflow-y-auto p-3" style={{ maxHeight }}>
          {preview.content}
        </div>
      </div>
    </div>
  );
}
