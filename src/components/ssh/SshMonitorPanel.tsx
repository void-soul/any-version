import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { RefreshCw, Trash2, X } from "lucide-react";

import { ignoreInvokeError, reportInvokeError } from "../../utils/invokeOrReport";
import { confirmHostKey } from "./hostKeyPrompt";

interface ProcessRow {
  pid: number;
  user: string;
  rssKb: number;
  cpu: number;
  name: string;
  command: string;
  location: string;
}

interface NetLinkRow {
  pid: number;
  name: string;
  proto: string;
  state: string;
  localIp: string;
  port: number;
}

const btn = "inline-flex items-center gap-1.5 rounded px-2.5 py-1.5 text-tiny border border-white/10 bg-black/30 text-slate-200 hover:bg-white/10 disabled:opacity-40";
const cell = "px-3 py-1.5";

/** FinalShell 式进程 / 监听端口面板；按需拉取，5 秒刷新一次。 */
export function SshMonitorPanel({ connectionId, view, title, onClose }: {
  connectionId: string;
  view: "process" | "net";
  title: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [sessionId, setSessionId] = useState("");
  const [processes, setProcesses] = useState<ProcessRow[]>([]);
  const [links, setLinks] = useState<NetLinkRow[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [sortKey, setSortKey] = useState<"cpu" | "rssKb" | "pid" | "user" | "name">("cpu");
  const [sortDesc, setSortDesc] = useState(true);
  const [selectedPid, setSelectedPid] = useState<number | null>(null);
  const sessionRef = useRef("");

  useEffect(() => {
    let disposed = false;
    void (async () => {
      const open = async () => invoke<string>("ssh_monitor_open", { connectionId });
      try {
        let sid: string;
        try {
          sid = await open();
        } catch (first) {
          if (disposed || !(await confirmHostKey(String(first), (m) => window.confirm(m)))) {
            throw first;
          }
          sid = await open();
        }
        if (disposed) {
          await invoke("ssh_monitor_close", { sessionId: sid })
            .catch((e) => ignoreInvokeError(e, "面板已卸载，监控会话由后端释放"));
          return;
        }
        sessionRef.current = sid;
        setSessionId(sid);
      } catch (e) {
        if (!disposed) setError(reportInvokeError(e, "ssh_monitor_open"));
      }
    })();

    return () => {
      disposed = true;
      const sid = sessionRef.current;
      sessionRef.current = "";
      if (sid) void invoke("ssh_monitor_close", { sessionId: sid })
        .catch((e) => ignoreInvokeError(e, "组件卸载时关闭监控会话"));
    };
  }, [connectionId]);

  const refresh = useCallback(async () => {
    const sid = sessionRef.current;
    if (!sid) return;
    setBusy(true);
    try {
      if (view === "process") {
        setProcesses(await invoke<ProcessRow[]>("ssh_monitor_processes", { sessionId: sid }));
      } else {
        setLinks(await invoke<NetLinkRow[]>("ssh_monitor_netlinks", { sessionId: sid }));
      }
      setError("");
    } catch (e) {
      setError(reportInvokeError(e, view === "process" ? "ssh_monitor_processes" : "ssh_monitor_netlinks"));
    } finally {
      setBusy(false);
    }
  }, [view]);

  useEffect(() => {
    if (!sessionId) return;
    void refresh();
    // 进程/网络比状态采样重，5 秒一次足够
    const timer = setInterval(() => void refresh(), 5000);
    return () => clearInterval(timer);
  }, [sessionId, refresh]);

  const sortedProcesses = useMemo(() => {
    const rows = [...processes];
    const direction = sortDesc ? -1 : 1;
    rows.sort((a, b) => {
      const left = a[sortKey];
      const right = b[sortKey];
      if (typeof left === "string" || typeof right === "string") {
        return String(left).localeCompare(String(right)) * direction;
      }
      return (left - right) * direction;
    });
    return rows;
  }, [processes, sortKey, sortDesc]);

  const toggleSort = (key: typeof sortKey) => {
    if (key === sortKey) {
      setSortDesc((value) => !value);
      return;
    }
    setSortKey(key);
    setSortDesc(key !== "user" && key !== "name");
  };

  const killSelected = async () => {
    if (!sessionId || !selectedPid) return;
    if (!window.confirm(t("ssh.confirmKill", { pid: selectedPid }))) return;
    try {
      setNotice(await invoke<string>("ssh_monitor_kill", { sessionId, pid: selectedPid }));
      setError("");
      setSelectedPid(null);
      await refresh();
    } catch (e) {
      setError(reportInvokeError(e, "ssh_monitor_kill"));
    }
  };

  const sortable = (key: typeof sortKey, label: string) => (
    <th
      className={`${cell} cursor-pointer select-none text-left`}
      onClick={() => toggleSort(key)}
    >
      {label}{sortKey === key ? (sortDesc ? " ↓" : " ↑") : ""}
    </th>
  );

  return (
    <div className="flex h-full min-h-0 flex-col gap-2 p-2 text-slate-200">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-tiny font-medium">{title}</span>
        <button className={btn} disabled={busy} onClick={() => void refresh()}>
          <RefreshCw className="h-3.5 w-3.5" />{t("sftp.refresh")}
        </button>
        {view === "process" && (
          <button
            className={`${btn} border-rose-400/30 text-rose-200`}
            disabled={!selectedPid || busy}
            title={t("ssh.killProcess")}
            onClick={() => void killSelected()}
          >
            <Trash2 className="h-3.5 w-3.5" />{t("ssh.killProcess")}
          </button>
        )}
        <button className={`${btn} ml-auto`} onClick={onClose}>
          <X className="h-3.5 w-3.5" />{t("ssh.close")}
        </button>
      </div>

      {error && <div className="rounded border border-rose-400/30 bg-rose-400/10 px-3 py-2 text-tiny text-rose-200">{error}</div>}
      {notice && <div className="rounded border border-emerald-400/30 bg-emerald-400/10 px-3 py-2 text-tiny text-emerald-200">{notice}</div>}

      <div className="min-h-0 flex-1 overflow-auto rounded border border-white/10">
        {view === "process" ? (
          <table className="w-full text-tiny">
            <thead className="sticky top-0 bg-slate-900/90 text-slate-400">
              <tr>
                {sortable("pid", t("ssh.colPid"))}
                {sortable("user", t("ssh.colUser"))}
                {sortable("rssKb", t("ssh.colMemory"))}
                {sortable("cpu", t("ssh.colCpu"))}
                {sortable("name", t("ssh.colName"))}
                <th className={`${cell} text-left`}>{t("ssh.colCommand")}</th>
                <th className={`${cell} text-left`}>{t("ssh.colLocation")}</th>
              </tr>
            </thead>
            <tbody>
              {!busy && processes.length === 0 && (
                <tr><td colSpan={7} className={`${cell} py-8 text-center text-slate-500`}>{t("sftp.empty")}</td></tr>
              )}
              {sortedProcesses.map((row) => (
                <tr
                  key={`${row.pid}-${row.command}`}
                  className={`cursor-pointer border-t border-white/5 hover:bg-white/5 ${
                    selectedPid === row.pid ? "bg-rose-500/10" : ""
                  }`}
                  onClick={() => setSelectedPid(row.pid)}
                >
                  <td className={`${cell} text-slate-400`}>{row.pid}</td>
                  <td className={cell}>{row.user}</td>
                  <td className={`${cell} text-slate-400`}>{(row.rssKb / 1024).toFixed(1)} MB</td>
                  <td className={`${cell} text-slate-400`}>{row.cpu.toFixed(1)}</td>
                  <td className={cell}>{row.name}</td>
                  <td className={`${cell} max-w-[36rem] truncate text-slate-400`} title={row.command}>{row.command}</td>
                  <td className={`${cell} max-w-[18rem] truncate text-slate-500`} title={row.location}>{row.location}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <table className="w-full text-tiny">
            <thead className="sticky top-0 bg-slate-900/90 text-slate-400">
              <tr>
                <th className={`${cell} text-left`}>{t("ssh.colPid")}</th>
                <th className={`${cell} text-left`}>{t("ssh.colName")}</th>
                <th className={`${cell} text-left`}>{t("ssh.colProto")}</th>
                <th className={`${cell} text-left`}>{t("ssh.colState")}</th>
                <th className={`${cell} text-left`}>{t("ssh.colLocalIp")}</th>
                <th className={`${cell} text-left`}>{t("ssh.colPort")}</th>
              </tr>
            </thead>
            <tbody>
              {!busy && links.length === 0 && (
                <tr><td colSpan={6} className={`${cell} py-8 text-center text-slate-500`}>{t("sftp.empty")}</td></tr>
              )}
              {links.map((row) => (
                <tr key={`${row.proto}-${row.localIp}-${row.port}-${row.pid}`} className="border-t border-white/5 hover:bg-white/5">
                  <td className={`${cell} text-slate-400`}>{row.pid || "—"}</td>
                  <td className={cell}>{row.name || "—"}</td>
                  <td className={`${cell} text-slate-400`}>{row.proto}</td>
                  <td className={`${cell} text-slate-400`}>{row.state}</td>
                  <td className={cell}>{row.localIp}</td>
                  <td className={`${cell} text-slate-400`}>{row.port}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
