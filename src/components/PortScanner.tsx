import React, { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { 
  Search, 
  Trash2, 
  HelpCircle, 
  ShieldAlert, 
  CheckCircle,
  AlertTriangle,
  Cpu
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

interface PortStatusTs {
  port: number;
  free: boolean;
  reserved: boolean;
  occupied: boolean;
  owner: PortOwner | null;
}

export default function PortScanner() {
  const [portInput, setPortInput] = useState("");
  const [status, setStatus] = useState<PortStatusTs | null>(null);
  const [checking, setChecking] = useState(false);
  const [releasing, setReleasing] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [successMsg, setSuccessMsg] = useState<string | null>(null);

  const handleCheck = async () => {
    if (!portInput.trim()) return;
    setChecking(true);
    setErrorMsg(null);
    setSuccessMsg(null);
    try {
      const res = await invoke<PortStatusTs>("check_port_status", { portStr: portInput.trim() });
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
      setSuccessMsg(`进程 ${status.owner.process_name} 已成功终止，端口已释放！`);
      // Re-check
      const res = await invoke<PortStatusTs>("check_port_status", { portStr: status.port.toString() });
      setStatus(res);
    } catch (e: any) {
      setErrorMsg(e);
    } finally {
      setReleasing(false);
    }
  };

  return (
    <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-5">
      <div className="flex items-center gap-2 pb-3 border-b border-white/5">
        <Search className="w-4 h-4 text-blue-400" />
        <h4 className="font-semibold text-white text-xs">TCP 端口扫描工具</h4>
      </div>

      <div className="flex gap-2">
        <input 
          type="number"
          value={portInput}
          onChange={(e) => setPortInput(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handleCheck()}
          className="flex-1 glass-input px-3.5 py-2 text-xs"
          placeholder="输入要排查的端口号 (e.g. 8080, 3306)"
        />
        <button
          onClick={handleCheck}
          disabled={checking || !portInput}
          className="px-5 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-lg text-xs font-semibold cursor-pointer transition-all"
        >
          {checking ? "正在排查..." : "排查端口"}
        </button>
      </div>

      {errorMsg && (
        <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-400 rounded-xl text-xs flex items-center gap-1.5 font-medium">
          <AlertTriangle className="w-4 h-4" />
          {errorMsg}
        </div>
      )}

      {successMsg && (
        <div className="p-3 bg-emerald-500/10 border border-emerald-500/20 text-emerald-400 rounded-xl text-xs flex items-center gap-1.5 font-medium">
          <CheckCircle className="w-4 h-4" />
          {successMsg}
        </div>
      )}

      {status && (
        <div className="space-y-4 pt-2">
          {/* Status badge and description */}
          <div className="flex items-center gap-3 justify-between bg-black/20 p-4 rounded-2xl border border-white/5">
            <div>
              <span className="text-[10px] text-slate-500 font-mono block">端口 {status.port} 状态</span>
              <span className="text-xs text-white font-semibold mt-1 block">
                {status.free && "该端口处于空闲可用状态"}
                {status.occupied && `已被进程 ${status.owner?.process_name} 占用`}
                {!status.occupied && status.reserved && "该端口未被占用，但属于系统保留端口"}
              </span>
            </div>

            <div className="flex flex-col gap-1.5 items-end">
              {status.occupied ? (
                <span className="px-2 py-0.5 rounded bg-red-500/10 border border-red-500/20 text-[9px] text-red-400 font-semibold">被占用</span>
              ) : status.reserved ? (
                <span className="px-2 py-0.5 rounded bg-amber-500/10 border border-amber-500/20 text-[9px] text-amber-400 font-semibold">保留排除</span>
              ) : (
                <span className="px-2 py-0.5 rounded bg-emerald-500/10 border border-emerald-500/20 text-[9px] text-emerald-400 font-semibold">空闲可用</span>
              )}
            </div>
          </div>

          {/* Reserved warning info */}
          {status.reserved && (
            <p className="text-[10px] text-amber-400/90 leading-relaxed bg-amber-500/5 border border-amber-500/10 p-3 rounded-xl flex items-start gap-1.5">
              <ShieldAlert className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" />
              此端口位于 Windows 系统动态排除端口段内（排除端口将无法绑定监听）。建议尝试更换其他端口，或运行 netsh 管理 TCP 排除段。
            </p>
          )}

          {/* Occupier information */}
          {status.occupied && status.owner && (
            <div className="space-y-3.5 bg-black/10 border border-white/5 rounded-2xl p-4">
              <div className="grid grid-cols-2 gap-4 font-mono text-[10px]">
                <div>
                  <span className="text-slate-500 block">进程 PID</span>
                  <span className="text-slate-300 font-semibold">{status.owner.pid}</span>
                </div>
                <div>
                  <span className="text-slate-500 block">可执行文件名称</span>
                  <span className="text-slate-300 font-semibold flex items-center gap-1">
                    <Cpu className="w-3 h-3 text-blue-400" />
                    {status.owner.process_name}
                  </span>
                </div>
              </div>

              <button
                onClick={handleRelease}
                disabled={releasing}
                className="w-full py-2 bg-red-600 hover:bg-red-500 disabled:opacity-50 text-white rounded-lg text-xs font-semibold cursor-pointer transition-all flex items-center justify-center gap-1.5 shadow-md shadow-red-500/10"
              >
                <Trash2 className="w-3.5 h-3.5" />
                {releasing ? "正在终止进程并释放端口..." : "强制终止占用进程并释放端口"}
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
