import React, { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, emit } from "@tauri-apps/api/event";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import {
  enable as enableAutostart,
  disable as disableAutostart,
  isEnabled as isAutostartEnabled,
} from "@tauri-apps/plugin-autostart";
import {
  FolderKanban,
  Save,
  RefreshCw,
  Info,
  CheckCircle2,
  ExternalLink,
  FolderOpen,
  AlertTriangle,
  Trash2,
  Loader2,
  FileText,
  Power,
  Rocket,
  Zap,
  Server,
  Database,
  Waypoints,
  Video,
  Globe,
  Keyboard,
  LayoutGrid,
  Sliders,
  Shield,
  Download,
  Upload,
  Layers,
  Folder,
} from "lucide-react";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import type { LauncherSetting } from "./launcher/types";
import { MODULE_ORDER, MODULE_DEFAULTS } from "../App";

interface Config {
  versions_dir?: string;
  links_dir?: string;
  data_dir?: string;
  sdk_dir?: string;
  node_projects_dir?: string;
}

import type { AiConfig } from "./ai/types";
import type { ProjectStatus } from "./project/types";

interface MigrateResult {
  moved_versions: boolean;
  moved_links: boolean;
  recreated_junctions: string[];
  updated_env_vars: string[];
  updated_path_entries: string[];
  errors: string[];
  old_dirs_remain: string[];
}

interface MigrateProgress {
  stage: string;
  current: number;
  total: number;
  file_name: string;
}

interface SkillMigrateProgress {
  stage: string;
  current: number;
  total: number;
  skill_name: string;
}

/** 托盘右键菜单配置（与后端 TrayMenuConfig 对应） */
interface TrayMenuConfig {
  show_mihomo: boolean;
  show_mihomo_profiles: boolean;
  show_mihomo_proxies: boolean;
  show_mihomo_mode: boolean;
  mihomo_proxy_limit: number;
}

