import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity, AlertTriangle, CheckCircle, Globe, Loader2,
  Network, Radio, RefreshCw, Signal, Wifi, X
} from "lucide-react";
import PortScanner from "./PortScanner";

interface NetConnection {
  proto: string;
  local: string;
  remote: string;
  state: string;
  pid: string;
  process: string;
}

interface IfaceTraffic {
  name: string;
  received_bytes: number;
  sent_bytes: number;
}

interface PingResult {
  host: string;
  resolved: string | null;
  sent: number;
  received: number;
  rtts: string[];
  raw: string;
}

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function fmtRate(bps: number): string {
  if (bps < 1024) return `${bps.toFixed(0)} B/s`;
  if (bps < 1024 * 1024) return `${(bps / 1024).toFixed(1)} KB/s`;
  return `${(bps / 1024 / 1024).toFixed(2)} MB/s`;
}

/** 网卡速率迷你趋势图：绿=下行、蓝=上行；悬停显示精确速率，点击放大 */
function TrendSpark({ data, onOpen }: { data: { rx: number; tx: number; t: number }[]; onOpen?: () => void }) {
  const W = 120, H = 26;
  const [hover, setHover] = useState<number | null>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  if (data.length < 2) return <svg width={W} height={H} className="block" />;
  const max = Math.max(1024, ...data.flatMap((d) => [d.rx, d.tx]));
  const pts = (key: "rx" | "tx") =>
    data.map((d, i) => {
      const x = (i / (data.length - 1)) * W;
      const y = H - (d[key] / max) * (H - 4) - 2;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    }).join(" ");
  const onMove = (e: React.MouseEvent) => {
    const r = svgRef.current?.getBoundingClientRect();
    if (!r) return;
    const x = e.clientX - r.left;
    const idx = Math.round((x / W) * (data.length - 1));
    setHover(Math.max(0, Math.min(data.length - 1, idx)));
  };
  const hx = hover !== null ? (hover / (data.length - 1)) * W : 0;
  const hd = hover !== null ? data[hover] : null;
  return (
    <div className="relative">
      <svg ref={svgRef} width={W} height={H} className="block cursor-pointer" aria-label="近 2 分钟速率趋势（悬停读数，点击放大）"
        onMouseMove={onMove} onMouseLeave={() => setHover(null)} onClick={onOpen}>
        <polyline points={pts("rx")} fill="none" stroke="#34d399" strokeWidth={1.2} strokeLinejoin="round" strokeLinecap="round" opacity={0.85} />
        <polyline points={pts("tx")} fill="none" stroke="#60a5fa" strokeWidth={1.2} strokeLinejoin="round" strokeLinecap="round" opacity={0.85} />
        {hover !== null && <line x1={hx} y1={0} x2={hx} y2={H} stroke="#94a3b8" strokeWidth={0.8} strokeDasharray="2 2" />}
        <text x={W - 2} y={H - 2} textAnchor="end" fontSize="6" fill="#475569">近2分钟</text>
      </svg>
      {hd && hover !== null && (
        <div className="pointer-events-none absolute -top-2 z-10 -translate-x-1/2 whitespace-nowrap rounded border border-white/10 bg-slate-950/95 px-1.5 py-0.5 font-mono text-[8px] text-slate-200 shadow-lg"
          style={{ left: `${(hover / (data.length - 1)) * 100}%` }}>
          <span className="text-emerald-400">↓ {fmtRate(hd.rx)}</span>
          <span className="ml-1 text-blue-400">↑ {fmtRate(hd.tx)}</span>
        </div>
      )}
    </div>
  );
}

