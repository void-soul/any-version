// 网络信息页 —— 1:1 复刻 clash-party src/renderer/src/pages/network.tsx
// （出口 IP 直连/经代理 · 延迟测试 · 网络拓扑 · 网卡枚举）
// 视觉主题改用 any-version 的 emerald/glass 风格；拓扑改为无依赖的可折叠树（功能等价）
import { useEffect, useRef, useState, useMemo, useCallback } from "react";
import {
  RefreshCw,
  Copy,
  Eye,
  EyeOff,
  Plus,
  X,
  Globe,
  Activity,
  Network,
  Router,
  Laptop,
  Cloud,
  Pause,
  Play,
} from "lucide-react";
import { mihomoApi } from "../mihomoApi";
import { cardCls, btnSec, btnPrimary, inputCls } from "./ui";

// ─── 常量 ────────────────────────────────────────────────────────────────────
const IP_PROVIDERS = [
  { key: "ip.sb", label: "IP.SB", url: "https://ip.sb" },
  { key: "ipwho.is", label: "ipwho.is", url: "https://ipwho.is" },
  { key: "ipapi.is", label: "ipapi.is", url: "https://ipapi.is" },
];

const DEFAULT_LATENCY_TARGETS = [
  { name: "Google", url: "https://www.google.com/generate_204" },
  { name: "GitHub", url: "https://github.com" },
  { name: "Cloudflare", url: "https://www.cloudflare.com" },
  { name: "YouTube", url: "https://www.youtube.com" },
  { name: "Bing", url: "https://www.bing.com" },
  { name: "Baidu", url: "https://www.baidu.com" },
];

const CONN_REFRESH_MS = 1500;

