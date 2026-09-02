import { useState, useEffect, useRef } from "react";
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

  Sliders,

  Upload,


  GripVertical,
  RotateCcw,
  Search,
  X,
  Languages,
  Brain,
  StickyNote,
} from "lucide-react";
import { DndContext, PointerSensor, closestCenter, useSensor, useSensors } from "@dnd-kit/core";
import { SortableContext, arrayMove, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  VEX_CYBER_ACCENT,
  VEX_THEME_PRESETS,
  VEX_THEME_STORE_KEY,
  resolveThemeAccent,
} from "../utils/brand";

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
  onToggleToolbar: () => void;
  onToggleEnabled: () => void;
  onRecordHotkey: () => void;
  onClearHotkey: () => void;
}) {
  const { t } = useTranslation();
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
        title={t("settings.dragSort")}
      >
        <GripVertical className="w-3.5 h-3.5" />
      </button>

      {/* 图标 + 标签 */}
      <Icon className="w-3.5 h-3.5 flex-shrink-0" style={{ color }} />
      <span className="text-[11px] text-slate-300 w-14 truncate flex-shrink-0">{label}</span>

      {/* 位置：顶栏/更多 */}
      {pinned ? (
        <span className="text-[9px] text-slate-600 w-9 text-center flex-shrink-0">{t("settings.pinned")}</span>
      ) : (
        <button
          onClick={onToggleToolbar}
          disabled={disabled}
          className={`px-1.5 py-0.5 rounded-md text-[9px] font-medium border transition cursor-pointer disabled:opacity-40 w-9 flex-shrink-0 ${
            inToolbar
              ? "bg-cyan-500/15 border-cyan-500/40 text-cyan-300"
              : "bg-white/5 border-white/10 text-slate-400"
          }`}
          title={inToolbar ? t("settings.toolbarTipIn") : t("settings.toolbarTipOut")}
        >
          {inToolbar ? t("settings.toolbar") : t("settings.more")}
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
          title={disabled ? t("settings.clickEnable") : t("settings.clickDisable")}
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
          title={isRecording ? t("settings.recHint") : hotkey ? t("settings.clickReRecord") : t("settings.clickRecordHotkey")}
        >
          {isRecording ? t("settings.pressKeys") : hotkey || t("settings.no")}
        </button>
        {hotkey && !isRecording && (
          <button
            onClick={onClearHotkey}
            className="text-slate-500 hover:text-rose-300 p-0.5 cursor-pointer"
            title={t("settings.clearShortcut")}
          >
            <X className="w-3 h-3" />
          </button>
        )}
      </div>
    </div>
  );
}
import type { LauncherSetting } from "./launcher/types";
import { MODULES, moduleLabel } from "../moduleRegistry";
import { useTranslation } from "react-i18next";
import DataSyncPanel from "./DataSyncPanel";
import VexAvatar from "./VexAvatar";
import VexGreeting from "./VexGreeting";

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
  const { t } = useTranslation();
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
      alert(t("settings.autostartFail", { err: String(e) }));
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
      alert(t("settings.saveFail", { err: String(e) }));
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
      alert(t("settings.setAutostartFail", { err: String(e) }));
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
    mindmapQuickHotkey: "Shift+F3",
    mindmapStickerHotkey: "Shift+F4",
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
  // 是否正在录制「思维导图速记」热键（独立字段，与模块热键分离）
  const recordingMindmapQuick = recordingField === "mindmap-quick";
  // 是否正在录制「思维导图贴纸」热键
  const recordingMindmapSticker = recordingField === "mindmap-sticker";
  // 始终持有最新 launcherCfg，供录制监听闭包（仅依赖 recordingField）安全读取
  const launcherCfgRef = useRef<LauncherSetting>(launcherCfg);
  useEffect(() => {
    launcherCfgRef.current = launcherCfg;
  }, [launcherCfg]);
  const [_savingLauncher, setSavingLauncher] = useState(false);
  const [_launcherSaved, setLauncherSaved] = useState(false);

  // ---- 外观：模块主题色 + 全局字体 + 模块顺序 + 模块布局 ----
  const [appearance, setAppearance] = useState<{
    moduleThemeColors: Record<string, string>;
    globalFont: string;
    customFontPath: string;
    moduleOrder: string[];
    toolbarModules: string[];
    disabledModules: string[];
    backgroundTexture: string;
    language: string;
  }>({
    moduleThemeColors: {},
    globalFont: "",
    customFontPath: "",
    moduleOrder: [],
    toolbarModules: [],
    disabledModules: [],
    backgroundTexture: "",
    language: "",
  });
  const [importingFont, setImportingFont] = useState(false);
  const [systemFonts, setSystemFonts] = useState<string[]>([]);
  const [fontSearch, setFontSearch] = useState("");
  const [fontRefreshing, setFontRefreshing] = useState(false);

  // 当前生效的全局主题色（用户可在下方「外观」里覆盖默认签名色）。
  const themeAccent = resolveThemeAccent(appearance.moduleThemeColors);

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
        backgroundTexture: string;
        language: string;
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

  const handleSetGlobalFont = async (font: string) => {
    setAppearance({ ...appearance, globalFont: font });
    try {
      await invoke("set_global_font", { font });
      emit("appearance-updated");
    } catch (e) {
      console.error("保存全局字体失败", e);
    }
  };

  const handleSetBackgroundTexture = async (texture: string) => {
    setAppearance({ ...appearance, backgroundTexture: texture });
    try {
      await invoke("set_background_texture", { texture });
      emit("appearance-updated");
    } catch (e) {
      console.error("保存背景纹理失败", e);
    }
  };

  // 切换界面语言：持久化到后端，并即时切换 i18n。
  const handleSetLanguage = async (lang: string) => {
    setAppearance({ ...appearance, language: lang });
    try {
      await invoke("set_language", { language: lang });
    } catch (e) {
      console.error("保存语言设置失败", e);
    }
    try {
      const { default: i18n } = await import("i18next");
      await i18n.changeLanguage(lang || undefined);
    } catch (e) {
      console.error("切换语言失败", e);
    }
  };

  const handleSetThemeAccent = async (color: string) => {
    setAppearance({
      ...appearance,
      moduleThemeColors: { ...appearance.moduleThemeColors, [VEX_THEME_STORE_KEY]: color },
    });
    try {
      await invoke("set_module_theme_color", { moduleId: VEX_THEME_STORE_KEY, color });
      emit("appearance-updated");
    } catch (e) {
      console.error("保存主题色失败", e);
    }
  };

  const handleResetThemeAccent = async () => {
    setAppearance({
      ...appearance,
      moduleThemeColors: {
        ...appearance.moduleThemeColors,
        [VEX_THEME_STORE_KEY]: VEX_CYBER_ACCENT,
      },
    });
    try {
      await invoke("set_module_theme_color", { moduleId: VEX_THEME_STORE_KEY, color: "" });
      emit("appearance-updated");
    } catch (e) {
      console.error("重置主题色失败", e);
    }
  };

  const handleImportFont = async () => {
    try {
      const selected = await openDialog({
        title: t("settings.pickFontFile"),
        multiple: false,
        filters: [
          { name: t("settings.fontFilter"), extensions: ["ttf", "otf", "woff", "woff2"] },
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
      alert(t("settings.fontImported", { family: res.family }));
    } catch (e: any) {
      alert(t("settings.importFontFail", { err: String(e) }));
    } finally {
      setImportingFont(false);
    }
  };

  const handleClearCustomFont = async () => {
    try {
      await invoke("clear_custom_font");
      setAppearance({ ...appearance, globalFont: "", customFontPath: "" });
      emit("appearance-updated");
      alert(t("settings.customFontRemoved"));
    } catch (e: any) {
      alert(t("settings.removeFontFail", { err: String(e) }));
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
      alert(t("settings.launcherSaveFail", { err: String(e) }));
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
      // 「划词翻译」/「思维导图速记」热键是独立字段，与模块热键分离。
      if (target === "selection-translate") {
        handleSaveLauncherConfig({ selectionTranslateHotkey: hotkeyStr });
      } else if (target === "mindmap-quick") {
        handleSaveLauncherConfig({ mindmapQuickHotkey: hotkeyStr });
      } else if (target === "mindmap-sticker") {
        handleSaveLauncherConfig({ mindmapStickerHotkey: hotkeyStr });
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
      .catch((e) => console.error("加载托盘菜单配置失败:", e));
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
      alert(t("settings.configSaveFail", { err: String(e) }));
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
        t("settings.confirmDeleteOldDirs", { dirs: migrateResult.old_dirs_remain.join("\n") }),
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
      alert(t("settings.deleteFail", { err: String(e) }));
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
        alert(t("settings.alreadyLatest"));
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
      if (!resp.ok) throw new Error(t("settings.checkFail", { status: resp.status }));
      const data = await resp.json();
      const tag = data.tag_name?.replace(/^v/, "") ?? "";
      const currentVer = appVersion || "1.0.0";
      if (tag && tag !== currentVer) {
        setLatestVersion(tag);
        setUpdateBody(data.body ?? null);
        setUpdateSource("github");
      } else {
        setUpdateError(null);
        alert(t("settings.alreadyLatest"));
      }
    } catch (e: any) {
      setUpdateError(e.message || t("settings.updateCheckFail"));
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
      setUpdateError(e.message || t("settings.updateInstallFail"));
      setInstalling(false);
    }
  };

  const handleBrowseFolder = async (setter: (v: string) => void) => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, title: t("settings.chooseFolder") });
      if (selected) setter(selected as string);
    } catch {
      alert(t("settings.folderPickerUnavailable"));
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
            {t("settings.dataDirSection")}
          </h3>
        </div>


        {loading ? (
          <div className="text-xs text-slate-400 py-6 flex items-center gap-2">
            <RefreshCw className="w-4 h-4 animate-spin text-[var(--module-accent)]" />
            {t("settings.loadingConfig")}
          </div>
        ) : (
          <div className="space-y-4">
            <div className="space-y-1.5">
              <label className="text-[10px] text-slate-500 uppercase font-semibold">
                {t("settings.dataDirLabel")}
              </label>
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  value={dataDir}
                  onChange={(e) => setDataDir(e.target.value)}
                  className="flex-1 glass-input px-3.5 py-2.5 text-xs font-mono"
                  placeholder="e.g. D:\Kira"
                />
                <button
                  onClick={() => handleBrowseFolder(setDataDir)}
                  className="p-2.5 bg-white/5 hover:bg-white/10 text-slate-400 hover:text-slate-200 rounded-lg border border-white/5 cursor-pointer transition-all flex-shrink-0"
                  title={t("settings.chooseFolder")}
                >
                  <FolderOpen className="w-4 h-4" />
                </button>
              </div>
            </div>

            {/* 派生路径只读展示 */}
            <div className="rounded-xl border border-white/5 bg-white/[0.02] p-3 space-y-1.5">
              <p className="text-[10px] text-slate-500 uppercase font-semibold">
                {t("settings.derivedDirs")}
              </p>
              <div className="text-[10px] font-mono text-slate-400 space-y-1">
                <div className="flex items-center gap-2">
                  <span className="text-cyan-400 flex-shrink-0">{t("settings.sdkDir")}</span>
                  <span className="truncate">
                    {(dataDir || "…").replace(/[\\/]+$/, "")}\sdk
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-emerald-400 flex-shrink-0">
                    {t("settings.nodeServices")}
                  </span>
                  <span className="truncate">
                    {(dataDir || "…").replace(/[\\/]+$/, "")}\node-projects
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-amber-400 flex-shrink-0">{t("settings.certs")}</span>
                  <span className="truncate">
                    {(dataDir || "…").replace(/[\\/]+$/, "")}\certs
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-slate-400 flex-shrink-0">
                    {t("settings.cacheDb")}
                  </span>
                  <span className="truncate">
                    {(dataDir || "…").replace(/[\\/]+$/, "")}
                    {t("settings.derivedTail")}
                  </span>
                </div>
              </div>
            </div>

            {/* 路径变更确认弹窗 */}
            {showMigrateConfirm && (
              <div className="p-4 bg-amber-500/10 border border-amber-500/20 rounded-xl space-y-3 animate-fadeIn">
                <h4 className="text-xs font-semibold text-amber-400 flex items-center gap-1.5">
                  <AlertTriangle className="w-4 h-4" />
                  {t("settings.confirmMigrate")}
                </h4>
                <div className="text-[10px] text-slate-300 space-y-1.5">
                  <p>{t("settings.migrateNotice")}</p>
                  <p className="text-amber-300">{t("settings.migrateStep1")}</p>
                  <p className="text-amber-300">{t("settings.migrateStep2")}</p>
                  <p className="text-amber-300">{t("settings.migrateStep3")}</p>
                  <p className="text-slate-400 mt-1">{t("settings.migrateHint")}</p>
                </div>
                <div className="flex items-center gap-2 pt-1">
                  <button
                    onClick={handleSave}
                    disabled={saving}
                    className="px-4 py-2 bg-amber-600 hover:bg-amber-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold cursor-pointer transition-all flex items-center gap-1.5"
                  >
                    <Save className="w-3 h-3" />
                    {saving ? t("settings.migrating") : t("settings.confirmMigrateSave")}
                  </button>
                  <button
                    onClick={() => setShowMigrateConfirm(false)}
                    className="px-4 py-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-xl text-xs font-medium cursor-pointer border border-white/10"
                  >
                    {t("common.cancel")}
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
                  {t("settings.migrateDone")}
                </h4>
                {migrateResult.moved_versions && (
                  <p className="text-slate-300">{t("settings.migMovedVersions")}</p>
                )}
                {migrateResult.moved_links && (
                  <p className="text-slate-300">{t("settings.migMovedLinks")}</p>
                )}
                {migrateResult.recreated_junctions.length > 0 && (
                  <p className="text-slate-300">
                    {t("settings.migRecreatedJunctions", { count: migrateResult.recreated_junctions.length })}{" "}
                    {migrateResult.recreated_junctions.join(", ")}
                  </p>
                )}
                {migrateResult.updated_env_vars.length > 0 && (
                  <p className="text-slate-300">
                    {t("settings.migUpdatedEnv")}{" "}
                    {migrateResult.updated_env_vars.join(", ")}
                  </p>
                )}
                {migrateResult.updated_path_entries.length > 0 && (
                  <p className="text-slate-300">
                    {t("settings.migUpdatedPath", { count: migrateResult.updated_path_entries.length })}
                  </p>
                )}

                {/* 旧目录清理提示 */}
                {migrateResult.old_dirs_remain.length > 0 && (
                  <div className="pt-2 mt-2 border-t border-amber-500/15 space-y-2">
                    <div className="flex items-start gap-1.5 text-amber-300">
                      <AlertTriangle className="w-3 h-3 mt-0.5 flex-shrink-0" />
                      <span>
                        {t("settings.oldDirsRemain")}
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
                        {t("settings.deletedOldDirs", { count: deletedOldDirs.length })}
                      </p>
                    ) : (
                      <button
                        onClick={handleDeleteOldDirs}
                        disabled={deletingOldDirs}
                        className="px-3 py-1.5 bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] hover:bg-[color-mix(in_srgb,var(--module-accent)_40%,transparent)] disabled:opacity-50 text-[var(--module-accent)] rounded-lg text-[10px] font-medium cursor-pointer transition-all flex items-center gap-1.5 border border-[var(--module-accent-ring)]"
                      >
                        <Trash2 className="w-3 h-3" />
                        {deletingOldDirs ? t("settings.deletingOld") : t("settings.deleteOldDirs")}
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
                      {t("settings.configSaved")}
                    </span>
                  )}
              </div>

              <button
                onClick={handleSaveClick}
                disabled={saving || !dataDir}
                className="px-6 py-2.5 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-white rounded-xl text-xs font-semibold shadow-lg shadow-[var(--module-accent-ring)] cursor-pointer transition-all flex items-center gap-1.5"
              >
                <Save className="w-3.5 h-3.5" />
                {saving ? t("settings.saving") : t("settings.saveConfig")}
              </button>
            </div>
          </div>
        )}
      </div>

      {/* 关于 Kira：名片 */}
      <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-3">
        <div className="flex items-center gap-4">
          <div className="relative flex-shrink-0">
            <VexAvatar size={56} />
            <span className="absolute -inset-1 rounded-full blur-md opacity-40 bg-[var(--module-accent)]" />
          </div>
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <span className="text-base font-black tracking-wide text-white">Kira</span>
              <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)]">v{appVersion || "1.0.0"}</span>
            </div>
            <p className="text-[11px] text-slate-400 mt-0.5">{t("settings.companionTagline")}</p>
            <p className="text-[11px] text-slate-300 mt-1 truncate">
              <VexGreeting />
            </p>
          </div>
        </div>
      </div>

      {/* 版本检查与升级 */}
      <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-4">
        <div className="flex items-center justify-between pb-3 border-b border-white/5">
          <div className="flex items-center gap-2">
            <RefreshCw className="w-4 h-4 text-[var(--module-accent)]" />
            <h3 className="text-xs font-semibold text-white">{t("settings.update")}</h3>
          </div>
          <button
            onClick={handleCheckUpdate}
            disabled={checkingUpdate}
            className="flex items-center gap-1.5 px-3 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg text-[10px] border border-white/5 cursor-pointer"
          >
            <RefreshCw
              className={`w-3 h-3 ${checkingUpdate ? "animate-spin" : ""}`}
            />
            {checkingUpdate ? t("settings.checkingUpdate") : t("settings.checkUpdate")}
          </button>
        </div>

        <div className="flex items-center gap-3 text-xs">
          <span className="text-slate-400">{t("settings.currentVersion")}</span>
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
                {t("settings.newVersionFound", { version: latestVersion })}
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
                {installing ? t("settings.installingUpdate") : t("settings.downloadInstall")}
              </button>
            ) : (
              <button
                onClick={handleDownloadUpdate}
                className="px-4 py-2 bg-emerald-600 hover:bg-emerald-500 text-white rounded-lg text-xs font-semibold cursor-pointer transition-all flex items-center gap-1.5"
              >
                <ExternalLink className="w-3 h-3" />
                {t("settings.gotoDownloadPage")}
              </button>
            )}
          </div>
        )}

        {latestVersion === null && !checkingUpdate && !updateError && (
          <p className="text-[10px] text-slate-500">
            {t("settings.updateHint")}
          </p>
        )}
      </div>

      {/* 应用行为 */}
      <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-4">
        <div className="flex items-center gap-2 pb-3 border-b border-white/5">
          <Power className="w-4 h-4 text-[var(--module-accent)]" />
          <h3 className="text-xs font-semibold text-white">{t("settings.behavior")}</h3>
        </div>

        <div className="flex items-center justify-between">
          <div className="space-y-0.5">
            <p className="text-xs font-medium text-slate-200">{t("settings.autostart")}</p>
            <p className="text-[9px] text-slate-500">
              {t("settings.autostartHint")}
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
            title={autostartOn ? t("settings.autostartOn") : t("settings.autostartOff")}
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
            <p className="text-xs font-medium text-slate-200">{t("settings.trayMenu")}</p>
            <p className="text-[9px] text-slate-500">
              {t("settings.trayHint")}
            </p>
          </div>
          {[
            [
              "show_mihomo",
              "settings.trayMihomoSub",
              "settings.trayMihomoDesc",
            ],
            ["show_mihomo_mode", "settings.trayModeSwitch", ""],
            ["show_mihomo_profiles", "settings.traySubSwitch", ""],
            ["show_mihomo_proxies", "settings.trayProxySwitch", ""],
          ].map(([key, label, desc]) => {
            const disabled =
              key.startsWith("show_mihomo_") && !trayCfg.show_mihomo;
            return (
              <div
                key={key}
                className={`flex items-center justify-between ${disabled ? "opacity-40" : ""}`}
              >
                <div className="space-y-0.5">
                  <p className="text-[11px] text-slate-200">{t(label)}</p>
                  {desc && <p className="text-[9px] text-slate-500">{t(desc)}</p>}
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
              <p className="text-[11px] text-slate-200">{t("settings.trayMaxNodes")}</p>
              <p className="text-[9px] text-slate-500">
                {t("settings.trayMaxNodesHint")}
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
              <h3 className="text-xs font-semibold text-white">{t("settings.servicesAutostart")}</h3>
              <p className="text-[9px] text-slate-500 mt-0.5">
                {t("settings.servicesAutostartHint")}
              </p>
            </div>
          </div>
          <span className="text-[10px] text-slate-400 bg-white/5 px-2 py-0.5 rounded-full border border-white/5">
            {t("settings.autostartCount", { count: autoStartServices.length })}
          </span>
        </div>

        <div className="space-y-3">
          {(() => {
            // 系统级核心常驻服务
            const builtinServices = [
              {
                id: "mihomo",
                name: t("settings.svcMihomo"),
                tag: t("settings.svcMihomoTag"),
                tagColor: "bg-indigo-500/10 text-indigo-400 border-indigo-500/20",
                icon: Waypoints,
                desc: t("settings.svcMihomoDesc"),
              },
              {
                id: "rtsp",
                name: t("settings.svcRtsp"),
                tag: t("settings.svcRtspTag"),
                tagColor: "bg-purple-500/10 text-purple-400 border-purple-500/20",
                icon: Video,
                desc: t("settings.svcRtspDesc"),
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
                let tag = t("settings.svcTagBackend");
                let tagColor = "bg-cyan-500/10 text-cyan-400 border-cyan-500/20";
                let desc = t("settings.svcDescAuto", { name: p.display_name });

                if (["mysql", "mongodb", "postgresql"].includes(p.id)) {
                  icon = Database;
                  tag = t("settings.svcTagDb");
                  tagColor = "bg-blue-500/10 text-blue-400 border-blue-500/20";
                  desc = t("settings.svcDescDb", { name: p.display_name });
                } else if (p.id === "redis") {
                  icon = Zap;
                  tag = t("settings.svcTagMiddleware");
                  tagColor = "bg-rose-500/10 text-rose-400 border-rose-500/20";
                  desc = t("settings.svcDescRedis");
                } else if (p.id === "nginx") {
                  icon = Globe;
                  tag = t("settings.svcTagWeb");
                  tagColor = "bg-green-500/10 text-green-400 border-green-500/20";
                  desc = t("settings.svcDescNginx");
                } else if (p.id === "frpc" || p.id === "frps") {
                  icon = Server;
                  tag = t("settings.svcTagTunnel");
                  tagColor = "bg-amber-500/10 text-amber-400 border-amber-500/20";
                  desc = t("settings.svcDescFrp", { name: p.id.toUpperCase() });
                }

                return {
                  id: p.id,
                  name: t("settings.svcName", { name: p.display_name }),
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
                          title={isEnabled ? t("settings.svcAutostartOn") : t("settings.svcAutostartOff")}
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
                    {t("settings.svcEmptyTip")}
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
              <h3 className="text-xs font-semibold text-white">{t("settings.appearance")}</h3>
              <p className="text-[9px] text-slate-500 mt-0.5">
                {t("settings.appearanceHint")}
              </p>
            </div>
          </div>
        </div>

        {/* 0. 主题色 */}
        <div className="space-y-2.5">
          <div className="flex items-center justify-between pt-3 border-t border-white/5">
            <p className="text-[11px] font-medium text-slate-200">{t("settings.themeColor")}</p>
            <div className="flex items-center gap-2">
              <div
                className="flex items-center gap-1.5 rounded-md border border-white/10 bg-black/30 px-2 py-1 font-mono text-[10px] text-slate-200"
                style={{ boxShadow: `0 0 8px ${themeAccent}55` }}
              >
                <span className="h-3 w-3 rounded-full" style={{ background: themeAccent }} />
                <span>{themeAccent}</span>
              </div>
              <button
                onClick={() => void handleResetThemeAccent()}
                className="rounded-md border border-white/10 px-2 py-1 text-[10px] text-slate-400 hover:text-white hover:bg-white/5 transition cursor-pointer"
                title={t("settings.resetTheme")}
              >
                {t("settings.resetThemeBtn")}
              </button>
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            {VEX_THEME_PRESETS.map((c) => (
              <button
                key={c}
                onClick={() => void handleSetThemeAccent(c)}
                title={c}
                className={`h-8 w-8 rounded-lg border transition cursor-pointer ${
                  c === themeAccent ? "border-white ring-2 ring-white/40" : "border-white/15 hover:border-white/50"
                }`}
                style={{ background: c }}
              />
            ))}
            <label
              className="relative h-8 w-8 rounded-lg border border-white/15 hover:border-white/50 cursor-pointer flex items-center justify-center overflow-hidden transition"
              title={t("settings.customColor")}
            >
              <span className="h-full w-full" style={{ background: "conic-gradient(red, yellow, lime, cyan, blue, magenta, red)" }} />
              <input
                type="color"
                value={themeAccent}
                onChange={(e) => void handleSetThemeAccent(e.target.value)}
                className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
              />
            </label>
          </div>
          <p className="text-[9px] text-slate-600">{t("settings.themePresetsHint")}</p>
        </div>

        {/* 1. 全局字体 */}
        <div className="pt-3 border-t border-white/5 space-y-2.5">
          <p className="text-[11px] font-medium text-slate-200">{t("settings.globalFont")}</p>
          <div className="flex items-center gap-2">
            <div className="relative flex-1 min-w-0">
              <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3 h-3 text-slate-500 pointer-events-none" />
              <input
                type="text"
                value={fontSearch}
                onChange={(e) => setFontSearch(e.target.value)}
                placeholder={t("settings.searchFontsPh", { count: systemFonts.length })}
                className="w-full glass-input pl-7 pr-2.5 py-1.5 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none focus:border-sky-400/50"
              />
            </div>
            <button
              onClick={refreshSystemFonts}
              disabled={fontRefreshing}
              className="p-1.5 rounded-lg text-[10px] text-slate-400 hover:text-slate-200 bg-white/5 hover:bg-white/10 border border-white/10 transition cursor-pointer flex-shrink-0 disabled:opacity-50"
              title={t("settings.refreshFontsTitle")}
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
              <option value="">{t("settings.defaultFont")}</option>
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
                title={t("settings.removeCustomFont")}
              >
                {t("settings.removeCustomFont")}
              </button>
            )}
            <button
              onClick={handleImportFont}
              disabled={importingFont}
              className="px-2.5 py-1.5 rounded-lg text-[10px] font-medium bg-cyan-600/20 border border-cyan-500/40 text-cyan-300 hover:bg-cyan-600/30 transition cursor-pointer whitespace-nowrap flex items-center gap-1 disabled:opacity-50"
              title={t("settings.importFontTitle")}
            >
              {importingFont ? <Loader2 className="w-3 h-3 animate-spin" /> : <Upload className="w-3 h-3" />}
              {t("settings.importFont")}
            </button>
          </div>
          {appearance.customFontPath && (
            <p className="text-[9px] text-emerald-400">
              {t("settings.usingCustomFont", { font: appearance.globalFont })}
            </p>
          )}
        </div>

        {/* 1.5 全局背景底图纹理 */}
        <div className="pt-3 border-t border-white/5 space-y-2.5">
          <p className="text-[11px] font-medium text-slate-200">{t("settings.backgroundTexture")}</p>
          <p className="text-[9px] text-slate-500">{t("settings.backgroundTextureHint")}</p>
          <div className="grid grid-cols-5 gap-1.5">
            {[
              { value: "", label: "settings.textureGrid", cls: "cyber-grid" },
              { value: "dots", label: "settings.textureDots", cls: "app-bg-dots" },
              { value: "scanline", label: "settings.textureScanline", cls: "app-bg-scanline" },
              { value: "aurora", label: "settings.textureAurora", cls: "app-bg-aurora" },
              { value: "solid", label: "settings.textureSolid", cls: "app-bg-solid" },
              { value: "diagonal", label: "settings.textureDiagonal", cls: "app-bg-diagonal" },
              { value: "cross", label: "settings.textureCross", cls: "app-bg-cross" },
              { value: "paper", label: "settings.texturePaper", cls: "app-bg-paper" },
              { value: "hex", label: "settings.textureHex", cls: "app-bg-hex" },
              { value: "circuit", label: "settings.textureCircuit", cls: "app-bg-circuit" },
              { value: "waves", label: "settings.textureWaves", cls: "app-bg-waves" },
              { value: "noise", label: "settings.textureNoise", cls: "app-bg-noise" },
              { value: "vignette", label: "settings.textureVignette", cls: "app-bg-vignette" },
              { value: "neonline", label: "settings.textureNeonline", cls: "app-bg-neonline" },
            ].map((tc) => {
              const active = (appearance.backgroundTexture || "") === tc.value;
              return (
                <button
                  key={tc.value}
                  onClick={() => handleSetBackgroundTexture(tc.value)}
                  className={`flex flex-col items-center gap-1 rounded-lg border p-1.5 transition cursor-pointer ${
                    active
                      ? "border-[var(--module-accent)] bg-[color-mix(in_srgb,var(--module-accent)_18%,transparent)]"
                      : "border-white/10 bg-white/[0.03] hover:border-white/25"
                  }`}
                  title={t(tc.label)}
                >
                  <span className={`h-7 w-full rounded ${tc.cls} bg-black/40`} />
                  <span className={`text-[9px] ${active ? "text-[var(--module-accent)]" : "text-slate-400"}`}>
                    {t(tc.label)}
                  </span>
                </button>
              );
            })}
          </div>
        </div>

        {/* 1.6 界面语言 */}
        <div className="pt-3 border-t border-white/5 space-y-2.5">
          <p className="text-[11px] font-medium text-slate-200">{t("settings.language")}</p>
          <p className="text-[9px] text-slate-500">{t("settings.languageHint")}</p>
          <div className="flex items-center gap-2">
            {[
              { value: "zh", label: t("settings.languageZh") },
              { value: "en", label: t("settings.languageEn") },
            ].map((opt) => {
              const active = (appearance.language || "") === opt.value;
              return (
                <button
                  key={opt.value}
                  onClick={() => handleSetLanguage(opt.value)}
                  className={`px-4 py-1.5 rounded-lg text-[11px] font-semibold transition cursor-pointer ${
                    active
                      ? "bg-[var(--module-accent)] text-white"
                      : "bg-white/5 text-slate-300 hover:bg-white/10 border border-white/10"
                  }`}
                >
                  {opt.label}
                </button>
              );
            })}
          </div>
        </div>

        {/* 2. 模块管理：统一配置（主题色 + 位置 + 启用 + 快捷键 + 拖拽排序） */}
        <div className="pt-3 border-t border-white/5 space-y-2.5">
          <div className="space-y-0.5">
            <p className="text-[11px] font-medium text-slate-200">{t("settings.modules")}</p>
            <p className="text-[9px] text-slate-500">
              {t("settings.modulesHint")}
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
                      label={moduleLabel(m.id)}
                      icon={m.icon}
                      color={m.color}
                      disabled={disabled}
                      inToolbar={inToolbar}
                      pinned={pinned}
                      hotkey={hotkey}
                      isRecording={isRecording}
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
                <p className="text-[11px] font-medium text-slate-200">{t("settings.translateHotkey")}</p>
                <p className="text-[9px] text-slate-500 truncate">
                  {t("settings.translateHotkeyHint")}
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
                  title={t("settings.clearTranslateHotkey")}
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
                  ? t("settings.pressKeys")
                  : launcherCfg.selectionTranslateHotkey
                    ? launcherCfg.selectionTranslateHotkey
                    : t("settings.clickToRecord")}
              </button>
            </div>
          </div>
          {/* 独立「思维导图节点速记」热键：呼出节点悬浮窗 */}
          <div className="mt-2 flex items-center justify-between gap-2 px-3 py-2 rounded-lg border border-white/5 bg-white/[0.02]">
            <div className="flex items-center gap-2 min-w-0">
              <div className="w-7 h-7 rounded-lg bg-cyan-500/15 border border-cyan-500/30 flex items-center justify-center shrink-0">
                <Brain className="w-3.5 h-3.5 text-cyan-400" />
              </div>
              <div className="min-w-0">
                <p className="text-[11px] font-medium text-slate-200">{t("settings.mindmapNodeHotkey")}</p>
                <p className="text-[9px] text-slate-500 truncate">
                  {t("settings.mindmapNodeHotkeyHint")}
                </p>
              </div>
            </div>
            <div className="flex items-center gap-1.5 shrink-0">
              {recordingMindmapQuick && (
                <button
                  onClick={() =>
                    handleSaveLauncherConfig({ mindmapQuickHotkey: "" })
                  }
                  className="p-1.5 rounded-md text-slate-500 hover:text-red-400 hover:bg-red-500/10 cursor-pointer"
                  title={t("settings.clearQuickHotkey")}
                >
                  <X className="w-3 h-3" />
                </button>
              )}
              <button
                onClick={() => setRecordingField(recordingMindmapQuick ? null : "mindmap-quick")}
                className={`min-w-[86px] px-2.5 py-1 rounded-md border text-[11px] text-center transition cursor-pointer ${
                  recordingMindmapQuick
                    ? "border-emerald-500/50 bg-emerald-500/15 text-emerald-300"
                    : "border-white/10 bg-white/5 text-slate-300 hover:bg-white/10 hover:text-white"
                }`}
              >
                {recordingMindmapQuick
                  ? t("settings.pressKeys")
                  : launcherCfg.mindmapQuickHotkey
                    ? launcherCfg.mindmapQuickHotkey
                    : t("settings.clickToRecord")}
              </button>
            </div>
          </div>
          {/* 独立「思维导图贴纸」热键：呼出贴纸悬浮窗（必须先选目标文档） */}
          <div className="mt-2 flex items-center justify-between gap-2 px-3 py-2 rounded-lg border border-white/5 bg-white/[0.02]">
            <div className="flex items-center gap-2 min-w-0">
              <div className="w-7 h-7 rounded-lg bg-amber-500/15 border border-amber-500/30 flex items-center justify-center shrink-0">
                <StickyNote className="w-3.5 h-3.5 text-amber-400" />
              </div>
              <div className="min-w-0">
                <p className="text-[11px] font-medium text-slate-200">{t("settings.mindmapStickerHotkey")}</p>
                <p className="text-[9px] text-slate-500 truncate">
                  {t("settings.stickerHotkeyHint")}
                </p>
              </div>
            </div>
            <div className="flex items-center gap-1.5 shrink-0">
              {recordingMindmapSticker && (
                <button
                  onClick={() =>
                    handleSaveLauncherConfig({ mindmapStickerHotkey: "" })
                  }
                  className="p-1.5 rounded-md text-slate-500 hover:text-red-400 hover:bg-red-500/10 cursor-pointer"
                  title={t("settings.clearStickerHotkey")}
                >
                  <X className="w-3 h-3" />
                </button>
              )}
              <button
                onClick={() => setRecordingField(recordingMindmapSticker ? null : "mindmap-sticker")}
                className={`min-w-[86px] px-2.5 py-1 rounded-md border text-[11px] text-center transition cursor-pointer ${
                  recordingMindmapSticker
                    ? "border-emerald-500/50 bg-emerald-500/15 text-emerald-300"
                    : "border-white/10 bg-white/5 text-slate-300 hover:bg-white/10 hover:text-white"
                }`}
              >
                {recordingMindmapSticker
                  ? t("settings.pressKeys")
                  : launcherCfg.mindmapStickerHotkey
                    ? launcherCfg.mindmapStickerHotkey
                    : t("settings.clickToRecord")}
              </button>
            </div>
          </div>
          <div className="flex items-center justify-between">
            <button
              onClick={handleResetModuleOrder}
              className="text-[10px] text-slate-500 hover:text-slate-300 transition cursor-pointer flex items-center gap-1"
            >
              <RotateCcw className="w-3 h-3" /> {t("settings.resetOrder")}
            </button>
            <span className="text-[9px] text-slate-600">{t("settings.dragToOrder")}</span>
          </div>
        </div>
      </div>

      {/* 数据备份与同步（统一快照，原「数据同步」模块迁入设置） */}
      <DataSyncPanel />

      {/* AI 配置 */}
      <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-4">
        <div className="flex items-center gap-2 pb-3 border-b border-white/5">
          <FolderKanban className="w-4 h-4 text-[var(--module-accent)]" />
          <h3 className="text-xs font-semibold text-white">{t("settings.aiConfig")}</h3>
        </div>

        <div className="space-y-1.5">
          <label className="text-[10px] text-slate-500 uppercase font-semibold">
            {t("settings.aiDefaultDir")}
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
              title={t("settings.chooseFolder")}
            >
              <FolderOpen className="w-4 h-4" />
            </button>
          </div>
          <p className="text-[9px] text-slate-500">
            {t("settings.aiDefaultDirHint")}
          </p>
        </div>

        {/* 翻译默认模型 */}
        <div className="space-y-2 pt-3 border-t border-white/5">
          <div>
            <label className="text-[10px] text-slate-300 uppercase font-semibold flex items-center gap-1">
              <Languages className="w-3 h-3 text-emerald-400" />
              {t("settings.translateDefaultModel")}
            </label>
            <p className="text-[9px] text-slate-600 mt-0.5">
              {t("settings.translateModelHint")}
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
                <option value="">{t("settings.noProvider")}</option>
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
              {t("settings.skillMarket")}
            </label>
            <p className="text-[9px] text-slate-600 mt-0.5">
              {t("settings.skillMarketHint")}
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
              {t("settings.skillMigrated")}
            </div>
          )}
        </div>

        <div className="flex items-center justify-between pt-4 border-t border-white/5">
          <div>
            {aiSaved && (
              <span className="text-xs font-medium text-emerald-400 flex items-center gap-1.5">
                <CheckCircle2 className="w-4 h-4" />
                {t("settings.saved")}
              </span>
            )}
          </div>
          <button
            onClick={handleSaveAiConfig}
            disabled={savingAi || !aiConfig}
            className="px-6 py-2.5 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-white rounded-xl text-xs font-semibold shadow-lg shadow-[var(--module-accent-ring)] cursor-pointer transition-all flex items-center gap-1.5"
          >
            <Save className="w-3.5 h-3.5" />
            {savingAi ? t("settings.saving") : t("settings.save")}
          </button>
        </div>
      </div>
    </div>
  );
}
