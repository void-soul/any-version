import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  Activity,
  Cable,
  Download,
  Loader2,
  Network,
  Pause,
  Play,
  PlugZap,
  Radio,
  Send,
  Trash2,
  Unplug,
} from "lucide-react";
import { formatBytes, nowStamp, type LineEnding } from "./serialUtils";
import {
  MAX_LOGS,
  MAX_PENDING,
  MAX_SEND_HISTORY,
  buildExportText,
  filterLogs,
  formatDuration,
  loadPrefs,
  loadSendHistory,
  loadTargets,
  normalizeHex,
  parseHeadersText,
  parseSseBlock,
  pushTarget,
  savePrefs,
  saveSendHistory,
  saveTargets,
  withLineEnding,
  type Dir,
  type LogEntry,
  type Proto,
  type SseMeta,
  type TargetMap,
} from "./wsDebugUtils";

interface MessagePayload {
  id: string;
  kind?: "text" | "binary";
  data?: string;
  text?: string;
  hex?: string;
  len?: number;
  from?: string;
  proto?: string;
}

const PROTO_TABS: Array<{ key: Proto; label: string }> = [
  { key: "ws", label: "WebSocket" },
  { key: "sse", label: "SSE" },
  { key: "tcp", label: "TCP" },
  { key: "udp", label: "UDP" },
];

const MAX_RECONNECT = 5;
const RECONNECT_DELAY = 1000;
const PING_TIMEOUT = 10_000;

