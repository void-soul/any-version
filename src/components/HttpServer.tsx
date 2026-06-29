import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { open } from "@tauri-apps/plugin-dialog";
import { 
  FolderOpen, 
  Play, 
  Square, 
  ExternalLink, 
  Server, 
  AlertCircle,
  Copy,
  CheckCircle
} from "lucide-react";

interface RunningServer {
  port: number;
  path: string;
}

export default function HttpServer() {
  const [path, setPath] = useState("");
  const [port, setPort] = useState(8080);
  const [runningServers, setRunningServers] = useState<RunningServer[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copiedPort, setCopiedPort] = useState<number | null>(null);

  const fetchRunningServers = async () => {
    try {
      const list = await invoke<RunningServer[]>("get_running_http_servers");
      setRunningServers(list);
    } catch (e: any) {
      console.error("加载运行中的服务器失败", e);
    }
  };

  useEffect(() => {
    fetchRunningServers();
    const interval = setInterval(fetchRunningServers, 3000);
    return () => clearInterval(interval);
  }, []);

  const handleSelectFolder = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "选择静态服务目录",
      });
      if (selected && typeof selected === "string") {
        setPath(selected);
      }
    } catch (e) {
      console.error(e);
      alert("文件夹选择器打开失败，请手动输入路径。");
    }
  };

  const handleStart = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!path) {
      setError("请选择或输入要服务的目录");
      return;
    }
    if (!port || port < 1 || port > 65535) {
      setError("请输入有效的端口号 (1-65535)");
      return;
    }

    setLoading(true);
    setError(null);
    try {
      await invoke("start_http_server", { path, port });
      await fetchRunningServers();
    } catch (err: any) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  };

  const handleStop = async (p: number) => {
    try {
      await invoke("stop_http_server", { port: p });
      await fetchRunningServers();
    } catch (err: any) {
      alert(`停止服务失败: ${err}`);
    }
  };

  const handleCopyLink = (p: number) => {
    const link = `http://localhost:${p}`;
    navigator.clipboard.writeText(link);
    setCopiedPort(p);
    setTimeout(() => setCopiedPort(null), 2000);
  };

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-sm font-semibold text-white">静态 HTTP 服务</h3>
        <p className="text-[11px] text-slate-400 mt-0.5">选择本地任意文件夹并启动静态文件网页服务，方便本地调试或局域网预览文件（支持 CORS 与目录列表）。</p>
      </div>

      {/* 启动表单 */}
      <form onSubmit={handleStart} className="glass-panel border border-white/5 rounded-2xl p-5 bg-white/2 space-y-4">
        <div className="grid grid-cols-1 md:grid-cols-4 gap-4">
          <div className="md:col-span-3 space-y-1.5">
            <label className="text-[10px] text-slate-500 uppercase font-semibold">服务根目录</label>
            <div className="flex items-center gap-2">
              <input
                type="text"
                value={path}
                onChange={(e) => setPath(e.target.value)}
                placeholder="e.g. D:\project\dist"
                className="flex-grow glass-input px-3 py-2 text-xs font-mono"
              />
              <button
                type="button"
                onClick={handleSelectFolder}
                className="p-2 bg-white/5 hover:bg-white/10 border border-white/5 text-slate-400 hover:text-slate-200 rounded-lg cursor-pointer transition-colors"
                title="选择文件夹"
              >
                <FolderOpen className="w-4 h-4" />
              </button>
            </div>
          </div>
          <div className="space-y-1.5">
            <label className="text-[10px] text-slate-500 uppercase font-semibold">服务端口</label>
            <input
              type="number"
              value={port}
              onChange={(e) => setPort(parseInt(e.target.value) || 8080)}
              min={1}
              max={65535}
              className="w-full glass-input px-3 py-2 text-xs font-mono"
            />
          </div>
        </div>

        {error && (
          <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-200 text-xs rounded-xl flex items-center gap-2">
            <AlertCircle className="w-4 h-4 text-red-400 flex-shrink-0" />
            <span>{error}</span>
          </div>
        )}

        <div className="flex justify-end">
          <button
            type="submit"
            disabled={loading}
            className="px-5 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold shadow-lg shadow-blue-500/20 cursor-pointer transition-all flex items-center gap-1.5"
          >
            <Play className="w-3.5 h-3.5" />
            {loading ? "正在启动..." : "启动服务"}
          </button>
        </div>
      </form>

      {/* 运行中的服务列表 */}
      <div className="glass-panel border border-white/5 rounded-2xl p-5 bg-white/2 space-y-4">
        <div className="flex items-center gap-2 border-b border-white/5 pb-3">
          <Server className="w-4 h-4 text-blue-400" />
          <h4 className="text-xs font-semibold text-white">正在运行的服务 ({runningServers.length})</h4>
        </div>

        {runningServers.length === 0 ? (
          <div className="text-center py-10 text-slate-500 text-xs">
            暂无运行中的静态服务器，在上方选择目录以开始。
          </div>
        ) : (
          <div className="space-y-3">
            {runningServers.map((srv) => (
              <div 
                key={srv.port}
                className="flex flex-col md:flex-row md:items-center justify-between p-3.5 bg-black/20 border border-white/5 rounded-xl gap-3"
              >
                <div className="space-y-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="w-2.5 h-2.5 rounded-full bg-emerald-500 animate-pulse" />
                    <button
                      onClick={() => openUrl(`http://localhost:${srv.port}`)}
                      className="text-xs font-bold font-mono text-emerald-400 hover:underline flex items-center gap-1 cursor-pointer"
                    >
                      http://localhost:{srv.port}
                      <ExternalLink className="w-3 h-3" />
                    </button>
                  </div>
                  <p className="text-[10px] text-slate-400 font-mono truncate" title={srv.path}>
                    目录: {srv.path}
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  <button
                    onClick={() => handleCopyLink(srv.port)}
                    className="p-2 bg-white/5 hover:bg-white/10 text-slate-400 hover:text-slate-200 rounded-lg border border-white/5 cursor-pointer transition-colors"
                    title="复制链接"
                  >
                    {copiedPort === srv.port ? (
                      <CheckCircle className="w-3.5 h-3.5 text-emerald-400" />
                    ) : (
                      <Copy className="w-3.5 h-3.5" />
                    )}
                  </button>
                  <button
                    onClick={() => handleStop(srv.port)}
                    className="px-3 py-2 bg-red-600/20 hover:bg-red-600/30 border border-red-500/20 hover:border-red-500/30 text-red-300 rounded-lg text-[10px] font-semibold cursor-pointer transition-all flex items-center gap-1"
                    title="关闭服务"
                  >
                    <Square className="w-3 h-3" />
                    停止
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
