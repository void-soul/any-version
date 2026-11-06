/**
 * 2API 请求日志的解析 / 归并 / 统计纯函数。
 *
 * 后端 `buddy2api_request_logs` 返回的是 `{ ts, source, message }` 纯文本行，
 * 一次请求会拆成多行（收到请求 → IN → 上游 → 用量 → 响应）。这里把它们归并成
 * 「一条请求一行」，供表格与图表消费。
 *
 * 设计约束：解析全部基于 message 文本，后端模板一旦变化，未知行降级为 info
 * 并保留原文，绝不丢日志、绝不抛异常。
 */

export interface RequestLogEntry {
  ts: string;
  source: string;
  message: string;
}

export type LogKind = "request" | "response" | "upstream" | "usage" | "error" | "info";

export interface UsageNumbers {
  credit: number | null;
  promptTokens: number | null;
  completionTokens: number | null;
  totalTokens: number | null;
  quotaPercent: number | null;
}

export interface ParsedLogLine {
  kind: LogKind;
  status: number | null;
  durationMs: number | null;
  model: string | null;
  path: string | null;
  usage: UsageNumbers | null;
  error: string | null;
}

export interface LogRow {
  key: string;
  ts: string;
  model: string | null;
  path: string | null;
  status: number | null;
  durationMs: number | null;
  credit: number | null;
  inTokens: number | null;
  outTokens: number | null;
  totalTokens: number | null;
  quotaPercent: number | null;
  error: string | null;
  rawLines: string[];
}

export interface LogSummary {
  total: number;
  success: number;
  failed: number;
  successRate: number;
  totalCredit: number;
  totalTokens: number;
  avgDurationMs: number;
}

export interface MinuteBucket {
  label: string;
  requests: number;
  tokens: number;
  credit: number;
}

const numberIn = (text: string, re: RegExp): number | null => {
  const m = re.exec(text);
  if (!m) return null;
  const n = Number(m[1]);
  return Number.isFinite(n) ? n : null;
};

/** 解析后端追加的「用量: 积分 … · A→B (共C) tok · 额度 X%」行。 */
export function parseUsageLine(message: string): UsageNumbers | null {
  if (!message.startsWith("用量:")) return null;
  const text = message;
  const tokens = /(\d+)\s*→\s*(\d+)\s*\(共(\d+)\)\s*tok/.exec(text);
  return {
    credit: numberIn(text, /积分\s*([\d.]+)/),
    promptTokens: tokens ? Number(tokens[1]) : null,
    completionTokens: tokens ? Number(tokens[2]) : null,
    totalTokens: tokens ? Number(tokens[3]) : numberIn(text, /共(\d+)\)\s*tok/),
    quotaPercent: numberIn(text, /额度\s*([\d.]+)\s*%/),
  };
}

