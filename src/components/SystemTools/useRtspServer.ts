import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { alertError } from "../shared/ThemedAlert";
import type { ConfirmRequest } from "../shared/ConfirmDialog";
import { nextRtspPort, serializeRtspInstances } from "./RtspServer.state";
import { DEFAULT_CONFIG, STORAGE_KEY, type CameraDevices, type RtspConfig, type RtspInstanceItem, type RtspServerStatus } from "./RtspServer.model";

export function useRtspServer() {
  const { t } = useTranslation();
  const [panelError, setPanelError] = useState("");
  const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null);
  const [batchBusy, setBatchBusy] = useState(false);
  const alive = useRef(false);
  const lifetime = useRef(0);
  const pollGeneration = useRef(0);
  const pollPending = useRef<number | null>(null);
  const scanning = useRef(false);
  const actions = useRef(new Set<string>());
  const copyTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastSaved = useRef<string | null>(null);
  const beginAction = (id: string) => {
    if (actions.current.has("*") || actions.current.has(id) || (id === "*" && actions.current.size > 0)) return false;
    actions.current.add(id); pollGeneration.current++;
    return true;
  };
  const endAction = (id: string) => { actions.current.delete(id); };
  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; lifetime.current++; pollGeneration.current++; if (copyTimer.current) clearTimeout(copyTimer.current); };
  }, []);
  const [devices, setDevices] = useState<CameraDevices>({ videoDevices: [], audioDevices: [] });
  const [loadingDevices, setLoadingDevices] = useState<boolean>(false);
  const [copiedUrl, setCopiedUrl] = useState<string | null>(null);
  const [allIps, setAllIps] = useState<{ name: string; ip: string }[]>([]);

  // 加载本机所有网卡 IP
  const loadAllIps = async () => {
    try {
      const list = await invoke<[string, string][]>("get_all_local_ips");
      if (alive.current) setAllIps(list.map(([name, ip]) => ({ name, ip })));
    } catch (e) {
      if (alive.current) setPanelError(String(e));
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
        if (Array.isArray(parsed)) {
          return parsed.map((item: any, idx: number) => {
            const id = item.id || `inst_${Date.now()}_${idx}`;
            return ({
            id,
            title: item.title || t("rtsp.nodeTitle", { n: idx + 1 }),
            config: { ...DEFAULT_CONFIG, ...item.config, id },
            status: { id, running: false, logs: [], uptimeSeconds: 0 },
            showLogs: false,
            collapsed: item.collapsed ?? true, // 默认收起/折叠，保持页面简洁
            actionLoading: false,
            error: null,
          }); });
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

  // Persist only configuration changes; polling/status/log updates have identical snapshots.
  const savedConfig = serializeRtspInstances(instances);
  useEffect(() => {
    if (lastSaved.current === savedConfig) return;
    try { localStorage.setItem(STORAGE_KEY, savedConfig); lastSaved.current = savedConfig; }
    catch (e) { setPanelError(String(e)); }
  }, [savedConfig]);

  // 扫描音视频设备
  const fetchDevices = async () => {
    if (scanning.current) return;
    scanning.current = true;
    setLoadingDevices(true);
    try {
      const res = await invoke<CameraDevices>("get_rtsp_camera_devices");
      if (alive.current) setDevices(res);
    } catch (e: any) {
      if (alive.current) setPanelError(String(e));
    } finally {
      scanning.current = false;
      if (alive.current) setLoadingDevices(false);
    }
  };

  // 轮询更新 RTSP 实例运行状态
  const pollStatuses = async (force = false) => {
    if (!alive.current || actions.current.size > 0 || (!force && pollPending.current !== null)) return;
    const generation = ++pollGeneration.current;
    pollPending.current = generation;
    const epoch = lifetime.current;
    try {
      const statuses = await invoke<RtspServerStatus[]>("get_all_rtsp_server_statuses");
      if (!alive.current || epoch !== lifetime.current || generation !== pollGeneration.current) return;
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
      if (alive.current && epoch === lifetime.current && generation === pollGeneration.current) setPanelError(String(e));
    } finally {
      if (pollPending.current === generation) pollPending.current = null;
    }
  };

  useEffect(() => {
    fetchDevices();
    pollStatuses();
    const interval = setInterval(() => { if (!document.hidden) void pollStatuses(); }, 2000);
    return () => clearInterval(interval);
  }, []);

  // 新增 RTSP 实例
  const handleAddInstance = () => {
    const newPort = nextRtspPort(instances);
    if (newPort === undefined) { setPanelError(t("rtsp.needPort")); return; }
    const newId = `inst_${crypto.randomUUID()}`;
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
    setInstances((previous) => [...previous, newInst]);
  };

  // 删除实例
  const handleDeleteInstance = async (id: string, e?: React.MouseEvent) => {
    e?.stopPropagation();
    if (!beginAction(id)) return;
    setInstances((previous) => previous.map((item) => item.id === id ? { ...item, actionLoading: true } : item));
    try {
      // Backend stop is idempotent; always stop even if the last poll said "stopped".
      await invoke("stop_rtsp_server", { id });
      if (alive.current) setInstances((previous) => previous.filter((item) => item.id !== id));
    } catch (error) {
      if (alive.current) setInstances((previous) => previous.map((item) => item.id === id ? { ...item, error: String(error), collapsed: false } : item));
    } finally {
      endAction(id);
      if (alive.current) setInstances((previous) => previous.map((item) => item.id === id ? { ...item, actionLoading: false } : item));
    }
  };

  const handleStopAll = async () => {
    if (!beginAction("*")) return;
    setBatchBusy(true);
    try { await invoke("stop_all_rtsp_servers"); }
    catch (error) { if (alive.current) setPanelError(String(error)); }
    finally { endAction("*"); if (alive.current) setBatchBusy(false); await pollStatuses(true); }
  };

  // 启动所有实例
  const handleStartAll = async () => {
    const pending = instances.filter((i) => !i.status.running);
    if (pending.length === 0 || actions.current.size > 0) return;

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
      if (!Number.isInteger(inst.config.port) || inst.config.port < 1024 || inst.config.port > 65535) {
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

    if (!beginAction("*")) return;
    setBatchBusy(true);
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

    endAction("*");
    if (!alive.current) return;
    setBatchBusy(false);
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
        if (inst.id !== id || inst.actionLoading) return inst;
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
    const others = instances.filter((i) => i.id !== source.id && !i.status.running && !i.actionLoading);
    if (others.length === 0) return;
    setConfirmRequest({ desc: t("rtsp.copyConfirm", { title: source.title, count: others.length }), onConfirm: () => setInstances((prev) =>
      prev.map((inst) => {
        if (inst.id === source.id || inst.actionLoading || inst.status.running) return inst;
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
    ) });
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
    if (!Number.isInteger(inst.config.port) || inst.config.port < 1024 || inst.config.port > 65535) {
      setInstances((prev) =>
        prev.map((i) => (i.id === inst.id ? { ...i, error: t("rtsp.needPort"), collapsed: false } : i))
      );
      return;
    }

    if (!beginAction(inst.id)) return;
    setInstances((prev) =>
      prev.map((i) => (i.id === inst.id ? { ...i, actionLoading: true, error: null } : i))
    );

    try {
      const res = await invoke<RtspServerStatus>("start_rtsp_server", {
        config: { ...inst.config, id: inst.id },
      });
      if (!alive.current) return;
      setInstances((prev) =>
        prev.map((i) => (i.id === inst.id ? { ...i, status: res, actionLoading: false } : i))
      );
    } catch (err: any) {
      if (!alive.current) return;
      const errStr = String(err);
      setInstances((prev) =>
        prev.map((i) =>
          i.id === inst.id ? { ...i, error: errStr, showLogs: true, collapsed: false, actionLoading: false } : i
        )
      );
    } finally { endAction(inst.id); }
  };

  // 停止单个实例
  const handleStopInstance = async (id: string, e?: React.MouseEvent) => {
    e?.stopPropagation();
    if (!beginAction(id)) return;
    setInstances((prev) =>
      prev.map((i) => (i.id === id ? { ...i, actionLoading: true, error: null } : i))
    );

    try {
      await invoke("stop_rtsp_server", { id });

    } catch (err: any) {
      if (alive.current) setInstances((prev) =>
        prev.map((i) => (i.id === id ? { ...i, error: String(err) } : i))
      );
    } finally {
      endAction(id);
      await pollStatuses(true);
      if (!alive.current) return;
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
      alertError(t("rtsp.pickerFail"));
    }
  };

  const handleCopy = async (text: string, e?: React.MouseEvent) => {
    e?.stopPropagation();
    try {
      await navigator.clipboard.writeText(text);
      if (!alive.current) return;
      setCopiedUrl(text);
      if (copyTimer.current) clearTimeout(copyTimer.current);
      copyTimer.current = setTimeout(() => setCopiedUrl(null), 2000);
    } catch (error) { if (alive.current) setPanelError(String(error)); }
  };

  const formatUptime = (seconds: number) => {
    const hrs = Math.floor(seconds / 3600);
    const mins = Math.floor((seconds % 3600) / 60);
    const secs = seconds % 60;
    if (hrs > 0) return t("rtsp.uptimeHms", { h: hrs, m: mins, s: secs });
    if (mins > 0) return t("rtsp.uptimeMs", { m: mins, s: secs });
    return t("rtsp.uptimeS", { s: secs });
  };

  return { panelError, batchBusy, confirmRequest, setConfirmRequest, t, devices, loadingDevices, copiedUrl, allIps, loadAllIps, instances, setInstances, fetchDevices, handleAddInstance, handleDeleteInstance, handleStopAll, handleStartAll, toggleCollapse, updateInstanceConfig, handleCopyConfigToAll, handleStartInstance, handleStopInstance, handleSelectFile, handleCopy, formatUptime };
}
