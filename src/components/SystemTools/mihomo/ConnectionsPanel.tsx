// 连接页 —— 1:1 复刻 clash-party src/renderer/src/pages/connections.tsx 行为
// （进程图标/应用名依赖 Electron 原生 API，Tauri 侧省略）
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pause, Play, X, Trash2, Search, ArrowUp, ArrowDown, Table2, List } from "lucide-react";
import { mihomoApi } from "../mihomoApi";
import { IMihomoConnection, openMihomoWs, closeConnection as apiClose, closeAllConnections, WsHandle } from "./ctrl";
import { calcTraffic, cardCls, Modal, btnSec } from "./ui";

const FILTER_KEY = "mihomo-connections-filter";

type OrderBy = "time" | "upload" | "download" | "uploadSpeed" | "downloadSpeed";
const ORDER_OPTIONS: { k: OrderBy; t: string }[] = [
  { k: "time", t: "connSortTime" },
  { k: "upload", t: "connSortUpload" },
  { k: "download", t: "connSortDownload" },
  { k: "uploadSpeed", t: "connSortUploadSpeed" },
  { k: "downloadSpeed", t: "connSortDownloadSpeed" },
];

// 表格列（对齐 clash-party CONNECTION_TABLE_COLUMNS）
const TABLE_COLUMNS: { key: string; t: string }[] = [
  { key: "host", t: "connColHost" },
  { key: "type", t: "connColType" },
  { key: "rule", t: "connColRule" },
  { key: "chains", t: "connColChains" },
  { key: "process", t: "connColProcess" },
  { key: "upload", t: "connColUpload" },
  { key: "download", t: "connColDownload" },
  { key: "uploadSpeed", t: "connColUploadSpeed" },
  { key: "downloadSpeed", t: "connColDownloadSpeed" },
  { key: "sourceIP", t: "connColSourceIP" },
  { key: "destinationIP", t: "connColDestIP" },
  { key: "start", t: "connColStart" },
];

function timeAgo(iso: string, t: (k: string, o?: any) => string): string {
  const s = Math.max(0, Math.floor((Date.now() - new Date(iso).getTime()) / 1000));
  if (s < 60) return t("mihomo.timeAgoS", { n: s });
  if (s < 3600) return t("mihomo.timeAgoM", { n: Math.floor(s / 60) });
  if (s < 86400) return t("mihomo.timeAgoH", { n: Math.floor(s / 3600) });
  return t("mihomo.timeAgoD", { n: Math.floor(s / 86400) });
}

function connHost(c: IMihomoConnection): string {
  const m = c.metadata;
  return (m.host || m.sniffHost || m.destinationIP || "") + (m.destinationPort ? `:${m.destinationPort}` : "");
}

