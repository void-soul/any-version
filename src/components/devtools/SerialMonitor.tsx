import { SerialRuleEditor } from "./SerialRuleEditor";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Usb, RefreshCw, Play, Square, Send, Trash2, Timer, FileUp, ListOrdered, Pause, Bot, Cable, Plus, Download, Upload, History, Filter } from "lucide-react";
import { applyChecksum, buildExportText, decodeBase64, emptySimRule, formatBytes, loadHistory, loadPrefs, loadSimRules, normalizeRules, nowStamp, portMeta, pushHistory, saveHistory, savePrefs, saveSimRules, splitSendLines, type ChecksumAlgo, type Direction, type Encoding, type LineEnding, type LogEntry, type PortInfo, type SimRule } from "./serialUtils";
import { MAX_LOGS, MAX_PENDING, normalizeHex, type SerialPayload, type SimPayload } from "./SerialMonitor.model";

/** 串口调试器：真实串口（连接/编码/HEX 收发/逐行/定时发送）+ 模拟设备（脚本应答）。 */
export default function SerialMonitor() {
  const { t } = useTranslation();
  const [initialPrefs] = useState(loadPrefs);

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
      void invoke("serial_sim_update_rules", { rules }).catch(() => {
        /* 校验失败时保留上一批规则，显式启动时再报错 */
      });
    }, 400);
    return () => clearTimeout(timer);
  }, [rules, simActive]);

  const recordLog = (dir: Direction, text: string, hex?: string, bytes = 0) => {
    const entry: LogEntry = { id: ++idRef.current, dir, text, hex, bytes, time: nowStamp() };
    if (pausedRef.current) {
      const pending = pendingRef.current;
      pending.push(entry);
      if (pending.length > MAX_PENDING) pending.splice(0, pending.length - MAX_PENDING);
      setPendingCount(pending.length);
    } else {
      setLogs((prev) => [...prev, entry].slice(-MAX_LOGS));
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
      setPorts(list);
      // 保留用户已选端口；端口被拔出时自动回落到第一个可用端口
      setPortName((cur) => (cur && list.some((p) => p.name === cur) ? cur : list[0]?.name ?? ""));
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
          const text = e.payload.data
            ? decodeBase64(e.payload.data, encodingRef.current)
            : e.payload.text;
          recordLog("rx", text, e.payload.hex, e.payload.len);
        })
      );
      // 模拟设备事件：dir 是设备视角 —— rx=设备收到（用户发出），tx=设备应答
      add(
        await listen<SimPayload>("serial://sim-data", (e) => {
          const dir: Direction = e.payload.dir === "tx" ? "dev" : "tx";
          const text = e.payload.data
            ? decodeBase64(e.payload.data, encodingRef.current)
            : e.payload.text;
          recordLog(dir, text, e.payload.hex, e.payload.len);
        })
      );
      add(
        await listen<{ port: string; reason?: string | null }>("serial://closed", (e) => {
          setOpen(false);
          stopCycle();
          if (e.payload.reason) {
            recordLog("sys", tRef.current("serial.sysReadError", { err: e.payload.reason }));
          } else {
            recordLog("sys", tRef.current("serial.sysPortClosed"));
          }
        })
      );
    })();
    return () => {
      cancelled = true;
      unlisteners.forEach((u) => u());
      if (cycleTimerRef.current) clearInterval(cycleTimerRef.current);
      // **卸载时主动关串口/停模拟设备**：只注销事件的话，后端的读线程与句柄会一直活着
      //（端口保持被占用、持续 emit），而重进本页时按钮显示「未连接」。
      if (openRef.current) {
        void invoke("serial_close", { portName: portNameRef.current }).catch(() => {
          /* 卸载路径，失败无可上报 */
        });
      }
      if (simActiveRef.current) {
        void invoke("serial_sim_stop").catch(() => {
          /* 卸载路径，失败无可上报 */
        });
      }
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
  const switchMode = async (m: "real" | "sim") => {
    if (m === mode) return;
    stopCycle();
    if (mode === "real" && open) {
      try {
        await invoke("serial_close", { portName });
      } catch {
        /* 忽略 */
      }
      setOpen(false);
    }
    if (mode === "sim" && simActive) {
      try {
        await invoke("serial_sim_stop");
      } catch {
        /* 忽略 */
      }
      setSimActive(false);
    }
    setMode(m);
    addLog("sys", m === "sim" ? t("serial.sysSwitchSim") : t("serial.sysSwitchReal"));
  };

  // ── 真实串口 ──
  const doOpen = async () => {
    try {
      await invoke("serial_open", {
        portName,
        baudRate: baud,
        dataBits,
        parity,
        stopBits,
        flowControl: flow,
      });
      setOpen(true);
      addLog("sys", t("serial.sysOpened", { port: portName, baud }));
      // 打开后按界面状态应用 DTR/RTS（部分开发板靠 DTR 复位）
      await invoke("serial_set_signals", { portName, dtr, rts }).catch(() => {
        /* 个别转接芯片不支持，忽略 */
      });
    } catch (e) {
      addLog("sys", t("serial.sysOpenFail", { err: String(e) }));
    }
  };
  const doClose = async () => {
    stopCycle();
    try {
      await invoke("serial_close", { portName });
    } catch {
      /* 忽略 */
    }
    setOpen(false);
    addLog("sys", t("serial.sysClosed", { port: portName }));
  };

  const toggleSignal = async (kind: "dtr" | "rts", value: boolean) => {
    if (!open) return;
    if (kind === "dtr") setDtr(value);
    else setRts(value);
    try {
      await invoke("serial_set_signals", {
        portName,
        dtr: kind === "dtr" ? value : null,
        rts: kind === "rts" ? value : null,
      });
    } catch (e) {
      addLog("sys", t("serial.sysSignalFail", { err: String(e) }));
    }
  };

  // ── 模拟设备 ──
  const updateRule = (i: number, patch: Partial<SimRule>) =>
    setRules((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const removeRule = (i: number) =>
    setRules((rs) => (rs.length > 1 ? rs.filter((_, j) => j !== i) : rs));

  const toggleSim = async () => {
    stopCycle();
    if (simActive) {
      try {
        await invoke("serial_sim_stop");
      } catch {
        /* 忽略 */
      }
      setSimActive(false);
      addLog("sys", t("serial.sysSimStopped"));
      return;
    }
    try {
      const res = await invoke<{ rule_count: number }>("serial_sim_start", { rules });
      setSimActive(true);
      addLog("sys", t("serial.sysSimStarted", { count: res.rule_count }));
    } catch (e) {
      addLog("sys", t("serial.sysStartFail", { err: String(e) }));
    }
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
      setRules(normalizeRules(JSON.parse(raw)));
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
    if (!active || !line) return false;

    let payload = line;
    if (sendHex && checksum !== "none") {
      const cs = applyChecksum(line, checksum);
      if (!cs) {
        addLog("sys", t("serial.csFail"));
        return false;
      }
      payload = cs.text;
    }

    try {
      if (mode === "sim") {
        const hit = await invoke<number>("serial_sim_write", {
          data: payload,
          hexMode: sendHex,
          encoding,
          lineEnding,
        });
        if (hit < 0) addLog("sys", t("serial.sysNoHit"));
      } else {
        const sentHex = sendHex ? normalizeHex(payload) ?? payload : undefined;
        const n = await invoke<number>("serial_write", {
          portName,
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
    }
  };
  writeOneRef.current = writeOne;

  /** 发送 textarea 当前内容（整体一段）。 */
  const doSend = () => void writeOne(sendText);

  /** 发送第 idx 行，并推进游标。 */
  const sendLineAt = async (idx: number) => {
    const ok = await writeOneRef.current(lines[idx]);
    if (ok) {
      const next = (idx + 1) % Math.max(1, lines.length);
      lineIdxRef.current = next;
      setLineIdx(next);
    }
  };

  /** 逐行发送全部：从当前游标开始把剩余行依次发完。 */
  const sendAllLines = async () => {
    const total = lines.length;
    let idx = lineIdxRef.current;
    for (let i = 0; i < total; i++) {
      const ok = await writeOneRef.current(lines[idx]);
      if (ok) {
        idx = (idx + 1) % Math.max(1, total);
        lineIdxRef.current = idx;
        setLineIdx(idx);
      }
      // 每行间隔 30ms，给对端处理时间
      await new Promise((r) => setTimeout(r, 30));
    }
  };

  const stopCycle = () => {
    if (cycleTimerRef.current) {
      clearInterval(cycleTimerRef.current);
      cycleTimerRef.current = null;
    }
    setCycling(false);
  };

  /** 定时循环发送：每 intervalMs 发送一行并推进游标。 */
  const toggleCycle = () => {
    if (cycling) {
      stopCycle();
      return;
    }
    const ms = Math.max(50, intervalMs || 1000);
    setCycling(true);
    cycleTimerRef.current = setInterval(() => {
      void sendLineAt(lineIdxRef.current);
    }, ms);
  };

  const importLinesFile = async () => {
    const f = await openDialog({ multiple: false });
    if (typeof f !== "string") return;
    try {
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
    if (pending.length > 0) setLogs((l) => [...l, ...pending].slice(-MAX_LOGS));
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

  const selectCls =
    "bg-black/30 border border-white/10 rounded px-2 py-1.5 text-body focus:outline-none focus:border-teal-500";
  const inputCls =
    "bg-black/30 border border-white/10 rounded px-2 py-1.5 font-mono text-body focus:outline-none focus:border-teal-500";

  return (
    <div className="h-full flex flex-col overflow-hidden p-3 gap-2.5 text-body">
      <div className="flex items-center gap-2 shrink-0">
        <Usb className="w-4 h-4 text-teal-400" />
        <h1 className="text-base font-semibold">{t("serial.title")}</h1>
        {/* 模式切换 */}
        <div className="flex rounded-ctl overflow-hidden border border-white/10 text-caption">
          <button
            onClick={() => void switchMode("real")}
            className={`px-3 py-1 flex items-center gap-1 cursor-pointer ${mode === "real" ? "bg-teal-600 text-white" : "bg-white/5 text-slate-400 hover:text-slate-200"}`}
          >
            <Cable className="w-3.5 h-3.5" /> {t("serial.modeReal")}
          </button>
          <button
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
          {!active ? t("serial.statusIdle") : mode === "sim" ? t("serial.statusSim") : t("serial.statusReal")}
        </span>
      </div>

      {/* 连接配置（真实串口）/ 应答脚本编辑器（模拟设备） */}
      {mode === "real" ? (
        <div className="shrink-0 flex flex-wrap items-center gap-2 bg-white/[0.03] border border-white/10 rounded-ctl p-3">
          <select value={portName} onChange={(e) => setPortName(e.target.value)} className={`${selectCls} min-w-44`}>
            {ports.length === 0 && <option value="">{t("serial.noPorts")}</option>}
            {ports.map((p) => (
              <option key={p.name} value={p.name} title={portMeta(p)}>
                {p.name} · {p.description}
              </option>
            ))}
          </select>
          <button onClick={() => void refreshPorts()} title={t("serial.refreshPorts")} className="p-1.5 rounded hover:bg-white/10 text-slate-400 cursor-pointer">
            <RefreshCw className="w-4 h-4" />
          </button>
          <label className="flex items-center gap-1 text-body text-slate-400">
            {t("serial.baud")}
            <input
              type="number"
              min={50}
              step={100}
              list="serial-baud-presets"
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
            <select value={dataBits} onChange={(e) => setDataBits(Number(e.target.value))} className={selectCls}>
              {[8, 7, 6, 5].map((v) => (
                <option key={v} value={v}>{v}</option>
              ))}
            </select>
          </label>
          <label className="flex items-center gap-1 text-body text-slate-400">
            {t("serial.parity")}
            <select value={parity} onChange={(e) => setParity(e.target.value as typeof parity)} className={selectCls}>
              <option value="none">None</option>
              <option value="even">Even</option>
              <option value="odd">Odd</option>
            </select>
          </label>
          <label className="flex items-center gap-1 text-body text-slate-400">
            {t("serial.stopBits")}
            <select value={stopBits} onChange={(e) => setStopBits(Number(e.target.value))} className={selectCls}>
              {[1, 2].map((v) => (
                <option key={v} value={v}>{v}</option>
              ))}
            </select>
          </label>
          <label className="flex items-center gap-1 text-body text-slate-400">
            {t("serial.flow")}
            <select value={flow} onChange={(e) => setFlow(e.target.value as typeof flow)} className={selectCls}>
              <option value="none">{t("serial.flowNone")}</option>
              <option value="hardware">{t("serial.flowHw")}</option>
              <option value="software">{t("serial.flowSw")}</option>
            </select>
          </label>
          {open && (
            <div className="flex items-center gap-1" title={t("serial.signalTip")}>
              <button
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
                disabled={flow === "hardware"}
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
            <button onClick={() => void doClose()} className="ml-auto px-3 py-1.5 rounded-md bg-red-600 hover:bg-red-500 text-sm flex items-center gap-1 cursor-pointer">
              <Square className="w-3.5 h-3.5" /> {t("serial.disconnect")}
            </button>
          ) : (
            <button
              onClick={() => void doOpen()}
              disabled={!portName || baud < 50}
              className="ml-auto px-3 py-1.5 rounded-md bg-teal-600 hover:bg-teal-500 disabled:opacity-40 text-sm flex items-center gap-1 cursor-pointer"
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
                <button onClick={() => void toggleSim()} className="px-3 py-1.5 rounded-md bg-red-600 hover:bg-red-500 flex items-center gap-1 cursor-pointer">
                  <Square className="w-3.5 h-3.5" /> {t("serial.stopDevice")}
                </button>
              ) : (
                <button onClick={() => void toggleSim()} className="px-3 py-1.5 rounded-md bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] flex items-center gap-1 cursor-pointer">
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
                className="w-36 bg-black/30 border border-white/10 rounded px-1.5 py-1 pl-6 text-caption focus:outline-none focus:border-teal-500"
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

          <button onClick={() => void doSend()} disabled={!active || !sendText} className="px-3 py-1.5 rounded-md bg-teal-600 hover:bg-teal-500 disabled:opacity-40 flex items-center gap-1 cursor-pointer">
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
              disabled={!active || lines.length < 2}
              title={t("serial.sendAllTitle")}
              className="px-2 py-1.5 rounded bg-slate-700 hover:bg-slate-600 disabled:opacity-40 flex items-center gap-1 cursor-pointer"
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
                disabled={cycling}
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
              {cycling ? t("serial.cycleStop") : t("serial.cycleStart")}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
