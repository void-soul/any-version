// 连接页 —— 1:1 复刻 clash-party src/renderer/src/pages/connections.tsx 行为
// （进程图标/应用名依赖 Electron 原生 API，Tauri 侧省略）
import { useEffect, useMemo, useRef, useState } from "react";
import { Pause, Play, X, Trash2, Search, ArrowUp, ArrowDown, Table2, List } from "lucide-react";
import { mihomoApi } from "../mihomoApi";
import { IMihomoConnection, openMihomoWs, closeConnection as apiClose, closeAllConnections, WsHandle } from "./ctrl";
import { calcTraffic, cardCls, Modal, btnSec } from "./ui";

const FILTER_KEY = "mihomo-connections-filter";

type OrderBy = "time" | "upload" | "download" | "uploadSpeed" | "downloadSpeed";
const ORDER_OPTIONS: { k: OrderBy; t: string }[] = [
  { k: "time", t: "时间" },
  { k: "upload", t: "上传量" },
  { k: "download", t: "下载量" },
  { k: "uploadSpeed", t: "上传速度" },
  { k: "downloadSpeed", t: "下载速度" },
];

// 表格列（对齐 clash-party CONNECTION_TABLE_COLUMNS）
const TABLE_COLUMNS: { key: string; t: string }[] = [
  { key: "host", t: "主机" },
  { key: "type", t: "类型" },
  { key: "rule", t: "规则" },
  { key: "chains", t: "代理链" },
  { key: "process", t: "进程" },
  { key: "upload", t: "上传" },
  { key: "download", t: "下载" },
  { key: "uploadSpeed", t: "上传速度" },
  { key: "downloadSpeed", t: "下载速度" },
  { key: "sourceIP", t: "来源 IP" },
  { key: "destinationIP", t: "目标 IP" },
  { key: "start", t: "时间" },
];

function timeAgo(iso: string): string {
  const s = Math.max(0, Math.floor((Date.now() - new Date(iso).getTime()) / 1000));
  if (s < 60) return `${s} 秒前`;
  if (s < 3600) return `${Math.floor(s / 60)} 分钟前`;
  if (s < 86400) return `${Math.floor(s / 3600)} 小时前`;
  return `${Math.floor(s / 86400)} 天前`;
}

function connHost(c: IMihomoConnection): string {
  const m = c.metadata;
  return (m.host || m.sniffHost || m.destinationIP || "") + (m.destinationPort ? `:${m.destinationPort}` : "");
}

export default function ConnectionsPanel({ info, running }: { info: any; running: boolean }) {
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
    return <div className={`${cardCls} p-6 text-center text-xs text-slate-400`}>核心未运行</div>;
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
              {k === "active" ? `活动中 (${active.length})` : `已关闭 (${closed.length})`}
            </button>
          ))}
        </div>
        <div className="relative flex-1 min-w-40">
          <Search className="w-3.5 h-3.5 absolute left-2.5 top-1/2 -translate-y-1/2 text-slate-500" />
          <input
            className="w-full h-8 pl-8 pr-2.5 rounded-lg bg-white/5 border border-white/10 text-xs text-white placeholder-slate-500 focus:outline-none focus:border-[var(--module-accent)]"
            placeholder="筛选连接"
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
              {ORDER_OPTIONS.map((o) => <option key={o.k} value={o.k} className="bg-slate-800">{o.t}</option>)}
            </select>
            <button className={btnSec} title="排序方向" onClick={() => patchCfg({ connectionDirection: direction === "asc" ? "desc" : "asc" })}>
              {direction === "asc" ? <ArrowUp className="w-3.5 h-3.5" /> : <ArrowDown className="w-3.5 h-3.5" />}
            </button>
          </>
        )}
        <button className={btnSec} title={viewMode === "list" ? "切换表格视图" : "切换列表视图"} onClick={() => patchCfg({ connectionViewMode: viewMode === "list" ? "table" : "list" })}>
          {viewMode === "list" ? <Table2 className="w-3.5 h-3.5" /> : <List className="w-3.5 h-3.5" />}
        </button>
        <button className={btnSec} title={paused ? "恢复" : "暂停"} onClick={() => setPaused((p) => !p)}>
          {paused ? <Play className="w-3.5 h-3.5" /> : <Pause className="w-3.5 h-3.5" />}
        </button>
        <button className={btnSec} title={tab === "active" ? "关闭全部" : "清空记录"} onClick={closeAll}>
          {tab === "active" ? <X className="w-3.5 h-3.5" /> : <Trash2 className="w-3.5 h-3.5" />}
        </button>
      </div>

      <div className="text-[11px] text-slate-400 px-1">
        ↑ {calcTraffic(connInfo.uploadTotal)} &nbsp; ↓ {calcTraffic(connInfo.downloadTotal)} &nbsp;·&nbsp; {filtered.length} 条
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
                  {(c.uploadSpeed || c.downloadSpeed) ? `↑ ${calcTraffic(c.uploadSpeed || 0)}/s ↓ ${calcTraffic(c.downloadSpeed || 0)}/s` : timeAgo(c.start)}
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
          {filtered.length === 0 && <div className={`${cardCls} p-6 text-center text-xs text-slate-400`}>暂无连接</div>}
        </div>
      ) : (
        <div className={`${cardCls} overflow-auto max-h-[60vh]`}>
          <table className="w-full text-[11px] whitespace-nowrap">
            <thead className="sticky top-0 bg-[#11151f] z-10">
              <tr className="text-slate-400 text-left">
                <th className="px-2 py-2 font-medium"></th>
                {TABLE_COLUMNS.map((c) => <th key={c.key} className="px-2 py-2 font-medium">{c.t}</th>)}
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
                  <td className="px-2 py-1.5">{timeAgo(c.start)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {filtered.length === 0 && <div className="p-6 text-center text-xs text-slate-400">暂无连接</div>}
        </div>
      )}

      {/* 详情弹窗（复刻 connection-detail-modal：展示全部字段） */}
      {detail && (
        <Modal title="连接详情" onClose={() => setDetail(null)}>
          <div className="space-y-1 text-[11px]">
            {[
              ["ID", detail.id],
              ["主机", connHost(detail)],
              ["下载", calcTraffic(detail.download)],
              ["上传", calcTraffic(detail.upload)],
              ["下载速度", `${calcTraffic(detail.downloadSpeed || 0)}/s`],
              ["上传速度", `${calcTraffic(detail.uploadSpeed || 0)}/s`],
              ["连接建立时间", timeAgo(detail.start)],
              ["规则", `${detail.rule}${detail.rulePayload ? `(${detail.rulePayload})` : ""}`],
              ["代理链", (detail.chains || []).slice().reverse().join(" → ")],
              ["连接类型", `${detail.metadata.type}(${detail.metadata.network})`],
              ["来源", `${detail.metadata.sourceIP}:${detail.metadata.sourcePort}`],
              ["目标", `${detail.metadata.destinationIP || detail.metadata.host}:${detail.metadata.destinationPort}`],
              ["嗅探域名", detail.metadata.sniffHost || "-"],
              ["进程", detail.metadata.process || "-"],
              ["进程路径", detail.metadata.processPath || "-"],
              ["远端目标", detail.metadata.remoteDestination || "-"],
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
