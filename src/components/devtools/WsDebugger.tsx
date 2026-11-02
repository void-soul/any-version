import { btnPrimary, btnDanger } from "../shared/Button";
import { useWsDebugger } from "./useWsDebugger";
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
import { formatBytes, type LineEnding } from "./serialUtils";
import type { Dir, Proto } from "./wsDebugUtils";

const PROTO_TABS: Array<{ key: Proto; label: string }> = [
  { key: "ws", label: "WebSocket" },
  { key: "sse", label: "SSE" },
  { key: "tcp", label: "TCP" },
  { key: "udp", label: "UDP" },
];

/** WebSocket / SSE / TCP / UDP 调试器。 */
export default function WsDebugger() {
  const {
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
  } = useWsDebugger();
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
      {connectionError && <p role="alert" className="ui-note-error p-3">{connectionError}</p>}
      {/* 标题 + 状态 + 协议切换 */}
      <div className="flex flex-wrap items-center gap-3 shrink-0">
        <Cable className="w-4 h-4 text-indigo-400" />
        <h1 className="text-title font-semibold">{t("wsdebug.title")}</h1>
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
              type="button" aria-pressed={proto === key}
              onClick={() => void switchProto(key)}
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
      <div className="shrink-0 flex flex-wrap items-center gap-2 bg-white/[0.03] border border-white/10 rounded-ctl p-3">
        <select
          value={recentValue}
          onChange={(e) => void selectTarget(e.target.value)}
          disabled={locked}
          className="ui-input max-w-36 px-2 h-9 text-body text-slate-300 "
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
          className="ui-input w-24 px-2 h-8 font-mono text-body "
          title={t("wsdebug.connIdTitle")}
        />
        {(proto === "ws" || proto === "sse") && (
          <input
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            disabled={locked}
            placeholder={proto === "ws" ? t("wsdebug.wsPlaceholder") : t("wsdebug.ssePlaceholder")}
            className="ui-input flex-1 px-3 h-9 font-mono text-body "
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
              className="ui-input flex-1 min-w-32 px-3 h-9 font-mono text-body "
            />
            <span className="text-slate-500 text-sm">:</span>
            <input
              value={port}
              inputMode="numeric" onChange={(e) => setPort(e.target.value)}
              disabled={locked}
              placeholder={t("wsdebug.portPlaceholder")}
              className="ui-input w-24 px-3 h-9 font-mono text-body "
            />
          </>
        )}
        {connected || connecting ? (
          <button
            onClick={() => void doDisconnect(true)}
            className={btnDanger}
          >
            <Unplug className="w-3.5 h-3.5" /> {connecting ? t("common.cancel") : t("wsdebug.disconnect")}
          </button>
        ) : (
          <button
            onClick={() => void doConnect()}
            disabled={connecting || (proto === "ws" || proto === "sse" ? !url : !host || !port)}
            className={btnPrimary}
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
        <textarea rows={2} maxLength={65536} aria-label={t("wsdebug.headersPlaceholder")}
          value={headersText}
          onChange={(e) => setHeadersText(e.target.value)}
          disabled={locked}
          placeholder={t("wsdebug.headersPlaceholder")}
          className="ui-input shrink-0 px-3 py-2 min-h-16 max-h-48 font-mono text-body resize-y"
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
            className="ui-input w-40 px-2 py-0.5 text-tiny "
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
            className="ui-input flex-1 p-2 font-mono text-body resize-none "
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
            className="ui-input px-2 h-8 text-body text-slate-300 "
          >
            <option value="none">{t("wsdebug.leNone")}</option>
            <option value="lf">{t("wsdebug.leLf")}</option>
            <option value="crlf">{t("wsdebug.leCrlf")}</option>
            <option value="cr">{t("wsdebug.leCr")}</option>
          </select>
          <button
            onClick={() => void doSend()}
            disabled={!connected || !sendData || sending}
            className={btnPrimary}
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
