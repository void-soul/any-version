import { useCallback, useEffect, useRef, useState } from "react";
import { DocumentFull, PositionInput, mmApi } from "./types";

/** Owns the MindmapPositionWriter feature, independently of page rendering. */
export function useMindmapPositionWriter({ full }: {
  full: DocumentFull;
}) {

  const [lastSaved, setLastSaved] = useState<number | null>(null);

  // 自动保存：拖放坐标先写入本地 pending，防抖批量 flush 到后端。
  // fullRef 始终指向最新 full，避免防抖回调读到陈旧闭包。
  const fullRef = useRef(full);

  useEffect(() => { fullRef.current = full; }, [full]);

  const pendingPos = useRef<PositionInput[]>([]);

  const pendingStickers = useRef<Map<string, { x: number; y: number }>>(new Map());

  const flushTimer = useRef<number | null>(null);

  const scheduleFlush = useCallback(() => {
    if (flushTimer.current) window.clearTimeout(flushTimer.current);
    flushTimer.current = window.setTimeout(() => {
      flushTimer.current = null;
      const pos = pendingPos.current;
      const stMap = pendingStickers.current;
      pendingPos.current = [];
      pendingStickers.current = new Map();
      const cur = fullRef.current;
      if (pos.length) void mmApi.updatePositions(cur.document.id, pos);
      stMap.forEach((p, sid) => {
        const s = cur.stickers.find(x => x.id === sid);
        if (s) void mmApi.upsertSticker({ documentId: cur.document.id, sticker: { ...s, positionX: p.x, positionY: p.y } });
      });
      setLastSaved(Date.now());
    }, 300);
  }, []);

  // 卸载时把未刷新的坐标立即写入后端，避免快速切换文档丢坐标
  useEffect(() => () => {
    if (flushTimer.current) window.clearTimeout(flushTimer.current);
    const pos = pendingPos.current;
    const stMap = pendingStickers.current;
    if (pos.length || stMap.size) {
      const cur = fullRef.current;
      if (pos.length) void mmApi.updatePositions(cur.document.id, pos);
      stMap.forEach((p, sid) => {
        const s = cur.stickers.find(x => x.id === sid);
        if (s) void mmApi.upsertSticker({ documentId: cur.document.id, sticker: { ...s, positionX: p.x, positionY: p.y } });
      });
    }
  }, []);
  return {
    lastSaved,
    fullRef,
    pendingPos,
    pendingStickers,
    scheduleFlush,
  };
}
