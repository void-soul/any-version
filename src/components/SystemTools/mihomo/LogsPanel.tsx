// 日志页 —— 1:1 复刻 clash-party src/renderer/src/pages/logs.tsx
// （500 条环形缓存 + 100ms 渲染节流 + 过滤持久化 + 自动滚动 + 清空）
import React, { useEffect, useMemo, useRef, useState } from "react";
import { MapPin, Trash2, Search } from "lucide-react";
import { IMihomoLog, openMihomoWs, WsHandle } from "./ctrl";
import { cardCls, btnSec } from "./ui";

const LOGS_FILTER_KEY = "mihomo-logs-filter";
const MAX_CACHED_LOGS = 500;
const RENDER_INTERVAL = 100;

// 模块级缓存：切页不丢日志（复刻 cachedLogs）
const cached: { log: IMihomoLog[]; trigger: (() => void) | null } = { log: [], trigger: null };

const LEVEL_COLOR: Record<string, string> = {
  error: "text-rose-400",
  warning: "text-amber-400",
  info: "text-emerald-400",
  debug: "text-sky-400",
};

export default function LogsPanel({ info, running, logLevel }: { info: any; running: boolean; logLevel?: string }) {
  const [logs, setLogs] = useState<IMihomoLog[]>(cached.log);
  const [filter, setFilter] = useState(() => localStorage.getItem(LOGS_FILTER_KEY) || "");
  const [trace, setTrace] = useState(true);
  const boxRef = useRef<HTMLDivElement>(null);
  const traceRef = useRef(trace);
  traceRef.current = trace;

  useEffect(() => { localStorage.setItem(LOGS_FILTER_KEY, filter); }, [filter]);

  // WS 订阅（level 取核心 log-level，复刻 mihomoLogs）
  useEffect(() => {
    if (!running || !info?.port) return;
    const ws: WsHandle = openMihomoWs(info, "logs", (data: IMihomoLog) => {
      data.time = new Date().toLocaleString();
      cached.log.push(data);
      if (cached.log.length > MAX_CACHED_LOGS) cached.log.splice(0, cached.log.length - MAX_CACHED_LOGS);
      cached.trigger?.();
    }, { level: logLevel || "info" });
    return () => ws.close();
  }, [running, info?.port, info?.secret, logLevel]);

  // 100ms 节流渲染（复刻 LOG_RENDER_INTERVAL_MS）
  useEffect(() => {
    let timer: any = null;
    cached.trigger = () => {
      if (timer !== null) return;
      timer = setTimeout(() => {
        timer = null;
        setLogs([...cached.log]);
        if (traceRef.current && boxRef.current) {
          boxRef.current.scrollTop = boxRef.current.scrollHeight;
        }
      }, RENDER_INTERVAL);
    };
    return () => { cached.trigger = null; if (timer) clearTimeout(timer); };
  }, []);

  const filtered = useMemo(() => {
    const kw = filter.trim().toLowerCase();
    if (!kw) return logs;
    return logs.filter((l) => l.payload.toLowerCase().includes(kw) || l.type.toLowerCase().includes(kw));
  }, [logs, filter]);

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <div className="relative flex-1">
          <Search className="w-3.5 h-3.5 absolute left-2.5 top-1/2 -translate-y-1/2 text-slate-500" />
          <input
            className="w-full h-8 pl-8 pr-2.5 rounded-lg bg-white/5 border border-white/10 text-xs text-white placeholder-slate-500 focus:outline-none focus:border-emerald-500"
            placeholder="筛选日志"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
        </div>
        <button
          className={`${btnSec} ${trace ? "!bg-emerald-600 !text-white" : ""}`}
          title="自动滚动"
          onClick={() => setTrace((p) => !p)}
        >
          <MapPin className="w-3.5 h-3.5" />
        </button>
        <button
          className={btnSec}
          title="清空日志"
          onClick={() => { cached.log = []; setLogs([]); }}
        >
          <Trash2 className="w-3.5 h-3.5 text-rose-300" />
        </button>
      </div>
      <div ref={boxRef} className={`${cardCls} p-3 h-[62vh] overflow-y-auto font-mono text-[11px] leading-relaxed`}>
        {filtered.length === 0 && <div className="text-slate-500 text-center pt-8">{running ? "暂无日志" : "核心未运行"}</div>}
        {filtered.map((l, i) => (
          <div key={i} className="flex gap-2 py-0.5 border-b border-white/[0.03]">
            <span className="text-slate-500 flex-shrink-0">{l.time}</span>
            <span className={`flex-shrink-0 uppercase font-bold ${LEVEL_COLOR[l.type] || "text-slate-400"}`}>{l.type}</span>
            <span className="text-slate-300 break-all select-text">{l.payload}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