// ─── 工具 ────────────────────────────────────────────────────────────────────
function fmtTraffic(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function getConciseIp(info: any): string {
  if (!info) return "";
  const parts = [info.country, info.region, info.city, info.organization].filter(
    (p) => p !== undefined && p !== "" && p != null
  );
  return parts.length > 0 ? parts.join(" / ") : info.ip ?? "";
}

// ─── 拓扑层级构建（复刻 clash-party buildHierarchy）────────────────────────────
type TopoType = "root" | "group" | "proxy" | "rule" | "client" | "port";
interface TopoNode {
  id: string;
  name: string;
  type: TopoType;
  connections: number;
  traffic: number;
  children?: TopoNode[];
}
interface ConnMeta {
  sourceIP?: string;
  sourcePort?: number | string;
  [k: string]: any;
}

function buildHierarchy(conns: any[]): TopoNode {
  const groupsMap = new Map<
    string,
    { data: TopoNode; proxies: Map<string, { data: TopoNode; rules: Map<string, { data: TopoNode; clients: Map<string, { data: TopoNode; ports: Map<string, TopoNode> }> }> }> }
  >();

  for (const conn of conns) {
    const meta: ConnMeta = conn.metadata || {};
    const clientIP = meta.sourceIP || "Unknown";
    const sourcePort = String(meta.sourcePort ?? "Unknown");
    const ruleType = conn.rule || "Direct";
    const fullRule = conn.rulePayload ? `${ruleType}: ${conn.rulePayload}` : ruleType;
    const chains: string[] = conn.chains || [];
    const proxy = chains[0] ?? "Direct";
    const group = chains.length > 1 ? (chains[1] ?? "Direct") : (chains[0] ?? "Direct");
    const traffic = (conn.download || 0) + (conn.upload || 0);

    if (!groupsMap.has(group)) {
      groupsMap.set(group, {
        data: { id: `group-${group}`, name: group, type: "group", connections: 0, traffic: 0 },
        proxies: new Map(),
      });
    }
    const groupEntry = groupsMap.get(group)!;
    groupEntry.data.connections++;
    groupEntry.data.traffic += traffic;

    if (!groupEntry.proxies.has(proxy)) {
      groupEntry.proxies.set(proxy, {
        data: { id: `proxy-${group}-${proxy}`, name: proxy, type: "proxy", connections: 0, traffic: 0 },
        rules: new Map(),
      });
    }
    const proxyEntry = groupEntry.proxies.get(proxy)!;
    proxyEntry.data.connections++;
    proxyEntry.data.traffic += traffic;

    if (!proxyEntry.rules.has(fullRule)) {
      proxyEntry.rules.set(fullRule, {
        data: { id: `rule-${group}-${proxy}-${fullRule}`, name: fullRule, type: "rule", connections: 0, traffic: 0 },
        clients: new Map(),
      });
    }
    const ruleEntry = proxyEntry.rules.get(fullRule)!;
    ruleEntry.data.connections++;
    ruleEntry.data.traffic += traffic;

    if (!ruleEntry.clients.has(clientIP)) {
      ruleEntry.clients.set(clientIP, {
        data: { id: `client-${group}-${proxy}-${fullRule}-${clientIP}`, name: clientIP, type: "client", connections: 0, traffic: 0 },
        ports: new Map(),
      });
    }
    const clientEntry = ruleEntry.clients.get(clientIP)!;
    clientEntry.data.connections++;
    clientEntry.data.traffic += traffic;

    if (!clientEntry.ports.has(sourcePort)) {
      clientEntry.ports.set(sourcePort, {
        id: `port-${group}-${proxy}-${fullRule}-${clientIP}-${sourcePort}`,
        name: sourcePort,
        type: "port",
        connections: 0,
        traffic: 0,
      });
    }
    const portNode = clientEntry.ports.get(sourcePort)!;
    portNode.connections++;
    portNode.traffic += traffic;
  }

  const rootChildren: TopoNode[] = [];
  groupsMap.forEach((ge) => {
    const groupChildren: TopoNode[] = [];
    const groupNode: TopoNode = { ...ge.data, children: groupChildren };
    ge.proxies.forEach((pe) => {
      const proxyChildren: TopoNode[] = [];
      const proxyNode: TopoNode = { ...pe.data, children: proxyChildren };
      pe.rules.forEach((re) => {
        const ruleChildren: TopoNode[] = [];
        const ruleNode: TopoNode = { ...re.data, children: ruleChildren };
        re.clients.forEach((ce) => {
          const portChildren = Array.from(ce.ports.values());
          ruleChildren.push({ ...ce.data, children: portChildren });
        });
        proxyChildren.push(ruleNode);
      });
      groupChildren.push(proxyNode);
    });
    rootChildren.push(groupNode);
  });

  return {
    id: "root",
    name: "Connections",
    type: "root",
    connections: conns.length,
    traffic: conns.reduce((s, c) => s + (c.download || 0) + (c.upload || 0), 0),
    children: rootChildren,
  };
}

const TYPE_COLORS: Record<TopoType, string> = {
  root: "text-slate-300",
  group: "text-emerald-300",
  proxy: "text-rose-300",
  rule: "text-sky-300",
  client: "text-amber-300",
  port: "text-slate-400",
};
const TYPE_DOT: Record<TopoType, string> = {
  root: "bg-slate-400",
  group: "bg-emerald-400",
  proxy: "bg-rose-400",
  rule: "bg-sky-400",
  client: "bg-amber-400",
  port: "bg-slate-500",
};

// ─── 拓扑树渲染（可折叠，无 D3）─────────────────────────────────────────────────
const TopoNodeRow: React.FC<{
  node: TopoNode;
  depth: number;
  collapsed: Set<string>;
  toggle: (id: string) => void;
}> = ({ node, depth, collapsed, toggle }) => {
  const hasChildren = !!node.children && node.children.length > 0;
  const isCollapsed = collapsed.has(node.id);
  const showChildren = hasChildren && !isCollapsed;
  return (
    <div>
      <div
        className={`flex items-center gap-2 py-1 pr-2 rounded-md hover:bg-white/5 ${
          hasChildren ? "cursor-pointer" : ""
        }`}
        style={{ paddingLeft: depth * 18 + 4 }}
        onClick={() => hasChildren && toggle(node.id)}
      >
        <span className={`inline-block w-2 h-2 rounded-full shrink-0 ${TYPE_DOT[node.type]}`} />
        <span className={`text-[12px] font-semibold ${TYPE_COLORS[node.type]}`}>{node.name}</span>
        <span className="text-[10px] text-slate-500 bg-white/5 rounded px-1.5 py-0.5">
          {node.connections} 连接
        </span>
        {node.traffic > 0 && (
          <span className="text-[10px] text-slate-500">{fmtTraffic(node.traffic)}</span>
        )}
        {hasChildren && (
          <span className="text-[10px] text-slate-500 ml-auto">{isCollapsed ? "▶" : "▼"}</span>
        )}
      </div>
      {showChildren &&
        node.children!.map((c) => (
          <TopoNodeRow key={c.id} node={c} depth={depth + 1} collapsed={collapsed} toggle={toggle} />
        ))}
    </div>
  );
};

// ─── 主面板 ───────────────────────────────────────────────────────────────────
export default function NetworkPanel() {
  // 出口 IP
  const [ipInfos, setIpInfos] = useState<Record<string, any>>({});
  const [loadingIp, setLoadingIp] = useState(false);
  const [hideIp, setHideIp] = useState(false);

  // 延迟测试
  const [latencyTargets, setLatencyTargets] = useState(DEFAULT_LATENCY_TARGETS);
  const [latencyResults, setLatencyResults] = useState<Record<string, number | null>>({});
  const [testingLatency, setTestingLatency] = useState(false);

  // 拓扑
  const [connections, setConnections] = useState<any[]>([]);
  const [isPaused, setIsPaused] = useState(false);
  const frozenRef = useRef<any[] | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  // 网卡
  const [interfaces, setInterfaces] = useState<any[]>([]);
  const [loadingItf, setLoadingItf] = useState(false);

  const refreshIp = useCallback(async () => {
    setLoadingIp(true);
    // 直连与经代理两路并发（此前串行，最坏要等两倍超时）
    const entries = await Promise.all(
      IP_PROVIDERS.map(async (p) => {
        const [direct, proxied] = await Promise.all([
          mihomoApi.fetchIpInfo(p.url, false, 6000).catch((e: any) => ({ __err: String(e) })),
          mihomoApi.fetchIpInfo(p.url, true, 6000).catch((e: any) => ({ __err: String(e) })),
        ]);
        const bad = (direct as any)?.__err && (proxied as any)?.__err;
        return [p.key, bad ? { error: (direct as any).__err } : { direct, proxied }] as const;
      })
    );
    setIpInfos(Object.fromEntries(entries));
    setLoadingIp(false);
  }, []);

  const refreshInterfaces = useCallback(async (force = false) => {
    setLoadingItf(true);
    try {
      const r: any = await mihomoApi.getInterfaces(force);
      setInterfaces(Array.isArray(r?.items) ? r.items : []);
    } catch {
    } finally {
      setLoadingItf(false);
    }
  }, []);

  // 加载配置（延迟目标 + 隐藏 IP 偏好）
  useEffect(() => {
    (async () => {
      try {
        const cfg: any = await mihomoApi.getAppConfig();
        const extra = cfg?.extra ?? {};
        if (Array.isArray(extra.networkLatencyTargets) && extra.networkLatencyTargets.length)
          setLatencyTargets(extra.networkLatencyTargets);
        if (typeof extra.networkHideIp === "boolean") setHideIp(extra.networkHideIp);
      } catch {}
    })();
    refreshIp();
    refreshInterfaces();
  }, [refreshIp, refreshInterfaces]);

  // 连接轮询
  useEffect(() => {
    if (isPaused) return;
    let alive = true;
    const tick = async () => {
      try {
        const info: any = await mihomoApi.getConnections();
        if (!alive) return;
        setConnections(info?.connections ?? []);
      } catch {}
    };
    tick();
    const id = setInterval(tick, CONN_REFRESH_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [isPaused]);

  const currentConnections = isPaused && frozenRef.current ? frozenRef.current : connections;

  const stats = useMemo(() => {
    const clients = new Set<string>();
    const rules = new Set<string>();
    const groups = new Set<string>();
    const proxies = new Set<string>();
    for (const c of currentConnections) {
      clients.add(c.metadata?.sourceIP || "Unknown");
      rules.add(c.rule || "Direct");
      const ch = c.chains || [];
      proxies.add(ch[0] ?? "Direct");
      groups.add(ch.length > 1 ? (ch[1] ?? "Direct") : (ch[0] ?? "Direct"));
    }
    return {
      clientCount: clients.size,
      ruleCount: rules.size,
      groupCount: groups.size,
      proxyCount: proxies.size,
      totalTraffic: currentConnections.reduce((s, c) => s + (c.download || 0) + (c.upload || 0), 0),
    };
  }, [currentConnections]);

  const hierarchy = useMemo(() => buildHierarchy(currentConnections), [currentConnections]);

  const toggleCollapse = useCallback((id: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const togglePause = useCallback(() => {
    if (isPaused) frozenRef.current = null;
    else frozenRef.current = [...connections];
    setIsPaused((p) => !p);
  }, [isPaused, connections]);

  // 延迟测试
  const runLatency = useCallback(async () => {
    setTestingLatency(true);
    const res: Record<string, number | null> = {};
    await Promise.all(
      latencyTargets.map(async (t) => {
        try {
          res[t.name] = await mihomoApi.measureLatency(t.url, true);
        } catch {
          res[t.name] = null;
        }
      })
    );
    setLatencyResults(res);
    setTestingLatency(false);
  }, [latencyTargets]);

  // 延迟目标编辑
  const [targetDraft, setTargetDraft] = useState({ name: "", url: "" });
  const updateTarget = (value: string, index: number, field: "name" | "url") => {
    const arr = [...latencyTargets];
    if (index < arr.length) {
      arr[index] = { ...arr[index], [field]: value };
      setLatencyTargets(arr);
    }
  };
  const removeTarget = (index: number) => {
    const arr = latencyTargets.filter((_, i) => i !== index);
    setLatencyTargets(arr);
    persistTargets(arr);
    setLatencyResults({});
  };
  const addTarget = () => {
    if (!targetDraft.name.trim() || !targetDraft.url.trim()) return;
    const arr = [...latencyTargets, { ...targetDraft }];
    setLatencyTargets(arr);
    persistTargets(arr);
    setTargetDraft({ name: "", url: "" });
    setLatencyResults({});
  };
  const persistTargets = (arr: typeof latencyTargets) => {
    mihomoApi.patchAppConfig({ extra: { networkLatencyTargets: arr } }).catch(() => {});
  };

  const copy = (text: string) => {
    navigator.clipboard?.writeText(text).catch(() => {});
  };

  const renderIpCard = (type: "direct" | "proxy") => (
    <div className={`${cardCls} p-3 flex-1 min-w-[220px]`}>
      <div className="flex items-center justify-between mb-2">
        <span className="text-[12px] font-semibold text-slate-300 flex items-center gap-1.5">
          {type === "direct" ? (
            <Laptop className="w-3.5 h-3.5 text-amber-300" />
          ) : (
            <Cloud className="w-3.5 h-3.5 text-emerald-300" />
          )}
          {type === "direct" ? "直连接口" : "经代理出口"}
        </span>
        <span className="text-[10px] text-slate-500">{type === "direct" ? "绕过代理" : "通过代理"}</span>
      </div>
      {IP_PROVIDERS.map((p) => {
        const info = ipInfos[p.key]?.[type];
        const err = ipInfos[p.key]?.error;
        return (
          <div key={p.key} className="mb-2 last:mb-0">
            <div className="flex items-center justify-between">
              <span className="text-[11px] text-slate-400">{p.label}</span>
              {info?.ip && (
                <button onClick={() => copy(info.ip)} title="复制 IP">
                  <Copy className="w-3 h-3 text-slate-500 hover:text-slate-200" />
                </button>
              )}
            </div>
            {err ? (
              <div className="text-[11px] text-rose-300/80">{err}</div>
            ) : info ? (
              <div className="text-[12px] text-white font-mono">
                {hideIp ? "•••.•••.•••.•••" : info.ip}
                {!hideIp && getConciseIp(info) && (
                  <span className="text-slate-400 ml-1 font-sans">· {getConciseIp(info)}</span>
                )}
              </div>
            ) : (
              <div className="text-[11px] text-slate-500">查询中…</div>
            )}
          </div>
        );
      })}
    </div>
  );

  return (
    <div className="space-y-3">
      {/* 出口 IP */}
      <div className={`${cardCls} p-4`}>
        <div className="flex items-center justify-between mb-3">
          <h3 className="text-sm font-bold text-white flex items-center gap-2">
            <Globe className="w-4 h-4 text-emerald-400" />
            出口 IP 信息
          </h3>
          <div className="flex items-center gap-2">
            <button
              className={btnSec}
              onClick={() => {
                setHideIp((h) => {
                  const v = !h;
                  mihomoApi.patchAppConfig({ extra: { networkHideIp: v } }).catch(() => {});
                  return v;
                });
              }}
              title={hideIp ? "显示 IP" : "隐藏 IP"}
            >
              {hideIp ? <EyeOff className="w-3.5 h-3.5" /> : <Eye className="w-3.5 h-3.5" />}
            </button>
            <button className={btnPrimary} onClick={refreshIp} disabled={loadingIp}>
              <RefreshCw className={`w-3.5 h-3.5 ${loadingIp ? "animate-spin" : ""}`} />
              刷新
            </button>
          </div>
        </div>
        <div className="flex flex-wrap gap-3">
          {renderIpCard("direct")}
          {renderIpCard("proxy")}
        </div>
      </div>

      {/* 延迟测试 */}
      <div className={`${cardCls} p-4`}>
        <div className="flex items-center justify-between mb-3">
          <h3 className="text-sm font-bold text-white flex items-center gap-2">
            <Activity className="w-4 h-4 text-emerald-400" />
            延迟测试
          </h3>
          <button className={btnPrimary} onClick={runLatency} disabled={testingLatency}>
            <RefreshCw className={`w-3.5 h-3.5 ${testingLatency ? "animate-spin" : ""}`} />
            测试
          </button>
        </div>
        <div className="space-y-1.5">
          {latencyTargets.map((t, i) => (
            <div key={i} className="flex items-center gap-2">
              <input
                className={`${inputCls} !w-24`}
                value={t.name}
                onChange={(e) => updateTarget(e.target.value, i, "name")}
                onBlur={() => persistTargets(latencyTargets)}
              />
              <input
                className={`${inputCls} flex-1`}
                value={t.url}
                onChange={(e) => updateTarget(e.target.value, i, "url")}
                onBlur={() => persistTargets(latencyTargets)}
              />
              <span
                className={`text-[12px] font-mono ${
                  latencyResults[t.name] == null
                    ? "text-slate-500"
                    : latencyResults[t.name]! < 300
                    ? "text-emerald-300"
                    : latencyResults[t.name]! < 800
                    ? "text-amber-300"
                    : "text-rose-300"
                }`}
              >
                {latencyResults[t.name] == null ? "—" : `${latencyResults[t.name]} ms`}
              </span>
              <button className={btnSec} onClick={() => removeTarget(i)}>
                <X className="w-3.5 h-3.5 text-amber-300" />
              </button>
            </div>
          ))}
          <div className="flex items-center gap-2 pt-1">
            <input
              className={`${inputCls} !w-28`}
              placeholder="名称"
              value={targetDraft.name}
              onChange={(e) => setTargetDraft((d) => ({ ...d, name: e.target.value }))}
            />
            <input
              className={`${inputCls} flex-1`}
              placeholder="https://..."
              value={targetDraft.url}
              onChange={(e) => setTargetDraft((d) => ({ ...d, url: e.target.value }))}
            />
            <button className={btnSec} onClick={addTarget}>
              <Plus className="w-3.5 h-3.5" />
            </button>
          </div>
        </div>
      </div>

      {/* 网络拓扑 */}
      <div className={`${cardCls} p-4`}>
        <div className="flex items-center justify-between mb-3">
          <h3 className="text-sm font-bold text-white flex items-center gap-2">
            <Network className="w-4 h-4 text-emerald-400" />
            网络拓扑
          </h3>
          <div className="flex items-center gap-3">
            <div className="flex flex-wrap gap-x-2 text-[11px] text-slate-500">
              <span>{stats.clientCount} 客户端</span>·<span>{stats.ruleCount} 规则</span>·<span>
                {stats.groupCount} 代理组
              </span>·<span>{stats.proxyCount} 节点</span>·<span>{fmtTraffic(stats.totalTraffic)}</span>
            </div>
            <button className={btnSec} onClick={togglePause} title={isPaused ? "继续" : "暂停"}>
              {isPaused ? <Play className="w-3.5 h-3.5" /> : <Pause className="w-3.5 h-3.5" />}
            </button>
          </div>
        </div>
        <div className="flex flex-wrap gap-3 text-[11px] text-slate-400 mb-2">
          <span className="flex items-center gap-1">
            <span className="w-2 h-2 rounded-full bg-emerald-400 inline-block" />
            代理组
          </span>
          <span className="flex items-center gap-1">
            <span className="w-2 h-2 rounded-full bg-rose-400 inline-block" />
            代理节点
          </span>
          <span className="flex items-center gap-1">
            <span className="w-2 h-2 rounded-full bg-sky-400 inline-block" />
            规则
          </span>
          <span className="flex items-center gap-1">
            <span className="w-2 h-2 rounded-full bg-amber-400 inline-block" />
            源 IP
          </span>
          <span className="flex items-center gap-1">
            <span className="w-2 h-2 rounded-full bg-slate-500 inline-block" />
            源端口
          </span>
        </div>
        {currentConnections.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-10 text-slate-500">
            <Network className="w-8 h-8 mb-2 animate-pulse" />
            <span className="text-sm">等待连接数据…</span>
          </div>
        ) : (
          <div className="overflow-x-auto max-h-[420px] overflow-y-auto">
            {hierarchy.children!.map((c) => (
              <TopoNodeRow key={c.id} node={c} depth={0} collapsed={collapsed} toggle={toggleCollapse} />
            ))}
          </div>
        )}
      </div>

      {/* 网卡信息 */}
      <div className={`${cardCls} p-4`}>
        <div className="flex items-center justify-between mb-3">
          <h3 className="text-sm font-bold text-white flex items-center gap-2">
            <Router className="w-4 h-4 text-emerald-400" />
            网络接口
          </h3>
          <button className={btnSec} onClick={() => refreshInterfaces(true)} disabled={loadingItf}>
            <RefreshCw className={`w-3.5 h-3.5 ${loadingItf ? "animate-spin" : ""}`} />
            刷新
          </button>
        </div>
        {interfaces.length === 0 ? (
          <div className="text-[12px] text-slate-500">{loadingItf ? "正在枚举网络接口…" : "未检测到网络接口"}</div>
        ) : (
          <div className="space-y-2">
            {interfaces.map((itf, i) => (
              <div key={i} className="rounded-lg bg-white/5 border border-white/10 p-3">
                <div className="flex items-center justify-between mb-1.5">
                  <span className="text-[13px] font-semibold text-white">{itf.name}</span>
                  <span
                    className={`text-[10px] px-1.5 py-0.5 rounded ${
                      itf.status === "Up"
                        ? "bg-emerald-500/20 text-emerald-300"
                        : "bg-slate-500/20 text-slate-400"
                    }`}
                  >
                    {itf.status}
                  </span>
                </div>
                <div className="text-[11px] text-slate-400 mb-1.5">{itf.description}</div>
                <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-[11px]">
                  <span className="text-slate-500">MAC</span>
                  <span className="text-slate-300 font-mono">{itf.mac || "—"}</span>
                  <span className="text-slate-500">速率</span>
                  <span className="text-slate-300">{itf.speed || "—"}</span>
                  <span className="text-slate-500">IPv4</span>
                  <span className="text-slate-300 font-mono">
                    {(itf.ipv4 || []).map((x: any) => x.address).join(", ") || "—"}
                  </span>
                  <span className="text-slate-500">IPv6</span>
                  <span className="text-slate-300 font-mono break-all">
                    {(itf.ipv6 || []).map((x: any) => x.address).join(", ") || "—"}
                  </span>
                  <span className="text-slate-500">网关</span>
                  <span className="text-slate-300 font-mono">{itf.gateway || "—"}</span>
                  <span className="text-slate-500">DNS</span>
                  <span className="text-slate-300 font-mono">{(itf.dns || []).join(", ") || "—"}</span>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