export default function ConnectionsPanel({ info, running }: { info: any; running: boolean }) {
  const { t } = useTranslation();
  const [filter, setFilter] = useState(() => localStorage.getItem(FILTER_KEY) || "");
  const [tab, setTab] = useState<"active" | "closed">("active");
  const [paused, setPaused] = useState(false);
  const [connInfo, setConnInfo] = useState<{ uploadTotal: number; downloadTotal: number }>({ uploadTotal: 0, downloadTotal: 0 });
  const [active, setActive] = useState<IMihomoConnection[]>([]);
  const [closed, setClosed] = useState<IMihomoConnection[]>([]);
  const [detail, setDetail] = useState<IMihomoConnection | null>(null);
  const [cfg, setCfg] = useState<any>(null);

  const allRef = useRef<Map<string, IMihomoConnection>>(new Map());
  const activeRef = useRef<IMihomoConnection[]>([]);
  const pausedRef = useRef(paused);
  pausedRef.current = paused;

  const viewMode: "list" | "table" = cfg?.connectionViewMode === "table" ? "table" : "list";
  const orderBy: OrderBy = (cfg?.connectionOrderBy as OrderBy) || "time";
  const direction: "asc" | "desc" = cfg?.connectionDirection === "desc" ? "desc" : "asc";

  const loadCfg = async () => { try { setCfg(await mihomoApi.getAppConfig()); } catch (err) { console.error("加载连接面板配置失败:", err); } };
  const patchCfg = async (p: any) => { await mihomoApi.patchAppConfig(p); loadCfg(); };

  useEffect(() => { loadCfg(); }, []);
  useEffect(() => { localStorage.setItem(FILTER_KEY, filter); }, [filter]);

  // WS 订阅（复刻 clash-party：速率 = 本次流量 - 上次流量，1s 推送间隔）
  useEffect(() => {
    if (!running || !info?.port) return;
    let ws: WsHandle | null = null;
    ws = openMihomoWs(info, "connections", (data) => {
      if (pausedRef.current) return;
      setConnInfo({ uploadTotal: data.uploadTotal || 0, downloadTotal: data.downloadTotal || 0 });
      const conns: IMihomoConnection[] = data.connections || [];
      const prevMap = new Map(activeRef.current.map((c) => [c.id, c]));
      const activeConns = conns.map((c) => {
        const prev = prevMap.get(c.id);
        return {
          ...c,
          isActive: true,
          downloadSpeed: prev ? c.download - prev.download : 0,
          uploadSpeed: prev ? c.upload - prev.upload : 0,
        };
      });
      // 累积全部连接（封顶 活动数+200，复刻 clash-party slice 策略）
      for (const c of activeConns) allRef.current.set(c.id, c);
      const activeIds = new Set(activeConns.map((c) => c.id));
      const all = Array.from(allRef.current.values());
      const closedConns = all
        .filter((c) => !activeIds.has(c.id))
        .map((c) => ({ ...c, isActive: false, downloadSpeed: 0, uploadSpeed: 0 }));
      const keep = all.slice(-(activeConns.length + 200));
      allRef.current = new Map(keep.map((c) => [c.id, c]));
      activeRef.current = activeConns;
      setActive(activeConns);
      setClosed(closedConns);
    });
    return () => ws?.close();
  }, [running, info?.port, info?.secret]);

  // 过滤 + 排序（复刻：JSON 全文匹配忽略大小写）
  const filtered = useMemo(() => {
    const list = tab === "active" ? active : closed;
    const kw = filter.trim().toLowerCase();
    let out = kw ? list.filter((c) => JSON.stringify(c).toLowerCase().includes(kw)) : [...list];
    out.sort((a, b) => {
      let cmp = 0;
      switch (orderBy) {
        case "time": cmp = new Date(a.start).getTime() - new Date(b.start).getTime(); break;
        case "upload": cmp = a.upload - b.upload; break;
        case "download": cmp = a.download - b.download; break;
        case "uploadSpeed": cmp = (a.uploadSpeed || 0) - (b.uploadSpeed || 0); break;
        case "downloadSpeed": cmp = (a.downloadSpeed || 0) - (b.downloadSpeed || 0); break;
      }
      return direction === "asc" ? cmp : -cmp;
    });
    return out;
  }, [active, closed, tab, filter, orderBy, direction]);

  // 关闭（复刻：active 调 API；closed 从本地移除）
  const close = (id: string) => {
    if (tab === "active") apiClose(id);
    else {
      allRef.current.delete(id);
      setClosed((l) => l.filter((c) => c.id !== id));
    }
  };
  const closeAll = () => {
    if (filter) { filtered.forEach((c) => close(c.id)); return; }
    if (tab === "active") closeAllConnections();
    else {
      const ids = new Set(closed.map((c) => c.id));
      ids.forEach((id) => allRef.current.delete(id));
      setClosed([]);
    }
  };

  if (!running) {
    return <div className={`${cardCls} p-6 text-center text-xs text-slate-400`}>{t("mihomo.coreNotRunning")}</div>;
  }

  return (
    <div className="space-y-2">
      {/* 顶栏 */}
      <div className="flex items-center gap-2 flex-wrap">
        <div className="flex items-center rounded-lg bg-white/5 border border-white/10 overflow-hidden">
          {(["active", "closed"] as const).map((k) => (
            <button
              key={k}
              onClick={() => setTab(k)}
              className={`px-3 py-1.5 text-[11px] font-semibold cursor-pointer transition-all ${
                tab === k ? (k === "active" ? "bg-[var(--module-accent)] text-white" : "bg-rose-500/60 text-white") : "text-slate-400 hover:text-slate-200"
              }`}
            >
              {k === "active" ? t("mihomo.connActive", { count: active.length }) : t("mihomo.connClosed", { count: closed.length })}
            </button>
          ))}
        </div>
        <div className="relative flex-1 min-w-40">
          <Search className="w-3.5 h-3.5 absolute left-2.5 top-1/2 -translate-y-1/2 text-slate-500" />
          <input
            className="w-full h-8 pl-8 pr-2.5 rounded-lg bg-white/5 border border-white/10 text-xs text-white placeholder-slate-500 focus:outline-none focus:border-[var(--module-accent)]"
            placeholder={t("mihomo.connFilter")}
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
        </div>
        {viewMode === "list" && (
          <>
            <select
              className="h-8 px-2 rounded-lg bg-white/10 border border-white/10 text-[11px] text-slate-200 cursor-pointer focus:outline-none"
              value={orderBy}
              onChange={(e) => patchCfg({ connectionOrderBy: e.target.value })}
            >
              {ORDER_OPTIONS.map((o) => <option key={o.k} value={o.k} className="bg-slate-800">{t(`mihomo.${o.t}`)}</option>)}
            </select>
            <button className={btnSec} title={t("mihomo.connSortDir")} onClick={() => patchCfg({ connectionDirection: direction === "asc" ? "desc" : "asc" })}>
              {direction === "asc" ? <ArrowUp className="w-3.5 h-3.5" /> : <ArrowDown className="w-3.5 h-3.5" />}
            </button>
          </>
        )}
        <button className={btnSec} title={viewMode === "list" ? t("mihomo.connViewTable") : t("mihomo.connViewList")} onClick={() => patchCfg({ connectionViewMode: viewMode === "list" ? "table" : "list" })}>
          {viewMode === "list" ? <Table2 className="w-3.5 h-3.5" /> : <List className="w-3.5 h-3.5" />}
        </button>
        <button className={btnSec} title={paused ? t("mihomo.connResume") : t("mihomo.connPause")} onClick={() => setPaused((p) => !p)}>
          {paused ? <Play className="w-3.5 h-3.5" /> : <Pause className="w-3.5 h-3.5" />}
        </button>
        <button className={btnSec} title={tab === "active" ? t("mihomo.connCloseAll") : t("mihomo.connClear")} onClick={closeAll}>
          {tab === "active" ? <X className="w-3.5 h-3.5" /> : <Trash2 className="w-3.5 h-3.5" />}
        </button>
      </div>

      <div className="text-[11px] text-slate-400 px-1">
        ↑ {calcTraffic(connInfo.uploadTotal)} &nbsp; ↓ {calcTraffic(connInfo.downloadTotal)} &nbsp;·&nbsp; {t("mihomo.connCount", { count: filtered.length })}
      </div>

      {/* 内容 */}
      {viewMode === "list" ? (
        <div className="space-y-1.5 max-h-[60vh] overflow-y-auto pr-1">
          {filtered.slice(0, 500).map((c) => (
            <div
              key={c.id}
              className={`${cardCls} px-3 py-2 cursor-pointer hover:border-white/20 flex items-center gap-3`}
              onClick={() => setDetail(c)}
            >
              <div className="min-w-0 flex-1">
                <div className="text-[12px] text-white truncate font-medium">
                  {connHost(c)}
                  {c.metadata.process && <span className="text-slate-400 font-normal"> · {c.metadata.process}</span>}
                </div>
                <div className="text-[10px] text-slate-500 truncate">
                  {c.metadata.type}({c.metadata.network}) · {c.rule}{c.rulePayload ? `(${c.rulePayload})` : ""} · {(c.chains || []).slice().reverse().join(" → ")}
                </div>
              </div>
              <div className="text-[10px] text-slate-400 text-right flex-shrink-0 font-mono">
                <div>↑ {calcTraffic(c.upload)} ↓ {calcTraffic(c.download)}</div>
                <div>
                  {(c.uploadSpeed || c.downloadSpeed) ? `↑ ${calcTraffic(c.uploadSpeed || 0)}/s ↓ ${calcTraffic(c.downloadSpeed || 0)}/s` : timeAgo(c.start, t)}
                </div>
              </div>
              <button
                className="p-1 rounded-md hover:bg-rose-500/20 text-slate-500 hover:text-rose-300 cursor-pointer flex-shrink-0"
                onClick={(e) => { e.stopPropagation(); close(c.id); }}
              >
                {tab === "active" ? <X className="w-3.5 h-3.5" /> : <Trash2 className="w-3.5 h-3.5" />}
              </button>
            </div>
          ))}
          {filtered.length === 0 && <div className={`${cardCls} p-6 text-center text-xs text-slate-400`}>{t("mihomo.connEmpty")}</div>}
        </div>
      ) : (
        <div className={`${cardCls} overflow-auto max-h-[60vh]`}>
          <table className="w-full text-[11px] whitespace-nowrap">
            <thead className="sticky top-0 bg-[#11151f] z-10">
              <tr className="text-slate-400 text-left">
                <th className="px-2 py-2 font-medium"></th>
                {TABLE_COLUMNS.map((c) => <th key={c.key} className="px-2 py-2 font-medium">{t(`mihomo.${c.t}`)}</th>)}
              </tr>
            </thead>
            <tbody>
              {filtered.slice(0, 500).map((c) => (
                <tr key={c.id} className="border-t border-white/5 hover:bg-white/[0.03] cursor-pointer text-slate-300" onClick={() => setDetail(c)}>
                  <td className="px-2 py-1.5">
                    <button className="p-0.5 rounded hover:bg-rose-500/20 text-slate-500 hover:text-rose-300 cursor-pointer" onClick={(e) => { e.stopPropagation(); close(c.id); }}>
                      <X className="w-3 h-3" />
                    </button>
                  </td>
                  <td className="px-2 py-1.5 max-w-52 truncate">{connHost(c)}</td>
                  <td className="px-2 py-1.5">{c.metadata.type}({c.metadata.network})</td>
                  <td className="px-2 py-1.5 max-w-40 truncate">{c.rule}{c.rulePayload ? `(${c.rulePayload})` : ""}</td>
                  <td className="px-2 py-1.5 max-w-52 truncate">{(c.chains || []).slice().reverse().join(" → ")}</td>
                  <td className="px-2 py-1.5 max-w-32 truncate">{c.metadata.process || "-"}</td>
                  <td className="px-2 py-1.5 font-mono">{calcTraffic(c.upload)}</td>
                  <td className="px-2 py-1.5 font-mono">{calcTraffic(c.download)}</td>
                  <td className="px-2 py-1.5 font-mono">{calcTraffic(c.uploadSpeed || 0)}/s</td>
                  <td className="px-2 py-1.5 font-mono">{calcTraffic(c.downloadSpeed || 0)}/s</td>
                  <td className="px-2 py-1.5 font-mono">{c.metadata.sourceIP}</td>
                  <td className="px-2 py-1.5 font-mono">{c.metadata.destinationIP || c.metadata.remoteDestination || "-"}</td>
                  <td className="px-2 py-1.5">{timeAgo(c.start, t)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {filtered.length === 0 && <div className="p-6 text-center text-xs text-slate-400">{t("mihomo.connEmpty")}</div>}
        </div>
      )}

      {/* 详情弹窗（复刻 connection-detail-modal：展示全部字段） */}
      {detail && (
        <Modal title={t("mihomo.connDetailTitle")} onClose={() => setDetail(null)}>
          <div className="space-y-1 text-[11px]">
            {[
              [t("mihomo.connDId"), detail.id],
              [t("mihomo.connDHost"), connHost(detail)],
              [t("mihomo.connDDown"), calcTraffic(detail.download)],
              [t("mihomo.connDUp"), calcTraffic(detail.upload)],
              [t("mihomo.connDDownSpeed"), `${calcTraffic(detail.downloadSpeed || 0)}/s`],
              [t("mihomo.connDUpSpeed"), `${calcTraffic(detail.uploadSpeed || 0)}/s`],
              [t("mihomo.connDStart"), timeAgo(detail.start, t)],
              [t("mihomo.connDRule"), `${detail.rule}${detail.rulePayload ? `(${detail.rulePayload})` : ""}`],
              [t("mihomo.connDChains"), (detail.chains || []).slice().reverse().join(" → ")],
              [t("mihomo.connDType"), `${detail.metadata.type}(${detail.metadata.network})`],
              [t("mihomo.connDSource"), `${detail.metadata.sourceIP}:${detail.metadata.sourcePort}`],
              [t("mihomo.connDTarget"), `${detail.metadata.destinationIP || detail.metadata.host}:${detail.metadata.destinationPort}`],
              [t("mihomo.connDSniff"), detail.metadata.sniffHost || "-"],
              [t("mihomo.connDProcess"), detail.metadata.process || "-"],
              [t("mihomo.connDProcessPath"), detail.metadata.processPath || "-"],
              [t("mihomo.connDRemote"), detail.metadata.remoteDestination || "-"],
              ["DSCP", String((detail.metadata as any).dscp ?? "-")],
            ].map(([k, v]) => (
              <div key={k as string} className="flex gap-2 py-1 border-b border-white/5">
                <span className="text-slate-400 w-28 flex-shrink-0">{k}</span>
                <span className="text-slate-200 break-all font-mono">{v}</span>
              </div>
            ))}
          </div>
        </Modal>
      )}
    </div>
  );
}