export function parseLogLine(entry: RequestLogEntry): ParsedLogLine {
  const m = entry.message;
  const base: ParsedLogLine = {
    kind: "info",
    status: null,
    durationMs: null,
    model: null,
    path: null,
    usage: null,
    error: null,
  };

  const usage = parseUsageLine(m);
  if (usage) return { ...base, kind: "usage", usage };

  if (/^→\s/.test(m)) {
    const req = /^→\s+[A-Z]+\s+(\S+)/.exec(m);
    return { ...base, kind: "request", path: req ? req[1] : null };
  }
  if (/^IN\s*\[/.test(m)) {
    const model = /\bmodel=(\S+)/.exec(m);
    return { ...base, kind: "info", model: model ? model[1] : null };
  }
  if (/^✗/.test(m)) {
    return { ...base, kind: "error", error: m.slice(1).trim(), status: numberIn(m, /\b(\d{3})\b/) };
  }
  if (/^←\s+上游/.test(m)) {
    return {
      ...base,
      kind: "upstream",
      status: numberIn(m, /上游\s+(\d{3})/),
      durationMs: numberIn(m, /\((\d+)ms\)/),
    };
  }
  if (/^←\s+\d{3}/.test(m)) {
    const req = /^←\s+(\d{3})\s+(\S+)\s+\((\d+)ms\)/.exec(m);
    return {
      ...base,
      kind: "response",
      status: req ? Number(req[1]) : null,
      path: req ? req[2] : null,
      durationMs: req ? Number(req[3]) : numberIn(m, /\((\d+)ms\)/),
    };
  }
  return base;
}

const emptyRow = (entry: RequestLogEntry, index: number): LogRow => ({
  key: `${entry.ts}-${index}`,
  ts: entry.ts,
  model: null,
  path: null,
  status: null,
  durationMs: null,
  credit: null,
  inTokens: null,
  outTokens: null,
  totalTokens: null,
  quotaPercent: null,
  error: null,
  rawLines: [],
});

const isRequestStart = (entry: RequestLogEntry) => /^→\s/.test(entry.message);

/** 以「→ METHOD path」为新请求起点；被环形缓冲截断的孤儿行独立成行，不丢信息。 */
export function groupIntoRequests(entries: RequestLogEntry[]): LogRow[] {
  const rows: LogRow[] = [];
  let current: LogRow | null = null;
  // 只有见过「→」起点之后的行才归并；被环形缓冲截断留下的孤儿行各自独立，
  // 否则一组没有起点的零散行会被错误粘成一整条请求。
  let anchored = false;

  entries.forEach((entry, index) => {
    const start = isRequestStart(entry);
    if (start || !current || !anchored) {
      current = emptyRow(entry, index);
      rows.push(current);
      anchored = start;
    }
    const row = current;
    row.rawLines.push(entry.message);

    const p = parseLogLine(entry);
    if (p.model) row.model = p.model;
    if (p.path && !row.path) row.path = p.path;
    // 响应行的耗时/状态优先于「上游」行（更接近用户感知的总耗时）。
    if (p.kind === "response") {
      if (p.status !== null) row.status = p.status;
      if (p.durationMs !== null) row.durationMs = p.durationMs;
    } else if (p.status !== null && row.status === null) {
      row.status = p.status;
      if (p.durationMs !== null) row.durationMs = p.durationMs;
    }
    if (p.error) row.error = p.error;
    if (p.usage) {
      if (p.usage.credit !== null) row.credit = p.usage.credit;
      if (p.usage.promptTokens !== null) row.inTokens = p.usage.promptTokens;
      if (p.usage.completionTokens !== null) row.outTokens = p.usage.completionTokens;
      if (p.usage.totalTokens !== null) row.totalTokens = p.usage.totalTokens;
      if (p.usage.quotaPercent !== null) row.quotaPercent = p.usage.quotaPercent;
    }
  });

  return rows;
}

export function summarize(rows: LogRow[]): LogSummary {
  const total = rows.length;
  const success = rows.filter((r) => r.status !== null && r.status >= 200 && r.status < 300).length;
  const failed = rows.filter((r) => r.status !== null && r.status >= 400).length;
  const durations = rows.filter((r) => r.durationMs !== null).map((r) => r.durationMs as number);
  return {
    total,
    success,
    failed,
    successRate: total > 0 ? success / total : 0,
    totalCredit: rows.reduce((n, r) => n + (r.credit ?? 0), 0),
    totalTokens: rows.reduce((n, r) => n + (r.totalTokens ?? 0), 0),
    avgDurationMs: durations.length > 0
      ? Math.round(durations.reduce((n, d) => n + d, 0) / durations.length)
      : 0,
  };
}

/** 按 `HH:MM` 分桶；`ts` 没有日期，跨小时会重叠，仅用于当前会话近况。 */
export function bucketByMinute(rows: LogRow[]): MinuteBucket[] {
  const map = new Map<string, MinuteBucket>();
  for (const row of rows) {
    const label = row.ts.slice(0, 5);
    const bucket = map.get(label) ?? { label, requests: 0, tokens: 0, credit: 0 };
    bucket.requests += 1;
    bucket.tokens += row.totalTokens ?? 0;
    bucket.credit += row.credit ?? 0;
    map.set(label, bucket);
  }
  const all = [...map.values()].sort((a, b) => a.label.localeCompare(b.label));
  return all.slice(-60);
}
