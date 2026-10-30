import { useCallback, useEffect, useRef, useState } from "react";
import { FIELD_ROW_MODES, type FieldRowMode, type DbDesignDocument } from "./types";
import { DEFAULT_DOCK_HEIGHT, DOCK_HEIGHT_KEY, FIELD_ROWS_KEY, MIN_CANVAS_HEIGHT, MIN_DOCK_HEIGHT } from "./DbDesignerPanel.model";

/** Owns the DbEditorDock feature, independently of page rendering. */
export function useDbEditorDock({ selectedId, doc }: {
  selectedId: string | null;
  doc: DbDesignDocument | null;
}) {

  // 供应商 / 模型列表来自 AI 模块的现有配置（与思维导图、API 模块同一份）

  const [dockOpen, setDockOpen] = useState(true);

  /** 用户手动收起后上锁：即使有选中也不自动展开（否则一点「收起」立刻被弹回来）。
   *  解锁时机 = 再次点展开按钮，或点画布取消选中（那条规则本身就是「收缩」）。 */
  const dockLockedRef = useRef(false);

  /** dock 高度（px）。拖分隔线调整，记住到 localStorage —— 属于「摆放」偏好，不进设计文件。 */
  const [dockHeight, setDockHeight] = useState<number>(() => {
    const raw = Number(localStorage.getItem(DOCK_HEIGHT_KEY));
    return Number.isFinite(raw) && raw >= MIN_DOCK_HEIGHT ? Math.round(raw) : DEFAULT_DOCK_HEIGHT;
  });

  const [dockDragging, setDockDragging] = useState(false);

  /** 节点卡片里显示哪些字段行：只主键+外键 / 全部。观感偏好，记 localStorage。 */
  const [fieldRows, setFieldRows] = useState<FieldRowMode>(() => {
    const saved = localStorage.getItem(FIELD_ROWS_KEY);
    return FIELD_ROW_MODES.includes(saved as FieldRowMode) ? (saved as FieldRowMode) : "keys";
  });

  const rootRef = useRef<HTMLDivElement>(null);

  const dragRef = useRef<{ startY: number; startHeight: number } | null>(null);

  const dockHeightRef = useRef(dockHeight);

  dockHeightRef.current = dockHeight;

  // 选中表 → 自动展开；点画布取消选中 → 自动收缩。
  // 用「节点是否真的还在文档里」判断：逆向替换整个文档后 selectedId 可能已失效，
  // 那时不该显示成「选中了」的样子。
  const selectedAlive = !!selectedId && !!doc?.nodes.some((n) => n.id === selectedId);

  useEffect(() => {
    if (selectedAlive) {
      if (dockLockedRef.current) return;
      setDockOpen(true);
    } else {
      dockLockedRef.current = false;
      setDockOpen(false);
    }
  }, [selectedAlive]);

  /** 拖分隔线：向上拖变高。clamp 住，别把画布挤到 0 或把 dock 压到看不见表头。 */
  const clampDockHeight = useCallback((px: number) => {
    const total = rootRef.current?.clientHeight ?? 0;
    const max = Math.max(MIN_DOCK_HEIGHT, total - MIN_CANVAS_HEIGHT);
    return Math.min(Math.max(Math.round(px), MIN_DOCK_HEIGHT), max);
  }, []);

  const onSplitterDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    dragRef.current = { startY: e.clientY, startHeight: dockHeightRef.current };
    setDockDragging(true);
  };

  const onSplitterMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag) return;
    setDockHeight(clampDockHeight(drag.startHeight + (drag.startY - e.clientY)));
  };

  const onSplitterUp = (e: React.PointerEvent<HTMLDivElement>) => {
    e.currentTarget.releasePointerCapture(e.pointerId);
    dragRef.current = null;
    setDockDragging(false);
  };

  /** 拖完/键盘调完再落盘：拖动过程中每帧都写 localStorage 太浪费。 */
  useEffect(() => {
    if (dockDragging) return;
    localStorage.setItem(DOCK_HEIGHT_KEY, String(dockHeight));
  }, [dockHeight, dockDragging]);
  return {
    dockOpen,
    setDockOpen,
    dockLockedRef,
    dockHeight,
    setDockHeight,
    dockDragging,
    fieldRows,
    setFieldRows,
    rootRef,
    clampDockHeight,
    onSplitterDown,
    onSplitterMove,
    onSplitterUp,
  };
}
