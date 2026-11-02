import { useState, useEffect, useRef } from "react";

import { useTranslation } from "react-i18next";
import { btnPrimary, btnDanger, inputCls, labelCls } from "./shared/Button";
import { alertError } from "./shared/ThemedAlert";
import { toastPortConflict } from "../utils/portConflict";
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
  AlertTriangle,
  Copy,
  CheckCircle
} from "lucide-react";

interface RunningServer {
  port: number;
  path: string;
}

export default function HttpServer() {
  const { t } = useTranslation();
  const [path, setPath] = useState("");
  const [portInput, setPortInput] = useState("8080");
  const port = Number(portInput);
  const [runningServers, setRunningServers] = useState<RunningServer[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copiedPort, setCopiedPort] = useState<number | null>(null);
  const [allowLan, setAllowLan] = useState(false);

  const [statusError, setStatusError] = useState("");
  const [stopping, setStopping] = useState<number[]>([]);
  const life = useRef(0);
  const mounted = useRef(false);
  const listRequest = useRef(0);
  const listPending = useRef<number | null>(null);
  const starting = useRef(false);
  const stops = useRef(new Set<number>());
  const copyTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const fetchRunningServers = async (force = false) => {
    if (!mounted.current || (!force && listPending.current !== null)) return;
    const epoch = life.current;
    const request = ++listRequest.current;
    listPending.current = request;
    try {
      const list = await invoke<RunningServer[]>("get_running_http_servers");
      if (mounted.current && epoch === life.current && request === listRequest.current) { setRunningServers(list); setStatusError(""); }
    } catch (e: any) {
      if (mounted.current && epoch === life.current && request === listRequest.current) setStatusError(String(e));
    } finally {
      if (listPending.current === request) listPending.current = null;
    }
  };

  useEffect(() => {
    mounted.current = true;
    void fetchRunningServers();
    const interval = setInterval(() => { if (!document.hidden) void fetchRunningServers(); }, 3000);
    return () => {
      mounted.current = false; life.current++; listRequest.current++; listPending.current = null;
      clearInterval(interval);
      if (copyTimer.current) clearTimeout(copyTimer.current);
    };
  }, []);

  const handleSelectFolder = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: t("httpserver.pickDirTitle"),
      });
      if (selected && typeof selected === "string") {
        setPath(selected);
      }
    } catch (e) {
      console.error(e);
      alertError(t("httpserver.pickerFail"));
    }
  };

  const handleStart = async (e: React.FormEvent) => {
    e.preventDefault();
    if (starting.current) return;
    if (!path.trim()) {
      setError(t("httpserver.needDir"));
      return;
    }
    if (!/^\d+$/.test(portInput) || !Number.isInteger(port) || port < 1 || port > 65535) {
      setError(t("httpserver.invalidPort"));
      return;
    }

    starting.current = true;
    const epoch = life.current;
    setLoading(true);
    setError(null);
    try {
      await invoke("start_http_server", {
        path: path.trim(),
        port,
        // 默认仅本地回环；仅当用户显式勾选“允许局域网访问”时才暴露到局域网
        host: allowLan ? "0.0.0.0" : "127.0.0.1",
      });
      await fetchRunningServers(true);
    } catch (err: any) {
      // 端口被占用时换成可操作 toast（查一次占用者 → 给「结束进程」按钮）；
      // 不是端口冲突（或查不到占用者）才回落到原来的纯文本错误。
      if (!mounted.current || epoch !== life.current) return;
      const handled = await toastPortConflict(port, t);
      if (!handled && mounted.current && epoch === life.current) setError(String(err));
    } finally {
      starting.current = false;
      if (mounted.current && epoch === life.current) setLoading(false);
    }
  };

  const handleStop = async (p: number) => {
    if (stops.current.has(p)) return;
    stops.current.add(p); setStopping([...stops.current]);
    try {
      await invoke("stop_http_server", { port: p });
      await fetchRunningServers(true);
    } catch (err: any) {
      if (mounted.current) setError(t("httpserver.stopFail", { err: String(err) }));
    } finally {
      stops.current.delete(p);
      if (mounted.current) setStopping([...stops.current]);
    }
  };

  const handleCopyLink = async (p: number) => {
    try {
      await navigator.clipboard.writeText(`http://localhost:${p}`);
      if (!mounted.current) return;
      setCopiedPort(p);
      if (copyTimer.current) clearTimeout(copyTimer.current);
      copyTimer.current = setTimeout(() => setCopiedPort(null), 2000);
    } catch (e) { if (mounted.current) setError(String(e)); }
  };

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-title font-semibold text-white">{t("httpserver.title")}</h3>
        <p className="text-caption text-slate-400 mt-0.5">{t("httpserver.subtitle")}</p>
      </div>

      {/* 启动表单 */}
      <form onSubmit={handleStart} className="ui-card p-4 space-y-4">
        <fieldset disabled={loading} className="grid min-w-0 grid-cols-1 md:grid-cols-4 gap-4" aria-busy={loading}>
          <div className="md:col-span-3 space-y-1.5">
            <label className={labelCls}>{t("httpserver.rootDir")}</label>
            <div className="flex items-center gap-2">
              <input
                type="text"
                value={path}
                onChange={(e) => setPath(e.target.value)}
                placeholder="e.g. D:\project\dist"
                aria-label={t("httpserver.rootDir")}
                className={`${inputCls} flex-1 min-w-0 font-mono`}
              />
              <button
                type="button"
                onClick={handleSelectFolder}
                className="ui-icon-btn w-8 h-8"
                title={t("httpserver.pickFolder")}
              >
                <FolderOpen className="w-4 h-4" />
              </button>
            </div>
          </div>
          <div className="space-y-1.5">
            <label className={labelCls}>{t("httpserver.port")}</label>
            <input
              type="number"
              value={portInput}
              onChange={(e) => setPortInput(e.target.value)}
              step={1}
              min={1}
              max={65535}
              aria-label={t("httpserver.port")}
              className={`${inputCls} font-mono`}
            />
          </div>
        </fieldset>

        <label className="flex items-center gap-2 text-caption text-slate-400 cursor-pointer select-none">
          <input
            type="checkbox"
            disabled={loading}
            checked={allowLan}
            onChange={(e) => setAllowLan(e.target.checked)}
            className="w-3.5 h-3.5 accent-[var(--module-accent)] cursor-pointer"
          />
          {t("httpserver.lanHint")}
        </label>

        {/*
          勾选「允许局域网访问」会把整个目录共享给同一网络内的所有人，必须显式告知。
          尤其要说明「不加密、无鉴权」——用户往往以为只是"自己手机上也能打开"。
        */}
        {allowLan && (
          <div className="flex items-start gap-2 p-3 bg-amber-500/10 border border-amber-500/25 rounded-card text-tiny text-amber-200">
            <AlertTriangle className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" />
            <span className="leading-relaxed">{t("httpserver.lanWarning")}</span>
          </div>
        )}

        {error && (
          <div role="alert" className="ui-note-error p-3 flex items-center gap-2">
            <AlertCircle className="w-4 h-4 text-red-400 flex-shrink-0" />
            <span>{error}</span>
          </div>
        )}

        <div className="flex justify-end">
          <button
            type="submit"
            disabled={loading}
            className={btnPrimary}
          >
            <Play className="w-3.5 h-3.5" />
            {loading ? t("httpserver.starting") : t("httpserver.start")}
          </button>
        </div>
      </form>

      {/* 运行中的服务列表 */}
      <div className="ui-card p-4 space-y-4">
        <div className="flex items-center gap-2 border-b border-white/5 pb-3">
          <Server className="w-4 h-4 text-blue-400" />
          <h4 className="text-body font-semibold text-white">{t("httpserver.runningTitle", { count: runningServers.length })}</h4>
        </div>

        {statusError && <p role="alert" className="ui-note-error">{statusError}</p>}
        {!statusError && runningServers.length === 0 ? (
          <div className="text-center py-10 text-slate-500 text-body">
            {t("httpserver.empty")}
          </div>
        ) : (
          <div className="space-y-3">
            {runningServers.map((srv) => (
              <div 
                key={srv.port}
                className="flex flex-col md:flex-row md:items-center justify-between p-3.5 bg-black/20 border border-white/5 rounded-card gap-3"
              >
                <div className="space-y-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="w-2.5 h-2.5 rounded-full bg-emerald-500 animate-pulse" />
                    <button
                      onClick={() => { void openUrl(`http://localhost:${srv.port}`).catch((e) => setError(String(e))); }}
                      className="text-body font-bold font-mono ui-link flex items-center gap-1 cursor-pointer"
                    >
                      http://localhost:{srv.port}
                      <ExternalLink className="w-3 h-3" />
                    </button>
                  </div>
                  <p className="text-tiny text-slate-400 font-mono truncate" title={srv.path}>
                    {t("httpserver.dirLabel", { path: srv.path })}
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  <button
                    onClick={() => handleCopyLink(srv.port)}
                    className="ui-icon-btn w-8 h-8"
                    title={t("httpserver.copyLink")}
                  >
                    {copiedPort === srv.port ? (
                      <CheckCircle className="w-3.5 h-3.5 text-emerald-400" />
                    ) : (
                      <Copy className="w-3.5 h-3.5" />
                    )}
                  </button>
                  <button
                    onClick={() => handleStop(srv.port)}
                    disabled={stopping.includes(srv.port)}
                    aria-busy={stopping.includes(srv.port)}
                    className={btnDanger}
                    title={t("httpserver.closeService")}
                  >
                    <Square className="w-3 h-3" />
                    {t("httpserver.stop")}
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
