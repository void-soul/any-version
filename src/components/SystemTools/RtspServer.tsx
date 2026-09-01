import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import {
  Video,
  Play,
  Square,
  RefreshCw,
  FolderOpen,
  Copy,
  CheckCircle,
  AlertCircle,
  Terminal,
  ChevronDown,
  ChevronUp,
  ChevronRight,
  Plus,
  Trash2,
  Radio,
  Sliders,
  Network,
  Cpu,

  Mic,
  MicOff,
} from "lucide-react";

interface CameraDevices {
  videoDevices: string[];
  audioDevices: string[];
}

interface RtspConfig {
  id?: string;
  sourceType: "camera" | "file" | "testsrc";
  cameraName?: string;
  filePath?: string;
  port: number;
  pathName: string;
  allowLan: boolean;
  loopFile: boolean;
  includeAudio: boolean;
  audioDevice?: string;
  resolution?: string;
  fps?: number;
  bitrateMbps?: number;
  gop?: number;
  transport?: "tcp" | "udp";
  videoCodec?: "h264" | "h265";
  gpuAccel?: "cpu" | "nvenc" | "qsv" | "amf" | "copy";
}

interface RtspServerStatus {
  id: string;
  running: boolean;
  pid?: number;
  mtxPid?: number;
  ffmpegPid?: number;
  localUrl?: string;
  lanUrl?: string;
  config?: RtspConfig;
  logs: string[];
  lastError?: string;
  uptimeSeconds: number;
}

interface RtspInstanceItem {
  id: string;
  title: string;
  config: RtspConfig;
  status: RtspServerStatus;
  showLogs: boolean;
  collapsed: boolean; // 控制每个实例卡片的展开/折叠
  actionLoading: boolean;
  error: string | null;
}

const STORAGE_KEY = "any_version_rtsp_instances_v3";

const DEFAULT_CONFIG: RtspConfig = {
  sourceType: "testsrc",
  port: 8554,
  pathName: "live",
  allowLan: false,
  loopFile: true,
  includeAudio: false,
  resolution: "default",
  fps: 30,
  bitrateMbps: 0,
  gop: 15,
  transport: "tcp",
  videoCodec: "h264",
  gpuAccel: "cpu",
};

