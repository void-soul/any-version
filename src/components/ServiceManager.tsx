import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { 
  Play, 
  Square, 
  RefreshCw, 
  AlertCircle,
  Database,
  Link,
  Cpu
} from "lucide-react";

interface ServiceInfo {
  name: string;
  status: string; // "running" | "stopped" | "not_installed"
  active_version: string;
  port: string;
  pid: number;
}

interface SdkInfo {
  name: string;
  category: string;
  active_version: string;
  installed_versions: string[];
}

export default function ServiceManager() {
  const [services, setServices] = useState<ServiceInfo[]>([]);
  const [sdks, setSdks] = useState<SdkInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [togglingName, setTogglingName] = useState<string | null>(null);

  const fetchData = async () => {
    setLoading(true);
    try {
      const svcs = await invoke<ServiceInfo[]>("get_running_services");
      const list = await invoke<SdkInfo[]>("get_sdks_list");
      setServices(svcs);
      setSdks(list);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchData();
  }, []);

  const handleStart = async (name: string, version: string) => {
    setTogglingName(name);
    try {
      await invoke("start_service", { name, version });
      await new Promise(r => setTimeout(r, 1000)); // wait brief moment for process spawn
      await fetchData();
    } catch (e: any) {
      alert(`启动失败: ${e}`);
    } finally {
      setTogglingName(null);
    }
  };

  const handleStop = async (name: string) => {
    setTogglingName(name);
    try {
      await invoke("stop_service", { name });
      await new Promise(r => setTimeout(r, 1000)); // wait brief moment for process teardown
      await fetchData();
    } catch (e: any) {
      alert(`停止失败: ${e}`);
    } finally {
      setTogglingName(null);
    }
  };

  const getStatusBadge = (status: string) => {
    switch (status) {
      case "running":
        return <span className="px-2 py-0.5 rounded-md text-[10px] font-semibold bg-emerald-500/10 text-emerald-400 border border-emerald-500/20">运行中</span>;
      case "not_installed":
        return <span className="px-2 py-0.5 rounded-md text-[10px] font-semibold bg-red-500/10 text-red-400 border border-red-500/20">未安装</span>;
      default:
        return <span className="px-2 py-0.5 rounded-md text-[10px] font-semibold bg-slate-500/10 text-slate-400 border border-slate-500/20">已停止</span>;
    }
  };

  return (
    <div className="flex-1 p-8 overflow-y-auto space-y-6 h-screen select-none">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-semibold text-white tracking-wide">本地服务管理</h2>
          <p className="text-xs text-slate-400 mt-1">管理 Nginx Web 服务器及 MySQL, Redis, MongoDB, PostgreSQL 数据库服务状态</p>
        </div>

        <button
          onClick={fetchData}
          disabled={loading}
          className="flex items-center gap-2 px-3.5 py-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-xl text-xs border border-white/5 cursor-pointer transition-all"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${loading ? "animate-spin" : ""}`} />
          刷新状态
        </button>
      </div>

      {/* Services grid */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
        {services.map((svc) => {
          const sdk = sdks.find(s => s.name === svc.name);
          const installed = sdk?.installed_versions || [];
          const isToggling = togglingName === svc.name;
          
          return (
            <div 
              key={svc.name}
              className="glass-panel rounded-2xl p-6 border border-white/5 space-y-4 flex flex-col justify-between"
            >
              {/* Service title & status */}
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <div className="w-10 h-10 rounded-xl bg-white/5 flex items-center justify-center text-slate-300 border border-white/5">
                    <Database className="w-5 h-5" />
                  </div>
                  <div>
                    <h3 className="text-sm font-semibold text-white capitalize">{svc.name}</h3>
                    <p className="text-[10px] text-slate-500 font-mono">
                      端口: {svc.port || "未定义"}
                    </p>
                  </div>
                </div>
                {getStatusBadge(svc.status)}
              </div>

              {/* Detail fields */}
              {svc.status === "running" && (
                <div className="grid grid-cols-2 gap-4 py-2 bg-black/20 rounded-xl p-3.5 border border-white/5 font-mono text-[11px]">
                  <div>
                    <span className="text-[10px] text-slate-500 block">启用版本</span>
                    <span className="text-slate-300 font-medium">{svc.active_version}</span>
                  </div>
                  <div>
                    <span className="text-[10px] text-slate-500 block">进程 PID</span>
                    <span className="text-slate-300 font-medium">{svc.pid}</span>
                  </div>
                </div>
              )}

              {/* Start form / Actions */}
              <div className="pt-2 flex items-center gap-3">
                {svc.status === "running" ? (
                  <button
                    onClick={() => handleStop(svc.name)}
                    disabled={isToggling}
                    className="flex-1 py-2 bg-red-600 hover:bg-red-500 disabled:opacity-50 text-white rounded-lg text-xs font-semibold cursor-pointer transition-all flex items-center justify-center gap-1.5 shadow-lg shadow-red-500/10"
                  >
                    <Square className="w-3.5 h-3.5 fill-current" />
                    {isToggling ? "正在停止..." : "停止服务"}
                  </button>
                ) : svc.status === "not_installed" ? (
                  <div className="flex-1 text-[11px] text-slate-500 flex items-center gap-1.5 bg-white/3 border border-white/5 p-3 rounded-lg">
                    <AlertCircle className="w-4 h-4 text-amber-500" />
                    请先在“SDK 版本管理”中下载或注册此服务的版本
                  </div>
                ) : (
                  <>
                    <select
                      id={`select-version-${svc.name}`}
                      className="flex-1 glass-input px-3 py-2 text-xs"
                      defaultValue={svc.active_version || (installed.length > 0 ? installed[0] : "")}
                      disabled={installed.length === 0}
                    >
                      {installed.length === 0 && <option value="">未安装任何版本</option>}
                      {installed.map((v) => (
                        <option key={v} value={v}>{v}</option>
                      ))}
                    </select>

                    <button
                      onClick={() => {
                        const el = document.getElementById(`select-version-${svc.name}`) as HTMLSelectElement;
                        if (el && el.value) handleStart(svc.name, el.value);
                      }}
                      disabled={isToggling || installed.length === 0}
                      className="px-5 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-lg text-xs font-semibold cursor-pointer transition-all flex items-center justify-center gap-1.5 shadow-lg shadow-blue-500/10"
                    >
                      <Play className="w-3.5 h-3.5 fill-current" />
                      {isToggling ? "正在启动..." : "启动服务"}
                    </button>
                  </>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
