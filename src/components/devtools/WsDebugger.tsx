import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
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
  const { t } = useTranslation();
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
        await listen<{ id: string; kind?: string; data?: string; text?: string; hex?: string; from?: string }>("wstool://message", (e) => {
          if (e.payload.kind === "binary") {
            push(setLogs, "rx", `${t("wsdebug.binary", { bytes: e.payload.hex?.split(" ").length ?? 0 })}${e.payload.data ?? e.payload.text ?? ""}`);
          } else {
            push(setLogs, "rx", e.payload.from ? `${t("wsdebug.binaryPrefix", { from: e.payload.from })}${e.payload.data ?? e.payload.text ?? ""}` : e.payload.data ?? e.payload.text ?? "");
          }
        })
      );
      un.push(await listen<{ id: string; url?: string; proto?: string }>("wstool://open", (e) => {
        setConnected(true);
        push(setLogs, "open", t("wsdebug.connected", { url: e.payload.url ?? e.payload.id }));
      }));
      un.push(await listen<{ id: string }>("wstool://closed", (e) => {
        setConnected(false);
        push(setLogs, "close", t("wsdebug.closed", { id: e.payload.id }));
      }));
      un.push(await listen<{ id: string; raw: string }>("sstool://event", (e) => push(setLogs, "event", e.payload.raw)));
      un.push(await listen<{ id: string }>("sstool://open", (e) => {
        setConnected(true);
        push(setLogs, "open", t("wsdebug.sseSubscribed", { id: e.payload.id }));
      }));
      un.push(await listen<{ id: string }>("sstool://closed", (e) => {
        setConnected(false);
        push(setLogs, "close", t("wsdebug.sseDisconnected", { id: e.payload.id }));
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
    push(setLogs, "close", t("wsdebug.disconnectedByUser"));
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
    <div className="h-full flex flex-col overflow-hidden p-3 gap-2.5 text-[12px]">
      {/* 标题 + 协议切换 */}
      <div className="flex items-center gap-3 shrink-0">
        <Cable className="w-4 h-4 text-indigo-400" />
        <h1 className="text-base font-semibold">{t("wsdebug.title")}</h1>
        <span className={`text-xs px-2 py-0.5 rounded-full ${connected ? "bg-emerald-900/60 text-emerald-300" : "bg-slate-800 text-slate-400"}`}>
          {connected ? t("wsdebug.connectedState") : t("wsdebug.notConnected")}
        </span>
        <div className="ml-auto flex gap-1 rounded-md bg-slate-900 p-0.5 text-[11px]">
          {PROTO_TABS.map(({ key, label }) => (
            <button
              key={key}
              onClick={() => switchProto(key)}
              className={`rounded px-2.5 py-1 text-[11px] transition-colors ${proto === key ? "bg-slate-700 text-white" : "text-slate-400 hover:text-slate-200"}`}
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
          title={t("wsdebug.connIdTitle")}
        />
        {(proto === "ws" || proto === "sse") && (
          <input
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder={proto === "ws" ? t("wsdebug.wsPlaceholder") : t("wsdebug.ssePlaceholder")}
            className="flex-1 bg-slate-800 border border-slate-700 rounded px-3 py-2 font-mono text-xs focus:outline-none focus:border-indigo-500"
          />
        )}
        {(proto === "tcp" || proto === "udp") && (
          <>
            <Network className="w-4 h-4 text-indigo-400 shrink-0" />
            <input
              value={host}
              onChange={(e) => setHost(e.target.value)}
              placeholder={t("wsdebug.hostPlaceholder")}
              className="flex-1 min-w-32 bg-slate-800 border border-slate-700 rounded px-3 py-2 font-mono text-xs focus:outline-none focus:border-indigo-500"
            />
            <span className="text-slate-500 text-sm">:</span>
            <input
              value={port}
              onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))}
              placeholder={t("wsdebug.portPlaceholder")}
              className="w-24 bg-slate-800 border border-slate-700 rounded px-3 py-2 font-mono text-xs focus:outline-none focus:border-indigo-500"
            />
          </>
        )}
        {connected ? (
          <button onClick={doDisconnect} className="px-3 py-2 rounded-md bg-red-600 hover:bg-red-500 text-xs flex items-center gap-1">
            <Unplug className="w-3.5 h-3.5" /> {t("wsdebug.disconnect")}
          </button>
        ) : (
          <button
            onClick={doConnect}
            disabled={(proto === "ws" || proto === "sse") ? !url : !host || !port}
            className="px-3 py-2 rounded-md bg-indigo-600 hover:bg-indigo-500 disabled:opacity-40 text-xs flex items-center gap-1"
          >
            <PlugZap className="w-3.5 h-3.5" /> {t("wsdebug.connect")}
          </button>
        )}
      </div>

      {(proto === "ws" || proto === "sse") && (
        <input
          value={headersText}
          onChange={(e) => setHeadersText(e.target.value)}
          placeholder={t("wsdebug.headersPlaceholder")}
          className="shrink-0 bg-slate-800 border border-slate-700 rounded px-3 py-1.5 font-mono text-xs focus:outline-none focus:border-indigo-500"
        />
      )}

      {/* 日志 */}
      <div ref={logRef} className="flex-1 min-h-0 overflow-auto rounded-lg border border-slate-800 bg-slate-950 p-3 font-mono text-[12px] leading-5">
        {logs.length === 0 && (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-slate-600 select-none">
            <Network className="h-8 w-8 opacity-40" />
            <span>{proto === "udp" ? t("wsdebug.emptyUdp") : t("wsdebug.empty")}</span>
          </div>
        )}
        <div className="flex flex-col gap-2">
          {logs.map((entry, i) => {
            const system = entry.dir === "sys" || entry.dir === "open" || entry.dir === "close";
            const outgoing = entry.dir === "tx";
            return (
              <div key={i} className={`flex ${system ? "justify-center" : outgoing ? "justify-end" : "justify-start"}`}>
                <div className={`flex max-w-[86%] items-end gap-2 ${outgoing ? "flex-row-reverse" : ""}`}>
                  <span className="shrink-0 text-[10px] text-slate-600">{entry.time}</span>
                  <div className={`rounded-xl px-3 py-2 ${system ? "bg-slate-800/80" : outgoing ? "bg-indigo-500/15" : entry.dir === "event" ? "bg-violet-500/15" : "bg-emerald-500/10"} ${colorCls(entry.dir)}`}>
                    <span className="mr-1.5 text-[10px] opacity-70">{entry.dir === "rx" ? t("wsdebug.dirRx") : entry.dir === "tx" ? t("wsdebug.dirTx") : entry.dir === "event" ? t("wsdebug.dirEvent") : entry.dir === "open" ? t("wsdebug.dirOpen") : entry.dir === "close" ? t("wsdebug.dirClose") : t("wsdebug.dirSys")}</span>
                    <span className="break-all whitespace-pre-wrap">{entry.text}</span>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
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
            placeholder={sendHex ? t("wsdebug.sendHexPlaceholder") : t("wsdebug.sendTextPlaceholder")}
            className="flex-1 bg-slate-950 border border-slate-700 rounded-md p-2 font-mono text-xs resize-none focus:outline-none focus:border-indigo-500"
          />
          <label className="flex items-center gap-1 text-xs cursor-pointer text-slate-300">
            <input type="checkbox" checked={sendHex} onChange={(e) => setSendHex(e.target.checked)} className="accent-indigo-500" /> HEX
          </label>
          <button onClick={doSend} disabled={!connected || !sendData} className="px-3 py-2 rounded-md bg-indigo-600 hover:bg-indigo-500 disabled:opacity-40 text-xs flex items-center gap-1">
            <Send className="w-3.5 h-3.5" /> {t("wsdebug.send")}
          </button>
          <button onClick={() => setLogs([])} className="p-2 rounded hover:bg-slate-800 text-slate-400 cursor-pointer" title={t("wsdebug.clearLog")}>
            <Trash2 className="w-4 h-4" />
          </button>
        </div>
      )}
      {proto === "sse" && (
        <div className="shrink-0 flex items-center gap-2 text-xs text-slate-500">
          <Radio className="w-3.5 h-3.5" /> {t("wsdebug.sseNote")}
        </div>
      )}
    </div>
  );
}
