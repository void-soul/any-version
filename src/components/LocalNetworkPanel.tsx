import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity, AlertTriangle, CheckCircle, Globe, Loader2, MapPin,
  Network, Radio, RefreshCw, Signal, Wifi
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

interface IpInfo {
  ip: string;
  country: string;
  region: string;
  city: string;
  isp: string;
  org: string;
  source: string;
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

  // —— 网卡流量（2s 轮询，计算速率）——
  const [traffic, setTraffic] = useState<Map<string, { rx: number; tx: number; rxRate: number; txRate: number }>>(new Map());
  const [trafficError, setTrafficError] = useState<string | null>(null);
  const prevTraffic = useRef<Map<string, IfaceTraffic> | null>(null);
  const prevTime = useRef<number | null>(null);

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const list = await invoke<IfaceTraffic[]>("net_iface_traffic");
        if (!alive) return;
        setTrafficError(null);
        const now = Date.now();
        const next = new Map<string, { rx: number; tx: number; rxRate: number; txRate: number }>();
        const dt = prevTime.current ? (now - prevTime.current) / 1000 : 0;
        for (const it of list) {
          const prev = prevTraffic.current?.get(it.name);
          const rxRate = prev && dt > 0 ? Math.max(0, (it.received_bytes - prev.received_bytes) / dt) : 0;
          const txRate = prev && dt > 0 ? Math.max(0, (it.sent_bytes - prev.sent_bytes) / dt) : 0;
          next.set(it.name, { rx: it.received_bytes, tx: it.sent_bytes, rxRate, txRate });
        }
        prevTraffic.current = new Map(list.map((it) => [it.name, it]));
        prevTime.current = now;
        setTraffic(next);
      } catch (e: any) {
        if (alive) setTrafficError(String(e));
      }
    };
    tick();
    const h = window.setInterval(tick, 2000);
    return () => { alive = false; window.clearInterval(h); };
  }, []);

  // —— IP 归属地 ——
  const [ipQueries, setIpQueries] = useState<Map<string, IpInfo | string>>(new Map());
  const [ipLoading, setIpLoading] = useState<string | null>(null);

  const lookupIp = async (ip: string) => {
    setIpLoading(ip);
    try {
      const info = await invoke<IpInfo>("ip_lookup", { ip });
      setIpQueries((m) => new Map(m).set(ip, info));
    } catch (e: any) {
      setIpQueries((m) => new Map(m).set(ip, String(e)));
    } finally {
      setIpLoading(null);
    }
  };

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

  const totalRxRate = useMemo(() => [...traffic.values()].reduce((s, t) => s + t.rxRate, 0), [traffic]);
  const totalTxRate = useMemo(() => [...traffic.values()].reduce((s, t) => s + t.txRate, 0), [traffic]);

  return (
    <div className="space-y-4 px-40 py-4">
      {/* 流量总览 */}
      <div className="glass-panel rounded-2xl p-5 border border-white/5 space-y-4">
        <div className="flex items-center gap-2 pb-2 border-b border-white/5">
          <Activity className="w-4 h-4 text-emerald-400" />
          <h4 className="font-semibold text-white text-xs">网卡流量</h4>
          {trafficError && <span className="text-[10px] text-red-400 ml-auto">{trafficError}</span>}
          {!trafficError && (
            <div className="ml-auto flex items-center gap-3 text-[10px] font-mono">
              <span className="text-emerald-400 flex items-center gap-1"><Wifi className="w-3 h-3" />↓ {fmtRate(totalRxRate)}</span>
              <span className="text-blue-400 flex items-center gap-1"><Wifi className="w-3 h-3" />↑ {fmtRate(totalTxRate)}</span>
            </div>
          )}
        </div>
        {traffic.size === 0 && !trafficError && (
          <p className="text-[10px] text-slate-500">正在读取网卡统计…</p>
        )}
        <div className="grid grid-cols-1 md:grid-cols-2 gap-2">
          {[...traffic.entries()].map(([name, t]) => (
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
            </div>
          ))}
        </div>
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
                    <td className="py-1.5 pr-3">进程</td>
                    <td className="py-1.5">归属地</td>
                  </tr>
                </thead>
                <tbody className="text-slate-300 divide-y divide-white/[0.03]">
                  {filteredConns.map((c, i) => {
                    const remoteIp = c.remote.replace(/:[^:]*$/, "").replace(/^\[|\]$/g, "");
                    const isRemote = remoteIp !== "" && remoteIp !== "*" && remoteIp !== "0.0.0.0" && !c.remote.startsWith("127.") && !c.remote.startsWith("[::1]");
                    const q = ipQueries.get(remoteIp);
                    return (
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
                        <td className="py-1">
                          {isRemote ? (
                            q === undefined ? (
                              <button onClick={() => lookupIp(remoteIp)} disabled={ipLoading !== null}
                                className="px-1.5 py-0.5 bg-white/5 hover:bg-white/10 text-slate-400 rounded text-[9px] border border-white/5 cursor-pointer flex items-center gap-1 disabled:opacity-50">
                                {ipLoading === remoteIp ? <Loader2 className="w-2.5 h-2.5 animate-spin" /> : <MapPin className="w-2.5 h-2.5" />}
                                查询
                              </button>
                            ) : typeof q === "string" ? (
                              <span className="text-red-400 text-[9px]" title={q}>查询失败</span>
                            ) : (
                              <span className="text-slate-300 text-[9px]" title={`${q.isp} ${q.org} (${q.source})`}>
                                {q.country}{q.city ? ` ${q.city}` : ""}
                              </span>
                            )
                          ) : (
                            <span className="text-slate-600">-</span>
                          )}
                        </td>
                      </tr>
                    );
                  })}
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
    </div>
  );
}
