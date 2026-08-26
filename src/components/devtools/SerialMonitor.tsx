import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  Usb,
  RefreshCw,
  Play,
  Square,
  Send,
  Trash2,
  Timer,
  FileUp,
  ListOrdered,
  Pause,
  Bot,
  Cable,
  Plus,
} from "lucide-react";

interface PortInfo {
  name: string;
  description: string;
}
interface LogEntry {
  /** rx=收到 tx=发出 sys=系统 dev=设备应答 */
  dir: "rx" | "tx" | "sys" | "dev";
  text: string;
  hex?: string;
  time: string;
}
interface SimRule {
  pattern: string;
  pattern_hex: boolean;
  match_type: "contains" | "prefix" | "exact" | "regex";
  response: string;
  response_hex: boolean;
  append_newline: boolean;
  delay_ms: number;
}

function now() {
  return new Date().toLocaleTimeString("zh-CN", { hour12: false }) + "." + String(Date.now() % 1000).padStart(3, "0");
}

const emptyRule = (): SimRule => ({
  pattern: "",
  pattern_hex: false,
  match_type: "contains",
  response: "",
  response_hex: false,
  append_newline: true,
  delay_ms: 0,
});

/** 串口调试器：真实串口（连接/HEX 收发/逐行/定时发送）+ 模拟设备（脚本应答）。 */
export default function SerialMonitor() {
  const [mode, setMode] = useState<"real" | "sim">("real");
  const [ports, setPorts] = useState<PortInfo[]>([]);
  const [portName, setPortName] = useState("");
  const [baud, setBaud] = useState(115200);
  const [dataBits, setDataBits] = useState(8);
  const [parity, setParity] = useState<"none" | "even" | "odd">("none");
  const [stopBits, setStopBits] = useState(1);
  const [flow, setFlow] = useState<"none" | "hardware" | "software">("none");

  const [open, setOpen] = useState(false); // 真实串口已打开
  const [simActive, setSimActive] = useState(false); // 模拟设备运行中
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [hexView, setHexView] = useState(false);
  const [sendText, setSendText] = useState("");
  const [sendHex, setSendHex] = useState(false);
  const [appendNl, setAppendNl] = useState(true);
  const logRef = useRef<HTMLDivElement>(null);

  // ── 模拟设备规则 ──
  const [rules, setRules] = useState<SimRule[]>([emptyRule()]);
  const active = mode === "real" ? open : simActive;

  // ── 多行逐行 / 定时发送 ──
  const lines = sendText.split("\n").map((l) => l.replace(/\r$/, "")).filter((l) => l.length > 0);
  const [lineIdx, setLineIdx] = useState(0);
  const [intervalMs, setIntervalMs] = useState(1000);
  const [cycling, setCycling] = useState(false);
  const cycleTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // 规则快照供定时器/异步回调读取
  const rulesRef = useRef(rules);
  useEffect(() => {
    rulesRef.current = rules;
  }, [rules]);

  const refreshPorts = useCallback(async () => {
    try {
      const list = await invoke<PortInfo[]>("serial_list_ports");
      setPorts(list);
      if (!portName && list.length > 0) setPortName(list[0].name);
    } catch (e) {
      setLogs((l) => [...l.slice(-500), { dir: "sys" as const, text: `枚举失败: ${e}`, time: now() }]);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    refreshPorts();
    let unlisteners: Array<() => void> = [];
    (async () => {
      unlisteners.push(
        await listen<{ port: string; hex: string; text: string; len: number }>("serial://data", (e) => {
          setLogs((l) =>
            [...l, { dir: "rx" as const, text: e.payload.text, hex: e.payload.hex, time: now() }].slice(-2000)
          );
        })
      );
      // 模拟设备事件：dir 是设备视角 —— rx=设备收到（用户发出），tx=设备应答
      unlisteners.push(
        await listen<{ port: string; dir: string; hex: string; text: string; len: number }>("serial://sim-data", (e) => {
          const dir: LogEntry["dir"] = e.payload.dir === "tx" ? "dev" : "tx";
          setLogs((l) =>
            [...l, { dir, text: e.payload.text, hex: e.payload.hex, time: now() }].slice(-2000)
          );
        })
      );
      unlisteners.push(
        await listen<{ port: string }>("serial://closed", () => {
          setOpen(false);
          stopCycle();
          setLogs((l) => [...l.slice(-500), { dir: "sys" as const, text: "端口已关闭", time: now() }]);
        })
      );
    })();
    return () => {
      unlisteners.forEach((u) => u());
      if (cycleTimerRef.current) clearInterval(cycleTimerRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshPorts]);

  // 自动滚动到底部
  useEffect(() => {
    if (logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [logs]);

  const addLog = (dir: LogEntry["dir"], text: string) =>
    setLogs((l) => [...l.slice(-2000), { dir, text, time: now() }]);

  // ── 模式切换：互斥关闭另一侧 ──
  const switchMode = async (m: "real" | "sim") => {
    if (m === mode) return;
    stopCycle();
    if (mode === "real" && open) {
      try {
        await invoke("serial_close", { portName });
      } catch { /* 忽略 */ }
      setOpen(false);
    }
    if (mode === "sim" && simActive) {
      try {
        await invoke("serial_sim_stop");
      } catch { /* 忽略 */ }
      setSimActive(false);
    }
    setMode(m);
    addLog("sys", m === "sim" ? "切换到模拟设备模式" : "切换到真实串口模式");
  };

  // ── 真实串口 ──
  const doOpen = async () => {
    try {
      await invoke("serial_open", { portName, baudRate: baud, dataBits, parity, stopBits, flowControl: flow });
      setOpen(true);
      addLog("sys", `${portName} 已打开 @${baud}`);
    } catch (e) {
      addLog("sys", `打开失败: ${e}`);
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
    addLog("sys", `${portName} 已关闭`);
  };

  // ── 模拟设备 ──
  const updateRule = (i: number, patch: Partial<SimRule>) =>
    setRules((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const removeRule = (i: number) => setRules((rs) => (rs.length > 1 ? rs.filter((_, j) => j !== i) : rs));

  const toggleSim = async () => {
    stopCycle();
    if (simActive) {
      try {
        await invoke("serial_sim_stop");
      } catch { /* 忽略 */ }
      setSimActive(false);
      addLog("sys", "模拟设备已停止");
      return;
    }
    try {
      const res = await invoke<{ rule_count: number }>("serial_sim_start", { rules: rulesRef.current });
      setSimActive(true);
      addLog("sys", `模拟设备已启动（${res.rule_count} 条应答规则）`);
    } catch (e) {
      addLog("sys", `启动失败: ${e}`);
    }
  };

  /** 发送单行数据（自动区分真实/模拟）。返回是否成功。 */
  const writeOne = async (line: string): Promise<boolean> => {
    if (!active || !line) return false;
    try {
      if (mode === "sim") {
        const hit = await invoke<number>("serial_sim_write", {
          data: line,
          hexMode: sendHex,
          appendNewline: appendNl,
          rules: rulesRef.current,
        });
        if (hit < 0) addLog("sys", "未命中任何规则（无应答）");
        return true;
      }
      await invoke("serial_write", { portName, data: line, hexMode: sendHex, appendNewline: appendNl });
      addLog("tx", line);
      return true;
    } catch (e) {
      addLog("sys", `发送失败: ${e}`);
      return false;
    }
  };

  /** 发送 textarea 当前内容（整体一段）。 */
  const doSend = () => void writeOne(sendText);

  /** 发送第 idx 行，并推进游标。 */
  const sendLineAt = async (idx: number) => {
    const ok = await writeOne(lines[idx]);
    if (ok) setLineIdx((idx + 1) % Math.max(1, lines.length));
  };

  /** 逐行发送全部：从当前游标开始把剩余行依次发完。 */
  const sendAllLines = async () => {
    for (let i = 0; i < lines.length; i++) {
      await sendLineAt(lineIdxRef.current);
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

  // 循环里读最新游标
  const lineIdxRef = useRef(0);
  useEffect(() => {
    lineIdxRef.current = lineIdx;
  }, [lineIdx]);

  const importLinesFile = async () => {
    const f = await openDialog({ multiple: false });
    if (typeof f !== "string") return;
    try {
      const content = await invoke<string>("read_text_file", { path: f });
      setSendText(content);
      setLineIdx(0);
      addLog("sys", `已导入 ${f.split(/[\\/]/).pop()}（${content.split("\n").filter((l) => l.trim()).length} 行）`);
    } catch (e) {
      addLog("sys", `导入失败: ${e}`);
    }
  };

  const selectCls =
    "bg-slate-800 border border-slate-700 rounded px-2 py-1.5 text-xs focus:outline-none focus:border-teal-500";
  const inputCls =
    "bg-slate-950 border border-slate-700 rounded px-2 py-1.5 font-mono text-xs focus:outline-none focus:border-teal-500";

  return (
    <div className="h-full flex flex-col overflow-hidden p-4 gap-3">
      <div className="flex items-center gap-2 shrink-0">
        <Usb className="w-5 h-5 text-teal-400" />
        <h1 className="text-lg font-semibold">串口调试器</h1>
        {/* 模式切换 */}
        <div className="flex rounded-lg overflow-hidden border border-slate-700 text-xs">
          <button
            onClick={() => void switchMode("real")}
            className={`px-3 py-1 flex items-center gap-1 cursor-pointer ${mode === "real" ? "bg-teal-600 text-white" : "bg-slate-800 text-slate-400 hover:text-slate-200"}`}
          >
            <Cable className="w-3.5 h-3.5" /> 真实串口
          </button>
          <button
            onClick={() => void switchMode("sim")}
            className={`px-3 py-1 flex items-center gap-1 cursor-pointer ${mode === "sim" ? "bg-violet-600 text-white" : "bg-slate-800 text-slate-400 hover:text-slate-200"}`}
          >
            <Bot className="w-3.5 h-3.5" /> 模拟设备
          </button>
        </div>
        <span
          className={`text-xs px-2 py-0.5 rounded-full ${
            active
              ? mode === "sim"
                ? "bg-violet-900/60 text-violet-300"
                : "bg-emerald-900/60 text-emerald-300"
              : "bg-slate-800 text-slate-400"
          }`}
        >
          {!active ? "未连接" : mode === "sim" ? "设备运行中" : "已连接"}
        </span>
        <div className="ml-auto flex items-center gap-3 text-[11px] text-slate-500">
          <label className="flex items-center gap-1 cursor-pointer hover:text-slate-300">
            <input type="checkbox" checked={hexView} onChange={(e) => setHexView(e.target.checked)} className="accent-teal-500" />
            HEX 视图
          </label>
          <button onClick={() => setLogs([])} className="flex items-center gap-1 hover:text-slate-300 cursor-pointer">
            <Trash2 className="w-3.5 h-3.5" /> 清空日志
          </button>
        </div>
      </div>

      {/* 连接配置（真实串口）/ 应答脚本编辑器（模拟设备） */}
      {mode === "real" ? (
        <div className="shrink-0 flex flex-wrap items-center gap-2 bg-slate-900/60 border border-slate-800 rounded-lg p-3">
          <select value={portName} onChange={(e) => setPortName(e.target.value)} className={`${selectCls} min-w-44`}>
            {ports.length === 0 && <option value="">（无可用串口）</option>}
            {ports.map((p) => (
              <option key={p.name} value={p.name}>
                {p.name} · {p.description}
              </option>
            ))}
          </select>
          <button onClick={refreshPorts} title="刷新端口列表" className="p-1.5 rounded hover:bg-slate-700 text-slate-400 cursor-pointer">
            <RefreshCw className="w-4 h-4" />
          </button>
          <label className="flex items-center gap-1 text-xs text-slate-400">
            波特率
            <select value={baud} onChange={(e) => setBaud(Number(e.target.value))} className={selectCls}>
              {[9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600].map((b) => (
                <option key={b} value={b}>{b}</option>
              ))}
            </select>
          </label>
          <label className="flex items-center gap-1 text-xs text-slate-400">
            数据位
            <select value={dataBits} onChange={(e) => setDataBits(Number(e.target.value))} className={selectCls}>
              {[8, 7, 6, 5].map((v) => (
                <option key={v} value={v}>{v}</option>
              ))}
            </select>
          </label>
          <label className="flex items-center gap-1 text-xs text-slate-400">
            校验
            <select value={parity} onChange={(e) => setParity(e.target.value as typeof parity)} className={selectCls}>
              <option value="none">None</option>
              <option value="even">Even</option>
              <option value="odd">Odd</option>
            </select>
          </label>
          <label className="flex items-center gap-1 text-xs text-slate-400">
            停止位
            <select value={stopBits} onChange={(e) => setStopBits(Number(e.target.value))} className={selectCls}>
              {[1, 2].map((v) => (
                <option key={v} value={v}>{v}</option>
              ))}
            </select>
          </label>
          <label className="flex items-center gap-1 text-xs text-slate-400">
            流控
            <select value={flow} onChange={(e) => setFlow(e.target.value as typeof flow)} className={selectCls}>
              <option value="none">无</option>
              <option value="hardware">硬件 RTS/CTS</option>
              <option value="software">软件 XON/XOFF</option>
            </select>
          </label>
          {open ? (
            <button onClick={doClose} className="ml-auto px-3 py-1.5 rounded-md bg-red-600 hover:bg-red-500 text-sm flex items-center gap-1 cursor-pointer">
              <Square className="w-3.5 h-3.5" /> 断开
            </button>
          ) : (
            <button onClick={doOpen} disabled={!portName} className="ml-auto px-3 py-1.5 rounded-md bg-teal-600 hover:bg-teal-500 disabled:opacity-40 text-sm flex items-center gap-1 cursor-pointer">
              <Play className="w-3.5 h-3.5" /> 连接
            </button>
          )}
        </div>
      ) : (
        <div className="shrink-0 bg-slate-900/60 border border-violet-900/60 rounded-lg p-3 space-y-2">
          <div className="flex items-center gap-2 text-xs text-slate-400">
            <Bot className="w-4 h-4 text-violet-400" />
            <span>
              应答脚本：数据命中<b className="text-slate-200">第一条</b>匹配的规则后按延迟自动应答；不占用真实串口，用于调试上位机协议。
            </span>
            <div className="ml-auto flex items-center gap-2">
              <button
                onClick={() => setRules((rs) => [...rs, emptyRule()])}
                className="px-2 py-1 rounded bg-slate-700 hover:bg-slate-600 flex items-center gap-1 cursor-pointer"
              >
                <Plus className="w-3.5 h-3.5" /> 添加规则
              </button>
              {simActive ? (
                <button onClick={() => void toggleSim()} className="px-3 py-1.5 rounded-md bg-red-600 hover:bg-red-500 flex items-center gap-1 cursor-pointer">
                  <Square className="w-3.5 h-3.5" /> 停止设备
                </button>
              ) : (
                <button onClick={() => void toggleSim()} className="px-3 py-1.5 rounded-md bg-violet-600 hover:bg-violet-500 flex items-center gap-1 cursor-pointer">
                  <Play className="w-3.5 h-3.5" /> 启动设备
                </button>
              )}
            </div>
          </div>
          {/* 表头 */}
          {rules.length > 0 && (
            <div className="grid grid-cols-[7rem_1fr_9rem_5rem_2rem] gap-2 text-[11px] text-slate-500 px-0.5">
              <span>匹配方式</span>
              <span>收到内容（匹配）</span>
              <span>应答内容</span>
              <span>延迟 ms</span>
              <span />
            </div>
          )}
          <div className="space-y-1.5 max-h-44 overflow-auto pr-1">
            {rules.map((r, i) => (
              <div key={i} className="grid grid-cols-[7rem_1fr_9rem_5rem_2rem] gap-2 items-start">
                <select
                  value={r.match_type}
                  onChange={(e) => updateRule(i, { match_type: e.target.value as SimRule["match_type"] })}
                  className={selectCls}
                >
                  <option value="contains">包含</option>
                  <option value="prefix">前缀</option>
                  <option value="exact">完全相等</option>
                  <option value="regex">正则</option>
                </select>
                <div className="space-y-1">
                  <textarea
                    value={r.pattern}
                    onChange={(e) => updateRule(i, { pattern: e.target.value })}
                    rows={1}
                    placeholder={r.pattern_hex ? "如 01 A0 FF" : "如 AT+CSQ"}
                    className={`${inputCls} w-full resize-y min-h-[2rem]`}
                  />
                  <label className="flex items-center gap-1 text-[11px] text-slate-500 cursor-pointer">
                    <input type="checkbox" checked={r.pattern_hex} onChange={(e) => updateRule(i, { pattern_hex: e.target.checked })} className="accent-violet-500" /> HEX 匹配
                  </label>
                </div>
                <div className="space-y-1">
                  <textarea
                    value={r.response}
                    onChange={(e) => updateRule(i, { response: e.target.value })}
                    rows={1}
                    placeholder={r.response_hex ? "如 01 B0 00" : "如 OK"}
                    className={`${inputCls} w-full resize-y min-h-[2rem]`}
                  />
                  <div className="flex items-center gap-2 text-[11px] text-slate-500">
                    <label className="flex items-center gap-1 cursor-pointer">
                      <input type="checkbox" checked={r.response_hex} onChange={(e) => updateRule(i, { response_hex: e.target.checked })} className="accent-violet-500" /> HEX
                    </label>
                    <label className="flex items-center gap-1 cursor-pointer" title="应答末尾追加换行符 \n">
                      <input type="checkbox" checked={r.append_newline} onChange={(e) => updateRule(i, { append_newline: e.target.checked })} className="accent-violet-500" /> \n
                    </label>
                  </div>
                </div>
                <input
                  type="number"
                  min={0}
                  step={10}
                  value={r.delay_ms}
                  onChange={(e) => updateRule(i, { delay_ms: Math.max(0, Number(e.target.value) || 0) })}
                  className={`${inputCls} w-full`}
                />
                <button
                  onClick={() => removeRule(i)}
                  disabled={rules.length <= 1}
                  title="删除此规则"
                  className="p-1.5 rounded text-slate-500 hover:text-red-400 hover:bg-slate-800 disabled:opacity-30 cursor-pointer"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* 收发日志 */}
      <div ref={logRef} className="flex-1 min-h-0 overflow-auto bg-slate-950 border border-slate-800 rounded-lg p-3 font-mono text-xs leading-relaxed">
        {logs.length === 0 && (
          <div className="h-full flex flex-col items-center justify-center text-slate-600 gap-2 select-none">
            {mode === "sim" ? <Bot className="w-8 h-8 opacity-40" /> : <Usb className="w-8 h-8 opacity-40" />}
            <span>{mode === "sim" ? "启动设备并发送数据，收到的内容与设备应答将显示在这里" : "连接串口后，收发数据将显示在这里"}</span>
          </div>
        )}
        {logs.map((entry, i) => (
          <div key={i} className="flex gap-2 break-all">
            <span className="text-slate-600 shrink-0">{entry.time}</span>
            <span className={`shrink-0 ${entry.dir === "rx" ? "text-emerald-400" : entry.dir === "tx" ? "text-cyan-400" : entry.dir === "dev" ? "text-violet-400" : "text-yellow-500"}`}>
              {entry.dir === "rx" ? "←" : entry.dir === "tx" ? "→" : entry.dir === "dev" ? "⇠" : "•"}
            </span>
            <span className={
              entry.dir === "rx" ? "text-slate-200"
              : entry.dir === "tx" ? "text-cyan-200"
              : entry.dir === "dev" ? "text-violet-200"
              : "text-yellow-200"
            }>
              {hexView && entry.hex !== undefined ? entry.hex : entry.text}
            </span>
          </div>
        ))}
      </div>

      {/* 发送区 */}
      <div className="shrink-0 bg-slate-900/60 border border-slate-800 rounded-lg p-3 space-y-2">
        {/* 行游标提示 */}
        {lines.length > 1 && (
          <div className="flex items-center justify-between text-[11px] text-slate-500">
            <span className="flex items-center gap-1">
              <ListOrdered className="w-3.5 h-3.5" />
              共 {lines.length} 行 · 游标在第 {Math.min(lineIdx + 1, lines.length)} 行：
              <code className="px-1 rounded bg-slate-800 text-teal-300 truncate max-w-64 inline-block">{lines[Math.min(lineIdx, lines.length - 1)]}</code>
            </span>
            <button onClick={() => setLineIdx(0)} className="hover:text-slate-300 cursor-pointer">重置游标</button>
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
          placeholder={
            sendHex
              ? "HEX 数据，如 01 A0 FF（多行时可用下方逐行发送）"
              : "要发送的内容…支持多行：Enter 发送整段；Ctrl+Enter 换行；多行内容可逐行/定时发送"
          }
          rows={3}
          className="w-full bg-slate-950 border border-slate-700 rounded-md p-2 font-mono text-xs resize-none focus:outline-none focus:border-teal-500"
        />
        <div className="flex flex-wrap items-center gap-2 text-xs text-slate-300">
          <label className="flex items-center gap-1 cursor-pointer">
            <input type="checkbox" checked={sendHex} onChange={(e) => setSendHex(e.target.checked)} className="accent-teal-500" /> HEX
          </label>
          <label className="flex items-center gap-1 cursor-pointer" title="发送时在末尾追加换行符 \n">
            <input type="checkbox" checked={appendNl} onChange={(e) => setAppendNl(e.target.checked)} className="accent-teal-500" /> 追加 \n
          </label>

          <button onClick={() => void doSend()} disabled={!active || !sendText} className="px-3 py-1.5 rounded-md bg-teal-600 hover:bg-teal-500 disabled:opacity-40 flex items-center gap-1 cursor-pointer">
            <Send className="w-3.5 h-3.5" /> 发送
          </button>

          {/* 多行工具 */}
          <div className="flex items-center gap-1 ml-auto">
            <button
              onClick={() => void importLinesFile()}
              title="从文本文件导入多行数据"
              className="p-1.5 rounded hover:bg-slate-700 text-slate-400 cursor-pointer"
            >
              <FileUp className="w-4 h-4" />
            </button>
            <button
              onClick={() => void sendAllLines()}
              disabled={!active || lines.length < 2}
              title="从当前游标开始逐行发送剩余所有行"
              className="px-2 py-1.5 rounded bg-slate-700 hover:bg-slate-600 disabled:opacity-40 flex items-center gap-1 cursor-pointer"
            >
              <ListOrdered className="w-3.5 h-3.5" /> 逐行发完
            </button>
            <label className="flex items-center gap-1 text-slate-400" title="定时循环发送的间隔">
              <Timer className="w-3.5 h-3.5" />
              <input
                type="number"
                min={50}
                step={100}
                value={intervalMs}
                onChange={(e) => setIntervalMs(Number(e.target.value))}
                disabled={cycling}
                className="w-16 bg-slate-800 border border-slate-700 rounded px-1.5 py-1"
              />
              ms
            </label>
            <button
              onClick={toggleCycle}
              disabled={!active || lines.length < 1}
              title="按间隔循环逐行发送（从当前游标开始）"
              className={`px-2 py-1.5 rounded disabled:opacity-40 flex items-center gap-1 cursor-pointer ${
                cycling ? "bg-red-600 hover:bg-red-500" : "bg-slate-700 hover:bg-slate-600"
              }`}
            >
              {cycling ? <Pause className="w-3.5 h-3.5" /> : <Play className="w-3.5 h-3.5" />}
              {cycling ? "停止循环" : "定时循环"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
