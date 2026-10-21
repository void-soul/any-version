import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle, CheckCircle, ChevronDown, ChevronUp, Globe,
  Loader2, Plug, Route, Signal,
} from "lucide-react";
import { SharedButton } from "../shared/Button";

interface PingResult {
  host: string;
  resolved: string | null;
  sent: number;
  received: number;
  rtts: string[];
  raw: string;
}

interface DnsLookupResult {
  host: string;
  addresses: string[];
  elapsed_ms: number;
}

interface TcpTestResult {
  host: string;
  resolved: string | null;
  port: number;
  ok: boolean;
  latency_ms: number;
  error: string | null;
}

interface TracertHop {
  hop: number;
  ip: string | null;
  rtts: string[];
  raw: string;
}

interface TracertResult {
  host: string;
  hops: TracertHop[];
  raw: string;
  truncated: boolean;
}

export default function NetworkDiagnostics() {
  const { t } = useTranslation();

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
      setPingResult(
        await invoke<PingResult>("ping_host", { host: pingHost.trim(), count: pingCount }),
      );
    } catch (e: any) {
      setPingError(String(e));
    } finally {
      setPinging(false);
    }
  };

  // —— DNS ——
  const [dnsHost, setDnsHost] = useState("");
  const [dnsResult, setDnsResult] = useState<DnsLookupResult | null>(null);
  const [dnsLoading, setDnsLoading] = useState(false);
  const [dnsError, setDnsError] = useState<string | null>(null);

  const runDns = async () => {
    if (!dnsHost.trim()) return;
    setDnsLoading(true);
    setDnsError(null);
    setDnsResult(null);
    try {
      setDnsResult(await invoke<DnsLookupResult>("dns_lookup", { host: dnsHost.trim() }));
    } catch (e: any) {
      setDnsError(String(e));
    } finally {
      setDnsLoading(false);
    }
  };

  // —— TCP ——
  const [tcpHost, setTcpHost] = useState("");
  const [tcpPort, setTcpPort] = useState("");
  const [tcpTimeout, setTcpTimeout] = useState(3000);
  const [tcpResult, setTcpResult] = useState<TcpTestResult | null>(null);
  const [tcpLoading, setTcpLoading] = useState(false);
  const [tcpError, setTcpError] = useState<string | null>(null);

  const runTcp = async () => {
    const port = Number(tcpPort.trim());
    if (!tcpHost.trim() || !Number.isInteger(port) || port < 1 || port > 65535) {
      setTcpError(t("netpan.invalidPort"));
      return;
    }
    setTcpLoading(true);
    setTcpError(null);
    setTcpResult(null);
    try {
      setTcpResult(
        await invoke<TcpTestResult>("tcp_port_test", {
          host: tcpHost.trim(),
          port,
          timeoutMs: tcpTimeout,
        }),
      );
    } catch (e: any) {
      setTcpError(String(e));
    } finally {
      setTcpLoading(false);
    }
  };

  // —— 路由追踪 ——
  const [traceHost, setTraceHost] = useState("");
  const [traceHops, setTraceHops] = useState(16);
  const [traceResult, setTraceResult] = useState<TracertResult | null>(null);
  const [traceLoading, setTraceLoading] = useState(false);
  const [traceError, setTraceError] = useState<string | null>(null);
  const [showRaw, setShowRaw] = useState(false);

  const runTrace = async () => {
    if (!traceHost.trim()) return;
    setTraceLoading(true);
    setTraceError(null);
    setTraceResult(null);
    setShowRaw(false);
    try {
      setTraceResult(
        await invoke<TracertResult>("tracert_host", {
          host: traceHost.trim(),
          maxHops: traceHops,
        }),
      );
    } catch (e: any) {
      setTraceError(String(e));
    } finally {
      setTraceLoading(false);
    }
  };

  return (
    <div className="h-full overflow-y-auto p-4">
      <div className="mx-auto grid max-w-[1400px] grid-cols-1 gap-3 lg:grid-cols-2">
        {/* Ping */}
        <DiagCard icon={<Signal className="h-4 w-4 text-[var(--module-accent)]" />} title="Ping">
          <div className="flex shrink-0 gap-2">
            <input
              value={pingHost}
              onChange={(e) => setPingHost(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void runPing()}
              className="ui-input h-8 flex-1 px-3"
              placeholder={t("netpan.pingPh")}
            />
            <select
              value={pingCount}
              onChange={(e) => setPingCount(Number(e.target.value))}
              className="ui-select h-8 px-2"
            >
              {[1, 2, 4, 6, 8, 10].map((n) => (
                <option key={n} value={n}>
                  {t("netpan.pingCount", { n })}
                </option>
              ))}
            </select>
            <SharedButton
              variant="primary"
              onClick={() => void runPing()}
              disabled={pinging || !pingHost.trim()}
            >
              {pinging && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              {pinging ? t("netpan.pinging") : t("netpan.ping")}
            </SharedButton>
          </div>

          <div className="mt-3 min-h-0 flex-1 overflow-y-auto">
            {pingError && <ErrorNote text={pingError} />}
            {!pingError && !pingResult && (
              <p className="text-tiny text-slate-600">{t("netpan.pingHint")}</p>
            )}
            {pingResult && (
              <div className="space-y-2">
                <div className="flex flex-wrap items-center gap-3 rounded-ctl border border-white/5 bg-black/20 p-2.5 text-tiny">
                  <span
                    className={
                      pingResult.received > 0
                        ? "flex items-center gap-1 text-emerald-400"
                        : "flex items-center gap-1 text-rose-400"
                    }
                  >
                    {pingResult.received > 0 ? (
                      <CheckCircle className="h-3 w-3" />
                    ) : (
                      <AlertTriangle className="h-3 w-3" />
                    )}
                    {t("netpan.packets", { rcv: pingResult.received, sent: pingResult.sent })}
                  </span>
                  {pingResult.resolved && (
                    <span className="flex items-center gap-1 font-mono text-slate-400">
                      <Globe className="h-3 w-3" />
                      {pingResult.resolved}
                    </span>
                  )}
                  {pingResult.rtts.length > 0 && (
                    <span className="font-mono text-slate-300">
                      {pingResult.rtts.join(" · ")}
                    </span>
                  )}
                </div>
                {pingResult.raw && (
                  <pre className="max-h-40 overflow-y-auto whitespace-pre-wrap rounded-ctl border border-white/5 bg-black/30 p-2 font-mono text-micro text-slate-400">
                    {pingResult.raw}
                  </pre>
                )}
              </div>
            )}
          </div>
        </DiagCard>

        {/* DNS */}
        <DiagCard icon={<Globe className="h-4 w-4 text-emerald-400" />} title={t("netpan.dnsTitle")}>
          <div className="flex shrink-0 gap-2">
            <input
              value={dnsHost}
              onChange={(e) => setDnsHost(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void runDns()}
              className="ui-input h-8 flex-1 px-3"
              placeholder={t("netpan.dnsPh")}
            />
            <SharedButton
              variant="primary"
              onClick={() => void runDns()}
              disabled={dnsLoading || !dnsHost.trim()}
            >
              {dnsLoading && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              {dnsLoading ? t("netpan.dnsResolving") : t("netpan.dnsResolve")}
            </SharedButton>
          </div>

          <div className="mt-3 min-h-0 flex-1 overflow-y-auto">
            {dnsError && <ErrorNote text={dnsError} />}
            {!dnsError && !dnsResult && (
              <p className="text-tiny text-slate-600">{t("netpan.dnsNoResult")}</p>
            )}
            {dnsResult && (
              <div className="space-y-2">
                <p className="text-micro text-slate-500">
                  {t("netpan.dnsElapsed", { ms: dnsResult.elapsed_ms })}
                </p>
                <div className="flex flex-wrap gap-1.5">
                  {dnsResult.addresses.map((ip) => (
                    <span
                      key={ip}
                      className="ui-chip px-2 py-1 font-mono"
                    >
                      {ip}
                    </span>
                  ))}
                </div>
              </div>
            )}
          </div>
        </DiagCard>

        {/* TCP */}
        <DiagCard icon={<Plug className="h-4 w-4 text-sky-400" />} title={t("netpan.tcpTitle")}>
          <div className="flex shrink-0 flex-wrap gap-2">
            <input
              value={tcpHost}
              onChange={(e) => setTcpHost(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void runTcp()}
              className="ui-input h-8 min-w-0 flex-1 px-3"
              placeholder={t("netpan.tcpHostPh")}
            />
            <input
              value={tcpPort}
              onChange={(e) => setTcpPort(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void runTcp()}
              inputMode="numeric"
              className="ui-input h-8 w-24 px-3"
              placeholder={t("netpan.tcpPortPh")}
            />
            <select
              value={tcpTimeout}
              onChange={(e) => setTcpTimeout(Number(e.target.value))}
              className="ui-select h-8 px-2"
            >
              {[1000, 2000, 3000, 5000].map((ms) => (
                <option key={ms} value={ms}>
                  {t("netpan.tcpTimeout", { ms })}
                </option>
              ))}
            </select>
            <SharedButton
              variant="primary"
              onClick={() => void runTcp()}
              disabled={tcpLoading || !tcpHost.trim() || !tcpPort.trim()}
            >
              {tcpLoading && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              {tcpLoading ? t("netpan.tcpTesting") : t("netpan.tcpTest")}
            </SharedButton>
          </div>

          <div className="mt-3 min-h-0 flex-1 overflow-y-auto">
            {tcpError && <ErrorNote text={tcpError} />}
            {!tcpError && !tcpResult && (
              <p className="text-tiny text-slate-600">{t("netpan.tcpNoResult")}</p>
            )}
            {tcpResult && (
              <div
                className={`ui-note ${tcpResult.ok ? "ui-note-ok" : "ui-note-error"}`}
              >
                {tcpResult.ok ? (
                  <CheckCircle className="h-3.5 w-3.5 shrink-0" />
                ) : (
                  <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
                )}
                <div className="min-w-0 space-y-1">
                  <p className="font-semibold">
                    {tcpResult.ok ? t("netpan.tcpOk") : t("netpan.tcpFail")}
                    <span className="ml-2 font-mono font-normal">
                      {tcpResult.host}:{tcpResult.port}
                    </span>
                  </p>
                  <p className="font-mono text-micro opacity-80">
                    {tcpResult.resolved || "-"} ·{" "}
                    {t("netpan.tcpLatency", { ms: tcpResult.latency_ms })}
                  </p>
                  {tcpResult.error && (
                    <p className="break-all font-mono text-micro opacity-70">
                      {tcpResult.error}
                    </p>
                  )}
                </div>
              </div>
            )}
          </div>
        </DiagCard>

        {/* 路由追踪 */}
        <DiagCard icon={<Route className="h-4 w-4 text-violet-400" />} title={t("netpan.traceTitle")}>
          <div className="flex shrink-0 gap-2">
            <input
              value={traceHost}
              onChange={(e) => setTraceHost(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void runTrace()}
              className="ui-input h-8 flex-1 px-3"
              placeholder={t("netpan.tracePh")}
            />
            <select
              value={traceHops}
              onChange={(e) => setTraceHops(Number(e.target.value))}
              className="ui-select h-8 px-2"
            >
              {[8, 16, 30].map((n) => (
                <option key={n} value={n}>
                  {t("netpan.traceHops", { n })}
                </option>
              ))}
            </select>
            <SharedButton
              variant="primary"
              onClick={() => void runTrace()}
              disabled={traceLoading || !traceHost.trim()}
            >
              {traceLoading && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              {traceLoading ? t("netpan.traceRunning") : t("netpan.traceStart")}
            </SharedButton>
          </div>

          <div className="mt-3 min-h-0 flex-1 overflow-y-auto">
            {traceError && <ErrorNote text={traceError} />}
            {!traceError && !traceResult && (
              <p className="text-tiny text-slate-600">{t("netpan.traceNoResult")}</p>
            )}
            {traceResult && (
              <div className="space-y-2">
                {traceResult.truncated && (
                  <p className="text-micro text-amber-400">{t("netpan.traceTruncated")}</p>
                )}
                <table className="w-full text-tiny">
                  <thead className="sticky top-0 bg-surface-deep">
                    <tr className="border-b border-white/5 font-semibold text-slate-400">
                      <td className="w-10 py-1 pr-2">{t("netpan.traceHop")}</td>
                      <td className="py-1 pr-2">{t("netpan.traceIp")}</td>
                      <td className="py-1">{t("netpan.traceRtt")}</td>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-white/[0.03]">
                    {traceResult.hops.map((h) => (
                      <tr key={`${h.hop}-${h.raw}`} className="hover:bg-white/[0.02]">
                        <td className="py-1 font-mono text-slate-500">{h.hop}</td>
                        <td className="py-1 font-mono text-slate-300">
                          {h.ip ?? (
                            <span className="text-amber-400">{t("netpan.traceNoIp")}</span>
                          )}
                        </td>
                        <td className="py-1 font-mono text-slate-400">
                          {h.rtts.length > 0 ? h.rtts.join(" · ") : "-"}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                <button
                  onClick={() => setShowRaw((v) => !v)}
                  className="ui-btn ui-btn-ghost h-7 px-2 text-micro"
                >
                  {showRaw ? (
                    <ChevronUp className="h-3 w-3" />
                  ) : (
                    <ChevronDown className="h-3 w-3" />
                  )}
                  {showRaw ? t("netpan.hideRaw") : t("netpan.showRaw")}
                </button>
                {showRaw && (
                  <pre className="max-h-48 overflow-y-auto whitespace-pre-wrap rounded-ctl border border-white/5 bg-black/30 p-2 font-mono text-micro text-slate-400">
                    {traceResult.raw}
                  </pre>
                )}
              </div>
            )}
          </div>
        </DiagCard>
      </div>
    </div>
  );
}

function DiagCard({
  icon,
  title,
  children,
}: {
  icon: ReactNode;
  title: string;
  children: ReactNode;
}) {
  return (
    <section className="ui-card flex min-h-[320px] flex-col p-4">
      <div className="mb-3 flex shrink-0 items-center gap-2 border-b border-white/5 pb-2">
        {icon}
        <h4 className="text-body font-semibold text-white">{title}</h4>
      </div>
      {children}
    </section>
  );
}

function ErrorNote({ text }: { text: string }) {
  return (
    <div className="ui-note ui-note-error">
      <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
      <span className="break-all">{text}</span>
    </div>
  );
}
