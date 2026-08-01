// mihomo 控制器 API 层 —— 1:1 复刻 clash-party src/main/core/mihomoApi.ts 的行为
import { mihomoApi } from "../mihomoApi";

// ---------- 类型（对齐 clash-party types.d.ts） ----------
export interface IMihomoProxy {
  name: string;
  type: string;
  udp: boolean;
  xudp?: boolean;
  tfo?: boolean;
  history: { time: string; delay: number }[];
  alive?: boolean;
  provider?: string;
  [k: string]: any;
}
export interface IMihomoGroup extends IMihomoProxy {
  all: string[];
  now: string;
  hidden?: boolean;
  icon?: string;
  fixed?: string;
  testUrl?: string;
  expectedStatus?: string;
}
export interface IMihomoMixedGroup extends Omit<IMihomoGroup, "all"> {
  all: IMihomoProxy[];
}
export interface IMihomoConnection {
  id: string;
  metadata: {
    network: string;
    type: string;
    sourceIP: string;
    destinationIP: string;
    sourcePort: string;
    destinationPort: string;
    host: string;
    process?: string;
    processPath?: string;
    sniffHost?: string;
    remoteDestination?: string;
    [k: string]: any;
  };
  upload: number;
  download: number;
  start: string;
  chains: string[];
  rule: string;
  rulePayload: string;
  // 前端补充（速率，对齐 clash-party connections 页）
  uploadSpeed?: number;
  downloadSpeed?: number;
  isActive?: boolean;
}
export interface IMihomoRule {
  type: string;
  payload: string;
  proxy: string;
  size?: number;
}
export interface IMihomoLog {
  type: "info" | "warning" | "error" | "debug";
  payload: string;
  time?: string;
}

// ---------- 基础 REST（走 Rust mihomo_api 透传） ----------
export const ctrlGet = (path: string) => mihomoApi.api("GET", path);
export const ctrlPut = (path: string, body?: any) => mihomoApi.api("PUT", path, body);
export const ctrlPost = (path: string, body?: any) => mihomoApi.api("POST", path, body);
export const ctrlDelete = (path: string) => mihomoApi.api("DELETE", path);
export const ctrlPatch = (path: string, body?: any) => mihomoApi.api("PATCH", path, body);

// ---------- 代理组（复刻 mihomoGroups：runtime 顺序 + provider 兜底 + GLOBAL） ----------
// mihomo 内核返回的代理组 type 是「首字母大写驼峰」：Selector / URLTest / Fallback / LoadBalance / Relay
// （之前写成小写导致过滤恒为空 → 始终走「全部节点」假组 → 切换时 404）
const GROUP_TYPE_SET = new Set(["selector", "urltest", "fallback", "loadbalance", "relay"]);
const normType = (t: string) => (t || "").toLowerCase().replace(/[-_]/g, "");
export const isGroupType = (t?: string) => !!t && GROUP_TYPE_SET.has(normType(t));
// 「全部节点」假组（内核中不存在，仅用于展示无所属组的独立节点）；切换时跳过
export const FAKE_GROUP_TYPE = "all";
export const ALL_NODES_GROUP = "全部节点";

export async function getMixedGroups(): Promise<IMihomoMixedGroup[]> {
  const [proxiesRes, providersRes] = await Promise.all([
    ctrlGet("/proxies"),
    ctrlGet("/providers/proxies"),
  ]);
  const proxies: Record<string, any> = proxiesRes?.proxies || {};
  const providers: Record<string, any> = providersRes?.providers || {};

  // provider 内的节点兜底解析（proxies 表里查不到时）
  const resolveProxy = (name: string): IMihomoProxy | undefined => {
    const p = proxies[name];
    if (p && !isGroupType(p.type)) return p as IMihomoProxy;
    for (const pk in providers) {
      const found = (providers[pk]?.proxies || []).find((p: any) => p.name === name);
      if (found) return { ...found, provider: pk };
    }
    return undefined;
  };

  const global = proxies["GLOBAL"] as IMihomoGroup | undefined;

  // 根组：优先 GLOBAL 嵌套的代理组；否则取配置里所有代理组类型
  let rootNames: string[];
  if (global && global.all?.length) {
    rootNames = global.all.filter((n) => proxies[n] && isGroupType(proxies[n].type));
  } else {
    rootNames = Object.keys(proxies).filter(
      (n) => proxies[n] && isGroupType(proxies[n].type)
    );
  }

  const groups: IMihomoMixedGroup[] = [];
  for (const name of rootNames) {
    const g = proxies[name] as IMihomoGroup;
    if (g.hidden) continue;
    const all = (g.all || [])
      .map((n) => resolveProxy(n))
      .filter(Boolean) as IMihomoProxy[];
    groups.push({ ...g, all });
  }

  // 兜底：若没有任何代理组，展示全部独立节点（避免「代理加载不出来」）
  if (groups.length === 0) {
    const loose = Object.keys(proxies)
      .filter((n) => proxies[n] && !isGroupType(proxies[n].type))
      .map((n) => proxies[n] as IMihomoProxy);
    if (loose.length) {
      groups.push({
        name: ALL_NODES_GROUP,
        type: FAKE_GROUP_TYPE,
        now: "",
        hidden: false,
        all: loose,
      } as unknown as IMihomoMixedGroup);
    }
  }
  return groups;
}