export default function RtspServer() {
  const { t } = useTranslation();
  const [devices, setDevices] = useState<CameraDevices>({ videoDevices: [], audioDevices: [] });
  const [loadingDevices, setLoadingDevices] = useState<boolean>(false);
  const [copiedUrl, setCopiedUrl] = useState<string | null>(null);
  const [allIps, setAllIps] = useState<{ name: string; ip: string }[]>([]);

  // 加载本机所有网卡 IP
  const loadAllIps = async () => {
    try {
      const list = await invoke<[string, string][]>("get_all_local_ips");
      setAllIps(list.map(([name, ip]) => ({ name, ip })));
    } catch {
      setAllIps([]);
    }
  };

  // 首次挂载时加载 IP 列表
  useEffect(() => {
    loadAllIps();
  }, []);

  // 多实例状态数组
  const [instances, setInstances] = useState<RtspInstanceItem[]>(() => {
    try {
      const saved = localStorage.getItem(STORAGE_KEY);
      if (saved) {
        const parsed = JSON.parse(saved);
        if (Array.isArray(parsed) && parsed.length > 0) {
          return parsed.map((item: any, idx: number) => ({
            id: item.id || `inst_${Date.now()}_${idx}`,
            title: item.title || t("rtsp.nodeTitle", { n: idx + 1 }),
            config: { ...DEFAULT_CONFIG, ...item.config, id: item.id },
            status: { id: item.id, running: false, logs: [], uptimeSeconds: 0 },
            showLogs: false,
            collapsed: item.collapsed ?? true, // 默认收起/折叠，保持页面简洁
            actionLoading: false,
            error: null,
          }));
        }
      }
    } catch (e) {
      console.error("读取 RTSP 本地实例失败:", e);
    }
    const initId = "inst_default_8554";
    return [
      {
        id: initId,
        title: t("rtsp.nodeTitle", { n: 1 }),
        config: { ...DEFAULT_CONFIG, id: initId, port: 8554, pathName: "live" },
        status: { id: initId, running: false, logs: [], uptimeSeconds: 0 },
        showLogs: false,
        collapsed: false, // 默认首个节点展开供编辑
        actionLoading: false,
        error: null,
      },
    ];
  });

  // 持久化保存参数配置（不含运行态/日志/进程）
  useEffect(() => {
    try {
      const toSave = instances.map((inst) => ({
        id: inst.id,
        title: inst.title,
        config: inst.config,
        collapsed: inst.collapsed,
      }));
      localStorage.setItem(STORAGE_KEY, JSON.stringify(toSave));
    } catch (e) {
      console.error("保存 RTSP 实例至 localStorage 失败:", e);
    }
  }, [instances]);

  // 扫描音视频设备
  const fetchDevices = async () => {
    setLoadingDevices(true);
    try {
      const res = await invoke<CameraDevices>("get_rtsp_camera_devices");
      setDevices(res);
    } catch (e: any) {
      console.warn("设备扫描提示:", e);
    } finally {
      setLoadingDevices(false);
    }
  };

  // 轮询更新 RTSP 实例运行状态
  const pollStatuses = async () => {
    try {
      const statuses = await invoke<RtspServerStatus[]>("get_all_rtsp_server_statuses");
      const statusMap = new Map(statuses.map((s) => [s.id, s]));

      setInstances((prev) =>
        prev.map((inst) => {
          const st = statusMap.get(inst.id);
          if (st) {
            return { ...inst, status: st };
          } else {
            return {
              ...inst,
              status: { ...inst.status, running: false, uptimeSeconds: 0 },
            };
          }
        })
      );
    } catch (e) {
      console.error("轮询 RTSP 服务器状态失败:", e);
    }
  };

  useEffect(() => {
    fetchDevices();
    pollStatuses();
    const interval = setInterval(pollStatuses, 2000);
    return () => clearInterval(interval);
  }, []);

  // 新增 RTSP 实例
  const handleAddInstance = () => {
    const newPort = 8554 + instances.length;
    const newId = `inst_${Date.now()}_${newPort}`;
    const newInst: RtspInstanceItem = {
      id: newId,
      title: t("rtsp.nodeTitle", { n: instances.length + 1 }),
      config: {
        ...DEFAULT_CONFIG,
        id: newId,
        port: newPort,
        pathName: `live${instances.length + 1}`,
      },
      status: { id: newId, running: false, logs: [], uptimeSeconds: 0 },
      showLogs: false,
      collapsed: false, // 新建节点自动展开供配置
      actionLoading: false,
      error: null,
    };
    setInstances([...instances, newInst]);
  };

  // 删除实例
  const handleDeleteInstance = async (id: string, e?: React.MouseEvent) => {
    e?.stopPropagation();
    const inst = instances.find((i) => i.id === id);
    if (inst?.status.running) {
      try {
        await invoke("stop_rtsp_server", { id });
      } catch (err) {
        console.error(err);
      }
    }
    setInstances(instances.filter((i) => i.id !== id));
  };

  // 停止所有实例
  const handleStopAll = async () => {
    try {
      await invoke("stop_all_rtsp_servers");
      await pollStatuses();
    } catch (e: any) {
      alert(t("rtsp.stopAllFail", { err: String(e) }));
    }
  };

  // 启动所有实例
  const handleStartAll = async () => {
    const pending = instances.filter((i) => !i.status.running);
    if (pending.length === 0) return;

    // 先校验所有待启动实例
    let hasError = false;
    const validated = pending.map((inst) => {
      if (inst.config.sourceType === "camera" && !inst.config.cameraName?.trim()) {
        hasError = true;
        return { ...inst, error: t("rtsp.needCamera"), collapsed: false };
      }
      if (inst.config.sourceType === "file" && !inst.config.filePath?.trim()) {
        hasError = true;
        return { ...inst, error: t("rtsp.needFilePath"), collapsed: false };
      }
      if (!inst.config.port || inst.config.port < 1024 || inst.config.port > 65535) {
        hasError = true;
        return { ...inst, error: t("rtsp.needPort"), collapsed: false };
      }
      return { ...inst, error: null, actionLoading: true };
    });

    // 有校验错误时更新状态并中止
    if (hasError) {
      setInstances((prev) => {
        const errorMap = new Map(validated.filter((v) => v.error).map((v) => [v.id, v]));
        return prev.map((inst) => errorMap.get(inst.id) || inst);
      });
      return;
    }

    // 全部校验通过，设置 loading 状态
    const loadingIds = new Set(pending.map((i) => i.id));
    setInstances((prev) => prev.map((i) => (loadingIds.has(i.id) ? { ...i, actionLoading: true, error: null } : i)));

    // 并行启动所有实例
    const results = await Promise.allSettled(
      pending.map((inst) =>
        invoke<RtspServerStatus>("start_rtsp_server", {
          config: { ...inst.config, id: inst.id },
        })
      )
    );

    // 根据结果更新各实例状态
    setInstances((prev) =>
      prev.map((inst) => {
        const pendingIdx = pending.findIndex((p) => p.id === inst.id);
        if (pendingIdx === -1) return inst;
        const result = results[pendingIdx];
        if (result.status === "fulfilled") {
          return { ...inst, status: result.value, actionLoading: false };
        } else {
          return {
            ...inst,
            error: String(result.reason),
            showLogs: true,
            collapsed: false,
            actionLoading: false,
          };
        }
      })
    );
  };

  // 切换展开/折叠
  const toggleCollapse = (id: string) => {
    setInstances((prev) =>
      prev.map((inst) => (inst.id === id ? { ...inst, collapsed: !inst.collapsed } : inst))
    );
  };

  // 单个实例参数修改
  const updateInstanceConfig = (id: string, updates: Partial<RtspConfig> | { title?: string }) => {
    setInstances((prev) =>
      prev.map((inst) => {
        if (inst.id !== id) return inst;
        if ("title" in updates && updates.title !== undefined) {
          return { ...inst, title: updates.title };
        }
        return {
          ...inst,
          config: { ...inst.config, ...updates },
        };
      })
    );
  };

  // 将一个实例的配置复制到所有其他实例（保留各实例的端口/路径，避免冲突）
  const handleCopyConfigToAll = (source: RtspInstanceItem) => {
    const others = instances.filter((i) => i.id !== source.id);
    if (others.length === 0) return;
    if (!confirm(t("rtsp.copyConfirm", { title: source.title, count: others.length }))) return;
    setInstances((prev) =>
      prev.map((inst) => {
        if (inst.id === source.id) return inst;
        return {
          ...inst,
          config: {
            ...source.config,
            id: inst.id,
            port: inst.config.port,
            pathName: inst.config.pathName,
          },
        };
      })
    );
  };

  // 启动单个实例
  const handleStartInstance = async (inst: RtspInstanceItem, e?: React.MouseEvent) => {
    e?.stopPropagation();
    if (inst.status.running) return;

    if (inst.config.sourceType === "camera" && !inst.config.cameraName?.trim()) {
      setInstances((prev) =>
        prev.map((i) => (i.id === inst.id ? { ...i, error: t("rtsp.needCamera"), collapsed: false } : i))
      );
      return;
    }
    if (inst.config.sourceType === "file" && !inst.config.filePath?.trim()) {
      setInstances((prev) =>
        prev.map((i) => (i.id === inst.id ? { ...i, error: t("rtsp.needFilePath"), collapsed: false } : i))
      );
      return;
    }
    if (!inst.config.port || inst.config.port < 1024 || inst.config.port > 65535) {
      setInstances((prev) =>
        prev.map((i) => (i.id === inst.id ? { ...i, error: t("rtsp.needPort"), collapsed: false } : i))
      );
      return;
    }

    setInstances((prev) =>
      prev.map((i) => (i.id === inst.id ? { ...i, actionLoading: true, error: null } : i))
    );

    try {
      const res = await invoke<RtspServerStatus>("start_rtsp_server", {
        config: { ...inst.config, id: inst.id },
      });
      setInstances((prev) =>
        prev.map((i) => (i.id === inst.id ? { ...i, status: res, actionLoading: false } : i))
      );
    } catch (err: any) {
      const errStr = String(err);
      setInstances((prev) =>
        prev.map((i) =>
          i.id === inst.id ? { ...i, error: errStr, showLogs: true, collapsed: false, actionLoading: false } : i
        )
      );
    }
  };

  // 停止单个实例
  const handleStopInstance = async (id: string, e?: React.MouseEvent) => {
    e?.stopPropagation();
    setInstances((prev) =>
      prev.map((i) => (i.id === id ? { ...i, actionLoading: true, error: null } : i))
    );

    try {
      await invoke("stop_rtsp_server", { id });
      await pollStatuses();
    } catch (err: any) {
      setInstances((prev) =>
        prev.map((i) => (i.id === id ? { ...i, error: String(err) } : i))
      );
    } finally {
      setInstances((prev) =>
        prev.map((i) => (i.id === id ? { ...i, actionLoading: false } : i))
      );
    }
  };

  // 选择视频文件
  const handleSelectFile = async (instId: string) => {
    try {
      const selected = await open({
        multiple: false,
        title: t("rtsp.pickVideoTitle"),
        filters: [
          {
            name: "Video Files",
            extensions: ["mp4", "mkv", "avi", "mov", "flv", "webm", "ts", "m4v"],
          },
        ],
      });
      if (selected && typeof selected === "string") {
        updateInstanceConfig(instId, { filePath: selected });
      }
    } catch (e) {
      console.error(e);
      alert(t("rtsp.pickerFail"));
    }
  };

  const handleCopy = (text: string, e?: React.MouseEvent) => {
    e?.stopPropagation();
    navigator.clipboard.writeText(text);
    setCopiedUrl(text);
    setTimeout(() => setCopiedUrl(null), 2000);
  };

  const formatUptime = (seconds: number) => {
    const hrs = Math.floor(seconds / 3600);
    const mins = Math.floor((seconds % 3600) / 60);
    const secs = seconds % 60;
    if (hrs > 0) return t("rtsp.uptimeHms", { h: hrs, m: mins, s: secs });
    if (mins > 0) return t("rtsp.uptimeMs", { m: mins, s: secs });
    return t("rtsp.uptimeS", { s: secs });
  };

  const runningCount = instances.filter((i) => i.status.running).length;

  return (
    <div className="flex h-full min-h-0 w-full max-w-[1100px] flex-col overflow-hidden px-6 py-4 mx-auto space-y-5 select-none text-slate-200">
      {/* 头部控制栏固定，实例列表单独滚动 */}
      <div className="flex shrink-0 flex-col md:flex-row md:items-center justify-between gap-4 border-b border-white/10 pb-4">
        <div className="flex items-center gap-3">
          <div className="p-2.5 rounded-xl bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] shadow-lg shadow-[var(--module-accent-ring)]">
            <Video className="w-6 h-6" />
          </div>
          <div>
            <h3 className="text-base font-bold text-white flex items-center gap-2">
              {t("rtsp.title")}
              {runningCount > 0 && (
                <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-[11px] font-medium bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] text-[var(--module-accent)] border border-[var(--module-accent-ring)]">
                  <span className="w-2 h-2 rounded-full bg-[var(--module-accent)] animate-pulse" />
                  {t("rtsp.runningCount", { count: runningCount })}
                </span>
              )}
            </h3>
            <p className="text-xs text-slate-400 mt-0.5">
              {t("rtsp.subtitle")}
            </p>
          </div>
        </div>

        <div className="flex items-center gap-2 self-start md:self-auto">
          <button
            onClick={handleAddInstance}
            className="px-3.5 py-2 rounded-xl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-xs font-semibold flex items-center gap-1.5 transition-all shadow-md shadow-[var(--module-accent-ring)] cursor-pointer"
          >
            <Plus className="w-4 h-4" />
            {t("rtsp.addInstance")}
          </button>

          {instances.length - runningCount > 0 && (
            <button
              onClick={handleStartAll}
              className="px-3.5 py-2 rounded-xl bg-cyan-500/20 hover:bg-cyan-500/30 text-cyan-300 border border-cyan-500/30 text-xs font-semibold flex items-center gap-1.5 transition-all cursor-pointer"
            >
              <Play className="w-3.5 h-3.5 fill-current" />
              {t("rtsp.startAll", { count: instances.length - runningCount })}
            </button>
          )}

          {runningCount > 0 && (
            <button
              onClick={handleStopAll}
              className="px-3.5 py-2 rounded-xl bg-rose-500/20 hover:bg-rose-500/30 text-rose-300 border border-rose-500/30 text-xs font-semibold flex items-center gap-1.5 transition-all cursor-pointer"
            >
              <Square className="w-3.5 h-3.5 fill-current" />
              {t("rtsp.stopAll", { count: runningCount })}
            </button>
          )}
        </div>
      </div>

      {/* 实例卡片列表（支持全面折叠与精简视图） */}
      <div className="min-h-0 flex-1 overflow-y-auto space-y-3 pr-1">
        {instances.map((inst) => {
          const isRunning = inst.status.running;
          const isLocked = isRunning || inst.actionLoading;

          return (
            <div
              key={inst.id}
              className={`glass-panel border rounded-2xl transition-all overflow-hidden ${
                isRunning
                  ? "border-[var(--module-accent-ring)] bg-[color-mix(in_srgb,var(--module-accent)_3%,transparent)] shadow-lg shadow-[var(--module-accent-ring)]"
                  : "border-white/10 bg-slate-900/40 hover:border-white/20"
              }`}
            >
              {/* ── 核心摘要栏（始终保留，点击可展开/折叠） ── */}
              <div
                onClick={() => toggleCollapse(inst.id)}
                className="p-4 flex items-center justify-between gap-3 cursor-pointer hover:bg-white/[0.02] transition-colors"
              >
                {/* 左侧：折叠指示箭头 + 实例图标 + 名称与摘要状态 */}
                <div className="flex items-center gap-3 min-w-0 flex-1">
                  <div className="text-slate-400 hover:text-white transition-colors">
                    {inst.collapsed ? <ChevronRight className="w-4 h-4" /> : <ChevronDown className="w-4 h-4" />}
                  </div>

                  <div
                    className={`p-2 rounded-xl flex-shrink-0 ${
                      isRunning
                        ? "bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] text-[var(--module-accent)] border border-[var(--module-accent-ring)]"
                        : "bg-white/5 text-slate-400 border border-white/10"
                    }`}
                  >
                    <Radio className="w-4 h-4" />
                  </div>

                  {/* 名称与关键摘要 */}
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2.5">
                      <span className="text-sm font-bold text-white truncate">{inst.title}</span>

                      {/* 运行状态 Tag */}
                      {isRunning ? (
                        <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md text-[10px] font-semibold bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] flex-shrink-0">
                          <span className="w-1.5 h-1.5 rounded-full bg-[var(--module-accent)] animate-pulse" />
                          {t("rtsp.runningPort", { port: inst.config.port })}
                        </span>
                      ) : (
                        <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md text-[10px] font-medium bg-white/5 text-slate-400 border border-white/10 flex-shrink-0">
                          {t("rtsp.stoppedPort", { port: inst.config.port })}
                        </span>
                      )}

                      {/* 视频源摘要 */}
                      <span className="text-[10px] text-slate-400 bg-white/5 px-2 py-0.5 rounded border border-white/5 flex-shrink-0 hidden sm:inline-block">
                        {inst.config.sourceType === "camera"
                          ? t("rtsp.sourceCamera", { name: inst.config.cameraName || t("rtsp.notSpecified") })
                          : inst.config.sourceType === "file"
                          ? t("rtsp.sourceFile", { name: inst.config.filePath ? inst.config.filePath.split(/[/\\]/).pop() : t("rtsp.notSpecified") })
                          : t("rtsp.sourceTestsrc")}
                      </span>
                    </div>

                    {/* 折叠时显示的轻量信息 */}
                    <div className="text-[11px] text-slate-400 font-mono mt-0.5 truncate flex items-center gap-2">
                      <span>{t("rtsp.pathLabel", { path: inst.config.pathName })}</span>
                      <span>·</span>
                      <span>{t("rtsp.protocolLabel", { proto: inst.config.transport?.toUpperCase() || "TCP" })}</span>
                      <span>·</span>
                      <span>{t("rtsp.codecLabel", { codec: inst.config.videoCodec?.toUpperCase() || "H264" })}</span>
                      {isRunning && inst.status.localUrl && (
                        <>
                          <span>·</span>
                          <span className="text-[var(--module-accent)] font-semibold">{inst.status.localUrl}</span>
                        </>
                      )}
                    </div>
                  </div>
                </div>

                {/* 右侧：快捷操作按钮 (启动/停止/删除) */}
                <div className="flex items-center gap-2 flex-shrink-0">
                  {!isRunning ? (
                    <button
                      onClick={(e) => handleStartInstance(inst, e)}
                      disabled={inst.actionLoading}
                      className="px-4 py-1.5 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-white rounded-xl text-xs font-semibold flex items-center gap-1.5 transition-all shadow-md shadow-[var(--module-accent-ring)] cursor-pointer"
                    >
                      <Play className="w-3.5 h-3.5 fill-current" />
                      {t("rtsp.startStream")}
                    </button>
                  ) : (
                    <button
                      onClick={(e) => handleStopInstance(inst.id, e)}
                      disabled={inst.actionLoading}
                      className="px-4 py-1.5 bg-rose-600 hover:bg-rose-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold flex items-center gap-1.5 transition-all shadow-md shadow-rose-600/20 cursor-pointer"
                    >
                      <Square className="w-3.5 h-3.5 fill-current" />
                      {t("rtsp.stopInstance")}
                    </button>
                  )}

                  {instances.length > 1 && !isRunning && (
                    <button
                      onClick={(e) => handleDeleteInstance(inst.id, e)}
                      className="p-1.5 text-slate-400 hover:text-rose-400 hover:bg-rose-500/10 rounded-xl transition-colors cursor-pointer border border-transparent hover:border-rose-500/20"
                      title={t("rtsp.deleteInstance")}
                    >
                      <Trash2 className="w-4 h-4" />
                    </button>
                  )}
                </div>
              </div>

              {/* ── 展开区域（仅当 !collapsed 时显示） ── */}
              {!inst.collapsed && (
                <div className="p-5 pt-0 space-y-4 border-t border-white/5">
                  {/* 编辑名称区 */}
                  <div className="flex items-center gap-2 pt-3">
                    <span className="text-xs text-slate-400 font-semibold flex-shrink-0">{t("rtsp.aliasLabel")}</span>
                    <input
                      value={inst.title}
                      onChange={(e) => updateInstanceConfig(inst.id, { title: e.target.value })}
                      disabled={isRunning}
                      placeholder={t("rtsp.aliasPh")}
                      className="text-xs font-bold text-white bg-white/5 border border-white/10 rounded-lg px-2.5 py-1 focus:border-[var(--module-accent)] focus:outline-none transition-all flex-1"
                    />
                    <button
                      onClick={(e) => { e.stopPropagation(); handleCopyConfigToAll(inst); }}
                      disabled={instances.length < 2}
                      title={instances.length < 2 ? t("rtsp.copyTitle") : t("rtsp.copyTitleN", { count: instances.length - 1 })}
                      className="px-2.5 py-1 rounded-lg bg-white/5 hover:bg-white/10 border border-white/10 text-slate-300 hover:text-white text-[11px] font-semibold flex items-center gap-1.5 transition-all cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed flex-shrink-0"
                    >
                      <Copy className="w-3.5 h-3.5" />
                      {t("rtsp.copyToAll", { count: instances.length - 1 })}
                    </button>
                  </div>

                  {/* 运行中推流状态与 RTSP 链接复制区 */}
                  {isRunning && (
                    <div className="rounded-xl bg-black/40 border border-[var(--module-accent-ring)] p-4 space-y-3">
                      <div className="flex flex-wrap items-center justify-between text-xs font-semibold text-[var(--module-accent)] gap-2 border-b border-white/5 pb-2">
                        <div className="flex items-center gap-2">
                          <span className="w-2.5 h-2.5 rounded-full bg-emerald-400 animate-ping" />
                          {t("rtsp.streamRunning", { mtx: inst.status.mtxPid ?? "N/A", ff: inst.status.ffmpegPid ?? "N/A" })}
                        </div>
                        <div className="text-[11px] text-slate-400 font-mono">
                          {t("rtsp.uptimeLabel", { time: formatUptime(inst.status.uptimeSeconds) })}
                        </div>
                      </div>

                      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                        {inst.status.localUrl && (
                          <div className="bg-white/[0.03] border border-white/10 rounded-xl p-3 flex items-center justify-between">
                            <div className="min-w-0 pr-2">
                              <div className="text-[10px] text-slate-400 font-semibold uppercase">{t("rtsp.loopbackLabel")}</div>
                              <div className="text-xs font-mono text-[var(--module-accent)] truncate mt-0.5">{inst.status.localUrl}</div>
                            </div>
                            <button
                              onClick={(e) => handleCopy(inst.status.localUrl!, e)}
                              className="p-2 text-slate-300 hover:text-white bg-white/5 hover:bg-white/10 rounded-lg transition-colors cursor-pointer flex-shrink-0"
                              title={t("rtsp.copyRtsp")}
                            >
                              {copiedUrl === inst.status.localUrl ? <CheckCircle className="w-4 h-4 text-[var(--module-accent)]" /> : <Copy className="w-4 h-4" />}
                            </button>
                          </div>
                        )}
                        {inst.config.allowLan && allIps.length > 0 && (
                          <div className="md:col-span-2 bg-white/[0.03] border border-white/10 rounded-xl p-3 space-y-2">
                            <div className="flex items-center justify-between">
                              <div className="text-[10px] text-slate-400 font-semibold uppercase">{t("rtsp.lanLabel")}</div>
                              <button
                                onClick={loadAllIps}
                                className="text-[10px] text-slate-400 hover:text-white flex items-center gap-0.5 cursor-pointer"
                              >
                                <RefreshCw className="w-3 h-3" /> {t("rtsp.refresh")}
                              </button>
                            </div>
                            <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                              {allIps.map(({ name, ip }) => {
                                const url = `rtsp://${ip}:${inst.config.port}/${inst.config.pathName}`;
                                return (
                                  <div
                                    key={ip}
                                    onClick={(e) => handleCopy(url, e)}
                                    className="flex items-center justify-between bg-black/30 border border-white/10 rounded-lg px-2.5 py-1.5 cursor-pointer hover:bg-white/[0.06] transition-colors group"
                                  >
                                    <div className="min-w-0 pr-2">
                                      <div className="text-[10px] text-slate-500 truncate">{name}</div>
                                      <div className="text-[11px] font-mono text-cyan-400 truncate">{url}</div>
                                    </div>
                                    <div className="flex-shrink-0">
                                      {copiedUrl === url ? (
                                        <CheckCircle className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                                      ) : (
                                        <Copy className="w-3.5 h-3.5 text-slate-500 group-hover:text-white transition-colors" />
                                      )}
                                    </div>
                                  </div>
                                );
                              })}
                            </div>
                          </div>
                        )}
                      </div>
                    </div>
                  )}

                  {/* 异常报错提示 */}
                  {inst.error && (
                    <div className="p-3.5 rounded-xl bg-rose-500/10 border border-rose-500/20 text-xs text-rose-300 flex items-start gap-2.5">
                      <AlertCircle className="w-4 h-4 flex-shrink-0 mt-0.5" />
                      <div className="min-w-0 whitespace-pre-wrap font-mono text-[11px] leading-relaxed">{inst.error}</div>
                    </div>
                  )}

                  {/* 3 卡片配置区 */}
                  <div className="grid grid-cols-1 lg:grid-cols-3 gap-4 items-start">
                    {/* ── 卡片 1：视频输入源 ── */}
                    <div className="rounded-2xl border border-white/10 bg-white/[0.02] overflow-hidden">
                      <div className="px-4 py-2.5 border-b border-white/5 bg-white/[0.02] flex items-center gap-2">
                        <div className="p-1.5 rounded-lg bg-[var(--module-accent-soft)] border border-[var(--module-accent-ring)]">
                          <Sliders className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                        </div>
                        <span className="text-xs font-semibold text-white">{t("rtsp.sourceTitle")}</span>
                      </div>
                      <div className="p-4 space-y-3">
                        <div className="grid grid-cols-3 gap-1.5">
                          {(["testsrc", "camera", "file"] as const).map((st) => (
                            <button
                              key={st}
                              disabled={isLocked}
                              onClick={() => updateInstanceConfig(inst.id, { sourceType: st })}
                              className={`h-9 text-xs font-medium rounded-xl border transition-all cursor-pointer ${
                                inst.config.sourceType === st
                                  ? "bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] border-[color-mix(in_srgb,var(--module-accent)_50%,transparent)] text-[var(--module-accent)] font-semibold shadow-sm"
                                  : "bg-white/5 border-white/10 text-slate-400 hover:text-white"
                              }`}
                            >
                              {st === "testsrc" ? t("rtsp.sourceTestsrcOpt") : st === "camera" ? t("rtsp.sourceCameraOpt") : t("rtsp.sourceFileOpt")}
                            </button>
                          ))}
                        </div>

                        {inst.config.sourceType === "camera" && (
                          <>
                            <div>
                              <div className="flex items-center justify-between text-[10px] text-slate-400 mb-1">
                                <span>{t("rtsp.selectCamera")}</span>
                                <button
                                  onClick={fetchDevices}
                                  disabled={loadingDevices || isLocked}
                                  className="hover:text-white flex items-center gap-0.5 cursor-pointer"
                                >
                                  <RefreshCw className={`w-3 h-3 ${loadingDevices ? "animate-spin" : ""}`} /> {t("rtsp.refresh")}
                                </button>
                              </div>
                              <select
                                value={inst.config.cameraName || ""}
                                disabled={isLocked}
                                onChange={(e) => updateInstanceConfig(inst.id, { cameraName: e.target.value })}
                                className="w-full h-9 px-2.5 rounded-xl bg-slate-900 border border-white/10 text-xs text-white focus:outline-none focus:border-[var(--module-accent)] cursor-pointer"
                              >
                                {devices.videoDevices.length === 0 ? (
                                  <option value="">{t("rtsp.noCameraFound")}</option>
                                ) : (
                                  devices.videoDevices.map((d) => (
                                    <option key={d} value={d}>
                                      {d}
                                    </option>
                                  ))
                                )}
                              </select>
                            </div>

                            {/* 音频/麦克风设置 */}
                            <div className="rounded-xl border border-white/10 bg-white/[0.02] p-3 space-y-2">
                              <label className="flex items-center justify-between cursor-pointer">
                                <span className="text-[11px] text-slate-300 flex items-center gap-1.5">
                                  {inst.config.includeAudio ? (
                                    <Mic className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                                  ) : (
                                    <MicOff className="w-3.5 h-3.5 text-slate-500" />
                                  )}
                                  {t("rtsp.enableAudio")}
                                </span>
                                <button
                                  type="button"
                                  role="switch"
                                  aria-checked={inst.config.includeAudio}
                                  disabled={isLocked}
                                  onClick={() =>
                                    updateInstanceConfig(inst.id, { includeAudio: !inst.config.includeAudio })
                                  }
                                  className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors cursor-pointer ${
                                    inst.config.includeAudio ? "bg-[var(--module-accent)]" : "bg-white/15"
                                  } ${isLocked ? "opacity-50 cursor-not-allowed" : ""}`}
                                >
                                  <span
                                    className={`inline-block h-3.5 w-3.5 rounded-full bg-white shadow-sm transition-transform ${
                                      inst.config.includeAudio ? "translate-x-4" : "translate-x-0.5"
                                    }`}
                                  />
                                </button>
                              </label>

                              {inst.config.includeAudio && (
                                <div>
                                  <span className="text-[10px] text-slate-400 block mb-1">{t("rtsp.selectMic")}</span>
                                  <select
                                    value={inst.config.audioDevice || ""}
                                    disabled={isLocked}
                                    onChange={(e) => updateInstanceConfig(inst.id, { audioDevice: e.target.value })}
                                    className="w-full h-9 px-2.5 rounded-xl bg-slate-900 border border-white/10 text-xs text-white focus:outline-none focus:border-[var(--module-accent)] cursor-pointer"
                                  >
                                    {devices.audioDevices.length === 0 ? (
                                      <option value="">{t("rtsp.noMicFound")}</option>
                                    ) : (
                                      devices.audioDevices.map((d) => (
                                        <option key={d} value={d}>
                                          {d}
                                        </option>
                                      ))
                                    )}
                                  </select>
                                  <p className="text-[10px] text-slate-500 mt-1 leading-relaxed">
                                    {t("rtsp.audioHint")}
                                  </p>
                                </div>
                              )}
                            </div>
                          </>
                        )}

                        {inst.config.sourceType === "file" && (
                          <div className="space-y-1.5">
                            <div className="flex items-center gap-1.5">
                              <input
                                value={inst.config.filePath || ""}
                                disabled={isLocked}
                                onChange={(e) => updateInstanceConfig(inst.id, { filePath: e.target.value })}
                                placeholder={t("rtsp.filePh")}
                                className="flex-1 h-9 px-2.5 rounded-xl bg-white/5 border border-white/10 text-xs text-white placeholder-slate-500 focus:outline-none focus:border-[var(--module-accent)]"
                              />
                              <button
                                disabled={isLocked}
                                onClick={() => handleSelectFile(inst.id)}
                                className="h-9 px-3 rounded-xl bg-white/10 hover:bg-white/20 text-white transition-colors cursor-pointer flex items-center justify-center"
                                title={t("rtsp.pickFile")}
                              >
                                <FolderOpen className="w-4 h-4" />
                              </button>
                            </div>
                            <label className="flex items-center gap-1.5 text-[11px] text-slate-400 cursor-pointer">
                              <input
                                type="checkbox"
                                disabled={isLocked}
                                checked={inst.config.loopFile}
                                onChange={(e) => updateInstanceConfig(inst.id, { loopFile: e.target.checked })}
                                className="rounded bg-white/5 border-white/20 text-[var(--module-accent)] focus:ring-0"
                              />
                              {t("rtsp.loopVideo")}
                            </label>
                          </div>
                        )}

                        {inst.config.sourceType === "testsrc" && (
                          <div className="text-[11px] text-slate-400 bg-white/[0.02] border border-white/5 rounded-xl p-2.5">
                            {t("rtsp.testsrcDesc")}
                          </div>
                        )}
                      </div>
                    </div>

                    {/* ── 卡片 2：网络与传输协议 ── */}
                    <div className="rounded-2xl border border-white/10 bg-white/[0.02] overflow-hidden">
                      <div className="px-4 py-2.5 border-b border-white/5 bg-white/[0.02] flex items-center gap-2">
                        <div className="p-1.5 rounded-lg bg-[var(--module-accent-soft)] border border-[var(--module-accent-ring)]">
                          <Network className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                        </div>
                        <span className="text-xs font-semibold text-white">{t("rtsp.netTitle")}</span>
                      </div>
                      <div className="p-4 space-y-3">
                        <div className="grid grid-cols-2 gap-2">
                          <div>
                            <span className="text-[10px] text-slate-400 block mb-1">{t("rtsp.portLabel")}</span>
                            <input
                              type="number"
                              disabled={isLocked}
                              value={inst.config.port}
                              onChange={(e) =>
                                updateInstanceConfig(inst.id, { port: parseInt(e.target.value) || 8554 })
                              }
                              className="w-full h-9 px-2.5 rounded-xl bg-white/5 border border-white/10 text-xs text-white focus:outline-none focus:border-[var(--module-accent)] font-mono"
                            />
                          </div>
                          <div>
                            <span className="text-[10px] text-slate-400 block mb-1">{t("rtsp.pathNameLabel")}</span>
                            <input
                              disabled={isLocked}
                              value={inst.config.pathName}
                              onChange={(e) => updateInstanceConfig(inst.id, { pathName: e.target.value })}
                              className="w-full h-9 px-2.5 rounded-xl bg-white/5 border border-white/10 text-xs text-white focus:outline-none focus:border-[var(--module-accent)] font-mono"
                            />
                          </div>
                        </div>

                        <div>
                          <span className="text-[10px] text-slate-400 block mb-1">{t("rtsp.transportLabel")}</span>
                          <select
                            disabled={isLocked}
                            value={inst.config.transport || "tcp"}
                            onChange={(e) =>
                              updateInstanceConfig(inst.id, { transport: e.target.value as "tcp" | "udp" })
                            }
                            className="w-full h-9 px-2.5 rounded-xl bg-slate-900 border border-white/10 text-xs text-white focus:outline-none focus:border-[var(--module-accent)] cursor-pointer"
                          >
                            <option value="tcp">{t("rtsp.optTcp")}</option>
                            <option value="udp">{t("rtsp.optUdp")}</option>
                          </select>
                        </div>

                        <div>
                          <label className="flex items-center gap-1.5 text-[11px] text-slate-400 cursor-pointer">
                            <input
                              type="checkbox"
                              disabled={isLocked}
                              checked={inst.config.allowLan}
                              onChange={(e) => updateInstanceConfig(inst.id, { allowLan: e.target.checked })}
                              className="rounded bg-white/5 border-white/20 text-[var(--module-accent)] focus:ring-0"
                            />
                            {t("rtsp.allowLan")}
                          </label>

                          {inst.config.allowLan && (
                            <div className="mt-2 space-y-1.5">
                              <div className="flex items-center justify-between">
                                <span className="text-[10px] text-slate-500">{t("rtsp.nicIpHint")}</span>
                                <button
                                  onClick={loadAllIps}
                                  className="text-[10px] text-slate-400 hover:text-white flex items-center gap-0.5 cursor-pointer"
                                >
                                  <RefreshCw className="w-3 h-3" /> {t("rtsp.refresh")}
                                </button>
                              </div>
                              {allIps.length === 0 ? (
                                <div className="text-[10px] text-slate-500 bg-white/[0.02] border border-white/5 rounded-lg px-2.5 py-1.5">
                                  {t("rtsp.noNicIp")}
                                </div>
                              ) : (
                                allIps.map(({ name, ip }) => {
                                  const url = `rtsp://${ip}:${inst.config.port}/${inst.config.pathName}`;
                                  return (
                                    <div
                                      key={ip}
                                      onClick={() => handleCopy(url)}
                                      className="flex items-center justify-between bg-white/[0.03] border border-white/10 rounded-lg px-2.5 py-1.5 cursor-pointer hover:bg-white/[0.06] transition-colors group"
                                    >
                                      <div className="min-w-0">
                                        <div className="text-[10px] text-slate-500 truncate">{name}</div>
                                        <div className="text-[11px] font-mono text-cyan-400 truncate">{url}</div>
                                      </div>
                                      <div className="flex-shrink-0 ml-2">
                                        {copiedUrl === url ? (
                                          <CheckCircle className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                                        ) : (
                                          <Copy className="w-3.5 h-3.5 text-slate-500 group-hover:text-white transition-colors" />
                                        )}
                                      </div>
                                    </div>
                                  );
                                })
                              )}
                            </div>
                          )}
                        </div>
                      </div>
                    </div>

                    {/* ── 卡片 3：画幅与编码选项 ── */}
                    <div className="rounded-2xl border border-white/10 bg-white/[0.02] overflow-hidden">
                      <div className="px-4 py-2.5 border-b border-white/5 bg-white/[0.02] flex items-center gap-2">
                        <div className="p-1.5 rounded-lg bg-[var(--module-accent-soft)] border border-[var(--module-accent-ring)]">
                          <Cpu className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                        </div>
                        <span className="text-xs font-semibold text-white">{t("rtsp.codecTitle")}</span>
                      </div>
                      <div className="p-4 space-y-3">
                        <div className="grid grid-cols-2 gap-2">
                          <div>
                            <span className="text-[10px] text-slate-400 block mb-1">{t("rtsp.codecFormat")}</span>
                            <select
                              disabled={isLocked}
                              value={inst.config.videoCodec || "h264"}
                              onChange={(e) =>
                                updateInstanceConfig(inst.id, { videoCodec: e.target.value as "h264" | "h265" })
                              }
                              className="w-full h-9 px-2.5 rounded-xl bg-slate-900 border border-white/10 text-xs text-white focus:outline-none focus:border-[var(--module-accent)] cursor-pointer font-semibold text-[var(--module-accent)]"
                            >
                              <option value="h264">{t("rtsp.optH264")}</option>
                              <option value="h265">{t("rtsp.optH265")}</option>
                            </select>
                          </div>
                          <div>
                            <span className="text-[10px] text-slate-400 block mb-1">{t("rtsp.resolution")}</span>
                            <select
                              disabled={isLocked}
                              value={inst.config.resolution || "default"}
                              onChange={(e) => updateInstanceConfig(inst.id, { resolution: e.target.value })}
                              className="w-full h-9 px-2.5 rounded-xl bg-slate-900 border border-white/10 text-xs text-white focus:outline-none focus:border-[var(--module-accent)] cursor-pointer"
                            >
                              <option value="default">{t("rtsp.optResDefault")}</option>
                              <option value="3840x2160">{t("rtsp.optRes4k")}</option>
                              <option value="2560x1440">{t("rtsp.optRes2k")}</option>
                              <option value="1920x1080">{t("rtsp.optRes1080")}</option>
                              <option value="1280x720">{t("rtsp.optRes720")}</option>
                              <option value="640x480">{t("rtsp.optRes480")}</option>
                            </select>
                          </div>
                          <div>
                            <span className="text-[10px] text-slate-400 block mb-1">{t("rtsp.fpsLabel")}</span>
                            <input
                              type="number"
                              disabled={isLocked}
                              min={1}
                              max={240}
                              value={inst.config.fps ?? 30}
                              onChange={(e) => updateInstanceConfig(inst.id, { fps: Math.max(1, Math.min(240, parseInt(e.target.value) || 30)) })}
                              className="w-full h-9 px-2.5 rounded-xl bg-slate-900 border border-white/10 text-xs text-white focus:outline-none focus:border-[var(--module-accent)]"
                              placeholder="30"
                            />
                            <span className="text-[9px] text-slate-500 mt-0.5 block">{inst.config.sourceType === "testsrc" ? t("rtsp.fpsHintTestsrc") : t("rtsp.fpsHint")}</span>
                          </div>
                        </div>

                        <div>
                          <span className="text-[10px] text-slate-400 block mb-1">{t("rtsp.encoderLabel")}</span>
                          <select
                            disabled={isLocked}
                            value={inst.config.gpuAccel || "cpu"}
                            onChange={(e) =>
                              updateInstanceConfig(inst.id, {
                                gpuAccel: e.target.value as any,
                              })
                            }
                            className="w-full h-9 px-2.5 rounded-xl bg-slate-900 border border-white/10 text-xs text-white focus:outline-none focus:border-[var(--module-accent)] cursor-pointer"
                          >
                            <option value="cpu">{t("rtsp.optCpu")}</option>
                            <option value="nvenc">{t("rtsp.optNvenc")}</option>
                            <option value="qsv">{t("rtsp.optQsv")}</option>
                            <option value="amf">{t("rtsp.optAmf")}</option>
                            <option value="copy">{t("rtsp.optCopy")}</option>
                          </select>
                        </div>
                        <div className="grid grid-cols-2 gap-3">
                          <div>
                            <span className="text-[10px] text-slate-400 block mb-1">{t("rtsp.bitrateLabel")}</span>
                            <input
                              type="number"
                              disabled={isLocked}
                              min={0}
                              step={0.1}
                              value={inst.config.bitrateMbps ?? 0}
                              onChange={(e) => updateInstanceConfig(inst.id, { bitrateMbps: Math.max(0, parseFloat(e.target.value) || 0) })}
                              className="w-full h-9 px-2.5 rounded-xl bg-slate-900 border border-white/10 text-xs text-white focus:outline-none focus:border-[var(--module-accent)]"
                              placeholder={t("rtsp.bitratePh")}
                            />
                            <span className="text-[9px] text-slate-500 mt-0.5 block">{inst.config.gpuAccel === "copy" ? t("rtsp.bitrateHintCopy") : t("rtsp.bitrateHint")}</span>
                          </div>
                          <div>
                            <span className="text-[10px] text-slate-400 block mb-1">{t("rtsp.gopLabel")}</span>
                            <input
                              type="number"
                              disabled={isLocked}
                              min={1}
                              max={600}
                              value={inst.config.gop ?? 15}
                              onChange={(e) => updateInstanceConfig(inst.id, { gop: Math.max(1, Math.min(600, parseInt(e.target.value) || 15)) })}
                              className="w-full h-9 px-2.5 rounded-xl bg-slate-900 border border-white/10 text-xs text-white focus:outline-none focus:border-[var(--module-accent)]"
                              placeholder="15"
                            />
                            <span className="text-[9px] text-slate-500 mt-0.5 block">{inst.config.gpuAccel === "copy" ? t("rtsp.gopHintCopy") : t("rtsp.gopHint")}</span>
                          </div>
                        </div>
                      </div>
                    </div>
                  </div>

                  {/* 日志查看区 */}
                  {inst.status.logs && inst.status.logs.length > 0 && (
                    <div className="pt-2 border-t border-white/5">
                      <button
                        onClick={() =>
                          setInstances((prev) =>
                            prev.map((i) => (i.id === inst.id ? { ...i, showLogs: !i.showLogs } : i))
                          )
                        }
                        className="text-[11px] text-slate-400 hover:text-white flex items-center gap-1.5 cursor-pointer"
                      >
                        <Terminal className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                        <span>{t("rtsp.viewLogs", { count: inst.status.logs.length })}</span>
                        {inst.showLogs ? <ChevronUp className="w-3.5 h-3.5" /> : <ChevronDown className="w-3.5 h-3.5" />}
                      </button>

                      {inst.showLogs && (
                        <pre className="mt-2 text-[10px] text-slate-300 bg-black/60 border border-white/10 rounded-xl p-3.5 max-h-52 overflow-y-auto font-mono whitespace-pre-wrap leading-relaxed">
                          {inst.status.logs.join("\n")}
                        </pre>
                      )}
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
