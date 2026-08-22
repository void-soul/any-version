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
  LayoutGrid,
  Sliders,
  Shield,
  Download,
  Upload,
  Layers,
  Folder,
  GripVertical,
  RotateCcw,
  Search,
  X,
  Languages,
} from "lucide-react";
import { DndContext, PointerSensor, closestCenter, useSensor, useSensors } from "@dnd-kit/core";
import { SortableContext, arrayMove, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";

// 可拖拽的模块配置行：主题色 + 位置 + 启用 + 快捷键（拖拽手柄独立，避免与控件冲突）
function ModuleConfigRow({
  id,
  label,
  icon: Icon,
  color,
  disabled,
  inToolbar,
  pinned,
  hotkey,
  isRecording,
  onColorChange,
  onToggleToolbar,
  onToggleEnabled,
  onRecordHotkey,
  onClearHotkey,
}: {
  id: string;
  label: string;
  icon: any;
  color: string;
  disabled: boolean;
  inToolbar: boolean;
  pinned: boolean;
  hotkey: string;
  isRecording: boolean;
  onColorChange: (color: string) => void;
  onToggleToolbar: () => void;
  onToggleEnabled: () => void;
  onRecordHotkey: () => void;
  onClearHotkey: () => void;
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging, setActivatorNodeRef } =
    useSortable({ id });
  return (
    <div
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={`flex items-center gap-2 px-2.5 py-2 rounded-xl bg-white/[0.03] border border-white/5 ${
        isDragging ? "opacity-60 ring-1 ring-[var(--module-accent-ring)] z-10" : ""
      } ${disabled ? "opacity-50" : ""}`}
      {...attributes}
    >
      {/* 拖拽手柄 */}
      <button
        ref={setActivatorNodeRef}
        {...listeners}
        className="cursor-grab active:cursor-grabbing text-slate-500 hover:text-slate-300 p-0.5 touch-none flex-shrink-0"
        title="拖动排序"
      >
        <GripVertical className="w-3.5 h-3.5" />
      </button>

      {/* 图标 + 标签 */}
      <Icon className="w-3.5 h-3.5 flex-shrink-0" style={{ color }} />
      <span className="text-[11px] text-slate-300 w-14 truncate flex-shrink-0">{label}</span>

      {/* 主题色 */}
      <input
        type="color"
        value={color}
        onChange={(e) => onColorChange(e.target.value)}
        title="模块主题色"
        className="w-6 h-5 rounded cursor-pointer border border-white/10 bg-transparent p-0 flex-shrink-0"
      />

      {/* 位置：顶栏/更多 */}
      {pinned ? (
        <span className="text-[9px] text-slate-600 w-9 text-center flex-shrink-0">固定</span>
      ) : (
        <button
          onClick={onToggleToolbar}
          disabled={disabled}
          className={`px-1.5 py-0.5 rounded-md text-[9px] font-medium border transition cursor-pointer disabled:opacity-40 w-9 flex-shrink-0 ${
            inToolbar
              ? "bg-cyan-500/15 border-cyan-500/40 text-cyan-300"
              : "bg-white/5 border-white/10 text-slate-400"
          }`}
          title={inToolbar ? "在顶栏，点击收进更多" : "在更多，点击移到顶栏"}
        >
          {inToolbar ? "顶栏" : "更多"}
        </button>
      )}

      {/* 启用开关 */}
      {pinned ? (
        <span className="w-8 flex-shrink-0" />
      ) : (
        <button
          onClick={onToggleEnabled}
          className={`relative w-8 rounded-full transition cursor-pointer flex-shrink-0 ${
            disabled ? "bg-white/10" : "bg-emerald-500/60"
          }`}
          style={{ height: 18 }}
          title={disabled ? "点击启用" : "点击禁用"}
        >
          <span
            className="absolute top-[2px] w-[14px] h-[14px] rounded-full bg-white transition-all"
            style={{ left: disabled ? 2 : 16 }}
          />
        </button>
      )}

      {/* 快捷键 */}
      <div className="flex items-center gap-1 flex-shrink-0">
        <button
          onClick={onRecordHotkey}
          className={`px-2 py-0.5 rounded-md text-[9px] font-mono border transition cursor-pointer ${
            isRecording
              ? "bg-amber-500/20 border-amber-500/40 text-amber-300"
              : hotkey
                ? "bg-white/5 border-white/10 text-slate-300 hover:bg-white/10"
                : "bg-white/[0.02] border-dashed border-white/15 text-slate-500 hover:text-slate-300"
          }`}
          title={isRecording ? "按下按键完成录制，Esc 取消" : hotkey ? "点击重新录制" : "点击录制快捷键"}
        >
          {isRecording ? "请按键…" : hotkey || "无"}
        </button>
        {hotkey && !isRecording && (
          <button
            onClick={onClearHotkey}
            className="text-slate-500 hover:text-rose-300 p-0.5 cursor-pointer"
            title="清除快捷键"
          >
            <X className="w-3 h-3" />
          </button>
        )}
      </div>
    </div>
  );
}
import type { LauncherSetting } from "./launcher/types";
import { MODULES } from "../moduleRegistry";
import DataSyncPanel from "./DataSyncPanel";

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
  // 划词翻译默认模型（provider + model，持久化到 translate_config.json）
  const [translateProvId, setTranslateProvId] = useState("");
  const [translateModelId, setTranslateModelId] = useState("");
  const [skillProgress, setSkillProgress] =
    useState<SkillMigrateProgress | null>(null);
  const [skillMigrated, setSkillMigrated] = useState(false);
  // 开机自启：反映操作系统真实注册状态（打开设置页时查询）
  // 应用通过 UAC manifest 始终以管理员身份运行，故开机自启天然具备管理员权限。
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
      const [cfg, tCfg] = await Promise.all([
        invoke<AiConfig>("get_ai_config"),
        invoke<{ providerId: string | null; modelId: string | null; targetLang: string | null }>(
          "get_translate_config",
        ),
      ]);
      setAiConfig(cfg);
      setAiDefaultPath(cfg.default_project_path || "");
      // 划词翻译默认模型：优先使用已保存的，否则默认第一个可用 provider + 其模型
      const defProv = tCfg.providerId && cfg.providers.some((p) => p.id === tCfg.providerId)
        ? tCfg.providerId!
        : (cfg.providers.find((p) => !p.openai_url)?.id || cfg.providers[0]?.id || "");
      setTranslateProvId(defProv);
      const selP = cfg.providers.find((p) => p.id === defProv);
      const defModel = tCfg.modelId && selP?.models.some((m) => m.id === tCfg.modelId)
        ? tCfg.modelId!
        : (selP?.models[0]?.id || "");
      setTranslateModelId(defModel);
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

  // 保存划词翻译默认模型
  const saveTranslateConfig = async (provId: string, modelId: string) => {
    try {
      await invoke("save_translate_config", {
        config: { providerId: provId, modelId: modelId, targetLang: null },
      });
    } catch (e: any) {
      console.error("保存翻译默认模型失败:", e);
    }
  };

  // 切换翻译默认供应商，联动模型
  const changeTranslateProvider = (pid: string) => {
    setTranslateProvId(pid);
    const p = aiConfig?.providers.find((x) => x.id === pid);
    const m = p?.models[0]?.id || "";
    setTranslateModelId(m);
    saveTranslateConfig(pid, m);
  };

  const changeTranslateModel = (mid: string) => {
    setTranslateModelId(mid);
    saveTranslateConfig(translateProvId, mid);
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
    moduleHotkeys: { launcher: "Alt+Space" },
    selectionTranslateHotkey: "F6",
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
  // 录制目标：null=未录制；否则为某个顶级模块的 moduleId（含「启动」= "launcher"）
  const [recordingField, setRecordingField] = useState<string | null>(null);
  // 是否正在录制「划词翻译」热键（独立字段，与模块热键分离）
  const recordingSelTrans = recordingField === "selection-translate";
  // 始终持有最新 launcherCfg，供录制监听闭包（仅依赖 recordingField）安全读取
  const launcherCfgRef = useRef<LauncherSetting>(launcherCfg);
  useEffect(() => {
    launcherCfgRef.current = launcherCfg;
  }, [launcherCfg]);
  const [savingLauncher, setSavingLauncher] = useState(false);
  const [launcherSaved, setLauncherSaved] = useState(false);

  // ---- 外观：模块主题色 + 全局字体 + 模块顺序 + 模块布局 ----
  const [appearance, setAppearance] = useState<{
    moduleThemeColors: Record<string, string>;
    globalFont: string;
    customFontPath: string;
    moduleOrder: string[];
    toolbarModules: string[];
    disabledModules: string[];
  }>({
    moduleThemeColors: {},
    globalFont: "",
    customFontPath: "",
    moduleOrder: [],
    toolbarModules: [],
    disabledModules: [],
  });
  const [importingFont, setImportingFont] = useState(false);
  const [systemFonts, setSystemFonts] = useState<string[]>([]);
  const [fontSearch, setFontSearch] = useState("");
  const [fontRefreshing, setFontRefreshing] = useState(false);

  // 模块默认外观：从统一注册表派生（含全部平级模块，含「更多」里的子工具）。
  const MODULE_APPEARANCE_DEFAULTS: Record<string, { label: string; color: string }> =
    Object.fromEntries(MODULES.map((m) => [m.id, { label: m.label, color: m.color }]));

  // 模块顺序：优先用户自定义顺序，缺失的模块（新增/子工具）追加到末尾，保证完整。
  const moduleOrderIds = (() => {
    const custom = appearance.moduleOrder;
    const all = MODULES.map((m) => m.id);
    if (custom.length === 0) return all;
    // 已有序的在前，未出现过的模块按默认顺序追加
    const ordered = custom.filter((id) => all.includes(id));
    const missing = all.filter((id) => !ordered.includes(id));
    return [...ordered, ...missing];
  })();

  // 统一模块映射（从注册表派生），用于模块管理区。
  const MODULE_MAP_LOCAL: Record<string, { id: string; label: string; color: string; icon: any; pinned?: boolean; defaultToolbar?: boolean }> =
    Object.fromEntries(MODULES.map((m) => [m.id, { id: m.id, label: m.label, color: m.color, icon: m.icon, pinned: m.pinned, defaultToolbar: m.defaultToolbar }]));

  // 判断模块当前是否「在顶栏」：优先用户显式布局，为空时回退默认顶栏。
  const isModuleInToolbar = (id: string): boolean => {
    if (MODULE_MAP_LOCAL[id]?.pinned) return true; // 设置模块始终顶栏
    if (appearance.toolbarModules.length > 0) {
      return appearance.toolbarModules.includes(id);
    }
    return !!MODULE_MAP_LOCAL[id]?.defaultToolbar;
  };

  const fetchAppearance = async () => {
    try {
      const ap = await invoke<{
        moduleThemeColors: Record<string, string>;
        globalFont: string;
        customFontPath: string;
        moduleOrder: string[];
        toolbarModules: string[];
        disabledModules: string[];
      }>("get_appearance_config");
      setAppearance(ap);
    } catch (e) {
      console.error("读取外观配置失败", e);
    }
  };

  const fetchSystemFonts = async () => {
    try {
      const fonts = await invoke<string[]>("list_system_fonts");
      setSystemFonts(fonts || []);
    } catch (e) {
      console.error("读取系统字体失败", e);
    }
  };

  const refreshSystemFonts = async () => {
    setFontRefreshing(true);
    try {
      await fetchSystemFonts();
    } finally {
      setFontRefreshing(false);
    }
  };

  // 按关键字过滤系统字体（大小写不敏感）
  const filteredFonts = (() => {
    const kw = fontSearch.trim().toLowerCase();
    if (!kw) return systemFonts;
    return systemFonts.filter((f) => f.toLowerCase().includes(kw));
  })();

  const saveModuleOrder = async (order: string[]) => {
    setAppearance({ ...appearance, moduleOrder: order });
    try {
      await invoke("set_module_order", { order });
      emit("appearance-updated");
    } catch (e) {
      console.error("保存模块顺序失败", e);
    }
  };

  // 保存模块布局（顶栏模块 + 禁用模块）
  const saveModuleLayout = async (toolbar: string[], disabled: string[]) => {
    setAppearance({ ...appearance, toolbarModules: toolbar, disabledModules: disabled });
    try {
      await invoke("set_module_layout", {
        toolbarModules: toolbar,
        disabledModules: disabled,
      });
      emit("appearance-updated");
    } catch (e) {
      console.error("保存模块布局失败", e);
    }
  };

  // 物化「当前实际顶栏模块列表」：若用户尚未显式保存布局，则以默认顶栏列表为准。
  // 避免在空列表上增删导致「只保留一个模块、其余全部落到更多」。
  const materializedToolbar = (): string[] => {
    if (appearance.toolbarModules.length > 0) return [...appearance.toolbarModules];
    return MODULES.filter((m) => m.defaultToolbar).map((m) => m.id);
  };

  // 切换模块启用/禁用
  const toggleModuleEnabled = (id: string) => {
    if (MODULE_MAP_LOCAL[id]?.pinned) return; // 设置模块不可禁用
    const disabled = appearance.disabledModules.includes(id)
      ? appearance.disabledModules.filter((x) => x !== id)
      : [...appearance.disabledModules, id];
    saveModuleLayout(materializedToolbar(), disabled);
  };

  // 切换模块在顶栏 / 更多
  const toggleModuleToolbar = (id: string) => {
    if (MODULE_MAP_LOCAL[id]?.pinned) return; // 设置模块始终顶栏
    const base = materializedToolbar();
    const toolbar = base.includes(id) ? base.filter((x) => x !== id) : [...base, id];
    saveModuleLayout(toolbar, appearance.disabledModules);
  };

  const handleModuleOrderDragEnd = (event: { active: any; over: any }) => {
    const { active, over } = event;
    if (over && active.id !== over.id) {
      const oldIndex = moduleOrderIds.indexOf(active.id);
      const newIndex = moduleOrderIds.indexOf(over.id);
      if (oldIndex >= 0 && newIndex >= 0) {
        saveModuleOrder(arrayMove(moduleOrderIds, oldIndex, newIndex));
      }
    }
  };

  const handleResetModuleOrder = () => {
    saveModuleOrder([]);
  };

  const orderSensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 4 } }));

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
      setAppearance({ ...appearance, globalFont: res.family, customFontPath: res.path });
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
    const target = recordingField; // 本次录制目标：模块 id（含「启动」= "launcher"）

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

      // 统一写入对应模块的快捷键（所有模块平等，含「启动」= "launcher"）；
      // 「划词翻译」热键是独立字段，与翻译模块热键分离。
      if (target === "selection-translate") {
        handleSaveLauncherConfig({ selectionTranslateHotkey: hotkeyStr });
      } else {
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
    fetchSystemFonts();
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
              AnyVersion 始终以管理员身份运行，开机自启同样具备完整管理员能力。
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

        {/* 1. 全局字体 */}
        <div className="pt-3 border-t border-white/5 space-y-2.5">
          <p className="text-[11px] font-medium text-slate-200">全局字体</p>
          <div className="flex items-center gap-2">
            <div className="relative flex-1 min-w-0">
              <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3 h-3 text-slate-500 pointer-events-none" />
              <input
                type="text"
                value={fontSearch}
                onChange={(e) => setFontSearch(e.target.value)}
                placeholder={`搜索字体（共 ${systemFonts.length} 个）`}
                className="w-full glass-input pl-7 pr-2.5 py-1.5 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none focus:border-sky-400/50"
              />
            </div>
            <button
              onClick={refreshSystemFonts}
              disabled={fontRefreshing}
              className="p-1.5 rounded-lg text-[10px] text-slate-400 hover:text-slate-200 bg-white/5 hover:bg-white/10 border border-white/10 transition cursor-pointer flex-shrink-0 disabled:opacity-50"
              title="刷新系统字体列表"
            >
              <RefreshCw className={`w-3.5 h-3.5 ${fontRefreshing ? "animate-spin" : ""}`} />
            </button>
          </div>
          <div className="flex items-center gap-2">
            <select
              value={appearance.globalFont}
              onChange={(e) => handleSetGlobalFont(e.target.value)}
              className="flex-1 min-w-0 glass-input text-xs"
            >
              <option value="">默认字体 (Inter / system-ui)</option>
              {filteredFonts.map((f) => (
                <option key={f} value={f}>
                  {f}
                </option>
              ))}
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

        {/* 2. 模块管理：统一配置（主题色 + 位置 + 启用 + 快捷键 + 拖拽排序） */}
        <div className="pt-3 border-t border-white/5 space-y-2.5">
          <div className="space-y-0.5">
            <p className="text-[11px] font-medium text-slate-200">模块管理</p>
            <p className="text-[9px] text-slate-500">
              所有模块地位平等。拖动排序、设置主题色、切换顶栏/更多、启用/禁用、录制快捷键，一站式配置。
            </p>
          </div>
          <DndContext sensors={orderSensors} collisionDetection={closestCenter} onDragEnd={handleModuleOrderDragEnd}>
            <SortableContext items={moduleOrderIds} strategy={verticalListSortingStrategy}>
              <div className="space-y-1.5">
                {moduleOrderIds.map((id) => {
                  const m = MODULE_MAP_LOCAL[id];
                  if (!m) return null;
                  const disabled = appearance.disabledModules.includes(id);
                  const inToolbar = isModuleInToolbar(id);
                  const pinned = !!m.pinned;
                  const hotkey = launcherCfg.moduleHotkeys?.[id] || "";
                  const isRecording = recordingField === id;
                  return (
                    <ModuleConfigRow
                      key={id}
                      id={id}
                      label={m.label}
                      icon={m.icon}
                      color={appearance.moduleThemeColors[id] || m.color}
                      disabled={disabled}
                      inToolbar={inToolbar}
                      pinned={pinned}
                      hotkey={hotkey}
                      isRecording={isRecording}
                      onColorChange={(c) => handleSetModuleColor(id, c)}
                      onToggleToolbar={() => toggleModuleToolbar(id)}
                      onToggleEnabled={() => toggleModuleEnabled(id)}
                      onRecordHotkey={() => setRecordingField(isRecording ? null : id)}
                      onClearHotkey={() => {
                        const cur = { ...(launcherCfg.moduleHotkeys || {}) };
                        delete cur[id];
                        handleSaveLauncherConfig({ moduleHotkeys: cur });
                      }}
                    />
                  );
                })}
              </div>
            </SortableContext>
          </DndContext>
          {/* 独立「翻译」热键：与翻译模块热键分离（模块热键=唤起面板看历史） */}
          <div className="mt-2 flex items-center justify-between gap-2 px-3 py-2 rounded-lg border border-white/5 bg-white/[0.02]">
            <div className="flex items-center gap-2 min-w-0">
              <div className="w-7 h-7 rounded-lg bg-emerald-500/15 border border-emerald-500/30 flex items-center justify-center shrink-0">
                <Languages className="w-3.5 h-3.5 text-emerald-400" />
              </div>
              <div className="min-w-0">
                <p className="text-[11px] font-medium text-slate-200">翻译热键</p>
                <p className="text-[9px] text-slate-500 truncate">
                  在任意程序选中文本后按下，直接悬浮翻译；与「翻译」模块热键（唤起面板看历史）相互独立
                </p>
              </div>
            </div>
            <div className="flex items-center gap-1.5 shrink-0">
              {launcherCfg.selectionTranslateHotkey && !recordingSelTrans && (
                <button
                  onClick={() =>
                    handleSaveLauncherConfig({ selectionTranslateHotkey: "" })
                  }
                  className="p-1.5 rounded-md text-slate-500 hover:text-red-400 hover:bg-red-500/10 cursor-pointer"
                  title="清除翻译热键"
                >
                  <X className="w-3 h-3" />
                </button>
              )}
              <button
                onClick={() => setRecordingField(recordingSelTrans ? null : "selection-translate")}
                className={`min-w-[86px] px-2.5 py-1 rounded-md border text-[11px] text-center transition cursor-pointer ${
                  recordingSelTrans
                    ? "border-emerald-500/50 bg-emerald-500/15 text-emerald-300"
                    : "border-white/10 bg-white/5 text-slate-300 hover:bg-white/10 hover:text-white"
                }`}
              >
                {recordingSelTrans
                  ? "请按键…"
                  : launcherCfg.selectionTranslateHotkey
                    ? launcherCfg.selectionTranslateHotkey
                    : "点击录制"}
              </button>
            </div>
          </div>
          <div className="flex items-center justify-between">
            <button
              onClick={handleResetModuleOrder}
              className="text-[10px] text-slate-500 hover:text-slate-300 transition cursor-pointer flex items-center gap-1"
            >
              <RotateCcw className="w-3 h-3" /> 恢复默认顺序
            </button>
            <span className="text-[9px] text-slate-600">拖动手柄排序，配置即时生效</span>
          </div>
        </div>
      </div>

      {/* 数据备份与同步（统一快照，原「数据同步」模块迁入设置） */}
      <DataSyncPanel />

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

        {/* 模块快捷键已整合到「外观 → 模块管理」区，此处仅保留启动器数据备份 */}
        {/* 1. 数据备份与恢复 */}
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

        {/* 翻译默认模型 */}
        <div className="space-y-2 pt-3 border-t border-white/5">
          <div>
            <label className="text-[10px] text-slate-300 uppercase font-semibold flex items-center gap-1">
              <Languages className="w-3 h-3 text-emerald-400" />
              翻译默认模型
            </label>
            <p className="text-[9px] text-slate-600 mt-0.5">
              全局热键翻译使用的模型（在任意程序选中文本后按翻译热键时生效）。
            </p>
          </div>
          <div className="grid grid-cols-2 gap-2">
            <select
              value={translateProvId}
              onChange={(e) => changeTranslateProvider(e.target.value)}
              disabled={!aiConfig || aiConfig.providers.length === 0}
              className="glass-input px-3 py-2 text-xs disabled:opacity-50"
            >
              {(!aiConfig || aiConfig.providers.length === 0) && (
                <option value="">（无已配置供应商）</option>
              )}
              {aiConfig?.providers.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
            <select
              value={translateModelId}
              onChange={(e) => changeTranslateModel(e.target.value)}
              disabled={!aiConfig || !translateProvId}
              className="glass-input px-3 py-2 text-xs disabled:opacity-50"
            >
              {(aiConfig?.providers.find((p) => p.id === translateProvId)?.models || []).map(
                (m) => (
                  <option key={m.id} value={m.id}>
                    {m.name || m.id}
                  </option>
                ),
              )}
            </select>
          </div>
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
