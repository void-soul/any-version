import { useState, useRef } from "react";
import { createModuleLayoutSaver } from "../utils/moduleLifecycle";
import { theamedAlert, alertError } from "./shared/ThemedAlert";
import { applyDensity } from "../utils/appearance";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { PointerSensor, useSensor, useSensors } from "@dnd-kit/core";
import { arrayMove } from "@dnd-kit/sortable";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { VEX_CYBER_ACCENT, VEX_THEME_STORE_KEY, resolveThemeAccent } from "../utils/brand";
import { MODULE_CATALOG as MODULES } from "../moduleCatalog";
import { useTranslation } from "react-i18next";

/** Owns the AppearanceSettings feature, independently of page rendering. */
export function useAppearanceSettings() {
  const { t } = useTranslation();

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
    motionPreference: string;
    density: string;
  }>({
    moduleThemeColors: {},
    globalFont: "",
    customFontPath: "",
    moduleOrder: [],
    toolbarModules: [],
    disabledModules: [],
    backgroundTexture: "",
    language: "",
    motionPreference: "",
    density: "",
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
        motionPreference: string;
        density: string;
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
  const layoutSaver = useRef<ReturnType<typeof createModuleLayoutSaver<typeof appearance>> | null>(null);
  if (!layoutSaver.current) layoutSaver.current = createModuleLayoutSaver({
    save: (toolbarModules, disabledModules) => invoke("set_module_layout", { toolbarModules, disabledModules }),
    read: () => invoke<typeof appearance>("get_appearance_config"),
    apply: setAppearance,
    notify: () => emit("appearance-updated"),
    report: (error) => { void alertError(`保存模块布局失败：${String(error)}`); },
  });
  const saveModuleLayout = layoutSaver.current;

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

  // 动效偏好：「跟随系统」（尊重系统的「减少动效」）/「始终开启」（无视系统设置）。
  // 系统被切成「最佳性能」时 prefers-reduced-motion 会变 reduce，头像辉光闪烁等
  // 自家动效会全部停掉 —— 这里给用户一个不听系统的出口。
  const handleSetMotionPreference = async (pref: string) => {
    setAppearance({ ...appearance, motionPreference: pref });
    try {
      await invoke("set_motion_preference", { preference: pref });
      emit("appearance-updated");
    } catch (e) {
      console.error("保存动效偏好失败", e);
    }
  };

  // 列表密度：紧凑（行更高，一屏更多）/ 舒适（默认）。
  // 后端只存 "compact" 一个开关值，具体像素由 App.css 的 --density-py 决定
  // —— 密度是纯视觉手感，不该让后端知道 8px / 4px 这种数。
  const handleSetDensity = async (value: string) => {
    setAppearance({ ...appearance, density: value });
    try {
      // 写入口与命令面板共用（utils/appearance.ts），避免两处各写一份 invoke + 广播
      await applyDensity(value);
    } catch (e) {
      console.error("保存列表密度失败", e);
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
      theamedAlert(t("settings.fontImported", { family: res.family }));
    } catch (e: any) {
      alertError(t("settings.importFontFail", { err: String(e) }));
    } finally {
      setImportingFont(false);
    }
  };

  const handleClearCustomFont = async () => {
    try {
      await invoke("clear_custom_font");
      setAppearance({ ...appearance, globalFont: "", customFontPath: "" });
      emit("appearance-updated");
      theamedAlert(t("settings.customFontRemoved"));
    } catch (e: any) {
      alertError(t("settings.removeFontFail", { err: String(e) }));
    }
  };
  return {
    appearance,
    importingFont,
    systemFonts,
    fontSearch,
    setFontSearch,
    fontRefreshing,
    themeAccent,
    moduleOrderIds,
    MODULE_MAP_LOCAL,
    isModuleInToolbar,
    fetchAppearance,
    fetchSystemFonts,
    refreshSystemFonts,
    filteredFonts,
    toggleModuleEnabled,
    toggleModuleToolbar,
    handleModuleOrderDragEnd,
    handleResetModuleOrder,
    orderSensors,
    handleSetGlobalFont,
    handleSetBackgroundTexture,
    handleSetMotionPreference,
    handleSetDensity,
    handleSetLanguage,
    handleSetThemeAccent,
    handleResetThemeAccent,
    handleImportFont,
    handleClearCustomFont,
  };
}
