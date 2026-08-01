// 流量统计存储 + 采样器 —— 1:1 复刻 clash-party
// renderer/utils/db.ts + hooks/use-traffic-logger.ts + utils/dataUsage.ts
import { IMihomoConnection, openMihomoWs, WsHandle } from "./ctrl";

export interface DataUsageLog {
  timestamp: number;
  sourceIP: string;
  host: string;
  process: string;
  outbound: string;
  upload: number;
  download: number;
}

export type DataUsageType = "sourceIP" | "host" | "outbound" | "process";

export interface AggregatedData {
  label: string;
  upload: number;
  download: number;
  total: number;
  count: number;
}

// ---------- IndexedDB（复刻 db.ts：logs 表，timestamp 索引） ----------
const DB_NAME = "mihomo-traffic";
const STORE = "logs";
let dbPromise: Promise<IDBDatabase> | null = null;

function openDb(): Promise<IDBDatabase> {
  if (dbPromise) return dbPromise;
  dbPromise = new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, 1);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(STORE)) {
        const store = db.createObjectStore(STORE, { autoIncrement: true });
        store.createIndex("timestamp", "timestamp");
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => { dbPromise = null; reject(req.error); };
  });
  return dbPromise;
}

export const trafficDb = {
  async addLogs(logs: DataUsageLog[]): Promise<void> {
    if (!logs.length) return;
    const db = await openDb();
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE, "readwrite");
      const store = tx.objectStore(STORE);
      for (const l of logs) store.add(l);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
    });
  },
  // 复刻 cleanup：删除保留期外数据
  async cleanup(before: number): Promise<void> {
    const db = await openDb();
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE, "readwrite");
      const idx = tx.objectStore(STORE).index("timestamp");
      const req = idx.openCursor(IDBKeyRange.upperBound(before, true));
      req.onsuccess = () => {
        const cur = req.result;
        if (cur) { cur.delete(); cur.continue(); }
      };
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
    });
  },
  // 复刻 iterate：按时间范围遍历
  async iterate(start: number, end: number, cb: (log: DataUsageLog) => void): Promise<void> {
    const db = await openDb();
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE, "readonly");
      const idx = tx.objectStore(STORE).index("timestamp");
      const req = idx.openCursor(IDBKeyRange.bound(start, end));
      req.onsuccess = () => {
        const cur = req.result;
        if (cur) { cb(cur.value as DataUsageLog); cur.continue(); }
        else resolve();
      };
      req.onerror = () => reject(req.error);
    });
  },
  async clearAll(): Promise<void> {
    const db = await openDb();
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE, "readwrite");
      tx.objectStore(STORE).clear();
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
    });
  },
};

// ---------- 采样器（复刻 use-traffic-logger：连接 WS 增量采样，5s 批量落库，30 天保留） ----------
const FLUSH_DELAY_MS = 5000;
const RETENTION_MS = 30 * 24 * 60 * 60 * 1000;

let loggerWs: WsHandle | null = null;
let loggerKey = "";

