import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity, ArrowDownToLine, ArrowUpFromLine, Loader2,
  Pause, Play, RefreshCw, Wifi, WifiOff,
} from "lucide-react";
import { formatBytes } from "../../utils/format";
import { SharedButton } from "../shared/Button";
import VexEmptyState from "../VexEmptyState";

interface NetAdapter {
  name: string;
  description: string;
  index: number;
  status: string;
  mac: string;
  speed: string;
  ipv4: string[];
  ipv6: string[];
  gateway: string[];
  dns: string[];
  rx_bytes: number;
  tx_bytes: number;
  rx_packets: number;
  tx_packets: number;
}

interface TrafficCounter {
  name: string;
  mac: string;
  rx_bytes: number;
  tx_bytes: number;
}

interface TrafficSample {
  ts_ms: number;
  adapters: TrafficCounter[];
}

interface Rate {
  rx: number;
  tx: number;
}

const CHART_SAMPLES = 90;

const normMac = (mac: string) => mac.toUpperCase().replace(/[^0-9A-F]/g, "");
const macKey = (mac: string) => `mac:${normMac(mac)}`;

function formatRate(bytesPerSec: number) {
  return `${formatBytes(bytesPerSec, 1)}/s`;
}

function statusStyle(status: string) {
  const s = status.toLowerCase();
  if (s === "up") {
    return {
      key: "netpan.statusUp",
      cls: "text-emerald-400 bg-emerald-500/10 border-emerald-500/20",
      Icon: Wifi,
    };
  }
  if (s === "disconnected") {
    return {
      key: "netpan.statusDisconnected",
      cls: "text-amber-400 bg-amber-500/10 border-amber-500/20",
      Icon: WifiOff,
    };
  }
  if (s === "down") {
    return {
      key: "netpan.statusDown",
      cls: "text-rose-400 bg-rose-500/10 border-rose-500/20",
      Icon: WifiOff,
    };
  }
  if (s === "not present") {
    return {
      key: "netpan.statusNotPresent",
      cls: "text-slate-400 bg-white/5 border-white/10",
      Icon: WifiOff,
    };
  }
  return {
    key: "netpan.statusUnknown",
    cls: "text-slate-400 bg-white/5 border-white/10",
    Icon: WifiOff,
  };
}

/** 地址列表太长时只显示前 n 个，完整值放在 title 里。 */
function joinList(list: string[], max = 2) {
  if (list.length === 0) return "-";
  const head = list.slice(0, max).join(" · ");
  return list.length > max ? `${head} +${list.length - max}` : head;
}

/**
 * `active` = 当前是否停留在「概览」页签。面板会常驻挂载它（切走时只隐藏），
 * 所以采样必须自己看这个标记停掉 —— 否则切到别的页签还在后台每秒采样。
 */