// 全部独立节点（含 provider 节点），用于「全部节点」Tab 兜底
export async function getAllProxies(): Promise<IMihomoProxy[]> {
  const [proxiesRes, providersRes] = await Promise.all([
    ctrlGet("/proxies"),
    ctrlGet("/providers/proxies"),
  ]);
  const proxies: Record<string, any> = proxiesRes?.proxies || {};
  const providers: Record<string, any> = providersRes?.providers || {};
  const list: IMihomoProxy[] = Object.values(proxies)
    .filter((p: any) => p && !isGroupType(p.type))
    .map((p: any) => p as IMihomoProxy);
  for (const pk in providers) {
    for (const p of providers[pk]?.proxies || []) {
      list.push({ ...p, provider: pk });
    }
  }
  return list;
}

// 最近一次延迟（对齐 clash-party：history 最后一条；-1=未测）
export function lastDelay(p: IMihomoProxy | undefined): number {
  if (!p) return -1;
  const h = p.history || [];
  if (!h.length) return -1;
  return h[h.length - 1].delay;
}

// ---------- 节点操作 ----------
// 复刻 mihomoChangeProxy：PUT /proxies/:group {name}
// 委托 mihomoApi.changeProxy，避免与 mihomoApi.ts 中重复实现发散（二者 URL 编码须一致）
export const changeProxy = (group: string, proxy: string) =>
  mihomoApi.changeProxy(group, proxy);
// 复刻 mihomoUnfixedProxy：DELETE /proxies/:group（取消 URLTest/Fallback 固定）
export const unfixedProxy = (group: string) => ctrlDelete(`/proxies/${encodeURIComponent(group)}`);

// 单节点延迟（复刻 mihomoProxyDelay）
export async function proxyDelay(name: string, url: string, timeout: number): Promise<number> {
  try {
    const r = await ctrlGet(
      `/proxies/${encodeURIComponent(name)}/delay?url=${encodeURIComponent(url)}&timeout=${timeout}`
    );
    // 与 lastDelay() 保持一致：测速缺失/失败返回 -1（表示未测速），避免与真实 0ms 混淆
    const d = r?.delay;
    return typeof d === "number" ? d : -1;
  } catch {
    return -1;
  }
}
// 组延迟（复刻 mihomoGroupDelay：核心侧并发）
export async function groupDelay(name: string, url: string, timeout: number): Promise<void> {
  try {
    await ctrlGet(
      `/group/${encodeURIComponent(name)}/delay?url=${encodeURIComponent(url)}&timeout=${timeout}`
    );
  } catch {
    /* 组内全部超时会返回错误，忽略 */
  }
}

// 并发池（复刻 clash-party 渲染端搜索态分批测延迟，concurrency 默认 50）
export async function pooledDelayTest(names: string[], url: string, timeout: number, concurrency = 50) {
  const queue = [...names];
  const workers = Array.from({ length: Math.max(1, Math.min(concurrency, queue.length)) }, async () => {
    while (queue.length) {
      const n = queue.shift();
      if (!n) break;
      await proxyDelay(n, url, timeout);
    }
  });
  await Promise.all(workers);
}

// ---------- Provider ----------
export const proxyProviders = async (): Promise<Record<string, any>> =>
  (await ctrlGet("/providers/proxies"))?.providers || {};
export const updateProxyProvider = (name: string) =>
  ctrlPut(`/providers/proxies/${encodeURIComponent(name)}`);
export const healthcheckProxyProvider = (name: string) =>
  ctrlGet(`/providers/proxies/${encodeURIComponent(name)}/healthcheck`);
export const ruleProviders = async (): Promise<Record<string, any>> =>
  (await ctrlGet("/providers/rules"))?.providers || {};
export const updateRuleProvider = (name: string) =>
  ctrlPut(`/providers/rules/${encodeURIComponent(name)}`);

// ---------- 规则 / 连接 ----------
export const getRules = async (): Promise<IMihomoRule[]> => (await ctrlGet("/rules"))?.rules || [];
export const closeConnection = (id: string) => ctrlDelete(`/connections/${id}`);
export const closeAllConnections = () => ctrlDelete("/connections");

// ---------- 运行时配置 patch（PATCH /configs） ----------
export const patchRuntimeConfigs = (patch: any) => ctrlPatch("/configs", patch);

// ---------- WebSocket（traffic / memory / logs / connections） ----------
export type WsHandle = { close: () => void };
export function openMihomoWs(
  info: { port: number; secret: string },
  path: "traffic" | "memory" | "connections" | string,
  onMessage: (data: any) => void,
  params: Record<string, string> = {}
): WsHandle {
  let ws: WebSocket | null = null;
  let closed = false;
  let retry: any = null;
  const qs = new URLSearchParams({ token: info.secret || "", ...params }).toString();
  const url = `ws://127.0.0.1:${info.port}/${path}?${qs}`;
  const connect = () => {
    if (closed) return;
    try {
      ws = new WebSocket(url);
      ws.onmessage = (e) => {
        try { onMessage(JSON.parse(e.data)); } catch { /* ignore */ }
      };
      ws.onclose = () => {
        if (!closed) retry = setTimeout(connect, 2000);
      };
      ws.onerror = () => { try { ws?.close(); } catch {} };
    } catch {
      if (!closed) retry = setTimeout(connect, 2000);
    }
  };
  connect();
  return {
    close: () => {
      closed = true;
      if (retry) clearTimeout(retry);
      try { ws?.close(); } catch {}
    },
  };
}
