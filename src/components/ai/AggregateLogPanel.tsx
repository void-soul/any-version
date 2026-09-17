import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Trash2 } from "lucide-react";
import type { AggregateLog } from "./types";

/// 聚合服务日志面板（仿服务页日志区：等宽、错误行着色、自动滚底）
export default function AggregateLogPanel({
  logs,
  onClear,
}: {
  logs: AggregateLog[];
  onClear: () => void;
}) {
  const { t } = useTranslation();
  const endRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    endRef.current?.scrollIntoView({ block: "end" });
  }, [logs]);

  return (
    <div className="rounded-lg border border-white/5 bg-black/30 overflow-hidden">
      <div className="px-2.5 py-1.5 border-b border-white/5 flex items-center justify-between">
        <span className="text-[10px] font-bold text-slate-400">{t("aggregate.logTitle")}</span>
        <button
          onClick={onClear}
          className="text-slate-600 hover:text-slate-300 cursor-pointer transition-colors"
          title={t("aggregate.logClear")}
        >
          <Trash2 className="w-3 h-3" />
        </button>
      </div>
      <div className="max-h-56 overflow-y-auto px-2.5 py-2 font-mono text-[10px] leading-relaxed space-y-0.5">
        {logs.length === 0 ? (
          <div className="text-slate-600">{t("aggregate.logEmpty")}</div>
        ) : (
          logs.map((log, idx) => (
            <div
              key={idx}
              className={
                log.level === "error"
                  ? "text-red-400"
                  : log.level === "warn"
                    ? "text-amber-400"
                    : "text-slate-400"
              }
            >
              <span className="text-slate-600 mr-1.5">[{log.phase}]</span>
              {log.line}
            </div>
          ))
        )}
        <div ref={endRef} />
      </div>
    </div>
  );
}
