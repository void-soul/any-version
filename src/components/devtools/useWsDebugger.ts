import { trimDebugLogs } from "./debugLogBudget";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { nowStamp, type LineEnding } from "./serialUtils";
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

const MAX_RECONNECT = 5;
const RECONNECT_DELAY = 1000;
const PING_TIMEOUT = 10_000;


export function useWsDebugger() {
  const { t } = useTranslation();
  const [initialPrefs] = useState(loadPrefs);
  const alive = useRef(false);
  const [sending, setSending] = useState(false);
  const [connectionError, setConnectionError] = useState("");
  const sendGate = useRef(false);
  const targetRequest = useRef(0);
  const connectedRef = useRef(false);
  const pingGate = useRef(false);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);

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
      if (!alive.current) return;
      const clip = (value: string | undefined, max: number) => !value || value.length <= max ? value : new TextDecoder().decode(new TextEncoder().encode(value.slice(0, max))) + tRef.current("wsdebug.truncatedMessage");
      const shownText = clip(text, 64 * 1024)!;
      const entry: LogEntry = {
        id: ++seqRef.current,
        dir,
        text: shownText,
        hex: clip(hex, 128 * 1024),
        bytes,
        time: nowStamp(),
        meta: meta ? { ...meta, data: clip(meta.data, 64 * 1024), id: clip(meta.id, 2048), event: clip(meta.event, 2048) } : undefined,
      };
      if (pausedRef.current) {
        pendingRef.current = trimDebugLogs([...pendingRef.current, entry], MAX_PENDING, 4 * 1024 * 1024);
        setPendingCount(pendingRef.current.length);
      } else {
        setLogs((prev) => {
          return trimDebugLogs([...prev, entry], MAX_LOGS);
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
    if (!alive.current || !autoReconnectRef.current || manualRef.current) return;
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
      if (alive.current && autoReconnectRef.current && !manualRef.current) void connectRef.current();
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
          if (cancelled || p.id !== activeIdRef.current) return;
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
          if (cancelled || e.payload.id !== activeIdRef.current) return;
          clearReconnectTimer();
          connectingRef.current = false;
          setConnecting(false);
          connectedRef.current = true;
          setConnected(true);
          setConnectedAt(Date.now());
          recordLog(
            "open",
            tRef.current("wsdebug.connected", { url: e.payload.url ?? connIdRef.current.trim() })
          );
          rememberTarget();
        })
      );

      add(
        await listen<{ id: string }>("wstool://closed", (e) => {
          if (cancelled || e.payload.id !== activeIdRef.current) return;
          const closedId = activeIdRef.current;
          activeIdRef.current = "";
          void invoke("wstool_disconnect", { id: closedId }).catch((error) => recordLog("sys", String(error)));
          pingSentRef.current.clear(); pingGate.current = false;
          if (pingTimerRef.current !== null) window.clearTimeout(pingTimerRef.current);
          connectingRef.current = false;
          setConnecting(false);
          connectedRef.current = false;
          setConnected(false);
          setConnectedAt(null);
          pingGate.current = false;
          setPingPending(false);
          setLatency(null);
          recordLog("close", tRef.current("wsdebug.closed", { id: connIdRef.current.trim() }));
          scheduleReconnect();
        })
      );

      add(
        await listen<{ id: string; raw: string }>("sstool://event", (e) => {
          if (cancelled || e.payload.id !== activeIdRef.current) return;
          const meta = parseSseBlock(e.payload.raw);
          const text = meta.data ?? e.payload.raw;
          recordLog("event", text, undefined, e.payload.raw.length, meta);
        })
      );

      add(
        await listen<{ id: string }>("sstool://open", (e) => {
          if (cancelled || e.payload.id !== activeIdRef.current) return;
          clearReconnectTimer();
          connectingRef.current = false;
          setConnecting(false);
          connectedRef.current = true;
          setConnected(true);
          setConnectedAt(Date.now());
          recordLog("open", tRef.current("wsdebug.sseSubscribed", { id: connIdRef.current.trim() }));
          rememberTarget();
        })
      );

      add(
        await listen<{ id: string; reason?: string | null }>("sstool://closed", (e) => {
          if (cancelled || e.payload.id !== activeIdRef.current) return;
          const closedId = activeIdRef.current;
          activeIdRef.current = "";
          void invoke("wstool_disconnect", { id: closedId }).catch((error) => recordLog("sys", String(error)));
          pingSentRef.current.clear(); pingGate.current = false;
          if (pingTimerRef.current !== null) window.clearTimeout(pingTimerRef.current);
          connectingRef.current = false;
          setConnecting(false);
          connectedRef.current = false;
          setConnected(false);
          setConnectedAt(null);
          if (e.payload.reason) recordLog("sys", e.payload.reason);
          recordLog("close", tRef.current("wsdebug.sseDisconnected", { id: connIdRef.current.trim() }));
          scheduleReconnect();
        })
      );

      add(
        await listen<{ id: string; token: string }>("wstool://pong", (e) => {
          if (cancelled || e.payload.id !== activeIdRef.current) return;
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
    })().catch((error) => { if (!cancelled) recordLog("sys", String(error)); });

    return () => {
      cancelled = true;
      connectEpochRef.current++; targetRequest.current++; manualRef.current = true;
      connectingRef.current = false; connectedRef.current = false;
      setConnected(false); setConnecting(false); setConnectedAt(null); setPingPending(false);
      pingGate.current = false; pingSentRef.current.clear();
      pendingRef.current = []; setPendingCount(0);
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

  const doDisconnect = useCallback(async (manual = true): Promise<boolean> => {
    manualRef.current = true; clearReconnectTimer();
    const epoch = ++connectEpochRef.current;
    const id = activeIdRef.current;
    setConnectionError("");
    if (id) {
      try { await invoke("wstool_disconnect", { id }); }
      catch (error) {
        if (alive.current && epoch === connectEpochRef.current) {
          setConnectionError(tRef.current("wsdebug.disconnectFailed", { err: String(error) }));
          connectedRef.current = true; setConnected(true); setConnecting(false);
        }
        return false;
      }
    }
    if (!alive.current || epoch !== connectEpochRef.current) return true;
    activeIdRef.current = ""; connectingRef.current = false; connectedRef.current = false;
    setConnected(false); setConnecting(false); setConnectedAt(null); setLatency(null); setPingPending(false);
    pingGate.current = false; pingSentRef.current.clear();
    if (pingTimerRef.current !== null) { window.clearTimeout(pingTimerRef.current); pingTimerRef.current = null; }
    if (manual) recordLog("close", tRef.current("wsdebug.disconnectedByUser"));
    return true;
  }, [clearReconnectTimer, recordLog]);

  const doConnect = async (auto = false) => {
    if (!alive.current || connectingRef.current || connectedRef.current) return;
    // 手动连接重置重连计数；自动重连必须保留计数，否则永远停不下来
    if (!auto) clearReconnectTimer();
    const logicalId = connId.trim();
    if (!logicalId) {
      recordLog("sys", t("wsdebug.connIdRequired"));
      return;
    }
    const id = `${logicalId}#${crypto.randomUUID()}`;
    const p = proto;
    const portNum = Number(port);
    if ((p === "tcp" || p === "udp") && (!host.trim() || !/^\d+$/.test(port.trim()) || !Number.isInteger(portNum) || portNum <= 0 || portNum > 65535)) {
      recordLog("sys", t("wsdebug.invalidTarget"));
      return;
    }
    if ((p === "ws" || p === "sse") && !url.trim()) {
      recordLog("sys", t("wsdebug.invalidTarget"));
      return;
    }

    setConnectionError("");
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
      if (!alive.current || connectEpochRef.current !== epoch) {
        await invoke("wstool_disconnect", { id }).catch(() => {});
        return;
      }
      if (activeIdRef.current === id) { connectedRef.current = true; setConnected(true); setConnectedAt((previous) => previous ?? Date.now()); }
    } catch (e) {
      if (alive.current && connectEpochRef.current === epoch) {
        activeIdRef.current = "";
        connectingRef.current = false;
        setConnecting(false);
        connectedRef.current = false; setConnected(false);
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
    const request = ++targetRequest.current;
    if (next === proto) return;
    manualRef.current = true;
    clearReconnectTimer();
    if (activeIdRef.current) {
      recordLog("sys", t("wsdebug.switchWillDisconnect"));
      if (!await doDisconnect(false)) return;
    }
    if (!alive.current || request !== targetRequest.current) return;
    setProto(next);
    setRecentValue("");
    setLatency(null);
    if (next === "ws") setUrl((u) => (u.startsWith("http") ? "ws://" : u || "ws://"));
    if (next === "sse") setUrl((u) => (u.startsWith("ws") ? "https://" : u));
  };

  const selectTarget = async (value: string) => {
    setRecentValue("");
    if (!value) return;
    const request = ++targetRequest.current;
    manualRef.current = true;
    clearReconnectTimer();
    if ((connectedRef.current || connectingRef.current) && !await doDisconnect(false)) return;
    if (!alive.current || request !== targetRequest.current) return;
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
    if (!connectedRef.current || proto !== "ws" || pingGate.current) return;
    pingGate.current = true;
    const id = activeIdRef.current;
    const token = `${Date.now().toString(36)}${Math.floor(Math.random() * 1000)}`;
    pingSentRef.current.set(token, performance.now());
    setPingPending(true);
    try {
      await invoke("ws_ping", { id, token });
    } catch (e) {
      pingSentRef.current.delete(token);
      pingGate.current = false; setPingPending(false);
      recordLog("sys", String(e));
      return;
    }
    if (!alive.current || activeIdRef.current !== id || !pingSentRef.current.has(token)) { pingGate.current = false; return; }
    if (pingTimerRef.current !== null) window.clearTimeout(pingTimerRef.current);
    pingTimerRef.current = window.setTimeout(() => {
      pingTimerRef.current = null;
      pingSentRef.current.delete(token);
      pingGate.current = false; setPingPending(false);
      setLatency(null);
      recordLog("sys", tRef.current("wsdebug.pingTimeout"));
    }, PING_TIMEOUT);
  };

  const canSend = connected && (proto === "ws" || proto === "tcp" || proto === "udp");

  const writeOne = async (raw: string): Promise<boolean> => {
    if (!alive.current || !connectedRef.current || !raw || sendGate.current) return false;
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
    sendGate.current = true; setSending(true);
    const id = activeIdRef.current;
    try {
      if (proto === "ws") {
        await invoke("ws_send", { id, data: payload, hexMode: sendHex });
      } else {
        await invoke("net_send", { id, data: payload, hexMode: sendHex });
      }
      if (!alive.current || id !== activeIdRef.current) return false;
      const bytes = sendHex
        ? payload.split(/\s+/).filter(Boolean).length
        : new TextEncoder().encode(payload).length;
      recordLog("tx", sendHex ? payload : raw, sendHex ? payload : undefined, bytes);
      return alive.current && id === activeIdRef.current;
    } catch (e) {
      if (alive.current && id === activeIdRef.current) recordLog("sys", String(e));
      return false;
    } finally { sendGate.current = false; setSending(false); }
  };

  const doSend = async () => {
    if (!canSend || !sendData) return;
    const ok = await writeOne(sendData);
    if (!ok) return;
    const value = sendData.trim();
    if (value && value.length <= 16 * 1024) {
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
        return trimDebugLogs([...prev, ...pending], MAX_LOGS);
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

  useEffect(() => { if (!autoReconnect) clearReconnectTimer(); }, [autoReconnect, clearReconnectTimer]);

  return {
    connectionError, sending,
    t,
    proto,
    connId,
    setConnId,
    url,
    setUrl,
    host,
    setHost,
    port,
    setPort,
    headersText,
    setHeadersText,
    connected,
    connecting,
    duration,
    latency,
    pingPending,
    targets,
    recentValue,
    logs,
    paused,
    pendingCount,
    filter,
    setFilter,
    showTime,
    setShowTime,
    autoScroll,
    setAutoScroll,
    hexView,
    setHexView,
    autoReconnect,
    setAutoReconnect,
    lineEnding,
    setLineEnding,
    stats,
    sendData,
    setSendData,
    sendHex,
    setSendHex,
    sendHistory,
    historyIdx,
    setHistoryIdx,
    logRef,
    doDisconnect,
    doConnect,
    switchProto,
    selectTarget,
    doPing,
    canSend,
    doSend,
    togglePause,
    clearLogs,
    exportLog,
    visibleLogs,
  };
}
