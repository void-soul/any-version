import { useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import {
  Search, Trash2, ShieldAlert, CheckCircle,
  AlertTriangle, Cpu, List
} from "lucide-react";

interface PortOwner {
  port: string;
  pid: string;
  process_name: string;
}

interface PortStatus {
  port: number;
  free: boolean;
  reserved: boolean;
  occupied: boolean;
  owner: PortOwner | null;
}

interface ReservedRange {
  start: number;
  end: number;
  process: string;
  /** 系统动态保留（netsh 输出带 `*`）：不是用户加的段，删除通常被拒绝 */
  managed?: boolean;
}

export default function PortScanner() {
  const { t } = useTranslation();
  const [portInput, setPortInput] = useState("");
  const [status, setStatus] = useState<PortStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [releasing, setReleasing] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [successMsg, setSuccessMsg] = useState<string | null>(null);
  const [reservedRanges, setReservedRanges] = useState<ReservedRange[] | null>(null);
  const [loadingReserved, setLoadingReserved] = useState(false);
  // 保留端口管理：起始端口 + 数量（添加/删除都按「起始 + 数量」这一组参数，与 netsh 一致）
  const [rangeStart, setRangeStart] = useState("");
  const [rangeCount, setRangeCount] = useState("");
  const [rangeBusy, setRangeBusy] = useState(false);

  const runRange = async (action: "add" | "delete") => {
    const start = Number(rangeStart.trim());
    const count = Number(rangeCount.trim() || "1");
    if (!Number.isInteger(start) || !Number.isInteger(count)) {
      setErrorMsg(t("portscan.rangeInvalid"));
      return;
    }
    setRangeBusy(true);
    setErrorMsg(null);
    setSuccessMsg(null);
    try {
      await invoke(action === "add" ? "add_reserved_ports" : "delete_reserved_ports", {
        start,
        count,
        protocol: "tcp",
      });
      setSuccessMsg(
        action === "add"
          ? t("portscan.addedRange", { start, end: start + count - 1 })
          : t("portscan.deletedRange", { start, end: start + count - 1 }),
      );
      // 列表变了，重新拉一次
      setReservedRanges(await invoke<ReservedRange[]>("get_reserved_ports"));
    } catch (e: any) {
      setErrorMsg(String(e));
    } finally {
      setRangeBusy(false);
    }
  };

  const deleteRow = async (r: ReservedRange) => {
    if (r.managed) return;
    setRangeBusy(true);
    setErrorMsg(null);
    setSuccessMsg(null);
    try {
      await invoke("delete_reserved_ports", {
        start: r.start,
        count: r.end - r.start + 1,
        protocol: "tcp",
      });
      setSuccessMsg(t("portscan.deletedRange", { start: r.start, end: r.end }));
      setReservedRanges(await invoke<ReservedRange[]>("get_reserved_ports"));
    } catch (e: any) {
      setErrorMsg(String(e));
    } finally {
      setRangeBusy(false);
    }
  };

  const handleCheck = async () => {
    if (!portInput.trim()) return;
    setChecking(true);
    setErrorMsg(null);
    setSuccessMsg(null);
    try {
      const res = await invoke<PortStatus>("check_port_status", { portStr: portInput.trim() });
      setStatus(res);
    } catch (e: any) {
      setErrorMsg(e);
      setStatus(null);
    } finally {
      setChecking(false);
    }
  };

  const handleRelease = async () => {
    if (!status || !status.occupied || !status.owner) return;
    if (!confirm(t("portscan.releaseConfirm", { proc: status.owner.process_name, pid: status.owner.pid, port: status.port }))) return;
    setReleasing(true);
    setErrorMsg(null);
    setSuccessMsg(null);
    try {
      const msg = await invoke<string>("kill_port_owner", { portStr: status.port.toString() });
      setSuccessMsg(msg);
      const res = await invoke<PortStatus>("check_port_status", { portStr: status.port.toString() });
      setStatus(res);
    } catch (e: any) {
      setErrorMsg(e);
    } finally {
      setReleasing(false);
    }
  };

  const handleShowReserved = async () => {
    if (reservedRanges) { setReservedRanges(null); return; }
    setLoadingReserved(true);
    setErrorMsg(null);
    try {
      const ranges = await invoke<ReservedRange[]>("get_reserved_ports");
      setReservedRanges(ranges);
    } catch (e: any) {
      setErrorMsg(e);
    } finally {
      setLoadingReserved(false);
    }
  };

  return (
    <div className="grid h-full min-h-0 grid-cols-2 gap-4">
      {/* 端口查询 */}
      <div className="glass-panel flex min-h-0 flex-col rounded-2xl border border-white/5 p-5">
        <div className="flex shrink-0 items-center gap-2 border-b border-white/5 pb-2">
          <Search className="w-4 h-4 text-blue-400" />
          <h4 className="font-semibold text-white text-xs">{t("portscan.title")}</h4>
        </div>

        <div className="mt-3 flex shrink-0 gap-2">
          <input
            type="number"
            value={portInput}
            onChange={(e) => setPortInput(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleCheck()}
            className="flex-1 glass-input px-3 py-2 text-xs"
            placeholder={t("portscan.portPh")}
          />
          <button onClick={handleCheck} disabled={checking || !portInput}
            className="px-4 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-lg text-xs font-semibold cursor-pointer transition-all">
            {checking ? t("portscan.checking") : t("portscan.check")}
          </button>
        </div>

        <div className="mt-3 min-h-0 flex-1 overflow-y-auto">
          {errorMsg && (
            <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-400 rounded-xl text-xs flex items-center gap-1.5">
              <AlertTriangle className="w-3.5 h-3.5" /> {errorMsg}
            </div>
          )}
          {successMsg && (
            <div className="p-3 bg-emerald-500/10 border border-emerald-500/20 text-emerald-400 rounded-xl text-xs flex items-center gap-1.5">
              <CheckCircle className="w-3.5 h-3.5" /> {successMsg}
            </div>
          )}

          {status && (
            <div className="space-y-3">
              <div className="flex items-center justify-between bg-black/20 p-3 rounded-xl border border-white/5">
                <div>
                  <span className="text-[10px] text-slate-500 font-mono">{t("portscan.portLabel", { n: status.port })}</span>
                  <p className="text-xs text-white font-semibold mt-0.5">
                    {status.free && t("portscan.free")}
                    {status.occupied && t("portscan.occupiedBy", { name: status.owner?.process_name })}
                    {!status.occupied && status.reserved && t("portscan.reserved")}
                  </p>
                </div>
                {status.occupied ? (
                  <span className="px-2 py-0.5 rounded bg-red-500/10 border border-red-500/20 text-[9px] text-red-400 font-semibold">{t("portscan.occupiedBadge")}</span>
                ) : status.reserved ? (
                  <span className="px-2 py-0.5 rounded bg-amber-500/10 border border-amber-500/20 text-[9px] text-amber-400 font-semibold">{t("portscan.reservedBadge")}</span>
                ) : (
                  <span className="px-2 py-0.5 rounded bg-emerald-500/10 border border-emerald-500/20 text-[9px] text-emerald-400 font-semibold">{t("portscan.freeBadge")}</span>
                )}
              </div>

              {status.reserved && (
                <p className="text-[10px] text-amber-400/90 bg-amber-500/5 border border-amber-500/10 p-2.5 rounded-lg flex items-start gap-1.5">
                  <ShieldAlert className="w-3 h-3 mt-0.5 flex-shrink-0" />
                  {t("portscan.reservedHint")}
                </p>
              )}

              {status.occupied && status.owner && (
                <div className="bg-black/10 border border-white/5 rounded-xl p-3 space-y-3">
                  <div className="grid grid-cols-2 gap-3 font-mono text-[10px]">
                    <div>
                      <span className="text-slate-500 block">{t("portscan.pid")}</span>
                      <span className="text-slate-300 font-semibold">{status.owner.pid}</span>
                    </div>
                    <div>
                      <span className="text-slate-500 block">{t("portscan.procName")}</span>
                      <span className="text-slate-300 font-semibold flex items-center gap-1">
                        <Cpu className="w-3 h-3 text-blue-400" /> {status.owner.process_name}
                      </span>
                    </div>
                  </div>
                  <button onClick={handleRelease} disabled={releasing}
                    className="w-full py-2 bg-red-600 hover:bg-red-500 disabled:opacity-50 text-white rounded-lg text-xs font-semibold cursor-pointer transition-all flex items-center justify-center gap-1.5">
                    <Trash2 className="w-3 h-3" />
                    {releasing ? t("portscan.killing") : t("portscan.killProc")}
                  </button>
                </div>
              )}
            </div>
          )}
        </div>
      </div>

      {/* 系统保留端口 */}
      <div className="glass-panel flex min-h-0 flex-col rounded-2xl border border-white/5 p-5">
        <div className="flex shrink-0 items-center justify-between">
          <div className="flex items-center gap-2">
            <ShieldAlert className="w-4 h-4 text-amber-400" />
            <h4 className="font-semibold text-white text-xs">{t("portscan.reservedTitle")}</h4>
          </div>
          <button onClick={handleShowReserved} disabled={loadingReserved}
            className="flex items-center gap-1.5 px-3 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg text-[10px] border border-white/5 cursor-pointer">
            <List className={`w-3 h-3 ${loadingReserved ? "animate-spin" : ""}`} />
            {reservedRanges ? t("portscan.collapse") : loadingReserved ? t("portscan.loading") : t("portscan.viewReserved")}
          </button>
        </div>

        {/* 管理：添加 / 删除一段保留端口（netsh excludedportrange，需要管理员权限） */}
        {reservedRanges && (
          <div className="mt-3 shrink-0 flex flex-wrap items-center gap-2">
            <input
              value={rangeStart}
              onChange={(e) => setRangeStart(e.target.value)}
              placeholder={t("portscan.rangeStartPh")}
              className="w-24 glass-input px-2 py-1 text-[10px]"
            />
            <input
              value={rangeCount}
              onChange={(e) => setRangeCount(e.target.value)}
              placeholder={t("portscan.rangeCountPh")}
              className="w-20 glass-input px-2 py-1 text-[10px]"
            />
            <button
              onClick={() => void runRange("add")}
              disabled={rangeBusy || !rangeStart.trim()}
              className="px-2.5 py-1 rounded-md text-[10px] font-semibold bg-amber-500/20 hover:bg-amber-500/30 text-amber-200 cursor-pointer disabled:opacity-50"
            >
              {t("portscan.addRange")}
            </button>
            <button
              onClick={() => void runRange("delete")}
              disabled={rangeBusy || !rangeStart.trim()}
              className="px-2.5 py-1 rounded-md text-[10px] font-semibold bg-rose-600/20 hover:bg-rose-600/30 text-rose-300 cursor-pointer disabled:opacity-50"
            >
              {t("portscan.deleteRange")}
            </button>
          </div>
        )}
        {reservedRanges && <p className="mt-1.5 shrink-0 text-[9px] text-slate-500">{t("portscan.rangeAdminHint")}</p>}

        <div className="mt-3 min-h-0 flex-1 overflow-y-auto">
          {reservedRanges && (
            reservedRanges.length === 0 ? (
              <p className="text-[10px] text-slate-500 py-2">{t("portscan.noReserved")}</p>
            ) : (
              <table className="w-full text-[10px]">
                <thead className="sticky top-0 bg-surface-deep">
                  <tr className="text-slate-400 font-semibold border-b border-white/5">
                    <td className="py-1.5 pr-3">{t("portscan.startPort")}</td>
                    <td className="py-1.5 pr-3">{t("portscan.endPort")}</td>
                    <td className="py-1.5 pr-3">{t("portscan.portCount")}</td>
                    <td className="py-1.5 pr-3">{t("portscan.relatedProc")}</td>
                    <td className="py-1.5">{t("portscan.op")}</td>
                  </tr>
                </thead>
                <tbody className="text-slate-300 divide-y divide-white/[0.03]">
                  {reservedRanges.map((r, i) => (
                    <tr key={i} className="hover:bg-white/[0.02]">
                      <td className="py-1 font-mono">
                        {r.managed && <span className="mr-0.5 text-amber-400" title={t("portscan.managedHint")}>*</span>}
                        {r.start}
                      </td>
                      <td className="py-1 font-mono">{r.end}</td>
                      <td className="py-1 font-mono text-slate-500">{r.end - r.start + 1}</td>
                      <td className="py-1 text-slate-400">{r.process || "-"}</td>
                      <td className="py-1">
                        <button
                          onClick={() => void deleteRow(r)}
                          disabled={rangeBusy || r.managed}
                          title={r.managed ? t("portscan.managedHint") : t("portscan.deleteRow")}
                          className="px-1.5 py-0.5 rounded text-[9px] bg-rose-600/15 hover:bg-rose-600/25 text-rose-300 cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed"
                        >
                          {t("portscan.deleteRow")}
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )
          )}
        </div>
      </div>
    </div>
  );
}
