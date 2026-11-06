import { ignoreInvokeError } from "../../utils/invokeOrReport";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";

import { applyChecksum, buildExportText, decodeBase64, emptySimRule, loadHistory, loadPrefs, loadSimRules, normalizeRules, nowStamp, pushHistory, saveHistory, savePrefs, saveSimRules, splitSendLines, type ChecksumAlgo, type Direction, type Encoding, type LineEnding, type LogEntry, type PortInfo, type SimRule } from "./serialUtils";
import { trimSerialLogs } from "./serialLogBudget";
import { MAX_LOGS, MAX_PENDING, normalizeHex, type SerialPayload, type SimPayload } from "./SerialMonitor.model";


export function useSerialMonitor() {
  const { t } = useTranslation();
  const [initialPrefs] = useState(loadPrefs);
  const [connectionBusy, setConnectionBusy] = useState(false);
  const [sending, setSending] = useState(false);
  const [sendingBatch, setSendingBatch] = useState(false);
  const alive = useRef(false);
  const life = useRef(0);
  const connectionGate = useRef(false);
  const sendGate = useRef(false);
  const batchGate = useRef(false);
  const sendGeneration = useRef(0);
  const portOwner = useRef<{ portName: string; sessionId: string } | null>(null);
  const simOwner = useRef<string | null>(null);
  useEffect(() => { alive.current = true; return () => { alive.current = false; life.current++; sendGeneration.current++; }; }, []);

  const [mode, setMode] = useState<"real" | "sim">("real");
  const [ports, setPorts] = useState<PortInfo[]>([]);
  const [portName, setPortName] = useState("");
  const [baud, setBaud] = useState(initialPrefs.baud);
  const [dataBits, setDataBits] = useState(initialPrefs.dataBits);
  const [parity, setParity] = useState<"none" | "even" | "odd">(initialPrefs.parity);
  const [stopBits, setStopBits] = useState(initialPrefs.stopBits);
  const [flow, setFlow] = useState<"none" | "hardware" | "software">(initialPrefs.flow);
  const [dtr, setDtr] = useState(true);
  const [rts, setRts] = useState(true);

  const [open, setOpen] = useState(false); // 真实串口已打开
  const [simActive, setSimActive] = useState(false); // 模拟设备运行中
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [pendingCount, setPendingCount] = useState(0);
  const [paused, setPaused] = useState(false);
  const [filter, setFilter] = useState("");

  const [hexView, setHexView] = useState(initialPrefs.hexView);
  const [showTime, setShowTime] = useState(initialPrefs.showTime);
  const [autoScroll, setAutoScroll] = useState(initialPrefs.autoScroll);
  const [encoding, setEncoding] = useState<Encoding>(initialPrefs.encoding);
  const [lineEnding, setLineEnding] = useState<LineEnding>(initialPrefs.lineEnding);
  const [checksum, setChecksum] = useState<ChecksumAlgo>(initialPrefs.checksum);

  const [sendText, setSendText] = useState("");
  const [sendHex, setSendHex] = useState(false);
  const [history, setHistory] = useState<string[]>(loadHistory);
  const [stats, setStats] = useState({ rxFrames: 0, rxBytes: 0, txFrames: 0, txBytes: 0 });
  const [rates, setRates] = useState({ rx: 0, tx: 0 });

  const logRef = useRef<HTMLDivElement>(null);
  const idRef = useRef(0);

  // ── 模拟设备规则 ──
  const [rules, setRules] = useState<SimRule[]>(() => loadSimRules() ?? [emptySimRule()]);
  const active = mode === "real" ? open : simActive;

  // ── 多行逐行 / 定时发送 ──
  const lines = useMemo(() => splitSendLines(sendText), [sendText]);
  const linesRef = useRef(lines);
  linesRef.current = lines;
  const [lineIdx, setLineIdx] = useState(0);
  const [intervalMs, setIntervalMs] = useState(1000);
  const [cycling, setCycling] = useState(false);
  const cycleTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // 卸载清理 / 定时回调要用**最新**的值：只挂载一次的 effect 闭包只会捕获首帧状态
  const portNameRef = useRef(portName);
  const openRef = useRef(open);
  const simActiveRef = useRef(simActive);
  const pausedRef = useRef(false);
  const encodingRef = useRef<Encoding>(encoding);
  const tRef = useRef(t);
  const pendingRef = useRef<LogEntry[]>([]);
  const lineIdxRef = useRef(0);
  const totalsRef = useRef({ rx: 0, tx: 0 });
  const lastTotalsRef = useRef({ rx: 0, tx: 0 });
  const writeOneRef = useRef<(line: string) => Promise<boolean>>(async () => false);

  useEffect(() => {
    portNameRef.current = portName;
    openRef.current = open;
    simActiveRef.current = simActive;
    encodingRef.current = encoding;
    tRef.current = t;
    lineIdxRef.current = lineIdx;
  }, [portName, open, simActive, encoding, t, lineIdx]);

  // 设置持久化：下次进入模块保持上次的串口参数与显示偏好
  useEffect(() => {
    savePrefs({
      baud,
      dataBits,
      parity,
      stopBits,
      flow,
      encoding,
      lineEnding,
      hexView,
      showTime,
      autoScroll,
      checksum,
    });
  }, [baud, dataBits, parity, stopBits, flow, encoding, lineEnding, hexView, showTime, autoScroll, checksum]);
  useEffect(() => saveSimRules(rules), [rules]);
  useEffect(() => saveHistory(history), [history]);

  // 运行中编辑规则：防抖同步到后端，无需重启模拟设备
  useEffect(() => {
    if (!simActive) return;
    const timer = setTimeout(() => {
      void invoke("serial_sim_update_rules", { rules, sessionId: simOwner.current }).catch((error) => {
        addLog("sys", t("serial.rulesUpdateFail", { err: String(error) }));
      });
    }, 400);
    return () => clearTimeout(timer);
  }, [rules, simActive]);

  const recordLog = (dir: Direction, text: string, hex?: string, bytes = 0) => {
    if (!alive.current) return;
    const entry: LogEntry = { id: ++idRef.current, dir, text, hex, bytes, time: nowStamp() };
    if (pausedRef.current) {
      const pending = trimSerialLogs([...pendingRef.current, entry], MAX_PENDING, 4 * 1024 * 1024);
      pendingRef.current = pending;
      setPendingCount(pending.length);
    } else {
      setLogs((prev) => trimSerialLogs([...prev, entry], MAX_LOGS));
    }

    if (dir !== "sys" && bytes > 0) {
      const isRx = dir === "rx" || dir === "dev";
      if (isRx) totalsRef.current.rx += bytes;
      else totalsRef.current.tx += bytes;
      setStats((s) => ({
        rxFrames: s.rxFrames + (isRx ? 1 : 0),
        rxBytes: s.rxBytes + (isRx ? bytes : 0),
        txFrames: s.txFrames + (isRx ? 0 : 1),
        txBytes: s.txBytes + (isRx ? 0 : bytes),
      }));
    }
  };
  const addLog = (dir: Direction, text: string, hex?: string, bytes = 0) =>
    recordLog(dir, text, hex, bytes);

  // 每秒刷新一次收发速率
  useEffect(() => {
    const timer = setInterval(() => {
      const nowTotals = totalsRef.current;
      setRates({
        rx: nowTotals.rx - lastTotalsRef.current.rx,
        tx: nowTotals.tx - lastTotalsRef.current.tx,
      });
      lastTotalsRef.current = { ...nowTotals };
    }, 1000);
    return () => clearInterval(timer);
  }, []);

  const refreshPorts = async () => {
    try {
      const list = await invoke<PortInfo[]>("serial_list_ports");
      if (!alive.current) return;
      setPorts(list);
      // 保留用户已选端口；端口被拔出时自动回落到第一个可用端口
      setPortName((cur) => (openRef.current ? cur : cur && list.some((p) => p.name === cur) ? cur : list[0]?.name ?? ""));
    } catch (e) {
      addLog("sys", t("serial.sysEnumFail", { err: String(e) }));
    }
  };

  useEffect(() => {
    void refreshPorts();
    let unlisteners: Array<() => void> = [];
    let cancelled = false;
    (async () => {
      // 卸载可能发生在 `await listen(...)` 返回**之前** —— 那时 unlisteners 还是空的，
      // 监听器就永久泄漏，之后还会对已卸载组件 setState。所以 resolve 时若已卸载，就地注销。
      const add = (fn: () => void) => {
        if (cancelled) fn();
        else unlisteners.push(fn);
      };
      add(
        await listen<SerialPayload>("serial://data", (e) => {
          if (cancelled || e.payload.port !== portNameRef.current || (e.payload.sessionId && e.payload.sessionId !== portOwner.current?.sessionId)) return;
          const text = e.payload.data
            ? decodeBase64(e.payload.data, encodingRef.current)
            : e.payload.text;
          recordLog("rx", text, e.payload.hex, e.payload.len);
        })
      );
      // 模拟设备事件：dir 是设备视角 —— rx=设备收到（用户发出），tx=设备应答
      add(
        await listen<SimPayload>("serial://sim-data", (e) => {
          if (cancelled || (e.payload.sessionId && e.payload.sessionId !== simOwner.current)) return;
          const dir: Direction = e.payload.dir === "tx" ? "dev" : "tx";
          const text = e.payload.data
            ? decodeBase64(e.payload.data, encodingRef.current)
            : e.payload.text;
          recordLog(dir, text, e.payload.hex, e.payload.len);
        })
      );
      add(
        await listen<{ port: string; reason?: string | null; sessionId?: string | null }>("serial://closed", (e) => {
          if (cancelled || !openRef.current || e.payload.port !== portNameRef.current || (e.payload.sessionId && e.payload.sessionId !== portOwner.current?.sessionId)) return;
          openRef.current = false; portOwner.current = null;
          setOpen(false);
          stopCycle();
          if (e.payload.reason) {
            recordLog("sys", tRef.current("serial.sysReadError", { err: e.payload.reason }));
          } else {
            recordLog("sys", tRef.current("serial.sysPortClosed"));
          }
        })
      );
    })().catch((error) => { if (!cancelled) addLog("sys", String(error)); });
    return () => {
      cancelled = true;
      unlisteners.forEach((u) => u());
      if (cycleTimerRef.current) clearInterval(cycleTimerRef.current);
      // **卸载时主动关串口/停模拟设备**：只注销事件的话，后端的读线程与句柄会一直活着
      //（端口保持被占用、持续 emit），而重进本页时按钮显示「未连接」。
      if (openRef.current) {
        void invoke("serial_close", portOwner.current ?? { portName: portNameRef.current }).catch((e) => ignoreInvokeError(e, "卸载路径，失败无可上报"));
      }
      openRef.current = false; portOwner.current = null; setOpen(false);
      if (simActiveRef.current) {
        void invoke("serial_sim_stop", { sessionId: simOwner.current }).catch((e) => ignoreInvokeError(e, "卸载路径，失败无可上报"));
      }
      simActiveRef.current = false; simOwner.current = null; setSimActive(false);
      pendingRef.current = []; setPendingCount(0);
      setCycling(false);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 自动滚动到底部
  useEffect(() => {
    if (!autoScroll || paused) return;
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [logs, autoScroll, paused, filter]);

  // ── 模式切换：互斥关闭另一侧 ──
  const switchMode = async (next: "real" | "sim") => {
    if (next === mode || connectionGate.current) return;
    connectionGate.current = true; setConnectionBusy(true); stopCycle();
    try {
      if (openRef.current) { await invoke("serial_close", portOwner.current ?? { portName }); openRef.current = false; portOwner.current = null; setOpen(false); }
      if (simActiveRef.current) { await invoke("serial_sim_stop", { sessionId: simOwner.current }); simActiveRef.current = false; simOwner.current = null; setSimActive(false); }
      if (alive.current) { setMode(next); addLog("sys", next === "sim" ? t("serial.sysSwitchSim") : t("serial.sysSwitchReal")); }
    } catch (error) { addLog("sys", String(error)); }
    finally { connectionGate.current = false; setConnectionBusy(false); }
  };

  const doOpen = async () => {
    if (connectionGate.current || openRef.current || !portName || !Number.isInteger(baud) || baud < 50) return;
    connectionGate.current = true; setConnectionBusy(true);
    const epoch = life.current;
    const owner = { portName, sessionId: crypto.randomUUID() };
    portOwner.current = owner;
    try {
      await invoke("serial_open", { ...owner, baudRate: baud, dataBits, parity, stopBits, flowControl: flow });
      if (!alive.current || epoch !== life.current) { await invoke("serial_close", owner); return; }
      portOwner.current = owner; openRef.current = true; setOpen(true);
      addLog("sys", t("serial.sysOpened", { port: portName, baud }));
      await invoke("serial_set_signals", { ...owner, dtr, rts }).catch((error) => addLog("sys", t("serial.sysSignalFail", { err: String(error) })));
    } catch (error) { addLog("sys", t("serial.sysOpenFail", { err: String(error) })); }
    finally { connectionGate.current = false; setConnectionBusy(false); }
  };
  const doClose = async () => {
    if (connectionGate.current) return;
    connectionGate.current = true; setConnectionBusy(true); stopCycle();
    try {
      await invoke("serial_close", portOwner.current ?? { portName });
      openRef.current = false; portOwner.current = null; setOpen(false);
      addLog("sys", t("serial.sysClosed", { port: portName }));
    } catch (error) { addLog("sys", String(error)); }
    finally { connectionGate.current = false; setConnectionBusy(false); }
  };

  const toggleSignal = async (kind: "dtr" | "rts", value: boolean) => {
    if (!openRef.current || connectionGate.current) return;
    connectionGate.current = true; setConnectionBusy(true);
    const previous = kind === "dtr" ? dtr : rts;
    if (kind === "dtr") setDtr(value);
    else setRts(value);
    try {
      await invoke("serial_set_signals", {
        ...(portOwner.current ?? { portName }),
        dtr: kind === "dtr" ? value : null,
        rts: kind === "rts" ? value : null,
      });
    } catch (e) {
      if (kind === "dtr") setDtr(previous); else setRts(previous);
      addLog("sys", t("serial.sysSignalFail", { err: String(e) }));
    } finally { connectionGate.current = false; setConnectionBusy(false); }
  };

  // ── 模拟设备 ──
  const updateRule = (i: number, patch: Partial<SimRule>) =>
    setRules((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const removeRule = (i: number) =>
    setRules((rs) => (rs.length > 1 ? rs.filter((_, j) => j !== i) : rs));

  const toggleSim = async () => {
    if (connectionGate.current) return;
    connectionGate.current = true; setConnectionBusy(true); stopCycle();
    const epoch = life.current;
    try {
      if (simActiveRef.current) {
        await invoke("serial_sim_stop", { sessionId: simOwner.current });
        simActiveRef.current = false; simOwner.current = null; setSimActive(false);
        addLog("sys", t("serial.sysSimStopped"));
      } else {
        const sessionId = crypto.randomUUID();
        simOwner.current = sessionId;
        const result = await invoke<{ rule_count: number }>("serial_sim_start", { rules, sessionId });
        if (!alive.current || epoch !== life.current) { await invoke("serial_sim_stop", { sessionId }); return; }
        simOwner.current = sessionId; simActiveRef.current = true; setSimActive(true);
        addLog("sys", t("serial.sysSimStarted", { count: result.rule_count }));
      }
    } catch (error) { addLog("sys", t("serial.sysStartFail", { err: String(error) })); }
    finally { connectionGate.current = false; setConnectionBusy(false); }
  };

  const exportRules = async () => {
    try {
      const target = await saveDialog({
        defaultPath: "serial-sim-rules.json",
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!target) return;
      await invoke("write_text_file", { path: target, content: JSON.stringify(rules, null, 2) });
      addLog("sys", t("serial.rulesExported", { path: target }));
    } catch (e) {
      addLog("sys", t("serial.rulesExportFail", { err: String(e) }));
    }
  };

  const importRules = async () => {
    try {
      const f = await openDialog({
        multiple: false,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (typeof f !== "string") return;
      const raw = await invoke<string>("read_text_file", { path: f });
      if (raw.length > 1024 * 1024) throw new Error(t("serial.rulesTooLarge"));
      const imported = normalizeRules(JSON.parse(raw));
      if (imported.length > 128) throw new Error(t("serial.rulesTooLarge"));
      setRules(imported);
      setLineIdx(0);
      addLog("sys", t("serial.rulesImported", {
        file: f.split(/[\\/]/).pop() ?? "",
      }));
    } catch (e) {
      addLog("sys", t("serial.rulesImportFail", { err: String(e) }));
    }
  };

  // ── 发送 ──
  /** 发送单行数据（自动区分真实/模拟），返回是否成功。 */
  const writeOne = async (line: string): Promise<boolean> => {
    if (!alive.current || !active || !line || sendGate.current || connectionGate.current || (mode === "real" ? !openRef.current : !simActiveRef.current)) return false;
    sendGate.current = true; setSending(true);

    let payload = line;
    if (sendHex && checksum !== "none") {
      const cs = applyChecksum(line, checksum);
      if (!cs) {
        addLog("sys", t("serial.csFail"));
        sendGate.current = false; setSending(false);
        return false;
      }
      payload = cs.text;
    }

    try {
      if (mode === "sim") {
        const hit = await invoke<number>("serial_sim_write", {
          sessionId: simOwner.current,
          data: payload,
          hexMode: sendHex,
          encoding,
          lineEnding,
        });
        if (hit < 0) addLog("sys", t("serial.sysNoHit"));
      } else {
        const sentHex = sendHex ? normalizeHex(payload) ?? payload : undefined;
        const n = await invoke<number>("serial_write", {
          ...(portOwner.current ?? { portName }),
          data: payload,
          hexMode: sendHex,
          encoding,
          lineEnding,
        });
        addLog("tx", sendHex ? sentHex! : line, sentHex, n);
      }
      setHistory((h) => pushHistory(h, line));
      return true;
    } catch (e) {
      addLog("sys", t("serial.sysSendFail", { err: String(e) }));
      return false;
    } finally { sendGate.current = false; setSending(false); }
  };
  writeOneRef.current = writeOne;

  /** 发送 textarea 当前内容（整体一段）。 */
  const doSend = () => void writeOne(sendText);

  /** 发送第 idx 行，并推进游标。 */
  const sendLineAt = async (idx: number) => {
    const currentLines = linesRef.current;
    const generation = sendGeneration.current;
    const ok = await writeOneRef.current(currentLines[idx]);
    if (ok && alive.current && generation === sendGeneration.current) {
      const next = (idx + 1) % Math.max(1, currentLines.length);
      lineIdxRef.current = next;
      setLineIdx(next);
    }
  };

  /** 逐行发送全部：从当前游标开始把剩余行依次发完。 */
  const sendAllLines = async () => {
    if (batchGate.current || !active || connectionGate.current) return;
    stopCycle();
    batchGate.current = true; setSendingBatch(true);
    const generation = sendGeneration.current;
    const batch = [...lines];
    let index = lineIdxRef.current;
    try {
      for (let count = 0; count < batch.length; count++) {
        if (!alive.current || generation !== sendGeneration.current) break;
        if (!await writeOneRef.current(batch[index])) break;
        index = (index + 1) % Math.max(1, batch.length);
        if (!alive.current || generation !== sendGeneration.current) break;
        lineIdxRef.current = index; setLineIdx(index);
        await new Promise((resolve) => setTimeout(resolve, 30));
      }
    } finally { batchGate.current = false; setSendingBatch(false); }
  };

  const stopCycle = () => {
    sendGeneration.current++;
    if (cycleTimerRef.current) {
      clearInterval(cycleTimerRef.current);
      cycleTimerRef.current = null;
    }
    setCycling(false);
  };

  /** 定时循环发送：每 intervalMs 发送一行并推进游标。 */
  const toggleCycle = () => {
    if (batchGate.current) { stopCycle(); return; }
    if (cycleTimerRef.current) {
      stopCycle();
      return;
    }
    if (!active || lines.length === 0 || batchGate.current || connectionGate.current) return;
    const ms = Math.max(50, intervalMs || 1000);
    setCycling(true);
    cycleTimerRef.current = setInterval(() => {
      void sendLineAt(lineIdxRef.current);
    }, ms);
  };

  const importLinesFile = async () => {
    try {
      const f = await openDialog({ multiple: false });
      if (typeof f !== "string") return;
      const content = await invoke<string>("read_text_file", { path: f });
      setSendText(content);
      setLineIdx(0);
      addLog("sys", t("serial.sysImported", {
        file: f.split(/[\\/]/).pop() ?? "",
        lines: content.split("\n").filter((l) => l.trim()).length,
      }));
    } catch (e) {
      addLog("sys", t("serial.sysImportFail", { err: String(e) }));
    }
  };

  // ── 日志操作 ──
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
    if (pending.length > 0) setLogs((l) => trimSerialLogs([...l, ...pending], MAX_LOGS));
  };

  const clearLogs = () => {
    setLogs([]);
    pendingRef.current = [];
    setPendingCount(0);
    setStats({ rxFrames: 0, rxBytes: 0, txFrames: 0, txBytes: 0 });
    totalsRef.current = { rx: 0, tx: 0 };
    lastTotalsRef.current = { rx: 0, tx: 0 };
    setRates({ rx: 0, tx: 0 });
  };

  const exportLog = async () => {
    if (logs.length === 0) {
      addLog("sys", t("serial.sysExportEmpty"));
      return;
    }
    try {
      const target = await saveDialog({
        defaultPath: `serial-log-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")}.txt`,
        filters: [{ name: "Text", extensions: ["txt", "log"] }],
      });
      if (!target) return;
      await invoke("write_text_file", {
        path: target,
        content: buildExportText(logs, hexView, showTime),
      });
      addLog("sys", t("serial.sysExportOk", { path: target }));
    } catch (e) {
      addLog("sys", t("serial.sysExportFail", { err: String(e) }));
    }
  };

  const visibleLogs = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return logs;
    return logs.filter(
      (e) => e.text.toLowerCase().includes(q) || (e.hex ?? "").toLowerCase().includes(q)
    );
  }, [logs, filter]);

  // 编辑内容后游标可能越界：回到首行，避免定时发送读到空行
  useEffect(() => {
    if (lines.length > 0 && lineIdx >= lines.length) {
      lineIdxRef.current = 0;
      setLineIdx(0);
    }
  }, [lines.length, lineIdx]);

  // 内容被清空时自动停止循环发送
  useEffect(() => {
    if (cycling && lines.length === 0) stopCycle();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cycling, lines.length]);

  // 校验预览：多行时看当前游标行
  const previewLine = lines.length > 1 ? lines[Math.min(lineIdx, lines.length - 1)] ?? "" : sendText;
  const checksumPreview =
    sendHex && checksum !== "none" ? applyChecksum(previewLine, checksum) : null;

  return {
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
  };
}