/** 网卡速率放大图：网格 + 时间轴 + 悬停十字线与精确读数 */
function TrendChartModal({ name, data, onClose }: { name: string; data: { rx: number; tx: number; t: number }[]; onClose: () => void }) {
  const [hover, setHover] = useState<number | null>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const W = 560, H = 200, PL = 12, PR = 12, PT = 14, PB = 24;
  const iw = W - PL - PR, ih = H - PT - PB;
  const n = data.length;
  const max = Math.max(1024, ...data.flatMap((d) => [d.rx, d.tx]));
  const xAt = (i: number) => (n <= 1 ? PL + iw / 2 : PL + (i / (n - 1)) * iw);
  const yAt = (v: number) => PT + ih - (v / max) * ih;
  const rxPts = data.map((d, i) => `${xAt(i).toFixed(1)},${yAt(d.rx).toFixed(1)}`).join(" ");
  const txPts = data.map((d, i) => `${xAt(i).toFixed(1)},${yAt(d.tx).toFixed(1)}`).join(" ");
  const gridY = [1, 0.75, 0.5, 0.25].map((f) => PT + ih * (1 - f));
  const gridVals = [1, 0.75, 0.5, 0.25].map((f) => max * f);
  const fmtT = (t: number) => new Date(t).toTimeString().slice(0, 8);
  const hd = hover !== null ? data[hover] : null;
  const hoverX = hover !== null ? xAt(hover) : 0;
  const latest = data[n - 1];
  const peakRx = Math.max(...data.map((d) => d.rx));
  const peakTx = Math.max(...data.map((d) => d.tx));
  const onMove = (e: React.MouseEvent) => {
    const r = svgRef.current?.getBoundingClientRect();
    if (!r || n < 1) return;
    const x = e.clientX - r.left;
    const idx = Math.round(((x / r.width) * W - PL) / iw * (n - 1));
    setHover(Math.max(0, Math.min(n - 1, idx)));
  };
  return createPortal(
    <div className="fixed inset-0 z-[200] modal-mask flex items-center justify-center bg-black/70 p-4 backdrop-blur-[3px]" onClick={onClose}>
      <div className="w-[min(94vw,640px)] rounded-xl border border-white/10 bg-[#0d1524] p-4 shadow-2xl" onClick={(e) => e.stopPropagation()}>
        <div className="mb-3 flex items-center justify-between">
          <h3 className="flex min-w-0 items-center gap-2 text-sm font-semibold text-white"><Radio className="h-4 w-4 shrink-0 text-emerald-400" /><span className="truncate">{name}</span></h3>
          <button type="button" className="rounded p-1 text-slate-400 hover:text-white" onClick={onClose} title="关闭"><X className="h-4 w-4" /></button>
        </div>
        <div className="mb-2 flex flex-wrap items-center gap-x-4 gap-y-1 font-mono text-[9px]">
          <span className="text-emerald-400">↓ 最新 {fmtRate(latest.rx)}</span>
          <span className="text-blue-400">↑ 最新 {fmtRate(latest.tx)}</span>
          <span className="text-slate-500">峰值 ↓ {fmtRate(peakRx)} · ↑ {fmtRate(peakTx)}</span>
          <span className="text-slate-500">近 2 分钟 · {n} 采样</span>
        </div>
        <div className="relative">
          <svg ref={svgRef} viewBox={`0 0 ${W} ${H}`} className="block h-auto w-full cursor-crosshair"
            onMouseMove={onMove} onMouseLeave={() => setHover(null)}>
            {gridY.map((gy, i) => (
              <g key={i}>
                <line x1={PL} y1={gy} x2={W - PR} y2={gy} stroke="rgba(255,255,255,0.07)" strokeWidth={1} />
                <text x={PL} y={gy - 3} fontSize="8" fill="#475569">{fmtRate(gridVals[i])}</text>
              </g>
            ))}
            <polyline points={rxPts} fill="none" stroke="#34d399" strokeWidth={1.6} strokeLinejoin="round" strokeLinecap="round" />
            <polyline points={txPts} fill="none" stroke="#60a5fa" strokeWidth={1.6} strokeLinejoin="round" strokeLinecap="round" />
            {hover !== null && <line x1={hoverX} y1={PT} x2={hoverX} y2={PT + ih} stroke="#94a3b8" strokeWidth={1} strokeDasharray="3 3" />}
            {n > 1 && (
              <>
                <text x={PL} y={H - 7} fontSize="8" fill="#475569">{fmtT(data[0].t)}</text>
                <text x={xAt(Math.floor(n / 2))} y={H - 7} fontSize="8" fill="#475569" textAnchor="middle">{fmtT(data[Math.floor(n / 2)].t)}</text>
                <text x={W - PR} y={H - 7} fontSize="8" fill="#475569" textAnchor="end">{fmtT(data[n - 1].t)}</text>
              </>
            )}
          </svg>
          {hd && hover !== null && n > 1 && (
            <div className="pointer-events-none absolute top-1 z-10 -translate-x-1/2 whitespace-nowrap rounded border border-white/10 bg-slate-950/95 px-2 py-1 font-mono text-[9px] text-slate-200 shadow-xl"
              style={{ left: `${(hover / (n - 1)) * 100}%` }}>
              <div className="text-slate-400">{fmtT(hd.t)}</div>
              <div className="text-emerald-400">↓ {fmtRate(hd.rx)}</div>
              <div className="text-blue-400">↑ {fmtRate(hd.tx)}</div>
            </div>
          )}
        </div>
      </div>
    </div>, document.body);
}