export function startTrafficLogger(info: { port: number; secret: string }): void {
  const key = `${info.port}:${info.secret}`;
  if (loggerWs && loggerKey === key) return;
  stopTrafficLogger();
  loggerKey = key;

  const lastData = new Map<string, { upload: number; download: number }>();
  let buffer: DataUsageLog[] = [];
  let flushTimer: any = null;
  let lastTotals = { upload: 0, download: 0 };
  const enabledAt = Date.now();

  const flush = async () => {
    const toFlush = buffer;
    if (!toFlush.length) return;
    buffer = [];
    try {
      await trafficDb.addLogs(toFlush);
      await trafficDb.cleanup(Date.now() - RETENTION_MS);
    } catch (e) {
      console.error("[TrafficLogger] flush failed", e);
    }
  };
  const scheduleFlush = () => {
    if (flushTimer) return;
    flushTimer = setTimeout(() => { flushTimer = null; flush(); }, FLUSH_DELAY_MS);
  };

  loggerWs = openMihomoWs(info, "connections", (data) => {
    const uploadTotal = data.uploadTotal || 0;
    const downloadTotal = data.downloadTotal || 0;
    // 复刻：totals 变小说明核心重启，重置状态
    if (uploadTotal < lastTotals.upload || downloadTotal < lastTotals.download) {
      lastData.clear();
      buffer = [];
    }
    lastTotals = { upload: uploadTotal, download: downloadTotal };

    const conns: IMihomoConnection[] = data.connections || [];
    if (!conns.length) { lastData.clear(); return; }

    const now = Date.now();
    let hasDeltas = false;
    const activeIds = new Set<string>();
    for (const conn of conns) {
      activeIds.add(conn.id);
      const cu = conn.upload || 0;
      const cd = conn.download || 0;
      const last = lastData.get(conn.id);
      lastData.set(conn.id, { upload: cu, download: cd });
      const isNew = !last && Date.parse(conn.start) >= enabledAt;
      const du = last ? Math.max(0, cu - last.upload) : isNew ? cu : 0;
      const dd = last ? Math.max(0, cd - last.download) : isNew ? cd : 0;
      if (du === 0 && dd === 0) continue;
      hasDeltas = true;
      buffer.push({
        timestamp: now,
        sourceIP: conn.metadata.sourceIP || "Inner",
        host: conn.metadata.host || conn.metadata.destinationIP || "Unknown",
        process: conn.metadata.process || "Unknown",
        outbound: conn.chains?.[0] || "DIRECT",
        upload: du,
        download: dd,
      });
    }
    for (const id of Array.from(lastData.keys())) {
      if (!activeIds.has(id)) lastData.delete(id);
    }
    if (hasDeltas) scheduleFlush();
  });
}

export function stopTrafficLogger(): void {
  loggerWs?.close();
  loggerWs = null;
  loggerKey = "";
}

// ---------- 聚合（复刻 dataUsage.ts） ----------
function addAgg(map: Map<string, AggregatedData>, label: string, log: DataUsageLog): void {
  const e = map.get(label);
  if (e) {
    e.upload += log.upload;
    e.download += log.download;
    e.total += log.upload + log.download;
    e.count += 1;
    return;
  }
  map.set(label, { label, upload: log.upload, download: log.download, total: log.upload + log.download, count: 1 });
}
const sortAgg = (m: Map<string, AggregatedData>) => Array.from(m.values()).sort((a, b) => b.total - a.total);
const dim = (t: DataUsageType, l: DataUsageLog) =>
  t === "sourceIP" ? l.sourceIP : t === "host" ? l.host : t === "outbound" ? l.outbound : l.process;

export async function getTrafficOverview(type: DataUsageType, start: number, end: number, bucketSizeMs: number) {
  const rankings = new Map<string, AggregatedData>();
  const buckets = new Map<number, { upload: number; download: number }>();
  for (let time = start; time <= end; time += bucketSizeMs) {
    buckets.set(Math.floor(time / bucketSizeMs) * bucketSizeMs, { upload: 0, download: 0 });
  }
  await trafficDb.iterate(start, end, (log) => {
    addAgg(rankings, dim(type, log), log);
    const b = buckets.get(Math.floor(log.timestamp / bucketSizeMs) * bucketSizeMs);
    if (b) { b.upload += log.upload; b.download += log.download; }
  });
  return {
    rankings: sortAgg(rankings),
    trend: Array.from(buckets.entries()).map(([timestamp, d]) => ({ timestamp, ...d })).sort((a, b) => a.timestamp - b.timestamp),
  };
}

export async function getSubStatsByHost(dimension: Exclude<DataUsageType, "host">, label: string, start: number, end: number) {
  const map = new Map<string, AggregatedData>();
  await trafficDb.iterate(start, end, (log) => {
    if (dim(dimension, log) === label) addAgg(map, log.host, log);
  });
  return sortAgg(map);
}

export async function getDevicesByHost(host: string, start: number, end: number) {
  const map = new Map<string, AggregatedData>();
  await trafficDb.iterate(start, end, (log) => {
    if (log.host === host) addAgg(map, log.sourceIP, log);
  });
  return sortAgg(map);
}

export async function getProxyStatsByHost(dimension: DataUsageType, parentLabel: string, host: string, start: number, end: number) {
  const map = new Map<string, AggregatedData>();
  await trafficDb.iterate(start, end, (log) => {
    if (log.host === host && dim(dimension, log) === parentLabel) addAgg(map, log.outbound, log);
  });
  return sortAgg(map);
}