export default function NetworkOverview({ active = true }: { active?: boolean }) {
  const { t } = useTranslation();
  const [adapters, setAdapters] = useState<NetAdapter[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [live, setLive] = useState(true);
  const [rates, setRates] = useState<Record<string, Rate>>({});
  const [totals, setTotals] = useState<Rate>({ rx: 0, tx: 0 });
  const [history, setHistory] = useState<Rate[]>([]);

  /** 只统计处于 Up 状态的网卡，避免未连接 / 已停用网卡的 0 速率参与汇总。 */
  const activeRef = useRef<{ names: Set<string>; macs: Set<string>; all: boolean }>({
    names: new Set(),
    macs: new Set(),
    all: true,
  });

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setAdapters(await invoke<NetAdapter[]>("net_adapters"));
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    const up = (adapters ?? []).filter((a) => a.status.toLowerCase() === "up");
    activeRef.current = {
      names: new Set(up.map((a) => a.name)),
      macs: new Set(up.map((a) => normMac(a.mac)).filter(Boolean)),
      all: up.length === 0,
    };
  }, [adapters]);

  // 实时采样：仅在本页可见且 live 打开时轮询；离开页签 / 窗口隐藏自动跳过。
  useEffect(() => {
    if (!live || !active) return;
    let alive = true;
    let timer: number | undefined;
    let prev: { ts: number; map: Record<string, Rate> } | null = null;

    const tick = async () => {
      if (document.hidden) {
        // 窗口隐藏期间不采样；清掉上一帧快照，避免恢复时把长间隔平均成瞬时速率
        prev = null;
        if (alive) timer = window.setTimeout(tick, 1500);
        return;
      }
      try {
        const sample = await invoke<TrafficSample>("net_traffic");
        if (!alive) return;

        const nextRates: Record<string, Rate> = {};
        const seconds = prev ? (sample.ts_ms - prev.ts) / 1000 : 0;
        let sumRx = 0;
        let sumTx = 0;
        const filter = activeRef.current;

        for (const c of sample.adapters) {
          const old = prev?.map[c.name] ?? prev?.map[macKey(c.mac)];
          const rx = prev && seconds > 0 && old ? Math.max(0, (c.rx_bytes - old.rx) / seconds) : 0;
          const tx = prev && seconds > 0 && old ? Math.max(0, (c.tx_bytes - old.tx) / seconds) : 0;
          const rate = { rx, tx };
          nextRates[c.name] = rate;
          if (normMac(c.mac)) nextRates[macKey(c.mac)] = rate;

          const included =
            filter.all || filter.names.has(c.name) || filter.macs.has(normMac(c.mac));
          if (included) {
            sumRx += rx;
            sumTx += tx;
          }
        }

        // 下一轮同时按名称与 MAC 建立快照，任一侧名称不一致也能对上计数
        const nextMap: Record<string, Rate> = {};
        for (const c of sample.adapters) {
          const value = { rx: c.rx_bytes, tx: c.tx_bytes };
          nextMap[c.name] = value;
          if (normMac(c.mac)) nextMap[macKey(c.mac)] = value;
        }
        const hasDelta = prev !== null && seconds > 0;
        prev = { ts: sample.ts_ms, map: nextMap };
        setRates(nextRates);
        setTotals({ rx: sumRx, tx: sumTx });
        // 第一帧（以及暂停/隐藏/切页签后的第一帧）没有可用差值，不往曲线里塞 0 点，
        // 否则每次回到概览都会在曲线上划出一道假谷底。
        if (hasDelta) {
          setHistory((h) => [...h, { rx: sumRx, tx: sumTx }].slice(-CHART_SAMPLES));
        }
      } catch {
        // 单次采样失败（网卡切换、休眠唤醒）不打断轮询
      }
      if (alive) timer = window.setTimeout(tick, 1000);
    };

    void tick();
    return () => {
      alive = false;
      if (timer) window.clearTimeout(timer);
    };
  }, [live]);

  const rateFor = (a: NetAdapter): Rate =>
    rates[a.name] ?? rates[macKey(a.mac)] ?? { rx: 0, tx: 0 };

  const sorted = [...(adapters ?? [])].sort((a, b) => {
    const upA = a.status.toLowerCase() === "up" ? 1 : 0;
    const upB = b.status.toLowerCase() === "up" ? 1 : 0;
    return upB - upA;
  });

  return (
    <div className="h-full overflow-y-auto p-4">
      <div className="mx-auto flex max-w-[1400px] flex-col gap-4">
        {/* 工具条 */}
        <div className="flex flex-wrap items-center gap-2">
          <Activity className="h-4 w-4 text-[var(--module-accent)]" />
          <h3 className="text-title font-semibold text-white">{t("netpan.overviewTitle")}</h3>
          <span className="text-micro text-slate-500">{t("netpan.overviewHint")}</span>
          <div className="flex-1" />
          <SharedButton onClick={() => setLive((v) => !v)}>
            {live ? <Pause className="h-3.5 w-3.5" /> : <Play className="h-3.5 w-3.5" />}
            {live ? t("netpan.liveOn") : t("netpan.liveOff")}
          </SharedButton>
          <SharedButton onClick={() => void load()} disabled={loading}>
            {loading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
            {t("netpan.refreshAdapters")}
          </SharedButton>
        </div>

        {error && (
          <div className="ui-note ui-note-error">
            <span>{t("netpan.adapterLoadFail", { err: error })}</span>
          </div>
        )}

        {/* 实时流量 */}
        <section className="ui-card p-4">
          <div className="flex flex-wrap items-center gap-2">
            <Activity className="h-4 w-4 text-sky-400" />
            <h4 className="text-body font-semibold text-white">{t("netpan.trafficTitle")}</h4>
            <span className="text-micro text-slate-500">{t("netpan.trafficHint")}</span>
            <div className="flex-1" />
            <span className="text-tiny font-mono text-sky-300">
              ↓ {formatRate(totals.rx)}
            </span>
            <span className="text-tiny font-mono text-violet-300">
              ↑ {formatRate(totals.tx)}
            </span>
          </div>
          <TrafficChart history={history} />
          <div className="mt-2 flex items-center gap-4 text-micro text-slate-500">
            <span className="flex items-center gap-1.5">
              <span className="inline-block h-0.5 w-3 bg-sky-400" />
              {t("netpan.download")}
            </span>
            <span className="flex items-center gap-1.5">
              <span className="inline-block h-0.5 w-3 bg-violet-400" />
              {t("netpan.upload")}
            </span>
            <span className="ml-auto">{live ? t("netpan.sampling") : t("netpan.paused")}</span>
          </div>
        </section>

        {/* 网卡列表 */}
        {loading && !adapters && (
          <div className="flex items-center gap-2 py-8 text-body text-slate-500">
            <Loader2 className="h-4 w-4 animate-spin" />
            {t("netpan.loadingAdapters")}
          </div>
        )}

        {adapters && adapters.length === 0 && (
          <VexEmptyState
            title={t("netpan.noAdapters")}
            desc={t("netpan.noAdaptersDesc")}
            action={{ label: t("netpan.refreshAdapters"), onClick: () => void load() }}
          />
        )}

        <div className="grid grid-cols-1 gap-3 xl:grid-cols-2 2xl:grid-cols-3">
          {sorted.map((a) => {
            const meta = statusStyle(a.status);
            const rate = rateFor(a);
            return (
              <section key={`${a.name}-${a.index}`} className="ui-card p-4">
                <div className="flex items-start gap-2">
                  <meta.Icon className={`mt-0.5 h-4 w-4 shrink-0 ${
                    a.status.toLowerCase() === "up" ? "text-emerald-400" : "text-slate-500"
                  }`} />
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-1.5">
                      <h5 className="truncate text-title font-semibold text-white">
                        {a.name || "-"}
                      </h5>
                      <span className={`rounded px-1.5 py-0.5 text-micro font-semibold border ${meta.cls}`}>
                        {t(meta.key)}
                      </span>
                      {a.speed && (
                        <span className="text-micro text-slate-500">{a.speed}</span>
                      )}
                    </div>
                    <p className="mt-0.5 truncate text-micro text-slate-500" title={a.description}>
                      {a.description || "-"}
                    </p>
                  </div>
                  <div className="shrink-0 text-right font-mono">
                    <p className="text-micro text-slate-600">{t("netpan.currentRate")}</p>
                    <p className="text-tiny text-sky-300">↓ {formatRate(rate.rx)}</p>
                    <p className="text-tiny text-violet-300">↑ {formatRate(rate.tx)}</p>
                  </div>
                </div>

                <div className="mt-3 space-y-1.5 text-tiny">
                  <InfoRow label={t("netpan.mac")} value={a.mac || "-"} />
                  <InfoRow
                    label={t("netpan.ipv4")}
                    value={joinList(a.ipv4)}
                    title={a.ipv4.join("\n")}
                    mono
                  />
                  {a.ipv6.length > 0 && (
                    <InfoRow
                      label={t("netpan.ipv6")}
                      value={joinList(a.ipv6, 1)}
                      title={a.ipv6.join("\n")}
                      mono
                    />
                  )}
                  <InfoRow
                    label={t("netpan.gateway")}
                    value={joinList(a.gateway)}
                    title={a.gateway.join("\n")}
                    mono
                  />
                  <InfoRow
                    label={t("netpan.dns")}
                    value={joinList(a.dns, 2)}
                    title={a.dns.join("\n")}
                    mono
                  />
                </div>

                <div className="mt-3 flex items-center gap-4 border-t border-white/5 pt-2 text-micro text-slate-500">
                  <span className="flex items-center gap-1">
                    <ArrowDownToLine className="h-3 w-3 text-sky-400" />
                    {t("netpan.received")} {formatBytes(a.rx_bytes)}
                  </span>
                  <span className="flex items-center gap-1">
                    <ArrowUpFromLine className="h-3 w-3 text-violet-400" />
                    {t("netpan.sent")} {formatBytes(a.tx_bytes)}
                  </span>
                  <span className="ml-auto font-mono">
                    {a.rx_packets + a.tx_packets > 0
                      ? t("netpan.packetCount", { n: a.rx_packets + a.tx_packets })
                      : ""}
                  </span>
                </div>
              </section>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function InfoRow({
  label,
  value,
  title,
  mono,
}: {
  label: string;
  value: string;
  title?: string;
  mono?: boolean;
}) {
  return (
    <div className="flex items-baseline gap-2">
      <span className="w-12 shrink-0 text-slate-600">{label}</span>
      <span
        className={`min-w-0 flex-1 truncate text-slate-300 ${mono ? "font-mono" : ""}`}
        title={title || value}
      >
        {value}
      </span>
    </div>
  );
}

function TrafficChart({ history }: { history: Rate[] }) {
  const { t } = useTranslation();
  const W = 1000;
  const H = 120;
  const max = Math.max(1, ...history.map((s) => Math.max(s.rx, s.tx)));
  const points = (key: "rx" | "tx") =>
    history
      .map((s, i) => {
        const x = (i / Math.max(1, history.length - 1)) * W;
        const y = H - (s[key] / max) * (H - 4) - 2;
        return `${x},${y}`;
      })
      .join(" ");

  return (
    <div className="relative mt-2 h-28 w-full overflow-hidden rounded-ctl border border-white/5 bg-black/20">
      {history.length < 2 && (
        <div className="absolute inset-0 flex items-center justify-center text-micro text-slate-600">
          {t("netpan.waitingSample")}
        </div>
      )}
      <svg
        className="absolute inset-0 h-full w-full"
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="none"
      >
        {[0.25, 0.5, 0.75].map((p) => (
          <line
            key={p}
            x1="0"
            x2={W}
            y1={H * p}
            y2={H * p}
            stroke="rgba(255,255,255,0.05)"
            strokeWidth="1"
            vectorEffect="non-scaling-stroke"
          />
        ))}
        {history.length > 1 && (
          <>
            <polyline
              points={points("tx")}
              fill="none"
              stroke="#a78bfa"
              strokeWidth="2"
              strokeLinejoin="round"
              strokeLinecap="round"
              vectorEffect="non-scaling-stroke"
            />
            <polyline
              points={points("rx")}
              fill="none"
              stroke="#38bdf8"
              strokeWidth="2"
              strokeLinejoin="round"
              strokeLinecap="round"
              vectorEffect="non-scaling-stroke"
            />
          </>
        )}
      </svg>
      <span className="absolute right-2 top-1 font-mono text-micro text-slate-500">
        {formatBytes(max, 1)}/s
      </span>
    </div>
  );
}
