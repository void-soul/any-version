import { useCallback, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle, CheckCircle, Globe, Loader2,
  Network, RefreshCw, Signal
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

interface PingResult {
  host: string;
  resolved: string | null;
  sent: number;
  received: number;
  rtts: string[];
  raw: string;
}

export default function LocalNetworkPanel() {
  const { t } = useTranslation();
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

  return (
    <div className="grid h-full grid-rows-3 gap-4 px-40 py-4">
      {/* 连接列表 */}
      <div className="glass-panel flex min-h-0 flex-col rounded-2xl border border-white/5 p-5">
        <div className="flex shrink-0 items-center gap-2 border-b border-white/5 pb-2">
          <Network className="w-4 h-4 text-blue-400" />
          <h4 className="font-semibold text-white text-xs">{t("netpan.connsTitle")}</h4>
          <button onClick={loadConns} disabled={connsLoading}
            className="ml-auto flex items-center gap-1.5 px-3 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg text-[10px] border border-white/5 cursor-pointer disabled:opacity-50">
            {connsLoading ? <Loader2 className="w-3 h-3 animate-spin" /> : <RefreshCw className="w-3 h-3" />}
            {conns ? t("netpan.refresh") : t("netpan.loadConns")}
          </button>
        </div>
        {connError && (
          <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-400 rounded-xl text-xs flex items-center gap-1.5">
            <AlertTriangle className="w-3.5 h-3.5" /> {connError}
          </div>
        )}
        {conns && (
          <div className="mt-3 flex min-h-0 flex-1 flex-col gap-2">
            <input
              value={connFilter}
              onChange={(e) => setConnFilter(e.target.value)}
              className="w-full shrink-0 glass-input px-3 py-2 text-xs"
              placeholder={t("netpan.filterPh")}
            />
            <p className="shrink-0 text-[10px] text-slate-500">{t("netpan.connCount", { filtered: filteredConns.length, total: conns.length })}</p>
            <div className="min-h-0 flex-1 overflow-y-auto">
              <table className="w-full text-[10px]">
                <thead className="sticky top-0 bg-[#0b0e14]">
                  <tr className="text-slate-400 font-semibold border-b border-white/5">
                    <td className="py-1.5 pr-3">{t("netpan.thProto")}</td>
                    <td className="py-1.5 pr-3">{t("netpan.thLocal")}</td>
                    <td className="py-1.5 pr-3">{t("netpan.thRemote")}</td>
                    <td className="py-1.5 pr-3">{t("netpan.thState")}</td>
                    <td className="py-1.5">{t("netpan.thProc")}</td>
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
                      <td className="py-1 text-slate-400">{c.process || (c.pid !== "0" ? `PID ${c.pid}` : t("netpan.system"))}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        )}
      </div>

      {/* Ping */}
      <div className="glass-panel flex min-h-0 flex-col rounded-2xl border border-white/5 p-5">
        <div className="flex shrink-0 items-center gap-2 border-b border-white/5 pb-2">
          <Signal className="w-4 h-4 text-violet-400" />
          <h4 className="font-semibold text-white text-xs">Ping</h4>
        </div>
        <div className="mt-3 flex shrink-0 gap-2">
          <input
            value={pingHost}
            onChange={(e) => setPingHost(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && runPing()}
            className="flex-1 glass-input px-3 py-2 text-xs"
            placeholder={t("netpan.pingPh")}
          />
          <select value={pingCount} onChange={(e) => setPingCount(Number(e.target.value))}
            className="glass-input px-2 py-2 text-xs cursor-pointer">
            {[1, 2, 4, 6, 8, 10].map((n) => <option key={n} value={n}>{t("netpan.pingCount", { n })}</option>)}
          </select>
          <button onClick={runPing} disabled={pinging || !pingHost.trim()}
            className="px-4 py-2 bg-violet-600 hover:bg-violet-500 disabled:opacity-50 text-white rounded-lg text-xs font-semibold cursor-pointer transition-all">
            {pinging ? t("netpan.pinging") : t("netpan.ping")}
          </button>
        </div>
        <div className="mt-3 min-h-0 flex-1 overflow-y-auto">
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
                  {t("netpan.packets", { rcv: pingResult.received, sent: pingResult.sent })}
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
      </div>

      {/* 端口排查 */}
      <PortScanner />
    </div>
  );
}