/** WebSocket / SSE / TCP / UDP 调试器。 */
export default function WsDebugger() {
  const { t } = useTranslation();
  const [initialPrefs] = useState(loadPrefs);

  // 连接
  const [proto, setProto] = useState<Proto>("ws");
  const [connId, setConnId] = useState("conn-1");
  const [url, setUrl] = useState("ws://");
  const [host, setHost] = useState("127.0.0.1");
  const [port, setPort] = useState("8080");
  const [headersText, setHeadersText] = useState("");
  const [connected, setConnected] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [connectedAt, setConnectedAt] = useState<number | null>(null);
  const [duration, setDuration] = useState("00:00:00");
  const [latency, setLatency] = useState<number | null>(null);
  const [pingPending, setPingPending] = useState(false);
  const [targets, setTargets] = useState<TargetMap>(loadTargets);
  const [recentValue, setRecentValue] = useState("");

  // 日志与偏好
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [paused, setPaused] = useState(false);
  const [pendingCount, setPendingCount] = useState(0);
  const [filter, setFilter] = useState("");
  const [showTime, setShowTime] = useState(initialPrefs.showTime);
  const [autoScroll, setAutoScroll] = useState(initialPrefs.autoScroll);
  const [hexView, setHexView] = useState(initialPrefs.hexView);
  const [autoReconnect, setAutoReconnect] = useState(initialPrefs.autoReconnect);
  const [lineEnding, setLineEnding] = useState<LineEnding>(initialPrefs.lineEnding);
  const [stats, setStats] = useState({ rxFrames: 0, rxBytes: 0, txFrames: 0, txBytes: 0 });

  // 发送
  const [sendData, setSendData] = useState("");
  const [sendHex, setSendHex] = useState(false);
  const [sendHistory, setSendHistory] = useState(loadSendHistory);
  const [historyIdx, setHistoryIdx] = useState(-1);

  const logRef = useRef<HTMLDivElement>(null);
  const seqRef = useRef(0);
  // 事件监听只挂载一次，闭包锁不住后续 state —— 所有「当前值」都从 ref 读。
  const tRef = useRef(t);
  const protoRef = useRef(proto);
  const connIdRef = useRef(connId);
  const urlRef = useRef(url);
  const hostRef = useRef(host);
  const portRef = useRef(port);
  const headersRef = useRef(headersText);
  const pausedRef = useRef(false);
  const pendingRef = useRef<LogEntry[]>([]);
  const activeIdRef = useRef("");
  const connectingRef = useRef(false);
  const connectEpochRef = useRef(0);
  const manualRef = useRef(false);
  const autoReconnectRef = useRef(autoReconnect);
  const reconnectTimerRef = useRef<number | null>(null);
  const reconnectAttemptsRef = useRef(0);
  const connectRef = useRef<() => Promise<void>>(async () => {});
  const pingSentRef = useRef<Map<string, number>>(new Map());
  const pingTimerRef = useRef<number | null>(null);

  tRef.current = t;
  protoRef.current = proto;
  connIdRef.current = connId;
  urlRef.current = url;
  hostRef.current = host;
  portRef.current = port;
  headersRef.current = headersText;
  autoReconnectRef.current = autoReconnect;

  const recordLog = useCallback(
    (dir: Dir, text: string, hex?: string, bytes = 0, meta?: SseMeta) => {
      const entry: LogEntry = {
        id: ++seqRef.current,
        dir,
        text,
        hex,
        bytes,
        time: nowStamp(),
        meta,
      };
      if (pausedRef.current) {
        pendingRef.current.push(entry);
        if (pendingRef.current.length > MAX_PENDING) {
          pendingRef.current.splice(0, pendingRef.current.length - MAX_PENDING);
        }
        setPendingCount(pendingRef.current.length);
      } else {
        setLogs((prev) => {
          const next = [...prev, entry];
          return next.length > MAX_LOGS ? next.slice(-MAX_LOGS) : next;
        });
      }
      if (bytes > 0 && (dir === "rx" || dir === "event" || dir === "tx")) {
        const isRx = dir !== "tx";
        setStats((s) => ({
          rxFrames: s.rxFrames + (isRx ? 1 : 0),
          rxBytes: s.rxBytes + (isRx ? bytes : 0),
          txFrames: s.txFrames + (isRx ? 0 : 1),
          txBytes: s.txBytes + (isRx ? 0 : bytes),
        }));
      }
    },
    []
  );

  const clearReconnectTimer = useCallback(() => {
    if (reconnectTimerRef.current !== null) {
      window.clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    }
    reconnectAttemptsRef.current = 0;
  }, []);

  const scheduleReconnect = useCallback(() => {
    if (!autoReconnectRef.current || manualRef.current) return;
    if (reconnectAttemptsRef.current >= MAX_RECONNECT) {
      recordLog("sys", tRef.current("wsdebug.reconnectGaveUp"));
      return;
    }
    reconnectAttemptsRef.current += 1;
    const attempt = reconnectAttemptsRef.current;
    recordLog("sys", tRef.current("wsdebug.reconnecting", { attempt }));
    if (reconnectTimerRef.current !== null) window.clearTimeout(reconnectTimerRef.current);
    reconnectTimerRef.current = window.setTimeout(() => {
      reconnectTimerRef.current = null;
      void connectRef.current();
    }, RECONNECT_DELAY);
  }, [recordLog]);

  const rememberTarget = useCallback(() => {
    const p = protoRef.current;
    const value =
      p === "ws" || p === "sse"
        ? urlRef.current.trim()
        : `${hostRef.current.trim()}:${portRef.current.trim()}`;
    if (!value || value === ":") return;
    setTargets((prev) => {
      const next = pushTarget(prev, p, value);
      saveTargets(next);
      return next;
    });
  }, []);

  // 事件监听（挂载一次）
  useEffect(() => {
    let cancelled = false;
    const un: Array<() => void> = [];
    const add = (fn: () => void) => {
      if (cancelled) fn();
      else un.push(fn);
    };

    (async () => {
      add(
        await listen<MessagePayload>("wstool://message", (e) => {
          const p = e.payload;
          if (p.id !== activeIdRef.current) return;
          const text = p.data ?? p.text ?? "";
          const bytes =
            p.len ??
            (p.hex
              ? p.hex.split(/\s+/).filter(Boolean).length
              : new TextEncoder().encode(text).length);
          const prefix = p.from ? `[${p.from}] ` : "";
          recordLog("rx", prefix + text, p.hex, bytes);
        })
      );

      add(
        await listen<{ id: string; url?: string }>("wstool://open", (e) => {
          if (e.payload.id !== activeIdRef.current) return;
          clearReconnectTimer();
          connectingRef.current = false;
          setConnecting(false);
          setConnected(true);
          setConnectedAt(Date.now());
          recordLog(
            "open",
            tRef.current("wsdebug.connected", { url: e.payload.url ?? e.payload.id })
          );
          rememberTarget();
        })
      );

      add(
        await listen<{ id: string }>("wstool://closed", (e) => {
          if (e.payload.id !== activeIdRef.current) return;
          activeIdRef.current = "";
          connectingRef.current = false;
          setConnecting(false);
          setConnected(false);
          setConnectedAt(null);
          setPingPending(false);
          setLatency(null);
          recordLog("close", tRef.current("wsdebug.closed", { id: e.payload.id }));
          scheduleReconnect();
        })
      );

      add(
        await listen<{ id: string; raw: string }>("sstool://event", (e) => {
          if (e.payload.id !== activeIdRef.current) return;
          const meta = parseSseBlock(e.payload.raw);
          const text = meta.data ?? e.payload.raw;
          recordLog("event", text, undefined, e.payload.raw.length, meta);
        })
      );

      add(
        await listen<{ id: string }>("sstool://open", (e) => {
          if (e.payload.id !== activeIdRef.current) return;
          clearReconnectTimer();
          connectingRef.current = false;
          setConnecting(false);
          setConnected(true);
          setConnectedAt(Date.now());
          recordLog("open", tRef.current("wsdebug.sseSubscribed", { id: e.payload.id }));
          rememberTarget();
        })
      );

      add(
        await listen<{ id: string }>("sstool://closed", (e) => {
          if (e.payload.id !== activeIdRef.current) return;
          activeIdRef.current = "";
          connectingRef.current = false;
          setConnecting(false);
          setConnected(false);
          setConnectedAt(null);
          recordLog("close", tRef.current("wsdebug.sseDisconnected", { id: e.payload.id }));
          scheduleReconnect();
        })
      );

      add(
        await listen<{ id: string; token: string }>("wstool://pong", (e) => {
          if (e.payload.id !== activeIdRef.current) return;
          const sentAt = pingSentRef.current.get(e.payload.token);
          if (sentAt === undefined) return;
          pingSentRef.current.delete(e.payload.token);
          if (pingTimerRef.current !== null) {
            window.clearTimeout(pingTimerRef.current);
            pingTimerRef.current = null;
          }
          setPingPending(false);
          setLatency(Math.max(0, Math.round(performance.now() - sentAt)));
        })
      );
    })();

    return () => {
      cancelled = true;
      un.forEach((u) => u());
      if (reconnectTimerRef.current !== null) window.clearTimeout(reconnectTimerRef.current);
      if (pingTimerRef.current !== null) window.clearTimeout(pingTimerRef.current);
      // 只注销事件会把后端会话留成孤儿：按「真实连接 ID」主动断开。
      const id = activeIdRef.current;
      activeIdRef.current = "";
      if (id) {
        void invoke("wstool_disconnect", { id }).catch(() => {
          /* 卸载路径，失败无可上报 */
        });
      }
    };
  }, [clearReconnectTimer, recordLog, rememberTarget, scheduleReconnect]);

  // 时长
  useEffect(() => {
    if (!connected || connectedAt === null) {
      setDuration("00:00:00");
      return;
    }
    const tick = () => setDuration(formatDuration(Date.now() - connectedAt));
    tick();
    const timer = window.setInterval(tick, 1000);
    return () => window.clearInterval(timer);
  }, [connected, connectedAt]);

  // 偏好持久化
  useEffect(() => {
    savePrefs({ showTime, autoScroll, hexView, autoReconnect, lineEnding });
  }, [showTime, autoScroll, hexView, autoReconnect, lineEnding]);

  // 自动滚动
  useEffect(() => {
    if (!autoScroll || paused) return;
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [logs, autoScroll, paused, hexView, filter]);

  const doDisconnect = useCallback(
    async (manual = true) => {
      manualRef.current = true;
      clearReconnectTimer();
      connectEpochRef.current += 1;
      connectingRef.current = false;
      const id = activeIdRef.current;
      activeIdRef.current = "";
      setConnected(false);
      setConnecting(false);
      setConnectedAt(null);
      setLatency(null);
      setPingPending(false);
      if (id) {
        try {
          await invoke("wstool_disconnect", { id });
        } catch {
          /* 已断开 / 会话不存在 */
        }
      }
      if (manual) recordLog("close", tRef.current("wsdebug.disconnectedByUser"));
    },
    [clearReconnectTimer, recordLog]
  );

  const doConnect = async (auto = false) => {
    if (connectingRef.current || connected) return;
    // 手动连接重置重连计数；自动重连必须保留计数，否则永远停不下来
    if (!auto) clearReconnectTimer();
    const id = connId.trim();
    if (!id) {
      recordLog("sys", t("wsdebug.connIdRequired"));
      return;
    }
    const p = proto;
    const portNum = Number(port);
    if ((p === "tcp" || p === "udp") && (!host.trim() || !Number.isInteger(portNum) || portNum <= 0 || portNum > 65535)) {
      recordLog("sys", t("wsdebug.invalidTarget"));
      return;
    }
    if ((p === "ws" || p === "sse") && !url.trim()) {
      recordLog("sys", t("wsdebug.invalidTarget"));
      return;
    }

    manualRef.current = false;
    connectingRef.current = true;
    setConnecting(true);
    const epoch = ++connectEpochRef.current;
    activeIdRef.current = id;

    try {
      if (p === "ws") {
        await invoke("ws_connect", { id, url: url.trim(), headers: parseHeadersText(headersText) });
      } else if (p === "sse") {
        await invoke("sse_connect", { id, url: url.trim(), headers: parseHeadersText(headersText) });
      } else {
        await invoke("net_connect", { id, protocol: p, host: host.trim(), port: portNum });
      }
      // 连接过程中用户切了协议 / 点了断开：后端会话可能刚好建成，补一次清理
      if (connectEpochRef.current !== epoch) {
        await invoke("wstool_disconnect", { id }).catch(() => {});
        return;
      }
    } catch (e) {
      if (connectEpochRef.current === epoch) {
        activeIdRef.current = "";
        connectingRef.current = false;
        setConnecting(false);
        setConnected(false);
        recordLog("sys", String(e));
        scheduleReconnect();
      }
    } finally {
      if (connectEpochRef.current === epoch) {
        connectingRef.current = false;
        setConnecting(false);
      }
    }
  };
  connectRef.current = () => doConnect(true);

  const switchProto = async (next: Proto) => {
    if (next === proto) return;
    manualRef.current = true;
    clearReconnectTimer();
    if (activeIdRef.current) {
      recordLog("sys", t("wsdebug.switchWillDisconnect"));
      await doDisconnect(false);
    }
    setProto(next);
    setRecentValue("");
    setLatency(null);
    if (next === "ws") setUrl((u) => (u.startsWith("http") ? "ws://" : u || "ws://"));
    if (next === "sse") setUrl((u) => (u.startsWith("ws") ? "https://" : u));
  };

  const selectTarget = async (value: string) => {
    setRecentValue("");
    if (!value) return;
    manualRef.current = true;
    clearReconnectTimer();
    if (connected || connecting) await doDisconnect(false);
    if (proto === "ws" || proto === "sse") {
      setUrl(value);
    } else {
      const idx = value.lastIndexOf(":");
      if (idx > 0) {
        setHost(value.slice(0, idx));
        setPort(value.slice(idx + 1));
      }
    }
  };

  const doPing = async () => {
    if (!connected || proto !== "ws" || pingPending) return;
    const id = activeIdRef.current;
    const token = `${Date.now().toString(36)}${Math.floor(Math.random() * 1000)}`;
    pingSentRef.current.set(token, performance.now());
    setPingPending(true);
    try {
      await invoke("ws_ping", { id, token });
    } catch (e) {
      pingSentRef.current.delete(token);
      setPingPending(false);
      recordLog("sys", String(e));
      return;
    }
    if (pingTimerRef.current !== null) window.clearTimeout(pingTimerRef.current);
    pingTimerRef.current = window.setTimeout(() => {
      pingTimerRef.current = null;
      pingSentRef.current.delete(token);
      setPingPending(false);
      setLatency(null);
      recordLog("sys", tRef.current("wsdebug.pingTimeout"));
    }, PING_TIMEOUT);
  };

  const canSend = connected && (proto === "ws" || proto === "tcp" || proto === "udp");

  const writeOne = async (raw: string): Promise<boolean> => {
    if (!connected || !raw) return false;
    let payload = raw;
    if (sendHex) {
      const hex = normalizeHex(raw);
      if (!hex) {
        recordLog("sys", t("wsdebug.hexInvalid"));
        return false;
      }
      payload = hex;
    } else if (proto === "tcp" || proto === "udp") {
      payload = withLineEnding(raw, lineEnding);
    }
    try {
      if (proto === "ws") {
        await invoke("ws_send", { id: activeIdRef.current, data: payload, hexMode: sendHex });
      } else {
        await invoke("net_send", { id: activeIdRef.current, data: payload, hexMode: sendHex });
      }
      const bytes = sendHex
        ? payload.split(/\s+/).filter(Boolean).length
        : new TextEncoder().encode(payload).length;
      recordLog("tx", sendHex ? payload : raw, sendHex ? payload : undefined, bytes);
      return true;
    } catch (e) {
      recordLog("sys", String(e));
      return false;
    }
  };

  const doSend = async () => {
    if (!canSend || !sendData) return;
    const ok = await writeOne(sendData);
    if (!ok) return;
    const value = sendData.trim();
    if (value) {
      setSendHistory((h) => {
        const next = [value, ...h.filter((x) => x !== value)].slice(0, MAX_SEND_HISTORY);
        saveSendHistory(next);
        return next;
      });
    }
    setHistoryIdx(-1);
  };

  const togglePause = () => {
    if (!paused) {
      pausedRef.current = true;
      setPaused(true);
      return;
    }
    pausedRef.current = false;
    setPaused(false);
    const pending = pendingRef.current;
    pendingRef.current = [];
    setPendingCount(0);
    if (pending.length > 0) {
      setLogs((prev) => {
        const next = [...prev, ...pending];
        return next.length > MAX_LOGS ? next.slice(-MAX_LOGS) : next;
      });
    }
  };

  const clearLogs = () => {
    pendingRef.current = [];
    setPendingCount(0);
    setLogs([]);
    setStats({ rxFrames: 0, rxBytes: 0, txFrames: 0, txBytes: 0 });
  };

  const exportLog = async () => {
    if (logs.length === 0) {
      recordLog("sys", t("wsdebug.exportEmpty"));
      return;
    }
    try {
      const target = await saveDialog({
        defaultPath: `net-debug-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")}.txt`,
        filters: [{ name: "Text", extensions: ["txt", "log"] }],
      });
      if (!target) return;
      await invoke("write_text_file", {
        path: target,
        content: buildExportText(logs, hexView, showTime),
      });
      recordLog("sys", t("wsdebug.exportOk", { path: target }));
    } catch (e) {
      recordLog("sys", t("wsdebug.exportFail", { err: String(e) }));
    }
  };

  const visibleLogs = useMemo(() => filterLogs(logs, filter), [logs, filter]);

  const dirLabel = (dir: Dir) =>
    dir === "rx"
      ? t("wsdebug.dirRx")
      : dir === "tx"
        ? t("wsdebug.dirTx")
        : dir === "event"
          ? t("wsdebug.dirEvent")
          : dir === "open"
            ? t("wsdebug.dirOpen")
            : dir === "close"
              ? t("wsdebug.dirClose")
              : t("wsdebug.dirSys");

  const colorCls = (dir: Dir) =>
    dir === "rx"
      ? "text-slate-200"
      : dir === "tx"
        ? "text-cyan-200"
        : dir === "event"
          ? "text-violet-200"
          : dir === "open"
            ? "text-emerald-400"
            : dir === "close"
              ? "text-red-400"
              : "text-yellow-500";

  const locked = connected || connecting;

  return (
    <div className="h-full flex flex-col overflow-hidden p-3 gap-2.5 text-body">
      {/* 标题 + 状态 + 协议切换 */}
      <div className="flex items-center gap-3 shrink-0">
        <Cable className="w-4 h-4 text-indigo-400" />
        <h1 className="text-base font-semibold">{t("wsdebug.title")}</h1>
        <span
          className={`text-body px-2 py-0.5 rounded-full ${
            connecting
              ? "bg-amber-900/60 text-amber-300"
              : connected
                ? "bg-emerald-900/60 text-emerald-300"
                : "bg-white/5 text-slate-400"
          }`}
        >
          {connecting
            ? t("wsdebug.connecting")
            : connected
              ? t("wsdebug.connectedState")
              : t("wsdebug.notConnected")}
        </span>
        {connected && (
          <span className="text-tiny text-slate-500 tabular-nums">{duration}</span>
        )}
        {proto === "ws" && (
          <button
            onClick={doPing}
            disabled={!connected || pingPending}
            className="text-tiny px-2 py-0.5 rounded-full bg-white/[0.06] hover:bg-white/10 disabled:opacity-40 text-slate-300 flex items-center gap-1"
            title={t("wsdebug.ping")}
          >
            <Activity className="w-3 h-3" />
            {pingPending
              ? t("wsdebug.pingPending")
              : latency !== null
                ? t("wsdebug.pingLatency", { ms: latency })
                : t("wsdebug.ping")}
          </button>
        )}
        <div className="ml-auto flex gap-1 rounded-md bg-white/[0.04] p-0.5 text-caption">
          {PROTO_TABS.map(({ key, label }) => (
            <button
              key={key}
              onClick={() => void switchProto(key)}
              disabled={locked}
              className={`rounded px-2.5 py-1 text-caption transition-colors disabled:opacity-40 ${
                proto === key ? "bg-white/10 text-white" : "text-slate-400 hover:text-slate-200"
              }`}
            >
              {label}
            </button>
          ))}
        </div>
      </div>

      {/* 连接配置 */}
      <div className="shrink-0 flex items-center gap-2 bg-white/[0.03] border border-white/10 rounded-ctl p-3">
        <select
          value={recentValue}
          onChange={(e) => void selectTarget(e.target.value)}
          disabled={locked}
          className="max-w-36 bg-black/30 border border-white/10 rounded px-2 py-2 text-body text-slate-300 focus:outline-none focus:border-indigo-500 disabled:opacity-40"
          title={t("wsdebug.recent")}
        >
          <option value="">
            {targets[proto].length > 0 ? t("wsdebug.recent") : t("wsdebug.recentEmpty")}
          </option>
          {targets[proto].map((v) => (
            <option key={v} value={v}>
              {v}
            </option>
          ))}
        </select>
        <input
          value={connId}
          onChange={(e) => setConnId(e.target.value)}
          disabled={locked}
          className="w-24 bg-black/30 border border-white/10 rounded px-2 py-1.5 font-mono text-body focus:outline-none focus:border-indigo-500 disabled:opacity-40"
          title={t("wsdebug.connIdTitle")}
        />
        {(proto === "ws" || proto === "sse") && (
          <input
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            disabled={locked}
            placeholder={proto === "ws" ? t("wsdebug.wsPlaceholder") : t("wsdebug.ssePlaceholder")}
            className="flex-1 bg-black/30 border border-white/10 rounded px-3 py-2 font-mono text-body focus:outline-none focus:border-indigo-500 disabled:opacity-40"
          />
        )}
        {(proto === "tcp" || proto === "udp") && (
          <>
            <Network className="w-4 h-4 text-indigo-400 shrink-0" />
            <input
              value={host}
              onChange={(e) => setHost(e.target.value)}
              disabled={locked}
              placeholder={t("wsdebug.hostPlaceholder")}
              className="flex-1 min-w-32 bg-black/30 border border-white/10 rounded px-3 py-2 font-mono text-body focus:outline-none focus:border-indigo-500 disabled:opacity-40"
            />
            <span className="text-slate-500 text-sm">:</span>
            <input
              value={port}
              onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))}
              disabled={locked}
              placeholder={t("wsdebug.portPlaceholder")}
              className="w-24 bg-black/30 border border-white/10 rounded px-3 py-2 font-mono text-body focus:outline-none focus:border-indigo-500 disabled:opacity-40"
            />
          </>
        )}
        {connected ? (
          <button
            onClick={() => void doDisconnect(true)}
            className="px-3 py-2 rounded-md bg-red-600 hover:bg-red-500 text-body flex items-center gap-1"
          >
            <Unplug className="w-3.5 h-3.5" /> {t("wsdebug.disconnect")}
          </button>
        ) : (
          <button
            onClick={() => void doConnect()}
            disabled={connecting || (proto === "ws" || proto === "sse" ? !url : !host || !port)}
            className="px-3 py-2 rounded-md bg-indigo-600 hover:bg-indigo-500 disabled:opacity-40 text-body flex items-center gap-1"
          >
            {connecting ? (
              <Loader2 className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <PlugZap className="w-3.5 h-3.5" />
            )}
            {connecting ? t("wsdebug.connecting") : t("wsdebug.connect")}
          </button>
        )}
      </div>

      {(proto === "ws" || proto === "sse") && (
        <input
          value={headersText}
          onChange={(e) => setHeadersText(e.target.value)}
          disabled={locked}
          placeholder={t("wsdebug.headersPlaceholder")}
          className="shrink-0 bg-black/30 border border-white/10 rounded px-3 py-1.5 font-mono text-body focus:outline-none focus:border-indigo-500 disabled:opacity-40"
        />
      )}

      {/* 日志工具栏 */}
      <div className="shrink-0 flex flex-wrap items-center gap-x-3 gap-y-1 rounded-ctl border border-white/10 bg-white/[0.03] px-3 py-1.5 text-tiny text-slate-400">
        <span className="tabular-nums">
          {t("wsdebug.statRx")} {stats.rxFrames} · {formatBytes(stats.rxBytes)}
        </span>
        <span className="tabular-nums">
          {t("wsdebug.statTx")} {stats.txFrames} · {formatBytes(stats.txBytes)}
        </span>
        <span className="tabular-nums">
          {t("wsdebug.statDuration")} {duration}
        </span>
        <label className="flex items-center gap-1 cursor-pointer">
          <input
            type="checkbox"
            checked={showTime}
            onChange={(e) => setShowTime(e.target.checked)}
            className="accent-indigo-500"
          />
          {t("wsdebug.showTime")}
        </label>
        <label className="flex items-center gap-1 cursor-pointer">
          <input
            type="checkbox"
            checked={autoScroll}
            onChange={(e) => setAutoScroll(e.target.checked)}
            className="accent-indigo-500"
          />
          {t("wsdebug.autoScroll")}
        </label>
        <label className="flex items-center gap-1 cursor-pointer">
          <input
            type="checkbox"
            checked={hexView}
            onChange={(e) => setHexView(e.target.checked)}
            className="accent-indigo-500"
          />
          {t("wsdebug.hexView")}
        </label>
        <label className="flex items-center gap-1 cursor-pointer">
          <input
            type="checkbox"
            checked={autoReconnect}
            onChange={(e) => setAutoReconnect(e.target.checked)}
            className="accent-indigo-500"
          />
          {t("wsdebug.autoReconnect")}
        </label>
        <button
          onClick={togglePause}
          className={`flex items-center gap-1 px-1.5 py-0.5 rounded hover:bg-white/10 ${
            paused ? "text-amber-300" : ""
          }`}
        >
          {paused ? <Play className="w-3 h-3" /> : <Pause className="w-3 h-3" />}
          {paused ? t("wsdebug.resumeReceive") : t("wsdebug.pauseReceive")}
          {paused && pendingCount > 0 ? ` (${pendingCount})` : ""}
        </button>
        <div className="ml-auto flex items-center gap-1.5">
          <input
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder={t("wsdebug.filterPlaceholder")}
            className="w-40 bg-black/30 border border-white/10 rounded px-2 py-0.5 text-tiny focus:outline-none focus:border-indigo-500"
          />
          <button
            onClick={() => void exportLog()}
            className="flex items-center gap-1 px-1.5 py-0.5 rounded hover:bg-white/10"
            title={t("wsdebug.exportLog")}
          >
            <Download className="w-3.5 h-3.5" /> {t("wsdebug.exportLog")}
          </button>
          <button
            onClick={clearLogs}
            className="flex items-center gap-1 px-1.5 py-0.5 rounded hover:bg-white/10"
            title={t("wsdebug.clearLog")}
          >
            <Trash2 className="w-3.5 h-3.5" /> {t("wsdebug.clearLog")}
          </button>
        </div>
      </div>

      {/* 日志 */}
      <div
        ref={logRef}
        className="flex-1 min-h-0 overflow-auto rounded-ctl border border-white/10 bg-white/[0.02] p-3 font-mono text-body leading-5"
      >
        {logs.length === 0 && (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-slate-600 select-none">
            <Network className="h-8 w-8 opacity-40" />
            <span>{proto === "udp" ? t("wsdebug.emptyUdp") : t("wsdebug.empty")}</span>
          </div>
        )}
        {logs.length > 0 && visibleLogs.length === 0 && (
          <div className="flex h-full items-center justify-center text-slate-600 select-none">
            {t("wsdebug.filterEmpty")}
          </div>
        )}
        <div className="flex flex-col gap-2">
          {visibleLogs.map((entry) => {
            const system = entry.dir === "sys" || entry.dir === "open" || entry.dir === "close";
            const outgoing = entry.dir === "tx";
            const body = hexView && entry.hex !== undefined ? entry.hex : entry.text;
            return (
              <div
                key={entry.id}
                className={`flex ${system ? "justify-center" : outgoing ? "justify-end" : "justify-start"}`}
              >
                <div className={`flex max-w-[86%] items-end gap-2 ${outgoing ? "flex-row-reverse" : ""}`}>
                  {showTime && <span className="shrink-0 text-tiny text-slate-600">{entry.time}</span>}
                  <div
                    className={`rounded-card px-3 py-2 ${
                      system
                        ? "bg-white/[0.06]"
                        : outgoing
                          ? "bg-indigo-500/15"
                          : entry.dir === "event"
                            ? "bg-[var(--module-accent-soft)]"
                            : "bg-emerald-500/10"
                    } ${colorCls(entry.dir)}`}
                  >
                    <span className="mr-1.5 text-tiny opacity-70">{dirLabel(entry.dir)}</span>
                    {entry.meta && (entry.meta.event || entry.meta.id) && (
                      <span className="mr-1.5 text-tiny opacity-70">
                        {entry.meta.event ? t("wsdebug.sseEventTag", { event: entry.meta.event }) : ""}
                        {entry.meta.id ? ` ${t("wsdebug.sseIdTag", { id: entry.meta.id })}` : ""}
                      </span>
                    )}
                    <span className="break-all whitespace-pre-wrap">{body}</span>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {/* 发送区 */}
      {canSend && (
        <div className="shrink-0 flex items-center gap-2 bg-white/[0.03] border border-white/10 rounded-ctl p-3">
          <textarea
            value={sendData}
            onChange={(e) => setSendData(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey) {
                e.preventDefault();
                void doSend();
                return;
              }
              if (e.key === "ArrowUp" && e.ctrlKey && sendHistory.length > 0) {
                e.preventDefault();
                const next = Math.min(historyIdx + 1, sendHistory.length - 1);
                setHistoryIdx(next);
                setSendData(sendHistory[next]);
                return;
              }
              if (e.key === "ArrowDown" && e.ctrlKey && historyIdx >= 0) {
                e.preventDefault();
                const next = historyIdx - 1;
                setHistoryIdx(next);
                setSendData(next < 0 ? "" : sendHistory[next]);
              }
            }}
            rows={2}
            placeholder={sendHex ? t("wsdebug.sendHexPlaceholder") : t("wsdebug.sendTextPlaceholder")}
            title={t("wsdebug.sendHistory")}
            className="flex-1 bg-black/30 border border-white/10 rounded-md p-2 font-mono text-body resize-none focus:outline-none focus:border-indigo-500"
          />
          <label className="flex items-center gap-1 text-body cursor-pointer text-slate-300">
            <input
              type="checkbox"
              checked={sendHex}
              onChange={(e) => setSendHex(e.target.checked)}
              className="accent-indigo-500"
            />{" "}
            HEX
          </label>
          <select
            value={lineEnding}
            onChange={(e) => setLineEnding(e.target.value as LineEnding)}
            disabled={sendHex || proto === "ws"}
            title={t("wsdebug.lineEnding")}
            className="bg-black/30 border border-white/10 rounded px-2 py-1.5 text-body text-slate-300 focus:outline-none focus:border-indigo-500 disabled:opacity-40"
          >
            <option value="none">{t("wsdebug.leNone")}</option>
            <option value="lf">{t("wsdebug.leLf")}</option>
            <option value="crlf">{t("wsdebug.leCrlf")}</option>
            <option value="cr">{t("wsdebug.leCr")}</option>
          </select>
          <button
            onClick={() => void doSend()}
            disabled={!connected || !sendData}
            className="px-3 py-2 rounded-md bg-indigo-600 hover:bg-indigo-500 disabled:opacity-40 text-body flex items-center gap-1"
          >
            <Send className="w-3.5 h-3.5" /> {t("wsdebug.send")}
          </button>
        </div>
      )}
      {proto === "sse" && (
        <div className="shrink-0 flex items-center gap-2 text-body text-slate-500">
          <Radio className="w-3.5 h-3.5" /> {t("wsdebug.sseNote")}
        </div>
      )}
    </div>
  );
}