export default function LocalNetworkPanel() {
  // —— 连接列表 ——
  const [conns, setConns] = useState<NetConnection[] | null>(null);
  const [connsLoading, setConnsLoading] = useState(false);
  const [connFilter, setConnFilter] = useState("");
  const [connError, setConnError] = useState<string | null>(null);

  const loadConns = useCallback(async () => {
    setConnsLoading(true);
    setConnError(null);
    try {
      setConns(await invoke<NetConnection[]>("net_connections"));
    } catch (e: any) {
      setConnError(String(e));
    } finally {
      setConnsLoading(false);
    }
  }, []);

  // —— 网卡流量（2s 轮询，计算速率 + 近 2 分钟速率趋势）——
  const [traffic, setTraffic] = useState<Map<string, { rx: number; tx: number; rxRate: number; txRate: number }>>(new Map());
  // 每块网卡近 60 个采样点的速率历史（2s/点 → 2 分钟，含采样时间戳），供趋势图使用
  const [history, setHistory] = useState<Map<string, { rx: number; tx: number; t: number }[]>>(new Map());
  const [trafficError, setTrafficError] = useState<string | null>(null);
  const prevTraffic = useRef<Map<string, IfaceTraffic> | null>(null);
  const prevTime = useRef<number | null>(null);
  const historyRef = useRef<Map<string, { rx: number; tx: number; t: number }[]>>(new Map());
  const aliveRef = useRef(true);
  // 网卡选择（默认全部）与放大查看的网卡
  const [selectedIface, setSelectedIface] = useState<string>("all");
  const [zoomIface, setZoomIface] = useState<string | null>(null);

  const tick = useCallback(async () => {
    try {
      const list = await invoke<IfaceTraffic[]>("net_iface_traffic");
      if (!aliveRef.current) return;
      setTrafficError(null);
      const now = Date.now();
      const next = new Map<string, { rx: number; tx: number; rxRate: number; txRate: number }>();
      const dt = prevTime.current ? (now - prevTime.current) / 1000 : 0;
      const histNext = new Map(historyRef.current);
      for (const it of list) {
        const prev = prevTraffic.current?.get(it.name);
        const rxRate = prev && dt > 0 ? Math.max(0, (it.received_bytes - prev.received_bytes) / dt) : 0;
        const txRate = prev && dt > 0 ? Math.max(0, (it.sent_bytes - prev.sent_bytes) / dt) : 0;
        next.set(it.name, { rx: it.received_bytes, tx: it.sent_bytes, rxRate, txRate });
        const arr = histNext.get(it.name) ?? [];
        arr.push({ rx: rxRate, tx: txRate, t: now });
        if (arr.length > 60) arr.shift();
        histNext.set(it.name, arr);
      }
      historyRef.current = histNext;
      prevTraffic.current = new Map(list.map((it) => [it.name, it]));
      prevTime.current = now;
      setTraffic(next);
      setHistory(histNext);
    } catch (e: any) {
      if (aliveRef.current) setTrafficError(String(e));
    }
  }, []);

  useEffect(() => {
    aliveRef.current = true;
    void tick();
    const h = window.setInterval(tick, 2000);
    return () => { aliveRef.current = false; window.clearInterval(h); };
  }, [tick]);

  // 读取失败时的重试：清除错误并立即重新采样
  const retryTraffic = () => { setTrafficError(null); void tick(); };

  // —— Ping ——
  const [pingHost, setPingHost] = useState("");
  const [pingCount, setPingCount] = useState(4);
  const [pingResult, setPingResult] = useState<PingResult | null>(null);
  const [pinging, setPinging] = useState(false);
  const [pingError, setPingError] = useState<string | null>(null);

  const runPing = async () => {
    if (!pingHost.trim()) return;
    setPinging(true);
    setPingError(null);
    setPingResult(null);
    try {
      setPingResult(await invoke<PingResult>("ping_host", { host: pingHost.trim(), count: pingCount }));
    } catch (e: any) {
      setPingError(String(e));
    } finally {
      setPinging(false);
    }
  };

  const filteredConns = useMemo(() => {
    if (!conns) return [];
    const f = connFilter.trim().toLowerCase();
    if (!f) return conns;
    return conns.filter((c) =>
      c.local.toLowerCase().includes(f) ||
      c.remote.toLowerCase().includes(f) ||
      c.state.toLowerCase().includes(f) ||
      c.process.toLowerCase().includes(f) ||
      c.proto.toLowerCase().includes(f)
    );
  }, [conns, connFilter]);

  // 当前要显示的网卡（全部或单选）；选中网卡消失时自动回到「全部」
  const visibleCards = useMemo(() => {
    if (selectedIface === "all") return [...traffic.entries()];
    const t = traffic.get(selectedIface);
    return t ? [[selectedIface, t] as const] : [];
  }, [traffic, selectedIface]);
  useEffect(() => {
    if (selectedIface !== "all" && !traffic.has(selectedIface)) setSelectedIface("all");
  }, [traffic, selectedIface]);

  const totalRxRate = useMemo(() => visibleCards.reduce((s, [, t]) => s + t.rxRate, 0), [visibleCards]);
  const totalTxRate = useMemo(() => visibleCards.reduce((s, [, t]) => s + t.txRate, 0), [visibleCards]);

  return (
    <div className="h-full overflow-y-auto space-y-4 px-40 py-4">
      {/* 流量总览 */}
      <div className="glass-panel rounded-2xl p-5 border border-white/5 space-y-4">
        <div className="flex flex-wrap items-center gap-2 pb-2 border-b border-white/5">
          <Activity className="w-4 h-4 text-emerald-400" />
          <h4 className="font-semibold text-white text-xs">网卡流量</h4>
          <select value={selectedIface} onChange={(e) => setSelectedIface(e.target.value)}
            className="ml-auto max-w-[200px] cursor-pointer rounded-md border border-white/10 bg-slate-950/70 px-1.5 py-1 text-[10px] text-slate-300 outline-none focus:border-emerald-400/60" title="选择要显示的网卡">
            <option value="all">全部网卡</option>
            {[...traffic.keys()].map((n) => <option key={n} value={n}>{n}</option>)}
          </select>
          {trafficError ? (
            <span className="flex items-center gap-2 text-[10px] text-red-400">
              {trafficError}
              <button onClick={retryTraffic} className="cursor-pointer rounded border border-red-400/40 px-1.5 py-0.5 text-[9px] text-red-300 hover:bg-red-400/10">重试</button>
            </span>
          ) : (
            <div className="flex items-center gap-3 text-[10px] font-mono">
              <span className="text-emerald-400 flex items-center gap-1"><Wifi className="w-3 h-3" />↓ {fmtRate(totalRxRate)}</span>
              <span className="text-blue-400 flex items-center gap-1"><Wifi className="w-3 h-3" />↑ {fmtRate(totalTxRate)}</span>
            </div>
          )}
        </div>
        {traffic.size === 0 && !trafficError && (
          <p className="text-[10px] text-slate-500">正在读取网卡统计…</p>
        )}
        <div className="grid grid-cols-1 md:grid-cols-2 gap-2">
          {visibleCards.map(([name, t]) => (
            <div key={name} className="bg-black/20 border border-white/5 rounded-xl p-3 space-y-1.5">
              <div className="flex items-center justify-between">
                <span className="text-[10px] text-slate-300 font-semibold flex items-center gap-1.5">
                  <Radio className="w-3 h-3 text-emerald-400" /> {name}
                </span>
                <span className="text-[9px] text-slate-500 font-mono">
                  累计 ↓ {fmtBytes(t.rx)} · ↑ {fmtBytes(t.tx)}
                </span>
              </div>
              <div className="flex items-center gap-2">
                <div className="flex-1 h-1.5 bg-white/5 rounded-full overflow-hidden">
                  <div className="h-full bg-emerald-500/60 rounded-full transition-all"
                    style={{ width: `${Math.min(100, (t.rxRate / (1024 * 1024)) * 100)}%` }} />
                </div>
                <span className="text-[9px] text-emerald-400 font-mono w-20 text-right">{fmtRate(t.rxRate)}</span>
              </div>
              <div className="flex items-center gap-2">
                <div className="flex-1 h-1.5 bg-white/5 rounded-full overflow-hidden">
                  <div className="h-full bg-blue-500/60 rounded-full transition-all"
                    style={{ width: `${Math.min(100, (t.txRate / (1024 * 1024)) * 100)}%` }} />
                </div>
                <span className="text-[9px] text-blue-400 font-mono w-20 text-right">{fmtRate(t.txRate)}</span>
              </div>
              <TrendSpark data={history.get(name) ?? []} onOpen={() => setZoomIface(name)} />
            </div>
          ))}
        </div>
        {visibleCards.length === 0 && !trafficError && (
          <p className="text-[10px] text-slate-500">暂无网卡统计</p>
        )}
      </div>

      {/* 连接列表 */}
      <div className="glass-panel rounded-2xl p-5 border border-white/5 space-y-3">
        <div className="flex items-center gap-2 pb-2 border-b border-white/5">
          <Network className="w-4 h-4 text-blue-400" />
          <h4 className="font-semibold text-white text-xs">网络连接</h4>
          <button onClick={loadConns} disabled={connsLoading}
            className="ml-auto flex items-center gap-1.5 px-3 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg text-[10px] border border-white/5 cursor-pointer disabled:opacity-50">
            {connsLoading ? <Loader2 className="w-3 h-3 animate-spin" /> : <RefreshCw className="w-3 h-3" />}
            {conns ? "刷新" : "加载连接"}
          </button>
        </div>
        {connError && (
          <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-400 rounded-xl text-xs flex items-center gap-1.5">
            <AlertTriangle className="w-3.5 h-3.5" /> {connError}
          </div>
        )}
        {conns && (
          <>
            <input
              value={connFilter}
              onChange={(e) => setConnFilter(e.target.value)}
              className="w-full glass-input px-3 py-2 text-xs"
              placeholder="过滤：地址 / 端口 / 状态 / 进程 / 协议"
            />
            <p className="text-[10px] text-slate-500">{filteredConns.length} 条连接（共 {conns.length}）</p>
            <div className="max-h-80 overflow-y-auto">
              <table className="w-full text-[10px]">
                <thead className="sticky top-0 bg-[#0b0e14]">
                  <tr className="text-slate-400 font-semibold border-b border-white/5">
                    <td className="py-1.5 pr-3">协议</td>
                    <td className="py-1.5 pr-3">本地地址</td>
                    <td className="py-1.5 pr-3">远程地址</td>
                    <td className="py-1.5 pr-3">状态</td>
                    <td className="py-1.5">进程</td>
                  </tr>
                </thead>
                <tbody className="text-slate-300 divide-y divide-white/[0.03]">
                  {filteredConns.map((c, i) => (
                    <tr key={i} className="hover:bg-white/[0.02]">
                      <td className="py-1 font-mono text-slate-400">{c.proto}</td>
                      <td className="py-1 font-mono">{c.local}</td>
                      <td className="py-1 font-mono">{c.remote}</td>
                      <td className="py-1">
                        <span className={c.state === "LISTENING" ? "text-amber-400" : c.state === "ESTABLISHED" ? "text-emerald-400" : "text-slate-500"}>
                          {c.state}
                        </span>
                      </td>
                      <td className="py-1 text-slate-400">{c.process || (c.pid !== "0" ? `PID ${c.pid}` : "系统")}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </>
        )}
      </div>

      {/* Ping */}
      <div className="glass-panel rounded-2xl p-5 border border-white/5 space-y-3">
        <div className="flex items-center gap-2 pb-2 border-b border-white/5">
          <Signal className="w-4 h-4 text-violet-400" />
          <h4 className="font-semibold text-white text-xs">Ping</h4>
        </div>
        <div className="flex gap-2">
          <input
            value={pingHost}
            onChange={(e) => setPingHost(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && runPing()}
            className="flex-1 glass-input px-3 py-2 text-xs"
            placeholder="域名或 IP（如 baidu.com / 8.8.8.8）"
          />
          <select value={pingCount} onChange={(e) => setPingCount(Number(e.target.value))}
            className="glass-input px-2 py-2 text-xs cursor-pointer">
            {[1, 2, 4, 6, 8, 10].map((n) => <option key={n} value={n}>{n} 次</option>)}
          </select>
          <button onClick={runPing} disabled={pinging || !pingHost.trim()}
            className="px-4 py-2 bg-violet-600 hover:bg-violet-500 disabled:opacity-50 text-white rounded-lg text-xs font-semibold cursor-pointer transition-all">
            {pinging ? "Ping 中..." : "Ping"}
          </button>
        </div>
        {pingError && (
          <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-400 rounded-xl text-xs flex items-center gap-1.5">
            <AlertTriangle className="w-3.5 h-3.5" /> {pingError}
          </div>
        )}
        {pingResult && (
          <div className="space-y-2">
            <div className="flex items-center gap-3 text-[10px] bg-black/20 border border-white/5 rounded-xl p-3">
              <span className={pingResult.received > 0 ? "text-emerald-400 flex items-center gap-1" : "text-red-400 flex items-center gap-1"}>
                {pingResult.received > 0 ? <CheckCircle className="w-3 h-3" /> : <AlertTriangle className="w-3 h-3" />}
                {pingResult.received}/{pingResult.sent} 收包
              </span>
              {pingResult.resolved && <span className="text-slate-400 font-mono flex items-center gap-1"><Globe className="w-3 h-3" /> {pingResult.resolved}</span>}
              {pingResult.rtts.length > 0 && <span className="text-slate-300 font-mono">{pingResult.rtts.join(" · ")}</span>}
            </div>
            {pingResult.raw && (
              <pre className="max-h-40 overflow-y-auto bg-black/30 border border-white/5 rounded-xl p-3 text-[9px] font-mono text-slate-400 whitespace-pre-wrap">{pingResult.raw}</pre>
            )}
          </div>
        )}
      </div>

      {/* 端口排查（原模块整体保留） */}
      <PortScanner />
      {zoomIface && history.get(zoomIface) && history.get(zoomIface)!.length >= 2 && (
        <TrendChartModal name={zoomIface} data={history.get(zoomIface)!} onClose={() => setZoomIface(null)} />
      )}
    </div>
  );
}
