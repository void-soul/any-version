import { useEffect, useRef } from "react";

/**
 * 页面不可见时暂停轮询，回到前台立即对齐一次（AUDIT-015 / 050 单一真源）。
 *
 * WebView2 会对后台标签做定时器节流；若不判断 visibility，回到前台会成批执行
 * 累积的 invoke 与全量 setState，表现为界面卡顿与无谓的后端压力。
 * 项目内 MusicPanel / HttpServer / useRtspServer 已有同样的手写模式，
 * 这里统一为一个 hook 供所有轮询复用。
 */
export function useVisibleInterval(callback: () => void, intervalMs: number): void {
  const callbackRef = useRef(callback);
  callbackRef.current = callback;

  useEffect(() => {
    const run = () => {
      if (typeof document !== "undefined" && document.hidden) return;
      callbackRef.current();
    };
    run();
    const id = window.setInterval(run, intervalMs);
    const onVisibility = () => {
      if (!document.hidden) run();
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      window.clearInterval(id);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [intervalMs]);
}