export default function GlobalSettings() {
  const [dataDir, setDataDir] = useState("");
  const [oldDataDir, setOldDataDir] = useState("");
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [success, setSuccess] = useState(false);
  const [migrateResult, setMigrateResult] = useState<MigrateResult | null>(
    null,
  );
  const [showMigrateConfirm, setShowMigrateConfirm] = useState(false);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [latestVersion, setLatestVersion] = useState<string | null>(null);
  const [updateBody, setUpdateBody] = useState<string | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [appVersion, setAppVersion] = useState("");
  // "plugin" = 来自 tauri-plugin-updater（可应用内下载安装）；"github" = 兜底（仅打开下载页）
  const [updateSource, setUpdateSource] = useState<"plugin" | "github" | null>(
    null,
  );
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<MigrateProgress | null>(null);
  const [deletingOldDirs, setDeletingOldDirs] = useState(false);
  const [deletedOldDirs, setDeletedOldDirs] = useState<string[] | null>(null);
  const [aiConfig, setAiConfig] = useState<AiConfig | null>(null);
  const [aiDefaultPath, setAiDefaultPath] = useState("");
  const [savingAi, setSavingAi] = useState(false);
  const [aiSaved, setAiSaved] = useState(false);
  const [skillProgress, setSkillProgress] =
    useState<SkillMigrateProgress | null>(null);
  const [skillMigrated, setSkillMigrated] = useState(false);
  // 开机自启：反映操作系统真实注册状态（打开设置页时查询）
  const [autostartOn, setAutostartOn] = useState(false);
  const [autostartBusy, setAutostartBusy] = useState(false);
  // 服务自启配置
  const [autoStartServices, setAutoStartServices] = useState<string[]>([]);
  const [sdkProjects, setSdkProjects] = useState<ProjectStatus[]>([]);
  const [autoStartBusyMap, setAutoStartBusyMap] = useState<Record<string, boolean>>({});

  const fetchAutoStartServices = async () => {
    try {
      const [list, projects] = await Promise.all([
        invoke<string[]>("get_auto_start_services"),
        invoke<ProjectStatus[]>("project_list_fast"),
      ]);
      setAutoStartServices(list || []);
      setSdkProjects(projects || []);
    } catch (e) {
      console.error(e);
    }
  };

  const handleToggleAutoStartService = async (serviceId: string, enabled: boolean) => {
    setAutoStartBusyMap((prev) => ({ ...prev, [serviceId]: true }));
    try {
      await invoke("set_auto_start_service", { serviceId, enabled });
      setAutoStartServices((prev) =>
        enabled ? [...prev.filter((id) => id !== serviceId), serviceId] : prev.filter((id) => id !== serviceId)
      );
    } catch (e: any) {
      alert(`设置服务自启失败: ${e}`);
    } finally {
      setAutoStartBusyMap((prev) => ({ ...prev, [serviceId]: false }));
    }
  };

  // 托盘右键菜单配置
  const [trayCfg, setTrayCfg] = useState<TrayMenuConfig>({
    show_mihomo: true,
    show_mihomo_profiles: true,
    show_mihomo_proxies: true,
    show_mihomo_mode: true,
    mihomo_proxy_limit: 30,
  });
  const [trayBusy, setTrayBusy] = useState(false);

  const saveTrayCfg = async (patch: Partial<TrayMenuConfig>) => {
    const next = { ...trayCfg, ...patch };
    setTrayCfg(next);
    setTrayBusy(true);
    try {
      await invoke("set_tray_menu_config", { value: next });
    } catch (e) {
      console.error(e);
    } finally {
      setTrayBusy(false);
    }
  };

  const fetchConfig = async () => {
    setLoading(true);
    setSuccess(false);
    try {
      const config = await invoke<Config>("get_config");
      // 数据目录（旧版本 config 可能无此字段，回退取后端默认值）
      if (config.data_dir) {
        setDataDir(config.data_dir);
        setOldDataDir(config.data_dir);
      } else {
        const dir = await invoke<string>("get_data_dir_cmd").catch(() => "");
        setDataDir(dir);
        setOldDataDir(dir);
      }
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  };

  const fetchVersion = async () => {
    try {
      const ver = await invoke<string>("get_app_version");
      setAppVersion(ver);
    } catch {
      setAppVersion("1.0.0");
    }
  };

  const fetchAiConfig = async () => {
    try {
      const cfg = await invoke<AiConfig>("get_ai_config");
      setAiConfig(cfg);
      setAiDefaultPath(cfg.default_project_path || "");
    } catch (e) {
      console.error(e);
    }
  };

  const handleSaveAiConfig = async () => {
    if (!aiConfig) return;
    setSavingAi(true);
    setAiSaved(false);
    setSkillMigrated(false);

    // 监听技能迁移进度
    const unlisten = await listen<SkillMigrateProgress>(
      "skill-migrate-progress",
      (event) => {
        setSkillProgress(event.payload);
      },
    );

    try {
      const updated: AiConfig = {
        ...aiConfig,
        default_project_path: aiDefaultPath,
      };
      const result = await invoke<{ ok: boolean; skill_migrated: boolean }>(
        "save_ai_config",
        { config: updated },
      );
      setAiConfig(updated);
      setAiSaved(true);
      if (result.skill_migrated) {
        setSkillMigrated(true);
      }
      setTimeout(() => setAiSaved(false), 3000);
      setTimeout(() => setSkillMigrated(false), 6000);
    } catch (e: any) {
      alert(`保存失败: ${e}`);
    } finally {
      unlisten();
      setSkillProgress(null);
      setSavingAi(false);
    }
  };

  const fetchAutostart = async () => {
    try {
      setAutostartOn(await isAutostartEnabled());
    } catch (e) {
      console.error(e);
    }
  };

  const handleToggleAutostart = async () => {
    if (autostartBusy) return;
    setAutostartBusy(true);
    try {
      if (autostartOn) {
        await disableAutostart();
        setAutostartOn(false);
      } else {
        await enableAutostart();
        setAutostartOn(true);
      }
    } catch (e: any) {
      alert(`设置开机自启失败: ${e}`);
      // 失败后以系统真实状态为准
      await fetchAutostart();
    } finally {
      setAutostartBusy(false);
    }
  };

  // 启动器配置 (全局唤起快捷键 + 视图设置)
  const [launcherCfg, setLauncherCfg] = useState<LauncherSetting>({
    showHideShortcutKey: "Alt+Space",
    moduleHotkeys: {},
    itemIconSize: 32,
    itemColumnNumber: 0,
    cardDensity: "cozy",
    showItemName: true,
    iconBackgroundColor: false,
    itemFontSize: 12,
    itemRadius: 12,
    itemBorder: true,
    categoryFontSize: 12,
    categoryGap: 24,
  });
  // 录制目标：null=未录制；"main"=主唤起热键；否则为某个顶级模块的 moduleId
  const [recordingField, setRecordingField] = useState<string | null>(null);
  // 始终持有最新 launcherCfg，供录制监听闭包（仅依赖 recordingField）安全读取
  const launcherCfgRef = useRef<LauncherSetting>(launcherCfg);
  useEffect(() => {
    launcherCfgRef.current = launcherCfg;
  }, [launcherCfg]);
  const [savingLauncher, setSavingLauncher] = useState(false);
  const [launcherSaved, setLauncherSaved] = useState(false);

  // ---- 外观：模块主题色 + 全局字体 ----
  const [appearance, setAppearance] = useState<{
    moduleThemeColors: Record<string, string>;
    globalFont: string;
    customFontPath: string;
  }>({ moduleThemeColors: {}, globalFont: "", customFontPath: "" });
  const [importingFont, setImportingFont] = useState(false);

  const MODULE_APPEARANCE_DEFAULTS: Record<string, { label: string; color: string }> = {
    launcher: { label: "启动", color: "#8b5cf6" },
    news: { label: "资讯", color: "#ea580c" },
    sdk: { label: "SDK", color: "#2563eb" },
    ai: { label: "AI", color: "#7c3aed" },
    tasks: { label: "任务", color: "#f59e0b" },
    node: { label: "服务", color: "#0891b2" },
    mihomo: { label: "代理", color: "#4f46e5" },
    cert: { label: "证书", color: "#0d9488" },
    tools: { label: "更多", color: "#059669" },
    settings: { label: "设置", color: "#dc2626" },
  };

  const fetchAppearance = async () => {
    try {
      const ap = await invoke<{
        moduleThemeColors: Record<string, string>;
        globalFont: string;
        customFontPath: string;
      }>("get_appearance_config");
      setAppearance(ap);
    } catch (e) {
      console.error("读取外观配置失败", e);
    }
  };

  const handleSetModuleColor = async (moduleId: string, color: string) => {
    const trimmed = color.trim();
    const next = { ...appearance.moduleThemeColors };
    if (trimmed && trimmed !== MODULE_APPEARANCE_DEFAULTS[moduleId]?.color) {
      next[moduleId] = trimmed;
    } else {
      delete next[moduleId];
    }
    setAppearance({ ...appearance, moduleThemeColors: next });
    try {
      await invoke("set_module_theme_color", { moduleId, color: trimmed });
      emit("appearance-updated");
    } catch (e) {
      console.error("保存模块主题色失败", e);
    }
  };

  const handleSetGlobalFont = async (font: string) => {
    setAppearance({ ...appearance, globalFont: font });
    try {
      await invoke("set_global_font", { font });
      emit("appearance-updated");
    } catch (e) {
      console.error("保存全局字体失败", e);
    }
  };

  const handleImportFont = async () => {
    try {
      const selected = await openDialog({
        title: "选择字体文件",
        multiple: false,
        filters: [
          { name: "字体文件", extensions: ["ttf", "otf", "woff", "woff2"] },
        ],
      });
      if (!selected) return;
      setImportingFont(true);
      const res = await invoke<{ family: string; path: string; ext: string }>(
        "import_custom_font",
        { src: selected as string }
      );
      setAppearance({ moduleThemeColors: appearance.moduleThemeColors, globalFont: res.family, customFontPath: res.path });
      emit("appearance-updated");
      alert(`字体导入成功：${res.family}`);
    } catch (e: any) {
      alert(`导入字体失败: ${e}`);
    } finally {
      setImportingFont(false);
    }
  };

  const handleClearCustomFont = async () => {
    try {
      await invoke("clear_custom_font");
      setAppearance({ ...appearance, globalFont: "", customFontPath: "" });
      emit("appearance-updated");
      alert("已移除自定义字体，恢复默认字体");
    } catch (e: any) {
      alert(`移除字体失败: ${e}`);
    }
  };

  const fetchLauncherConfig = async () => {
    try {
      const cfg = await invoke<LauncherSetting>("launcher_get_settings");
      if (cfg) setLauncherCfg(cfg);
    } catch (e) {
      console.error(e);
    }
  };

  const handleSaveLauncherConfig = async (patch?: Partial<LauncherSetting>) => {
    const next = patch ? { ...launcherCfgRef.current, ...patch } : launcherCfgRef.current;
    if (patch) setLauncherCfg(next);
    setSavingLauncher(true);
    setLauncherSaved(false);
    try {
      await invoke("launcher_save_settings", { settings: next });
      setLauncherSaved(true);
      setTimeout(() => setLauncherSaved(false), 2500);
    } catch (e: any) {
      alert(`保存启动器配置失败: ${e}`);
    } finally {
      setSavingLauncher(false);
    }
  };

  // 全局热键录制监听（基于 e.code 解析，支持 F1-F12、Shift/Ctrl/Alt+组合、单键等）
  useEffect(() => {
    if (!recordingField) return;
    const target = recordingField; // 本次录制目标：main 或 模块 id

    const handleKeyCapture = (e: KeyboardEvent) => {
      // Escape 取消录制
      if (e.code === "Escape") {
        e.preventDefault();
        setRecordingField(null);
        return;
      }

      // 忽略单独按下的修饰键，等待最终按键
      if (["ShiftLeft", "ShiftRight", "ControlLeft", "ControlRight", "AltLeft", "AltRight", "MetaLeft", "MetaRight"].includes(e.code)) {
        return;
      }

      e.preventDefault();
      e.stopPropagation();

      const parts: string[] = [];
      if (e.ctrlKey) parts.push("Ctrl");
      if (e.altKey) parts.push("Alt");
      if (e.shiftKey) parts.push("Shift");
      if (e.metaKey) parts.push("Win");

      let key = "";
      const code = e.code;
      if (code.startsWith("Key")) {
        key = code.slice(3); // KeyA -> A
      } else if (code.startsWith("Digit")) {
        key = code.slice(5); // Digit1 -> 1
      } else if (code.startsWith("F") && /^(F[1-9]|F1[0-2])$/.test(code)) {
        key = code; // F1..F12
      } else {
        const map: Record<string, string> = {
          Space: "Space",
          Enter: "Enter",
          Tab: "Tab",
          ArrowUp: "Up",
          ArrowDown: "Down",
          ArrowLeft: "Left",
          ArrowRight: "Right",
          Backquote: "`",
          Backspace: "Backspace",
          Delete: "Delete",
          Insert: "Insert",
          Home: "Home",
          End: "End",
          PageUp: "PageUp",
          PageDown: "PageDown",
          Escape: "Esc",
        };
        key = map[code] || "";
      }

      if (!key) {
        // 无法识别的按键，忽略（如纯修饰键已在上游拦截）
        return;
      }

      const formattedKey = key.length === 1 ? key.toUpperCase() : key;
      if (!parts.includes(formattedKey)) {
        parts.push(formattedKey);
      }
      const hotkeyStr = parts.join("+");

      if (target === "main") {
        handleSaveLauncherConfig({ showHideShortcutKey: hotkeyStr });
      } else {
        // 写入对应模块的独立热键
        const cur = launcherCfgRef.current.moduleHotkeys || {};
        const nextMap = { ...cur, [target]: hotkeyStr };
        handleSaveLauncherConfig({ moduleHotkeys: nextMap });
      }
      setRecordingField(null);
    };

    const handleWindowClick = () => {
      setRecordingField(null);
    };

    window.addEventListener("keydown", handleKeyCapture, true);
    window.addEventListener("mousedown", handleWindowClick);
    return () => {
      window.removeEventListener("keydown", handleKeyCapture, true);
      window.removeEventListener("mousedown", handleWindowClick);
    };
  }, [recordingField]);

  const handleLauncherExport = async () => {
    try {
      const jsonStr = await invoke<string>("launcher_export_backup");
      const filePath = await saveDialog({
        title: "保存启动器备份文件",
        defaultPath: `anyversion_launcher_backup_${new Date().toISOString().slice(0, 10)}.json`,
        filters: [{ name: "JSON Backup", extensions: ["json"] }],
      });
      if (filePath) {
        await invoke("write_text_file", { path: filePath, content: jsonStr });
        alert("启动器备份导出成功！");
      }
    } catch (e: any) {
      alert(`导出失败: ${e}`);
    }
  };

  const handleLauncherImport = async () => {
    try {
      const selected = await openDialog({
        title: "选择启动器备份文件 (支持 DawnLauncher .db/.json 及 AnyVersion 备份)",
        multiple: false,
        filters: [
          { name: "支持的所有备份格式 (*.db, *.json)", extensions: ["db", "json"] },
          { name: "Dawn Launcher 数据库备份 (*.db)", extensions: ["db"] },
          { name: "JSON 备份 (*.json)", extensions: ["json"] },
        ],
      });
      if (selected && typeof selected === "string") {
        const count = await invoke<number>("launcher_import_backup_file", { filePath: selected });
        await fetchLauncherConfig();
        alert(`启动器数据已成功导入与恢复！共导入 ${count || 0} 个快捷启动项目。`);
      }
    } catch (e: any) {
      alert(`导入恢复失败: ${e}`);
    }
  };

  useEffect(() => {
    fetchConfig();
    fetchVersion();
    fetchAiConfig();
    fetchAutostart();
    fetchAutoStartServices();
    fetchLauncherConfig();
    fetchAppearance();
    invoke<TrayMenuConfig>("get_tray_menu_config")
      .then(setTrayCfg)
      .catch(() => {});
  }, []);

  const pathsChanged = (): boolean => {
    const normalize = (s: string) => s.trim().replace(/[\\/]+$/, "");
    return normalize(dataDir) !== normalize(oldDataDir);
  };

  const handleSaveClick = () => {
    if (!dataDir) return;
    if (pathsChanged()) {
      setShowMigrateConfirm(true);
    } else {
      handleSave();
    }
  };

  const handleSave = async () => {
    if (!dataDir) return;
    setSaving(true);
    setSuccess(false);
    setMigrateResult(null);
    setShowMigrateConfirm(false);
    setDeletedOldDirs(null);
    setProgress(null);

    // 监听进度事件
    const unlisten = await listen<MigrateProgress>(
      "migrate-progress",
      (event) => {
        setProgress(event.payload);
      },
    );

    try {
      const result = await invoke<MigrateResult>("update_config", {
        dataDir: dataDir || oldDataDir,
      });
      setMigrateResult(result);
      setSuccess(true);
      await fetchConfig();
    } catch (e: any) {
      alert(`保存配置失败: ${e}`);
    } finally {
      unlisten();
      setProgress(null);
      setSaving(false);
    }
  };

  const handleDeleteOldDirs = async () => {
    if (!migrateResult?.old_dirs_remain?.length) return;
    if (
      !confirm(
        `确定要删除以下旧目录吗？\n\n${migrateResult.old_dirs_remain.join("\n")}\n\n删除后无法恢复！`,
      )
    )
      return;
    setDeletingOldDirs(true);
    try {
      const deleted = await invoke<string[]>("delete_old_storage_dirs", {
        dirs: migrateResult.old_dirs_remain,
      });
      setDeletedOldDirs(deleted);
      // 清除残留目录列表
      setMigrateResult({ ...migrateResult, old_dirs_remain: [] });
    } catch (e: any) {
      alert(`删除失败: ${e}`);
    } finally {
      setDeletingOldDirs(false);
    }
  };

  const handleCheckUpdate = async () => {
    setCheckingUpdate(true);
    setUpdateError(null);
    setLatestVersion(null);
    setUpdateSource(null);
    try {
      // 1) 优先走 Tauri 官方更新器：可应用内下载 + 安装 + 重启
      try {
        const update = await check();
        if (update) {
          setLatestVersion(update.version);
          setUpdateBody(update.body ?? null);
          setUpdateSource("plugin");
          return;
        }
        alert("当前已是最新版本！");
        return;
      } catch (pluginErr) {
        // 插件未配置 / 网络异常时，降级为 GitHub API 通知（仅打开下载页）
        console.warn(
          "[updater] plugin check failed, fallback to GitHub API:",
          pluginErr,
        );
      }
      // 2) 兜底：GitHub REST 通知
      const resp = await fetch(
        "https://api.github.com/repos/void-soul/any-version/releases/latest",
        {
          headers: { Accept: "application/vnd.github.v3+json" },
        },
      );
      if (!resp.ok) throw new Error("检查失败: " + resp.status);
      const data = await resp.json();
      const tag = data.tag_name?.replace(/^v/, "") ?? "";
      const currentVer = appVersion || "1.0.0";
      if (tag && tag !== currentVer) {
        setLatestVersion(tag);
        setUpdateBody(data.body ?? null);
        setUpdateSource("github");
      } else {
        setUpdateError(null);
        alert("当前已是最新版本！");
      }
    } catch (e: any) {
      setUpdateError(e.message || "检查更新失败");
    } finally {
      setCheckingUpdate(false);
    }
  };

  // 应用内下载并安装更新，完成后重启
  const handleInstallUpdate = async () => {
    try {
      setInstalling(true);
      setUpdateError(null);
      const update = await check();
      if (!update) {
        setInstalling(false);
        return;
      }
      await update.downloadAndInstall((event) => {
        // event: { event: 'Started'|'Progress'|'Finished', data: {...} }
        if (event.event === "Finished") {
          // 下载完成，准备重启
        }
      });
      await relaunch();
    } catch (e: any) {
      setUpdateError(e.message || "更新安装失败");
      setInstalling(false);
    }
  };

  const handleBrowseFolder = async (setter: (v: string) => void) => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, title: "选择文件夹" });
      if (selected) setter(selected as string);
    } catch {
      alert("文件夹选择器不可用，请手动输入路径。");
    }
  };

  const handleDownloadUpdate = () => {
    window.open(
      "https://github.com/void-soul/any-version/releases/latest",
      "_blank",
    );
  };

  return (
    <div className="flex-1 p-8 space-y-6 select-none max-w-3xl mx-auto">
      {/* Header */}

      <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-6">
        <div className="flex items-center gap-2 pb-3 border-b border-white/5">
          <FolderKanban className="w-4 h-4 text-[var(--module-accent)]" />
          <h3 className="text-xs font-semibold text-white">
            AnyVersion 工作目录说明
          </h3>
        </div>

        <div className="p-4 bg-indigo-500/5 border border-indigo-500/15 rounded-xl space-y-2 text-[10px] text-slate-300 leading-relaxed">
          <p className="font-semibold text-indigo-300 text-[11px]">
            存储目录说明
          </p>
          <p>
            •{" "}
            <span className="font-mono text-slate-200">
              数据目录 (data_dir)
            </span>
            ：唯一可配置路径，承载所有可变数据。SDK（合并了原「存储目录 +
            链接目录」， 内部用 <span className="font-mono">_versions</span>{" "}
            存多版本库、<span className="font-mono">sdk/</span>
            根放每种工具的激活锚点）、Node
            服务项目、证书、缓存、数据库等全部作为其子目录自动派生。
            设为非系统盘（如 D 盘）可避免占用 C 盘空间。
          </p>
        </div>

        {loading ? (
          <div className="text-xs text-slate-400 py-6 flex items-center gap-2">
            <RefreshCw className="w-4 h-4 animate-spin text-[var(--module-accent)]" />
            正在读取系统配置...
          </div>
        ) : (
          <div className="space-y-4">
            <div className="space-y-1.5">
              <label className="text-[10px] text-slate-500 uppercase font-semibold">
                数据目录 (data_dir)
              </label>
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  value={dataDir}
                  onChange={(e) => setDataDir(e.target.value)}
                  className="flex-1 glass-input px-3.5 py-2.5 text-xs font-mono"
                  placeholder="e.g. D:\AnyVersion"
                />
                <button
                  onClick={() => handleBrowseFolder(setDataDir)}
                  className="p-2.5 bg-white/5 hover:bg-white/10 text-slate-400 hover:text-slate-200 rounded-lg border border-white/5 cursor-pointer transition-all flex-shrink-0"
                  title="选择文件夹"
                >
                  <FolderOpen className="w-4 h-4" />
                </button>
              </div>
              <p className="text-[9px] text-slate-500">
                唯一数据根目录（默认 ~/.any-version）。SDK、Node
                服务项目、证书、缓存、数据库等 全部自动放在它的子目录下（
                <span className="font-mono">sdk/</span>、
                <span className="font-mono">node-projects/</span>、
                <span className="font-mono">certs/</span>、
                <span className="font-mono">tasks.db</span>{" "}
                等）。改到非系统盘可节约 C 盘空间。
              </p>
            </div>

            {/* 派生路径只读展示 */}
            <div className="rounded-xl border border-white/5 bg-white/[0.02] p-3 space-y-1.5">
              <p className="text-[10px] text-slate-500 uppercase font-semibold">
                自动派生的子目录
              </p>
              <div className="text-[10px] font-mono text-slate-400 space-y-1">
                <div className="flex items-center gap-2">
                  <span className="text-cyan-400 flex-shrink-0">SDK</span>
                  <span className="truncate">
                    {(dataDir || "…").replace(/[\\/]+$/, "")}\sdk
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-emerald-400 flex-shrink-0">
                    Node 服务
                  </span>
                  <span className="truncate">
                    {(dataDir || "…").replace(/[\\/]+$/, "")}\node-projects
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-amber-400 flex-shrink-0">证书</span>
                  <span className="truncate">
                    {(dataDir || "…").replace(/[\\/]+$/, "")}\certs
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-slate-400 flex-shrink-0">
                    缓存/数据库
                  </span>
                  <span className="truncate">
                    {(dataDir || "…").replace(/[\\/]+$/, "")}
                    \tasks.db、version_cache、backup 等
                  </span>
                </div>
              </div>
            </div>

            {/* 路径变更确认弹窗 */}
            {showMigrateConfirm && (
              <div className="p-4 bg-amber-500/10 border border-amber-500/20 rounded-xl space-y-3 animate-fadeIn">
                <h4 className="text-xs font-semibold text-amber-400 flex items-center gap-1.5">
                  <AlertTriangle className="w-4 h-4" />
                  确认路径迁移
                </h4>
                <div className="text-[10px] text-slate-300 space-y-1.5">
                  <p>检测到存储路径已更改，AnyVersion 将执行以下操作：</p>
                  <p className="text-amber-300">
                    1. 将旧目录下的所有已安装版本文件移动到新目录
                  </p>
                  <p className="text-amber-300">
                    2. 更新所有 junction 链接的指向
                  </p>
                  <p className="text-amber-300">
                    3. 更新 PATH 环境变量中的旧路径为新路径
                  </p>
                  <p className="text-slate-400 mt-1">
                    整个过程无需手动操作，已安装的 SDK 不会丢失。
                  </p>
                </div>
                <div className="flex items-center gap-2 pt-1">
                  <button
                    onClick={handleSave}
                    disabled={saving}
                    className="px-4 py-2 bg-amber-600 hover:bg-amber-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold cursor-pointer transition-all flex items-center gap-1.5"
                  >
                    <Save className="w-3 h-3" />
                    {saving ? "正在迁移..." : "确认迁移并保存"}
                  </button>
                  <button
                    onClick={() => setShowMigrateConfirm(false)}
                    className="px-4 py-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-xl text-xs font-medium cursor-pointer border border-white/10"
                  >
                    取消
                  </button>
                </div>
              </div>
            )}

            {/* 迁移进度条 */}
            {progress && (
              <div className="p-3 bg-[var(--module-accent-soft)] border border-[var(--module-accent-ring)] rounded-xl space-y-2 animate-fadeIn">
                <div className="flex items-center justify-between text-[10px]">
                  <span className="text-[var(--module-accent)] font-semibold flex items-center gap-1.5">
                    <Loader2 className="w-3 h-3 animate-spin" />
                    {progress.stage}
                  </span>
                  {progress.total > 0 && (
                    <span className="text-[var(--module-accent)] font-mono">
                      {progress.current}/{progress.total}
                    </span>
                  )}
                </div>
                {progress.total > 0 && (
                  <div className="w-full bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] rounded-full h-1.5 overflow-hidden">
                    <div
                      className="bg-[var(--module-accent)] h-full rounded-full transition-all duration-300"
                      style={{
                        width: `${Math.round((progress.current / progress.total) * 100)}%`,
                      }}
                    />
                  </div>
                )}
                {progress.file_name && (
                  <div className="flex items-center gap-1 text-[9px] text-slate-400">
                    <FileText className="w-2.5 h-2.5 flex-shrink-0" />
                    <span className="truncate">{progress.file_name}</span>
                  </div>
                )}
              </div>
            )}

            {/* 迁移结果展示 */}
            {migrateResult && (
              <div className="p-4 bg-emerald-500/5 border border-emerald-500/15 rounded-xl space-y-2 text-[10px]">
                <h4 className="text-xs font-semibold text-emerald-400">
                  迁移完成
                </h4>
                {migrateResult.moved_versions && (
                  <p className="text-slate-300">✓ 版本文件已移动到新目录</p>
                )}
                {migrateResult.moved_links && (
                  <p className="text-slate-300">✓ 链接目录已移动到新目录</p>
                )}
                {migrateResult.recreated_junctions.length > 0 && (
                  <p className="text-slate-300">
                    ✓ 已重建 {migrateResult.recreated_junctions.length} 个
                    junction 链接:{" "}
                    {migrateResult.recreated_junctions.join(", ")}
                  </p>
                )}
                {migrateResult.updated_env_vars.length > 0 && (
                  <p className="text-slate-300">
                    ✓ 已更新环境变量:{" "}
                    {migrateResult.updated_env_vars.join(", ")}
                  </p>
                )}
                {migrateResult.updated_path_entries.length > 0 && (
                  <p className="text-slate-300">
                    ✓ 已更新 {migrateResult.updated_path_entries.length} 个 PATH
                    条目
                  </p>
                )}

                {/* 旧目录清理提示 */}
                {migrateResult.old_dirs_remain.length > 0 && (
                  <div className="pt-2 mt-2 border-t border-amber-500/15 space-y-2">
                    <div className="flex items-start gap-1.5 text-amber-300">
                      <AlertTriangle className="w-3 h-3 mt-0.5 flex-shrink-0" />
                      <span>
                        以下旧目录仍存在，您可以安全删除以释放磁盘空间：
                      </span>
                    </div>
                    {migrateResult.old_dirs_remain.map((dir, i) => (
                      <p
                        key={i}
                        className="font-mono text-[9px] text-slate-400 pl-5"
                      >
                        {dir}
                      </p>
                    ))}
                    {deletedOldDirs ? (
                      <p className="text-emerald-400 text-[10px] flex items-center gap-1">
                        <CheckCircle2 className="w-3 h-3" />
                        已删除 {deletedOldDirs.length} 个旧目录
                      </p>
                    ) : (
                      <button
                        onClick={handleDeleteOldDirs}
                        disabled={deletingOldDirs}
                        className="px-3 py-1.5 bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] hover:bg-[color-mix(in_srgb,var(--module-accent)_40%,transparent)] disabled:opacity-50 text-[var(--module-accent)] rounded-lg text-[10px] font-medium cursor-pointer transition-all flex items-center gap-1.5 border border-[var(--module-accent-ring)]"
                      >
                        <Trash2 className="w-3 h-3" />
                        {deletingOldDirs ? "正在删除..." : "删除旧目录"}
                      </button>
                    )}
                  </div>
                )}
              </div>
            )}

            <div className="flex items-center justify-between pt-4 border-t border-white/5">
              <div>
                {success &&
                  !migrateResult?.moved_versions &&
                  !migrateResult?.moved_links && (
                    <span className="text-xs font-medium text-emerald-400 flex items-center gap-1.5">
                      <CheckCircle2 className="w-4 h-4" />
                      配置已保存
                    </span>
                  )}
              </div>

              <button
                onClick={handleSaveClick}
                disabled={saving || !dataDir}
                className="px-6 py-2.5 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-white rounded-xl text-xs font-semibold shadow-lg shadow-[var(--module-accent-ring)] cursor-pointer transition-all flex items-center gap-1.5"
              >
                <Save className="w-3.5 h-3.5" />
                {saving ? "正在保存..." : "保存配置"}
              </button>
            </div>
          </div>
        )}
      </div>

      {/* 版本检查与升级 */}
      <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-4">
        <div className="flex items-center justify-between pb-3 border-b border-white/5">
          <div className="flex items-center gap-2">
            <RefreshCw className="w-4 h-4 text-[var(--module-accent)]" />
            <h3 className="text-xs font-semibold text-white">版本检查与升级</h3>
          </div>
          <button
            onClick={handleCheckUpdate}
            disabled={checkingUpdate}
            className="flex items-center gap-1.5 px-3 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg text-[10px] border border-white/5 cursor-pointer"
          >
            <RefreshCw
              className={`w-3 h-3 ${checkingUpdate ? "animate-spin" : ""}`}
            />
            {checkingUpdate ? "检查中..." : "检查更新"}
          </button>
        </div>

        <div className="flex items-center gap-3 text-xs">
          <span className="text-slate-400">当前版本:</span>
          <span className="font-mono text-slate-200 bg-black/20 px-2 py-0.5 rounded">
            v{appVersion || "1.0.0"}
          </span>
        </div>

        {updateError && (
          <div className="p-3 bg-[var(--module-accent-soft)] border border-[var(--module-accent-ring)] rounded-xl text-[10px] text-[var(--module-accent)]">
            {updateError}
          </div>
        )}

        {latestVersion && (
          <div className="p-4 bg-emerald-500/5 border border-emerald-500/15 rounded-xl space-y-2">
            <div className="flex items-center gap-2">
              <span className="text-xs font-semibold text-emerald-300">
                发现新版本: v{latestVersion}
              </span>
              {updateBody && (
                <span className="text-[10px] text-slate-400">
                  ({updateBody.substring(0, 80)}...)
                </span>
              )}
            </div>
            {updateSource === "plugin" ? (
              <button
                onClick={handleInstallUpdate}
                disabled={installing}
                className="px-4 py-2 bg-emerald-600 hover:bg-emerald-500 text-white rounded-lg text-xs font-semibold cursor-pointer transition-all flex items-center gap-1.5 disabled:opacity-60"
              >
                <Loader2
                  className={`w-3 h-3 ${installing ? "animate-spin" : ""}`}
                />
                {installing ? "正在下载并安装..." : "下载并安装更新"}
              </button>
            ) : (
              <button
                onClick={handleDownloadUpdate}
                className="px-4 py-2 bg-emerald-600 hover:bg-emerald-500 text-white rounded-lg text-xs font-semibold cursor-pointer transition-all flex items-center gap-1.5"
              >
                <ExternalLink className="w-3 h-3" />
                前往下载页面
              </button>
            )}
          </div>
        )}

        {latestVersion === null && !checkingUpdate && !updateError && (
          <p className="text-[10px] text-slate-500">
            点击「检查更新」查看是否有新版本可用。
          </p>
        )}
      </div>

      {/* 应用行为 */}
      <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-4">
        <div className="flex items-center gap-2 pb-3 border-b border-white/5">
          <Power className="w-4 h-4 text-[var(--module-accent)]" />
          <h3 className="text-xs font-semibold text-white">应用行为</h3>
        </div>

        <div className="flex items-center justify-between">
          <div className="space-y-0.5">
            <p className="text-xs font-medium text-slate-200">开机自启</p>
            <p className="text-[9px] text-slate-500">
              系统启动时自动运行 AnyVersion，并静默驻留到系统托盘。
            </p>
          </div>
          <button
            onClick={handleToggleAutostart}
            disabled={autostartBusy}
            role="switch"
            aria-checked={autostartOn}
            className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors cursor-pointer disabled:opacity-50 ${
              autostartOn ? "bg-[var(--module-accent)]" : "bg-white/10"
            }`}
            title={autostartOn ? "已开启开机自启" : "已关闭开机自启"}
          >
            <span
              className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                autostartOn ? "translate-x-4" : "translate-x-0.5"
              }`}
            />
          </button>
        </div>

        {/* 托盘右键菜单 */}
        <div className="pt-3 border-t border-white/5 space-y-3">
          <div className="space-y-0.5">
            <p className="text-xs font-medium text-slate-200">托盘右键菜单</p>
            <p className="text-[9px] text-slate-500">
              选择需要在系统托盘右键菜单中显示的快捷开关。
            </p>
          </div>
          {[
            [
              "show_mihomo",
              "Mihomo 子菜单",
              "内核启停，以及下方的模式 / 订阅 / 节点切换",
            ],
            ["show_mihomo_mode", "· 模式切换（规则 / 全局 / 直连）", ""],
            ["show_mihomo_profiles", "· 订阅切换", ""],
            ["show_mihomo_proxies", "· 代理组节点切换", ""],
          ].map(([key, label, desc]) => {
            const disabled =
              key.startsWith("show_mihomo_") && !trayCfg.show_mihomo;
            return (
              <div
                key={key}
                className={`flex items-center justify-between ${disabled ? "opacity-40" : ""}`}
              >
                <div className="space-y-0.5">
                  <p className="text-[11px] text-slate-200">{label}</p>
                  {desc && <p className="text-[9px] text-slate-500">{desc}</p>}
                </div>
                <button
                  onClick={() => saveTrayCfg({ [key]: !(trayCfg as any)[key] })}
                  disabled={disabled || trayBusy}
                  role="switch"
                  aria-checked={!!(trayCfg as any)[key]}
                  className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors cursor-pointer disabled:cursor-not-allowed ${
                    (trayCfg as any)[key] ? "bg-[var(--module-accent)]" : "bg-white/10"
                  }`}
                >
                  <span
                    className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                      (trayCfg as any)[key]
                        ? "translate-x-4"
                        : "translate-x-0.5"
                    }`}
                  />
                </button>
              </div>
            );
          })}
          <div
            className={`flex items-center justify-between ${!trayCfg.show_mihomo || !trayCfg.show_mihomo_proxies ? "opacity-40" : ""}`}
          >
            <div className="space-y-0.5">
              <p className="text-[11px] text-slate-200">· 每组最多列出节点数</p>
              <p className="text-[9px] text-slate-500">
                节点过多会让托盘菜单变得很长。
              </p>
            </div>
            <input
              type="number"
              min={1}
              max={200}
              value={trayCfg.mihomo_proxy_limit}
              disabled={
                !trayCfg.show_mihomo || !trayCfg.show_mihomo_proxies || trayBusy
              }
              onChange={(e) =>
                saveTrayCfg({
                  mihomo_proxy_limit: Math.max(1, Number(e.target.value) || 1),
                })
              }
              className="w-20 glass-input px-2 py-1 text-xs text-right"
            />
          </div>
        </div>
      </div>

      {/* 服务自启管理 */}
      <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-4">
        <div className="flex items-center justify-between pb-3 border-b border-white/5">
          <div className="flex items-center gap-2">
            <Rocket className="w-4 h-4 text-[var(--module-accent)]" />
            <div>
              <h3 className="text-xs font-semibold text-white">服务自启管理</h3>
              <p className="text-[9px] text-slate-500 mt-0.5">
                在打开 AnyVersion 时自动拉起已勾选的服务（与开机自启协同，开机即可就绪）
              </p>
            </div>
          </div>
          <span className="text-[10px] text-slate-400 bg-white/5 px-2 py-0.5 rounded-full border border-white/5">
            已启用 {autoStartServices.length} 个自启服务
          </span>
        </div>

        <div className="space-y-3">
          {(() => {
            // 系统级核心常驻服务
            const builtinServices = [
              {
                id: "mihomo",
                name: "Mihomo 代理服务",
                tag: "网络代理",
                tagColor: "bg-indigo-500/10 text-indigo-400 border-indigo-500/20",
                icon: Waypoints,
                desc: "启动软件时自动运行 Mihomo 核心代理",
              },
              {
                id: "rtsp",
                name: "RTSP 媒体服务",
                tag: "流媒体",
                tagColor: "bg-purple-500/10 text-purple-400 border-purple-500/20",
                icon: Video,
                desc: "启动软件时自动按上次配置开启推流",
              },
            ];

            // 动态从 SDK 模块中筛选已托管且已安装（或已配置本地路径）的服务
            const activeSdkServices = sdkProjects
              .filter((p) => {
                const isSvc = p.category === "service" || (p as any).is_service;
                const isInstalled = (p.installed_versions && p.installed_versions.length > 0) || !!p.install_root;
                return isSvc && p.managed && isInstalled;
              })
              .map((p) => {
                let icon = Server;
                let tag = "后台服务";
                let tagColor = "bg-cyan-500/10 text-cyan-400 border-cyan-500/20";
                let desc = `自动启动本地 ${p.display_name} 服务`;

                if (["mysql", "mongodb", "postgresql"].includes(p.id)) {
                  icon = Database;
                  tag = "数据库";
                  tagColor = "bg-blue-500/10 text-blue-400 border-blue-500/20";
                  desc = `自动启动本地 ${p.display_name} 数据库服务`;
                } else if (p.id === "redis") {
                  icon = Zap;
                  tag = "中间件";
                  tagColor = "bg-rose-500/10 text-rose-400 border-rose-500/20";
                  desc = "自动启动本地 Redis 内存数据库服务";
                } else if (p.id === "nginx") {
                  icon = Globe;
                  tag = "Web 服务";
                  tagColor = "bg-green-500/10 text-green-400 border-green-500/20";
                  desc = "自动启动本地 Nginx 反向代理与 Web 服务";
                } else if (p.id === "frpc" || p.id === "frps") {
                  icon = Server;
                  tag = "内网穿透";
                  tagColor = "bg-amber-500/10 text-amber-400 border-amber-500/20";
                  desc = `自动启动 FRP ${p.id.toUpperCase()} 穿透服务`;
                }

                return {
                  id: p.id,
                  name: `${p.display_name} 服务`,
                  tag,
                  tagColor,
                  icon,
                  desc,
                };
              });

            const displayServices = [...builtinServices, ...activeSdkServices];

            return (
              <>
                <div className="grid grid-cols-1 md:grid-cols-2 gap-3 pt-1">
                  {displayServices.map(({ id, name, tag, tagColor, icon: Icon, desc }) => {
                    const isEnabled = autoStartServices.includes(id);
                    const isBusy = !!autoStartBusyMap[id];

                    return (
                      <div
                        key={id}
                        className={`p-3 rounded-xl border transition-all flex items-center justify-between gap-3 ${
                          isEnabled
                            ? "bg-white/[0.04] border-white/10 shadow-sm"
                            : "bg-black/20 border-white/5 opacity-80 hover:opacity-100"
                        }`}
                      >
                        <div className="flex items-center gap-2.5 min-w-0">
                          <div
                            className={`w-8 h-8 rounded-lg flex items-center justify-center flex-shrink-0 ${
                              isEnabled
                                ? "bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)]"
                                : "bg-white/5 text-slate-400 border border-white/5"
                            }`}
                          >
                            <Icon className="w-4 h-4" />
                          </div>
                          <div className="min-w-0">
                            <div className="flex items-center gap-1.5">
                              <span className="text-xs font-semibold text-slate-200 truncate">
                                {name}
                              </span>
                              <span
                                className={`text-[9px] px-1.5 py-0.2 rounded border font-medium ${tagColor}`}
                              >
                                {tag}
                              </span>
                            </div>
                            <p className="text-[10px] text-slate-500 truncate mt-0.5">
                              {desc}
                            </p>
                          </div>
                        </div>

                        <button
                          onClick={() => handleToggleAutoStartService(id, !isEnabled)}
                          disabled={isBusy}
                          role="switch"
                          aria-checked={isEnabled}
                          className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors cursor-pointer disabled:opacity-50 ${
                            isEnabled ? "bg-[var(--module-accent)]" : "bg-white/10"
                          }`}
                          title={isEnabled ? "已启用自启" : "已禁用自启"}
                        >
                          <span
                            className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                              isEnabled ? "translate-x-4" : "translate-x-0.5"
                            }`}
                          />
                        </button>
                      </div>
                    );
                  })}
                </div>

                {activeSdkServices.length === 0 && (
                  <p className="text-[10px] text-slate-500 italic mt-2">
                    💡 提示：在「SDK」模块中安装并托管 MySQL、Redis、MongoDB、Nginx 等服务后，将自动在此处显示自启开关。
                  </p>
                )}
              </>
            );
          })()}
        </div>
      </div>

      {/* 外观 (Appearance) */}
      <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-5">
        <div className="flex items-center justify-between pb-3 border-b border-white/5">
          <div className="flex items-center gap-2">
            <Sliders className="w-4 h-4 text-cyan-400" />
            <div>
              <h3 className="text-xs font-semibold text-white">外观 (Appearance)</h3>
              <p className="text-[9px] text-slate-500 mt-0.5">
                为每个顶级模块设置各自的主题色，并配置全局字体
              </p>
            </div>
          </div>
        </div>

        {/* 1. 模块主题色 */}
        <div>
          <p className="text-[11px] font-medium text-slate-200 mb-2">顶级模块主题色</p>
          <div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
            {Object.entries(MODULE_APPEARANCE_DEFAULTS).map(([id, cfg]) => {
              const current = appearance.moduleThemeColors[id] || cfg.color;
              return (
                <div
                  key={id}
                  className="flex items-center justify-between gap-2 px-2.5 py-2 rounded-xl bg-white/[0.03] border border-white/5"
                >
                  <div className="flex items-center gap-2 min-w-0">
                    <span
                      className="w-4 h-4 rounded-full flex-shrink-0 border border-white/20"
                      style={{ backgroundColor: current }}
                    />
                    <span className="text-[11px] text-slate-300 truncate">{cfg.label}</span>
                  </div>
                  <input
                    type="color"
                    value={current}
                    onChange={(e) => handleSetModuleColor(id, e.target.value)}
                    title={`${cfg.label} 主题色：${current}`}
                    className="w-7 h-6 rounded cursor-pointer border border-white/10 bg-transparent p-0"
                  />
                </div>
              );
            })}
          </div>
          <p className="text-[9px] text-slate-500 mt-2">
            点击色块选择颜色，即可为对应模块（启动/资讯/SDK/AI/任务/服务/代理/证书/更多/设置）设置独立主题色；选择回默认色即恢复原色。
          </p>
        </div>

        {/* 2. 全局字体 */}
        <div className="pt-3 border-t border-white/5 space-y-2.5">
          <p className="text-[11px] font-medium text-slate-200">全局字体</p>
          <div className="flex items-center gap-2">
            <select
              value={appearance.globalFont}
              onChange={(e) => handleSetGlobalFont(e.target.value)}
              className="flex-1 min-w-0 glass-input text-xs"
            >
              <option value="">默认字体 (Inter / system-ui)</option>
              <option value="Segoe UI">Segoe UI</option>
              <option value="Microsoft YaHei">微软雅黑</option>
              <option value="SimHei">黑体</option>
              <option value="KaiTi">楷体</option>
              <option value="SimSun">宋体</option>
              <option value="Arial">Arial</option>
              <option value="Georgia">Georgia</option>
              <option value="Consolas">Consolas</option>
              <option value="Courier New">Courier New</option>
              <option value="PingFang SC">苹方</option>
              <option value="Noto Sans CJK SC">Noto Sans CJK SC</option>
            </select>
            {appearance.customFontPath && (
              <button
                onClick={handleClearCustomFont}
                className="px-2.5 py-1.5 rounded-lg text-[10px] font-medium bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] border border-[color-mix(in_srgb,var(--module-accent)_40%,transparent)] text-[var(--module-accent)] hover:bg-[color-mix(in_srgb,var(--module-accent)_30%,transparent)] transition cursor-pointer whitespace-nowrap"
                title="移除自定义字体"
              >
                移除自定义字体
              </button>
            )}
            <button
              onClick={handleImportFont}
              disabled={importingFont}
              className="px-2.5 py-1.5 rounded-lg text-[10px] font-medium bg-cyan-600/20 border border-cyan-500/40 text-cyan-300 hover:bg-cyan-600/30 transition cursor-pointer whitespace-nowrap flex items-center gap-1 disabled:opacity-50"
              title="从本地导入 .ttf/.otf/.woff/.woff2 字体文件"
            >
              {importingFont ? <Loader2 className="w-3 h-3 animate-spin" /> : <Upload className="w-3 h-3" />}
              导入字体
            </button>
          </div>
          {appearance.customFontPath && (
            <p className="text-[9px] text-emerald-400">
              已使用自定义字体：{appearance.globalFont}（即时生效，重启保留）
            </p>
          )}
        </div>
      </div>

      {/* 启动器配置 (Launcher) */}
      <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-5">
        <div className="flex items-center justify-between pb-3 border-b border-white/5">
          <div className="flex items-center gap-2">
            <Rocket className="w-4 h-4 text-purple-400" />
            <div>
              <h3 className="text-xs font-semibold text-white">快捷启动设置 (Launcher)</h3>
              <p className="text-[9px] text-slate-500 mt-0.5">
                自定义全局唤起/隐藏主程序界面的快捷键，所有分类及项目外观均已自动优化
              </p>
            </div>
          </div>
          <div className="flex items-center gap-2">
            {launcherSaved && (
              <span className="text-xs font-medium text-emerald-400 flex items-center gap-1.5 animate-fadeIn">
                <CheckCircle2 className="w-3.5 h-3.5" />
                已保存
              </span>
            )}
            <button
              onClick={() => handleSaveLauncherConfig()}
              disabled={savingLauncher}
              className="px-4 py-1.5 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-white rounded-xl text-xs font-semibold shadow-md shadow-[var(--module-accent-ring)] cursor-pointer transition flex items-center gap-1.5"
            >
              <Save className="w-3 h-3" />
              {savingLauncher ? "保存中..." : "保存设置"}
            </button>
          </div>
        </div>

        {/* 1. 启动器全局呼出快捷键 */}
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <div className="space-y-0.5">
              <p className="text-xs font-medium text-slate-200 flex items-center gap-1.5">
                <Keyboard className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                唤起/隐藏主程序界面快捷键
              </p>
              <p className="text-[9px] text-slate-500">
                在 Windows 任意界面按下此快捷键，即可立即唤起或隐藏 AnyVersion 主窗口。
              </p>
            </div>
            <div className="flex items-center gap-2">
              <div
                className={`px-3 py-1.5 rounded-xl border text-xs font-mono text-center min-w-[120px] outline-none transition ${
                  recordingField === "main"
                    ? "bg-[color-mix(in_srgb,var(--module-accent)_30%,transparent)] border-[var(--module-accent)] text-[var(--module-accent)] animate-pulse"
                    : "bg-black/30 border-white/10 text-white"
                }`}
              >
                {recordingField === "main"
                  ? "请按下按键..."
                  : launcherCfg.showHideShortcutKey || "无"}
              </div>

              <button
                onClick={() =>
                  setRecordingField(
                    recordingField === "main" ? null : "main"
                  )
                }
                className={`px-3 py-1.5 rounded-xl text-xs font-medium border transition cursor-pointer ${
                  recordingField === "main"
                    ? "bg-amber-500/20 border-amber-500/40 text-amber-300"
                    : "bg-white/5 border-white/10 text-slate-300 hover:bg-white/10"
                }`}
              >
                {recordingField === "main" ? "取消录制" : "录制按键"}
              </button>

              <button
                onClick={() => handleSaveLauncherConfig({ showHideShortcutKey: "Alt+Space" })}
                className="px-2.5 py-1.5 bg-white/5 hover:bg-white/10 text-slate-400 hover:text-slate-200 rounded-xl text-xs border border-white/5 cursor-pointer"
                title="重置为 Alt+Space"
              >
                重置
              </button>
            </div>
          </div>

          <div className="flex items-center gap-2 pt-0.5">
            <span className="text-[10px] text-slate-500">或选常用预设：</span>
            {["Alt+Space", "F6", "Ctrl+Space", "Ctrl+`"].map((preset) => (
              <button
                key={preset}
                onClick={() => handleSaveLauncherConfig({ showHideShortcutKey: preset })}
                className={`px-2 py-0.5 text-[11px] rounded-lg border transition cursor-pointer font-mono ${
                  launcherCfg.showHideShortcutKey === preset
                    ? "bg-[var(--module-accent-soft)] border-[var(--module-accent)] text-[var(--module-accent)]"
                    : "bg-white/5 hover:bg-[var(--module-accent-soft)] hover:border-[var(--module-accent-ring)] text-slate-300 border-white/10"
                }`}
              >
                {preset}
              </button>
            ))}
          </div>

          {/* 各顶级模块的独立唤起快捷键 */}
          <div className="pt-3 mt-3 border-t border-white/5 space-y-1.5">
            <p className="text-[10px] text-slate-400">
              模块专属快捷键：按下后直接唤起窗口并打开对应模块（「启动」模块即上方主热键）。
            </p>
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-x-6 gap-y-1.5">
              {MODULE_ORDER.filter((id) => id !== "launcher").map((id) => {
                const label = MODULE_DEFAULTS[id]?.label || id;
                const current = launcherCfg.moduleHotkeys?.[id] || "";
                const isRecording = recordingField === id;
                return (
                  <div key={id} className="flex items-center gap-2">
                    <span className="w-12 text-[11px] text-slate-300 shrink-0">{label}</span>
                    <div
                      className={`px-2 py-1 rounded-lg border text-[11px] font-mono text-center min-w-[86px] transition ${
                        isRecording
                          ? "bg-[color-mix(in_srgb,var(--module-accent)_30%,transparent)] border-[var(--module-accent)] text-[var(--module-accent)] animate-pulse"
                          : "bg-black/30 border-white/10 text-white"
                      }`}
                    >
                      {isRecording ? "请按下按键..." : current || "未设置"}
                    </div>
                    <button
                      onClick={() => setRecordingField(isRecording ? null : id)}
                      className={`px-2 py-1 rounded-lg text-[11px] border transition cursor-pointer ${
                        isRecording
                          ? "bg-amber-500/20 border-amber-500/40 text-amber-300"
                          : "bg-white/5 border-white/10 text-slate-300 hover:bg-white/10"
                      }`}
                    >
                      {isRecording ? "取消" : "录制"}
                    </button>
                    {current && (
                      <button
                        onClick={() => {
                          const cur = { ...(launcherCfg.moduleHotkeys || {}) };
                          delete cur[id];
                          handleSaveLauncherConfig({ moduleHotkeys: cur });
                        }}
                        className="px-2 py-1 rounded-lg text-[11px] border border-white/10 text-slate-400 hover:text-rose-300 hover:border-rose-500/30 bg-white/5 cursor-pointer"
                        title="清除该模块快捷键"
                      >
                        清除
                      </button>
                    )}
                  </div>
                );
              })}
            </div>
          </div>
        </div>

        {/* 2. 数据备份与恢复 */}
        <div className="pt-3 border-t border-white/5 flex items-center justify-between">
          <div className="space-y-0.5">
            <p className="text-xs font-medium text-slate-200">启动器数据备份与恢复</p>
            <p className="text-[9px] text-slate-500">
              导出所有分类与快捷方式数据为 JSON 文件，或从备份文件（支持 DawnLauncher .db/.json）中导入恢复
            </p>
          </div>
          <div className="flex items-center gap-2">
            <button
              onClick={handleLauncherExport}
              className="px-3.5 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded-xl text-xs border border-white/10 transition cursor-pointer flex items-center gap-1.5"
            >
              <Download className="w-3.5 h-3.5 text-[var(--module-accent)]" />
              导出备份 (JSON)
            </button>
            <button
              onClick={handleLauncherImport}
              className="px-3.5 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded-xl text-xs border border-white/10 transition cursor-pointer flex items-center gap-1.5"
            >
              <Upload className="w-3.5 h-3.5 text-[var(--module-accent)]" />
              导入备份 (Dawn / AnyVersion)
            </button>
          </div>
        </div>
      </div>

      {/* AI 配置 */}
      <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-4">
        <div className="flex items-center gap-2 pb-3 border-b border-white/5">
          <FolderKanban className="w-4 h-4 text-[var(--module-accent)]" />
          <h3 className="text-xs font-semibold text-white">AI 配置</h3>
        </div>

        <div className="space-y-1.5">
          <label className="text-[10px] text-slate-500 uppercase font-semibold">
            AI 默认项目目录
          </label>
          <div className="flex items-center gap-2">
            <input
              type="text"
              value={aiDefaultPath}
              onChange={(e) => setAiDefaultPath(e.target.value)}
              className="flex-1 glass-input px-3.5 py-2.5 text-xs font-mono"
              placeholder="e.g. C:\Users\Admin\projects"
            />
            <button
              onClick={() => handleBrowseFolder(setAiDefaultPath)}
              className="p-2.5 bg-white/5 hover:bg-white/10 text-slate-400 hover:text-slate-200 rounded-lg border border-white/5 cursor-pointer transition-all flex-shrink-0"
              title="选择文件夹"
            >
              <FolderOpen className="w-4 h-4" />
            </button>
          </div>
          <p className="text-[9px] text-slate-500">
            启动 AI 工具时的默认工作目录。
          </p>
        </div>

        {/* 技能市场配置 */}
        <div className="space-y-2 pt-3 border-t border-white/5">
          <div>
            <label className="text-[10px] text-slate-300 uppercase font-semibold">
              技能市场（skills.sh）
            </label>
            <p className="text-[9px] text-slate-600 mt-0.5">
              作为 skills.sh 的 GUI，技能直接托管在公共仓库（默认
              ~/.agents/skills），无需单独配置托管目录。
            </p>
          </div>

          {/* 技能迁移进度 */}
          {skillProgress && (
            <div className="p-2.5 bg-violet-500/10 border border-violet-500/20 rounded-xl space-y-1.5 animate-fadeIn">
              <div className="flex items-center justify-between text-[9px]">
                <span className="text-violet-300 font-semibold flex items-center gap-1">
                  <RefreshCw className="w-3 h-3 animate-spin" />
                  {skillProgress.stage}
                </span>
                {skillProgress.total > 0 && (
                  <span className="text-violet-400 font-mono">
                    {skillProgress.current}/{skillProgress.total}
                  </span>
                )}
              </div>
              {skillProgress.total > 0 && (
                <div className="w-full bg-violet-500/20 rounded-full h-1 overflow-hidden">
                  <div
                    className="bg-violet-400 h-full rounded-full transition-all duration-300"
                    style={{
                      width: `${Math.round((skillProgress.current / skillProgress.total) * 100)}%`,
                    }}
                  />
                </div>
              )}
              {skillProgress.skill_name && (
                <div className="text-[8px] text-slate-400 truncate">
                  {skillProgress.skill_name}
                </div>
              )}
            </div>
          )}

          {/* 技能迁移完成 */}
          {skillMigrated && !skillProgress && (
            <div className="p-2.5 bg-emerald-500/10 border border-emerald-500/20 rounded-xl text-[10px] text-emerald-400 flex items-center gap-1.5 animate-fadeIn">
              <CheckCircle2 className="w-3.5 h-3.5 flex-shrink-0" />
              技能已迁移到新目录，工具链接已更新。
            </div>
          )}
        </div>

        <div className="flex items-center justify-between pt-4 border-t border-white/5">
          <div>
            {aiSaved && (
              <span className="text-xs font-medium text-emerald-400 flex items-center gap-1.5">
                <CheckCircle2 className="w-4 h-4" />
                已保存
              </span>
            )}
          </div>
          <button
            onClick={handleSaveAiConfig}
            disabled={savingAi || !aiConfig}
            className="px-6 py-2.5 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-white rounded-xl text-xs font-semibold shadow-lg shadow-[var(--module-accent-ring)] cursor-pointer transition-all flex items-center gap-1.5"
          >
            <Save className="w-3.5 h-3.5" />
            {savingAi ? "保存中..." : "保存"}
          </button>
        </div>
      </div>
    </div>
  );
}
