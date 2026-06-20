import React, { useState } from "react";
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
}

export default function PortScanner() {
  const [portInput, setPortInput] = useState("");
  const [status, setStatus] = useState<PortStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [releasing, setReleasing] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [successMsg, setSuccessMsg] = useState<string | null>(null);
  const [reservedRanges, setReservedRanges] = useState<ReservedRange[] | null>(null);
  const [loadingReserved, setLoadingReserved] = useState(false);

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
    if (!confirm(`确定要结束进程 ${status.owner.process_name} (PID: ${status.owner.pid}) 以释放端口 ${status.port} 吗？`)) return;
    setReleasing(true);
    setErrorMsg(null);
    setSuccessMsg(null);
    try {
      await invoke("kill_port_owner", { portStr: status.port.toString() });
      setSuccessMsg(`进程 ${status.owner.process_name} 已终止，端口已释放！`);
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
    <div className="space-y-4">
      {/* 端口查询 */}
      <div className="glass-panel rounded-2xl p-5 border border-white/5 space-y-4">
        <div className="flex items-center gap-2 pb-2 border-b border-white/5">
          <Search className="w-4 h-4 text-blue-400" />
          <h4 className="font-semibold text-white text-xs">TCP 端口排查</h4>
        </div>

        <div className="flex gap-2">
          <input
            type="number"
            value={portInput}
            onChange={(e) => setPortInput(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleCheck()}
            className="flex-1 glass-input px-3 py-2 text-xs"
            placeholder="输入端口号 (如 8080)"
          />
          <button onClick={handleCheck} disabled={checking || !portInput}
            className="px-4 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-lg text-xs font-semibold cursor-pointer transition-all">
            {checking ? "排查中..." : "排查"}
          </button>
        </div>

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
                <span className="text-[10px] text-slate-500 font-mono">端口 {status.port}</span>
                <p className="text-xs text-white font-semibold mt-0.5">
                  {status.free && "空闲可用"}
                  {status.occupied && `被 ${status.owner?.process_name} 占用`}
                  {!status.occupied && status.reserved && "系统保留端口（无法使用）"}
                </p>
              </div>
              {status.occupied ? (
                <span className="px-2 py-0.5 rounded bg-red-500/10 border border-red-500/20 text-[9px] text-red-400 font-semibold">被占用</span>
              ) : status.reserved ? (
                <span className="px-2 py-0.5 rounded bg-amber-500/10 border border-amber-500/20 text-[9px] text-amber-400 font-semibold">保留</span>
              ) : (
                <span className="px-2 py-0.5 rounded bg-emerald-500/10 border border-emerald-500/20 text-[9px] text-emerald-400 font-semibold">空闲</span>
              )}
            </div>

            {status.reserved && (
              <p className="text-[10px] text-amber-400/90 bg-amber-500/5 border border-amber-500/10 p-2.5 rounded-lg flex items-start gap-1.5">
                <ShieldAlert className="w-3 h-3 mt-0.5 flex-shrink-0" />
                此端口在 Windows 动态排除范围内，无法绑定。可运行 netsh 管理排除段。
              </p>
            )}

            {status.occupied && status.owner && (
              <div className="bg-black/10 border border-white/5 rounded-xl p-3 space-y-3">
                <div className="grid grid-cols-2 gap-3 font-mono text-[10px]">
                  <div>
                    <span className="text-slate-500 block">PID</span>
                    <span className="text-slate-300 font-semibold">{status.owner.pid}</span>
                  </div>
                  <div>
                    <span className="text-slate-500 block">进程名</span>
                    <span className="text-slate-300 font-semibold flex items-center gap-1">
                      <Cpu className="w-3 h-3 text-blue-400" /> {status.owner.process_name}
                    </span>
                  </div>
                </div>
                <button onClick={handleRelease} disabled={releasing}
                  className="w-full py-2 bg-red-600 hover:bg-red-500 disabled:opacity-50 text-white rounded-lg text-xs font-semibold cursor-pointer transition-all flex items-center justify-center gap-1.5">
                  <Trash2 className="w-3 h-3" />
                  {releasing ? "终止中..." : "强制终止进程"}
                </button>
              </div>
            )}
          </div>
        )}
      </div>

      {/* 系统保留端口 */}
      <div className="glass-panel rounded-2xl p-5 border border-white/5 space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <ShieldAlert className="w-4 h-4 text-amber-400" />
            <h4 className="font-semibold text-white text-xs">系统保留端口</h4>
          </div>
          <button onClick={handleShowReserved} disabled={loadingReserved}
            className="flex items-center gap-1.5 px-3 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg text-[10px] border border-white/5 cursor-pointer">
            <List className={`w-3 h-3 ${loadingReserved ? "animate-spin" : ""}`} />
            {reservedRanges ? "收起" : loadingReserved ? "加载中..." : "查看保留端口"}
          </button>
        </div>

        {reservedRanges && (
          reservedRanges.length === 0 ? (
            <p className="text-[10px] text-slate-500 py-2">未检测到系统保留端口范围。</p>
          ) : (
            <div className="max-h-64 overflow-y-auto">
              <table className="w-full text-[10px]">
                <thead>
                  <tr className="text-slate-400 font-semibold border-b border-white/5">
                    <td className="py-1.5 pr-3">起始端口</td>
                    <td className="py-1.5 pr-3">结束端口</td>
                    <td className="py-1.5 pr-3">端口数</td>
                    <td className="py-1.5">关联进程</td>
                  </tr>
                </thead>
                <tbody className="text-slate-300 divide-y divide-white/[0.03]">
                  {reservedRanges.map((r, i) => (
                    <tr key={i} className="hover:bg-white/[0.02]">
                      <td className="py-1 font-mono">{r.start}</td>
                      <td className="py-1 font-mono">{r.end}</td>
                      <td className="py-1 font-mono text-slate-500">{r.end - r.start + 1}</td>
                      <td className="py-1 text-slate-400">{r.process || "-"}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )
        )}
      </div>
    </div>
  );
}
