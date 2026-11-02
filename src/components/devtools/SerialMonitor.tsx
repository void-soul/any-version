import { btnPrimary, btnDanger, btnSecondary } from "../shared/Button";
import { useSerialMonitor } from "./useSerialMonitor";
import { SerialRuleEditor } from "./SerialRuleEditor";
import { Usb, RefreshCw, Play, Square, Send, Trash2, Timer, FileUp, ListOrdered, Pause, Bot, Cable, Plus, Download, Upload, History, Filter } from "lucide-react";
import { emptySimRule, formatBytes, portMeta, type ChecksumAlgo, type Encoding, type LineEnding } from "./serialUtils";

/** 串口调试器：真实串口（连接/编码/HEX 收发/逐行/定时发送）+ 模拟设备（脚本应答）。 */
export default function SerialMonitor() {
  const {
    connectionBusy, sending, sendingBatch,
    t,
    mode,
    ports,
    portName,
    setPortName,
    baud,
    setBaud,
    dataBits,
    setDataBits,
    parity,
    setParity,
    stopBits,
    setStopBits,
    flow,
    setFlow,
    dtr,
    rts,
    open,
    simActive,
    logs,
    pendingCount,
    paused,
    filter,
    setFilter,
    hexView,
    setHexView,
    showTime,
    setShowTime,
    autoScroll,
    setAutoScroll,
    encoding,
    setEncoding,
    lineEnding,
    setLineEnding,
    checksum,
    setChecksum,
    sendText,
    setSendText,
    sendHex,
    setSendHex,
    history,
    stats,
    rates,
    logRef,
    rules,
    setRules,
    active,
    lines,
    lineIdx,
    setLineIdx,
    intervalMs,
    setIntervalMs,
    cycling,
    refreshPorts,
    switchMode,
    doOpen,
    doClose,
    toggleSignal,
    updateRule,
    removeRule,
    toggleSim,
    exportRules,
    importRules,
    doSend,
    sendAllLines,
    toggleCycle,
    importLinesFile,
    togglePause,
    clearLogs,
    exportLog,
    visibleLogs,
    checksumPreview,
  } = useSerialMonitor();
  const selectCls = "ui-input h-8 px-2";
  const inputCls = "ui-input h-8 px-2 font-mono";

  return (
    <div className="h-full flex flex-col overflow-hidden p-3 gap-2.5 text-body">
      <div className="flex flex-wrap items-center gap-2 shrink-0">
        <Usb className="w-4 h-4 text-teal-400" />
        <h1 className="text-title font-semibold">{t("serial.title")}</h1>
        {/* 模式切换 */}
        <div className="flex rounded-ctl overflow-hidden border border-white/10 text-caption">
          <button
            type="button" disabled={connectionBusy || sendingBatch} aria-pressed={mode === "real"}
            onClick={() => void switchMode("real")}
            className={`px-3 py-1 flex items-center gap-1 cursor-pointer ${mode === "real" ? "bg-[var(--module-accent)] text-white" : "bg-white/5 text-slate-400 hover:text-slate-200"}`}
          >
            <Cable className="w-3.5 h-3.5" /> {t("serial.modeReal")}
          </button>
          <button
            type="button" disabled={connectionBusy || sendingBatch} aria-pressed={mode === "sim"}
            onClick={() => void switchMode("sim")}
            className={`px-3 py-1 flex items-center gap-1 cursor-pointer ${mode === "sim" ? "bg-[var(--module-accent)] text-white" : "bg-white/5 text-slate-400 hover:text-slate-200"}`}
          >
            <Bot className="w-3.5 h-3.5" /> {t("serial.modeSim")}
          </button>
        </div>
        <span
          className={`text-caption px-2 py-0.5 rounded-full ${
            active
              ? mode === "sim"
                ? "bg-violet-900/60 text-violet-300"
                : "bg-emerald-900/60 text-emerald-300"
              : "bg-white/5 text-slate-400"
          }`}
        >
          {connectionBusy ? t("common.loading") : !active ? t("serial.statusIdle") : mode === "sim" ? t("serial.statusSim") : t("serial.statusReal")}
        </span>
      </div>

      {/* 连接配置（真实串口）/ 应答脚本编辑器（模拟设备） */}
      {mode === "real" ? (
        <div className="shrink-0 flex flex-wrap items-center gap-2 bg-white/[0.03] border border-white/10 rounded-ctl p-3">
          <select disabled={open || connectionBusy} aria-label={t("serial.modeReal")} value={portName} onChange={(e) => setPortName(e.target.value)} className={`${selectCls} min-w-44`}>
            {ports.length === 0 && <option value="">{t("serial.noPorts")}</option>}
            {ports.map((p) => (
              <option key={p.name} value={p.name} title={portMeta(p)}>
                {p.name} · {p.description}
              </option>
            ))}
          </select>
          <button disabled={open || connectionBusy} onClick={() => void refreshPorts()} title={t("serial.refreshPorts")} className="p-1.5 rounded hover:bg-white/10 text-slate-400 cursor-pointer">
            <RefreshCw className="w-4 h-4" />
          </button>
          <label className="flex items-center gap-1 text-body text-slate-400">
            {t("serial.baud")}
            <input
              type="number"
              min={50}
              step={100}
              list="serial-baud-presets"
              disabled={open || connectionBusy}
              value={baud}
              onChange={(e) => setBaud(Number(e.target.value) || 0)}
              className={`${inputCls} w-24`}
            />
            <datalist id="serial-baud-presets">
              {[9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600].map((b) => (
                <option key={b} value={b} />
              ))}
            </datalist>
          </label>
          <label className="flex items-center gap-1 text-body text-slate-400">
            {t("serial.dataBits")}
            <select disabled={open || connectionBusy} value={dataBits} onChange={(e) => setDataBits(Number(e.target.value))} className={selectCls}>
              {[8, 7, 6, 5].map((v) => (
                <option key={v} value={v}>{v}</option>
              ))}
            </select>
          </label>
          <label className="flex items-center gap-1 text-body text-slate-400">
            {t("serial.parity")}
            <select disabled={open || connectionBusy} value={parity} onChange={(e) => setParity(e.target.value as typeof parity)} className={selectCls}>
              <option value="none">None</option>
              <option value="even">Even</option>
              <option value="odd">Odd</option>
            </select>
          </label>
          <label className="flex items-center gap-1 text-body text-slate-400">
            {t("serial.stopBits")}
            <select disabled={open || connectionBusy} value={stopBits} onChange={(e) => setStopBits(Number(e.target.value))} className={selectCls}>
              {[1, 2].map((v) => (
                <option key={v} value={v}>{v}</option>
              ))}
            </select>
          </label>
          <label className="flex items-center gap-1 text-body text-slate-400">
            {t("serial.flow")}
            <select disabled={open || connectionBusy} value={flow} onChange={(e) => setFlow(e.target.value as typeof flow)} className={selectCls}>
              <option value="none">{t("serial.flowNone")}</option>
              <option value="hardware">{t("serial.flowHw")}</option>
              <option value="software">{t("serial.flowSw")}</option>
            </select>
          </label>
          {open && (
            <div className="flex items-center gap-1" title={t("serial.signalTip")}>
              <button
                disabled={connectionBusy}
                onClick={() => void toggleSignal("dtr", !dtr)}
                title={`${t("serial.signalTip")} · ${t("serial.dtr")}`}
                className={`px-2 py-1 rounded text-caption border cursor-pointer ${
                  dtr
                    ? "bg-teal-600/20 border-teal-500/40 text-teal-300"
                    : "border-white/10 text-slate-500 hover:text-slate-300"
                }`}
              >
                {t("serial.dtr")}
              </button>
              <button
                onClick={() => void toggleSignal("rts", !rts)}
                disabled={connectionBusy || flow === "hardware"}
                title={flow === "hardware" ? t("serial.rtsFlowLocked") : `${t("serial.signalTip")} · ${t("serial.rts")}`}
                className={`px-2 py-1 rounded text-caption border cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed ${
                  rts
                    ? "bg-teal-600/20 border-teal-500/40 text-teal-300"
                    : "border-white/10 text-slate-500 hover:text-slate-300"
                }`}
              >
                {t("serial.rts")}
              </button>
            </div>
          )}
          {open ? (
            <button disabled={connectionBusy} onClick={() => void doClose()} className={`${btnDanger} ml-auto`}>
              <Square className="w-3.5 h-3.5" /> {t("serial.disconnect")}
            </button>
          ) : (
            <button
              onClick={() => void doOpen()}
              disabled={connectionBusy || !portName || !Number.isInteger(baud) || baud < 50}
              className={`${btnPrimary} ml-auto`}
            >
              <Play className="w-3.5 h-3.5" /> {t("serial.connect")}
            </button>
          )}
        </div>
      ) : (
        <div className="shrink-0 bg-white/[0.03] border border-[var(--module-accent-ring)] rounded-ctl p-3 space-y-2">
          <div className="flex items-center gap-2 text-body text-slate-400">
            <Bot className="w-4 h-4 text-[var(--module-accent)]" />
            <span>
              <span dangerouslySetInnerHTML={{ __html: t("serial.simDesc") }} />
            </span>
            <div className="ml-auto flex items-center gap-2">
              <button
                onClick={() => void importRules()}
                title={t("serial.importRules")}
                className="p-1.5 rounded hover:bg-white/10 text-slate-400 cursor-pointer"
              >
                <Upload className="w-4 h-4" />
              </button>
              <button
                onClick={() => void exportRules()}
                title={t("serial.exportRules")}
                className="p-1.5 rounded hover:bg-white/10 text-slate-400 cursor-pointer"
              >
                <Download className="w-4 h-4" />
              </button>
              <button
                onClick={() => setRules((rs) => [...rs, emptySimRule()])}
                className="px-2 py-1 rounded bg-slate-700 hover:bg-slate-600 flex items-center gap-1 cursor-pointer"
              >
                <Plus className="w-3.5 h-3.5" /> {t("serial.addRule")}
              </button>
              {simActive ? (
                <button disabled={connectionBusy} onClick={() => void toggleSim()} className={btnDanger}>
                  <Square className="w-3.5 h-3.5" /> {t("serial.stopDevice")}
                </button>
              ) : (
                <button disabled={connectionBusy} onClick={() => void toggleSim()} className={btnPrimary}>
                  <Play className="w-3.5 h-3.5" /> {t("serial.startDevice")}
                </button>
              )}
            </div>
          </div>
          {/* 表头 */}
          {rules.length > 0 && (
            <div className="grid grid-cols-[7rem_1fr_9rem_5rem_2rem] gap-2 text-caption text-slate-500 px-0.5">
              <span>{t("serial.colMatch")}</span>
              <span>{t("serial.colRecv")}</span>
              <span>{t("serial.colResp")}</span>
              <span>{t("serial.colDelay")}</span>
              <span />
            </div>
          )}
          <SerialRuleEditor
            rules={rules}
            updateRule={updateRule}
            selectCls={selectCls}
            inputCls={inputCls}
            removeRule={removeRule}
          />
        </div>
      )}

      {/* 收发日志（含统计 / 过滤 / 导出） */}
      <div className="flex-1 min-h-0 flex flex-col rounded-ctl border border-white/10 bg-white/[0.02] overflow-hidden">
        <div className="shrink-0 flex flex-wrap items-center gap-x-3 gap-y-1.5 px-3 py-1.5 border-b border-white/10 text-caption text-slate-500">
          <span className="text-emerald-400/80">
            {t("serial.statRx", { frames: stats.rxFrames, size: formatBytes(stats.rxBytes) })}
          </span>
          <span className="text-cyan-400/80">
            {t("serial.statTx", { frames: stats.txFrames, size: formatBytes(stats.txBytes) })}
          </span>
          <span>{t("serial.statRate", { rx: formatBytes(rates.rx), tx: formatBytes(rates.tx) })}</span>
          <span className="w-px h-3 bg-white/10" />
          <label className="flex items-center gap-1 cursor-pointer hover:text-slate-300">
            <input type="checkbox" checked={showTime} onChange={(e) => setShowTime(e.target.checked)} className="accent-teal-500" />
            {t("serial.showTime")}
          </label>
          <label className="flex items-center gap-1 cursor-pointer hover:text-slate-300">
            <input type="checkbox" checked={autoScroll} onChange={(e) => setAutoScroll(e.target.checked)} className="accent-teal-500" />
            {t("serial.autoScroll")}
          </label>
          <label className="flex items-center gap-1 cursor-pointer hover:text-slate-300">
            <input type="checkbox" checked={hexView} onChange={(e) => setHexView(e.target.checked)} className="accent-teal-500" />
            {t("serial.hexView")}
          </label>
          <button
            onClick={togglePause}
            className={`flex items-center gap-1 cursor-pointer hover:text-slate-300 ${paused ? "text-amber-300" : ""}`}
            title={paused ? t("serial.resumeReceive") : t("serial.pauseReceive")}
          >
            {paused ? <Play className="w-3.5 h-3.5" /> : <Pause className="w-3.5 h-3.5" />}
            {paused ? t("serial.resumeReceive") : t("serial.pauseReceive")}
            {paused && pendingCount > 0 && (
              <span className="px-1 rounded bg-amber-500/20 text-amber-300">{pendingCount}</span>
            )}
          </button>

          <div className="ml-auto flex items-center gap-2">
            <div className="relative">
              <Filter className="absolute left-1.5 top-1/2 -translate-y-1/2 w-3 h-3 text-slate-600" />
              <input
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
                placeholder={t("serial.filterPlaceholder")}
                aria-label={t("serial.filterPlaceholder")} className="ui-input h-8 w-36 pl-6 pr-2"
              />
            </div>
            <button onClick={() => void exportLog()} className="flex items-center gap-1 hover:text-slate-300 cursor-pointer">
              <Download className="w-3.5 h-3.5" /> {t("serial.exportLog")}
            </button>
            <button onClick={clearLogs} className="flex items-center gap-1 hover:text-slate-300 cursor-pointer">
              <Trash2 className="w-3.5 h-3.5" /> {t("serial.clearLog")}
            </button>
          </div>
        </div>

        <div ref={logRef} className="flex-1 min-h-0 overflow-auto p-3 font-mono text-body leading-5">
          {logs.length === 0 && (
            <div className="h-full flex flex-col items-center justify-center text-slate-600 gap-2 select-none">
              {mode === "sim" ? <Bot className="w-8 h-8 opacity-40" /> : <Usb className="w-8 h-8 opacity-40" />}
              <span>{mode === "sim" ? t("serial.logHintSim") : t("serial.logHintReal")}</span>
            </div>
          )}
          {logs.length > 0 && visibleLogs.length === 0 && (
            <div className="h-full flex items-center justify-center text-slate-600 select-none">
              {t("serial.filterEmpty")}
            </div>
          )}
          <div className="flex flex-col gap-2">
            {visibleLogs.map((entry) => {
              const system = entry.dir === "sys";
              const outgoing = entry.dir === "tx";
              return (
                <div key={entry.id} className={`flex ${system ? "justify-center" : outgoing ? "justify-end" : "justify-start"}`}>
                  <div className={`flex max-w-[86%] items-end gap-2 ${outgoing ? "flex-row-reverse" : ""}`}>
                    {showTime && <span className="shrink-0 text-tiny text-slate-600">{entry.time}</span>}
                    <div className={`rounded-card px-3 py-2 ${system ? "bg-white/[0.06] text-yellow-200" : outgoing ? "bg-cyan-500/15 text-cyan-100" : entry.dir === "dev" ? "bg-[var(--module-accent-soft)] text-[var(--module-accent)]" : "bg-emerald-500/10 text-slate-200"}`}>
                      <span className="mr-1.5 text-tiny opacity-70">{entry.dir === "rx" ? t("serial.dirDevice") : entry.dir === "tx" ? t("serial.dirSend") : entry.dir === "dev" ? t("serial.dirResp") : t("serial.dirSys")}</span>
                      <span className="break-all whitespace-pre-wrap">{hexView && entry.hex !== undefined ? entry.hex : entry.text}</span>
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </div>

      {/* 发送区 */}
      <div className="shrink-0 bg-white/[0.03] border border-white/10 rounded-ctl p-3 space-y-2">
        {/* 行游标提示 */}
        {lines.length > 1 && (
          <div className="flex items-center justify-between text-caption text-slate-500">
            <span className="flex items-center gap-1">
              <ListOrdered className="w-3.5 h-3.5" />
              {t("serial.cursorInfo", { lines: lines.length, idx: Math.min(lineIdx + 1, lines.length) })}
              <code className="px-1 rounded bg-white/5 text-teal-300 truncate max-w-64 inline-block">{lines[Math.min(lineIdx, lines.length - 1)]}</code>
            </span>
            <button onClick={() => setLineIdx(0)} className="hover:text-slate-300 cursor-pointer">{t("serial.resetCursor")}</button>
          </div>
        )}
        <textarea
          disabled={sendingBatch}
          value={sendText}
          onChange={(e) => setSendText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.ctrlKey && !e.shiftKey) {
              e.preventDefault();
              void doSend();
            }
          }}
          placeholder={sendHex ? t("serial.hexPlaceholder") : t("serial.textPlaceholder")}
          rows={3}
          className="w-full bg-black/30 border border-white/10 rounded-md p-2 font-mono text-body resize-none focus:outline-none focus:border-teal-500"
        />
        <div className="flex flex-wrap items-center gap-2 text-body text-slate-300">
          <label className="flex items-center gap-1 cursor-pointer">
            <input type="checkbox" checked={sendHex} onChange={(e) => setSendHex(e.target.checked)} className="accent-teal-500" /> HEX
          </label>
          <label className="flex items-center gap-1 text-slate-400" title={t("serial.lineEndingTip")}>
            {t("serial.lineEnding")}
            <select
              value={lineEnding}
              onChange={(e) => setLineEnding(e.target.value as LineEnding)}
              disabled={sendHex}
              className={`${selectCls} disabled:opacity-40`}
            >
              <option value="none">{t("serial.leNone")}</option>
              <option value="lf">{t("serial.leLf")}</option>
              <option value="crlf">{t("serial.leCrlf")}</option>
              <option value="cr">{t("serial.leCr")}</option>
            </select>
          </label>
          <label className="flex items-center gap-1 text-slate-400">
            {t("serial.encoding")}
            <select
              disabled={sendingBatch}
              value={encoding}
              onChange={(e) => setEncoding(e.target.value as Encoding)}
              className={selectCls}
            >
              <option value="utf-8">{t("serial.encUtf8")}</option>
              <option value="gbk">{t("serial.encGbk")}</option>
            </select>
          </label>
          <label className="flex items-center gap-1 text-slate-400" title={t("serial.csTip")}>
            {t("serial.checksum")}
            <select
              value={checksum}
              onChange={(e) => setChecksum(e.target.value as ChecksumAlgo)}
              disabled={!sendHex}
              className={`${selectCls} disabled:opacity-40`}
            >
              <option value="none">{t("serial.csNone")}</option>
              <option value="crc16">{t("serial.csCrc16")}</option>
              <option value="xor">{t("serial.csXor")}</option>
              <option value="sum">{t("serial.csSum")}</option>
            </select>
            {checksumPreview && (
              <span className="text-caption text-teal-400" title={t("serial.csTip")}>
                {t("serial.csValue", { hex: checksumPreview.checksumHex })}
              </span>
            )}
          </label>

          <button onClick={() => void doSend()} disabled={!active || !sendText || connectionBusy || sending || sendingBatch} className={btnPrimary}>
            <Send className="w-3.5 h-3.5" /> {t("serial.send")}
          </button>

          {/* 多行工具 */}
          <div className="flex items-center gap-1 ml-auto">
            <History className="w-3.5 h-3.5 text-slate-500" />
            <select
              value=""
              onChange={(e) => {
                if (e.target.value) setSendText(e.target.value);
              }}
              title={history.length ? t("serial.history") : t("serial.historyEmpty")}
              className={`${selectCls} max-w-40`}
            >
              <option value="">{history.length ? t("serial.history") : t("serial.historyEmpty")}</option>
              {history.map((h, i) => (
                <option key={i} value={h}>
                  {h.length > 60 ? `${h.slice(0, 60)}…` : h}
                </option>
              ))}
            </select>
            <button
              onClick={() => void importLinesFile()}
              title={t("serial.importTitle")}
              className="p-1.5 rounded hover:bg-white/10 text-slate-400 cursor-pointer"
            >
              <FileUp className="w-4 h-4" />
            </button>
            <button
              onClick={() => void sendAllLines()}
              disabled={!active || lines.length < 2 || sending || sendingBatch || connectionBusy}
              title={t("serial.sendAllTitle")}
              className={btnSecondary}
            >
              <ListOrdered className="w-3.5 h-3.5" /> {t("serial.sendAll")}
            </button>
            <label className="flex items-center gap-1 text-slate-400" title={t("serial.cycleTip")}>
              <Timer className="w-3.5 h-3.5" />
              <input
                type="number"
                min={50}
                step={100}
                value={intervalMs}
                onChange={(e) => setIntervalMs(Number(e.target.value))}
                disabled={cycling || sendingBatch}
                className="w-16 bg-black/30 border border-white/10 rounded px-1.5 py-1"
              />
              ms
            </label>
            <button
              onClick={toggleCycle}
              disabled={!active || lines.length < 1}
              title={t("serial.cycleTitle")}
              className={`px-2 py-1.5 rounded disabled:opacity-40 flex items-center gap-1 cursor-pointer ${
                cycling ? "bg-red-600 hover:bg-red-500" : "bg-slate-700 hover:bg-slate-600"
              }`}
            >
              {cycling ? <Pause className="w-3.5 h-3.5" /> : <Play className="w-3.5 h-3.5" />}
              {sendingBatch ? t("common.cancel") : cycling ? t("serial.cycleStop") : t("serial.cycleStart")}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
