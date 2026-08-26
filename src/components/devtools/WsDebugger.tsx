import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Cable, PlugZap, Unplug, Send, Trash2, Radio, Network } from "lucide-react";

interface LogEntry {
  dir: "open" | "close" | "rx" | "tx" | "sys" | "event";
  text: string;
  time: string;
}

type Proto = "ws" | "sse" | "tcp" | "udp";

function now() {
  return new Date().toLocaleTimeString("zh-CN", { hour12: false });
}
const push = (setLogs: React.Dispatch<React.SetStateAction<LogEntry[]>>, dir: LogEntry["dir"], text: string) =>
  setLogs((l) => [...l.slice(-1000), { dir, text, time: now() }]);

const PROTO_TABS: Array<{ key: Proto; label: string }> = [
  { key: "ws", label: "WebSocket" },
  { key: "sse", label: "SSE" },
  { key: "tcp", label: "TCP" },
  { key: "udp", label: "UDP" },
];

/** WebSocket / SSE / TCP / UDP 调试器。 */
export default function WsDebugger() {
  const [proto, setProto] = useState<Proto>("ws");
  const [connId, setConnId] = useState("conn-1");
  const [url, setUrl] = useState("ws://");
  const [host, setHost] = useState("127.0.0.1");
  const [port, setPort] = useState("8080");
  const [headersText, setHeadersText] = useState("");
  const [connected, setConnected] = useState(false);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [sendData, setSendData] = useState("");
  const [sendHex, setSendHex] = useState(false);
  const logRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const un: Array<() => void> = [];
    (async () => {
      un.push(
        await listen<{ id: string; kind?: string; data?: string; hex?: string; from?: string }>("wstool://message", (e) => {
          if (e.payload.kind === "binary") {
            push(setLogs, "rx", `[二进制 ${e.payload.hex?.split(" ").length ?? 0}B] ${e.payload.data ?? ""}`);
          } else {
            push(setLogs, "rx", e.payload.from ? `[${e.payload.from}] ${e.payload.data ?? ""}` : e.payload.data ?? "");
          }
        })
      );
      un.push(await listen<{ id: string; url?: string; proto?: string }>("wstool://open", (e) => {
        setConnected(true);
        push(setLogs, "open", `连接已建立 (${e.payload.url ?? e.payload.id})`);
      }));
      un.push(await listen<{ id: string }>("wstool://closed", (e) => {
        setConnected(false);
        push(setLogs, "close", `连接已关闭 (${e.payload.id})`);
      }));
      un.push(await listen<{ id: string; raw: string }>("sstool://event", (e) => push(setLogs, "event", e.payload.raw)));
      un.push(await listen<{ id: string }>("sstool://open", (e) => {
        setConnected(true);
        push(setLogs, "open", `SSE 已订阅 (${e.payload.id})`);
      }));
      un.push(await listen<{ id: string }>("sstool://closed", (e) => {
        setConnected(false);
        push(setLogs, "close", `SSE 已断开 (${e.payload.id})`);
      }));
    })();
    return () => un.forEach((u) => u());
  }, []);

  useEffect(() => {
    if (logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [logs]);

  // 切换协议时重置连接状态与地址占位
  const switchProto = (p: Proto) => {
    setProto(p);
    setConnected(false);
    if (p === "ws") setUrl((u) => (u.startsWith("http") ? "ws://" : u));
    if (p === "sse") setUrl((u) => (u.startsWith("ws") ? "https://" : u));
  };

  const parseHeaders = useCallback((): string[][] | null => {
    const lines = headersText.split("\n").map((l) => l.trim()).filter(Boolean);
    if (lines.length === 0) return null;
    return lines.map((l) => l.split(":").map((s) => s.trim()));
  }, [headersText]);

  const doConnect = async () => {
    try {
      if (proto === "ws") {
        await invoke("ws_connect", { id: connId, url, headers: parseHeaders() });
      } else if (proto === "sse") {
        await invoke("sse_connect", { id: connId, url, headers: parseHeaders() });
      } else {
        await invoke("net_connect", { id: connId, protocol: proto, host, port: Number(port) });
      }
    } catch (e) {
      push(setLogs, "sys", String(e));
    }
  };
  const doDisconnect = async () => {
    try {
      await invoke("wstool_disconnect", { id: connId });
    } catch {
      /* 忽略 */
    }
    setConnected(false);
    push(setLogs, "close", "已主动断开");
  };
  const doSend = async () => {
    if (!sendData || !connected) return;
    try {
      if (proto === "ws") {
        await invoke("ws_send", { id: connId, data: sendData, hexMode: sendHex });
      } else if (proto === "tcp" || proto === "udp") {
        await invoke("net_send", { id: connId, data: sendData, hexMode: sendHex });
      } else {
        return; // SSE 单向
      }
      push(setLogs, "tx", sendHex ? `[HEX] ${sendData}` : sendData);
    } catch (e) {
      push(setLogs, "sys", String(e));
    }
  };

  const colorCls = (dir: LogEntry["dir"]) =>
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

  const canSend = proto === "ws" || proto === "tcp" || proto === "udp";

  return (
    <div className="h-full flex flex-col overflow-hidden p-4 gap-3">
      {/* 标题 + 协议切换 */}
      <div className="flex items-center gap-3 shrink-0">
        <Cable className="w-5 h-5 text-indigo-400" />
        <h1 className="text-lg font-semibold">网络调试器</h1>
        <span className={`text-xs px-2 py-0.5 rounded-full ${connected ? "bg-emerald-900/60 text-emerald-300" : "bg-slate-800 text-slate-400"}`}>
          {connected ? "已连接" : "未连接"}
        </span>
        <div className="ml-auto flex gap-1 bg-slate-900 rounded-md p-0.5">
          {PROTO_TABS.map(({ key, label }) => (
            <button
              key={key}
              onClick={() => switchProto(key)}
              className={`px-3 py-1 rounded text-sm transition-colors ${proto === key ? "bg-slate-700 text-white" : "text-slate-400 hover:text-slate-200"}`}
            >
              {label}
            </button>
          ))}
        </div>
      </div>

      {/* 连接配置 */}
      <div className="shrink-0 flex items-center gap-2 bg-slate-900/60 border border-slate-800 rounded-lg p-3">
        <input
          value={connId}
          onChange={(e) => setConnId(e.target.value)}
          className="w-24 bg-slate-800 border border-slate-700 rounded px-2 py-1.5 font-mono text-xs focus:outline-none focus:border-indigo-500"
          title="连接 ID（多开时区分）"
        />
        {(proto === "ws" || proto === "sse") && (
          <input
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder={proto === "ws" ? "ws://localhost:8080/path 或 wss://…" : "https://example.com/stream"}
            className="flex-1 bg-slate-800 border border-slate-700 rounded px-3 py-2 font-mono text-xs focus:outline-none focus:border-indigo-500"
          />
        )}
        {(proto === "tcp" || proto === "udp") && (
          <>
            <Network className="w-4 h-4 text-indigo-400 shrink-0" />
            <input
              value={host}
              onChange={(e) => setHost(e.target.value)}
              placeholder="主机，如 127.0.0.1"
              className="flex-1 min-w-32 bg-slate-800 border border-slate-700 rounded px-3 py-2 font-mono text-xs focus:outline-none focus:border-indigo-500"
            />
            <span className="text-slate-500 text-sm">:</span>
            <input
              value={port}
              onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))}
              placeholder="端口"
              className="w-24 bg-slate-800 border border-slate-700 rounded px-3 py-2 font-mono text-xs focus:outline-none focus:border-indigo-500"
            />
          </>
        )}
        {connected ? (
          <button onClick={doDisconnect} className="px-3 py-2 rounded-md bg-red-600 hover:bg-red-500 text-xs flex items-center gap-1">
            <Unplug className="w-3.5 h-3.5" /> 断开
          </button>
        ) : (
          <button
            onClick={doConnect}
            disabled={(proto === "ws" || proto === "sse") ? !url : !host || !port}
            className="px-3 py-2 rounded-md bg-indigo-600 hover:bg-indigo-500 disabled:opacity-40 text-xs flex items-center gap-1"
          >
            <PlugZap className="w-3.5 h-3.5" /> 连接
          </button>
        )}
      </div>

      {(proto === "ws" || proto === "sse") && (
        <input
          value={headersText}
          onChange={(e) => setHeadersText(e.target.value)}
          placeholder="附加请求头（可选，每行一条，格式 Key: Value）"
          className="shrink-0 bg-slate-800 border border-slate-700 rounded px-3 py-1.5 font-mono text-xs focus:outline-none focus:border-indigo-500"
        />
      )}

      {/* 日志 */}
      <div ref={logRef} className="flex-1 min-h-0 overflow-auto bg-slate-950 border border-slate-800 rounded-lg p-3 font-mono text-xs leading-relaxed">
        {logs.length === 0 && (
          <div className="flex flex-col items-center justify-center h-full text-slate-600 gap-2 select-none">
            <Network className="w-8 h-8 opacity-40" />
            <span>{proto === "udp" ? "等待 UDP 数据报…（发送一条消息即可收到回显）" : "暂无消息…"}</span>
          </div>
        )}
        {logs.map((entry, i) => (
          <div key={i} className="flex gap-2 break-all whitespace-pre-wrap">
            <span className="text-slate-600 shrink-0">{entry.time}</span>
            <span className={`shrink-0 ${colorCls(entry.dir)}`}>
              {entry.dir === "rx" ? "←" : entry.dir === "tx" ? "→" : entry.dir === "event" ? "◈" : "•"}
            </span>
            <span className={colorCls(entry.dir)}>{entry.text}</span>
          </div>
        ))}
      </div>

      {/* 发送区 */}
      {canSend && (
        <div className="shrink-0 flex items-center gap-2 bg-slate-900/60 border border-slate-800 rounded-lg p-3">
          <textarea
            value={sendData}
            onChange={(e) => setSendData(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                doSend();
              }
            }}
            rows={2}
            placeholder={sendHex ? "HEX 字节，如 01 A0 FF" : "要发送的文本…"}
            className="flex-1 bg-slate-950 border border-slate-700 rounded-md p-2 font-mono text-xs resize-none focus:outline-none focus:border-indigo-500"
          />
          <label className="flex items-center gap-1 text-xs cursor-pointer text-slate-300">
            <input type="checkbox" checked={sendHex} onChange={(e) => setSendHex(e.target.checked)} className="accent-indigo-500" /> HEX
          </label>
          <button onClick={doSend} disabled={!connected || !sendData} className="px-3 py-2 rounded-md bg-indigo-600 hover:bg-indigo-500 disabled:opacity-40 text-xs flex items-center gap-1">
            <Send className="w-3.5 h-3.5" /> 发送
          </button>
          <button onClick={() => setLogs([])} className="p-2 rounded hover:bg-slate-800 text-slate-400 cursor-pointer" title="清空日志">
            <Trash2 className="w-4 h-4" />
          </button>
        </div>
      )}
      {proto === "sse" && (
        <div className="shrink-0 flex items-center gap-2 text-xs text-slate-500">
          <Radio className="w-3.5 h-3.5" /> SSE 为单向订阅，服务端事件以原始格式展示在上方日志区。
        </div>
      )}
    </div>
  );
}
