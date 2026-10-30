import { BackgroundTextureSettings } from "./BackgroundTextureSettings";
import { ApplicationFontSelector } from "./ApplicationFontSelector";
import { ThemeAccentSettings } from "./ThemeAccentSettings";
import { ApplicationStartupSettings } from "./ApplicationStartupSettings";
import { ApplicationUpdateReady } from "./ApplicationUpdateReady";
import { StorageMigrationResult } from "./StorageMigrationResult";
import { StorageMigrationProgress } from "./StorageMigrationProgress";
import { StorageMigrationConfirm } from "./StorageMigrationConfirm";
import { StoragePathPreview } from "./StoragePathPreview";
import { useAppearanceSettings } from "./useAppearanceSettings";
import { useState, useEffect, useRef } from "react";
import { theamedAlert, alertError } from "./shared/ThemedAlert";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { listen } from "@tauri-apps/api/event";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { enable as enableAutostart, disable as disableAutostart, isEnabled as isAutostartEnabled } from "@tauri-apps/plugin-autostart";
import { FolderKanban, Save, RefreshCw, CheckCircle2, FolderOpen, Power, Sliders, RotateCcw, Search } from "lucide-react";
import { DndContext, closestCenter } from "@dnd-kit/core";
import { SortableContext, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { ConfirmDialogHost } from "./shared/ConfirmDialog";
import type { ConfirmRequest } from "./shared/ConfirmDialog";
import type { AppSettings } from "../contracts/appSettings";
import type { AppSettingsPatch } from "../contracts/appSettings";
import { loadAppSettings, patchAppSettings } from "../services/appSettings";
import { moduleLabel } from "../moduleCatalog";
import { useTranslation } from "react-i18next";
import DataSyncPanel from "./DataSyncPanel";
import VexGlowAvatar from "./VexGlowAvatar";
import VexGreeting from "./VexGreeting";
import { type Config, type MigrateProgress, type MigrateResult, type TrayMenuConfig } from "./GlobalSettings.model";
import { ModuleConfigRow } from "./GlobalSettings.ModuleConfigRow";

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
  // 数据目录真的换了：进程内缓存（数据库连接等）还指向旧目录，必须重启才彻底生效
  const [needRestart, setNeedRestart] = useState(false);
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
  // 统一确认弹窗请求（替代原生 window.confirm）
  const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null);
  // 开机自启：反映操作系统真实注册状态（打开设置页时查询）
  // 应用通过 UAC manifest 始终以管理员身份运行，故开机自启天然具备管理员权限。
  const [autostartOn, setAutostartOn] = useState(false);
  const [autostartBusy, setAutostartBusy] = useState(false);

  // 托盘右键菜单配置
  const [trayCfg, setTrayCfg] = useState<TrayMenuConfig>({
    enabled: true,
    show_mihomo: true,
  });

  const saveTrayCfg = async (patch: Partial<TrayMenuConfig>) => {
    const next = { ...trayCfg, ...patch };
    setTrayCfg(next);
    try {
      await invoke("set_tray_menu_config", { value: next });
    } catch (e) {
      console.error(e);
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
      alertError(t("settings.setAutostartFail", { err: String(e) }));
      // 失败后以系统真实状态为准
      await fetchAutostart();
    } finally {
      setAutostartBusy(false);
    }
  };

  // 启动器配置 (全局唤起快捷键 + 视图设置)
  const [launcherCfg, setLauncherCfg] = useState<AppSettings>({
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
  // 始终持有最新 launcherCfg，供录制监听闭包（仅依赖 recordingField）安全读取
  const launcherCfgRef = useRef<AppSettings>(launcherCfg);
  useEffect(() => {
    launcherCfgRef.current = launcherCfg;
  }, [launcherCfg]);
  const [_savingLauncher, setSavingLauncher] = useState(false);
  const [_launcherSaved, setLauncherSaved] = useState(false);

  // ---- 外观：模块主题色 + 全局字体 + 模块顺序 + 模块布局 ----
  const {
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
  } = useAppearanceSettings();

  const fetchLauncherConfig = async () => {
    try {
      const cfg = await loadAppSettings();
      if (cfg) setLauncherCfg(cfg);
    } catch (e) {
      console.error(e);
    }
  };

  const handleSaveLauncherConfig = async (patch: AppSettingsPatch) => {
    setSavingLauncher(true);
    setLauncherSaved(false);
    try {
      const saved = await patchAppSettings(patch);
      setLauncherCfg(saved);
      launcherCfgRef.current = saved;
      setLauncherSaved(true);
      setTimeout(() => setLauncherSaved(false), 2500);
    } catch (e: any) {
      alertError(t("settings.launcherSaveFail", { err: String(e) }));
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
        handleSaveLauncherConfig({ moduleHotkeys: { [target]: hotkeyStr } });
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
    fetchAutostart();
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

    const migrated = pathsChanged();
    try {
      const result = await invoke<MigrateResult>("update_config", {
        dataDir: dataDir || oldDataDir,
      });
      setMigrateResult(migrated ? null : result);
      setSuccess(true);
      // Scheduling does not change the live storage root. Keep the requested
      // destination visible until the next process applies the migration.
      if (migrated) {
        setNeedRestart(true);
      } else {
        await fetchConfig();
      }
    } catch (e: any) {
      alertError(t("settings.configSaveFail", { err: String(e) }));
    } finally {
      unlisten();
      setProgress(null);
      setSaving(false);
    }
  };

  /** 重启应用：新数据目录要重新读配置才真正生效（含运行组件是否齐全的检测） */
  const restartApp = async () => {
    try {
      await relaunch();
    } catch (e) {
      alertError(t("settings.restartManual", { err: String(e) }));
    }
  };

  /** 真正删除迁移后残留的旧目录（由确认弹窗确认后调用） */
  const doDeleteOldDirs = async (dirs: string[]) => {
    setDeletingOldDirs(true);
    try {
      const deleted = await invoke<string[]>("delete_old_storage_dirs", { dirs });
      setDeletedOldDirs(deleted);
      // 清除残留目录列表
      setMigrateResult((prev) => (prev ? { ...prev, old_dirs_remain: [] } : prev));
    } catch (e: any) {
      alertError(t("settings.deleteFail", { err: String(e) }));
    } finally {
      setDeletingOldDirs(false);
    }
  };

  const handleDeleteOldDirs = async () => {
    const remain = migrateResult?.old_dirs_remain ?? [];
    if (!remain.length) return;
    setConfirmRequest({
      title: t("settings.deleteOldDirsTitle"),
      danger: true,
      width: 480,
      confirmText: t("settings.deleteOldDirsOk"),
      desc: (
        <div className="space-y-2">
          <p className="whitespace-pre-line">
            {t("settings.confirmDeleteOldDirs", { dirs: remain.join("\n") })}
          </p>
        </div>
      ),
      onConfirm: () => {
        void doDeleteOldDirs(remain);
      },
    });
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
        theamedAlert(t("settings.alreadyLatest"));
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
        theamedAlert(t("settings.alreadyLatest"));
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
      theamedAlert(t("settings.folderPickerUnavailable"));
    }
  };

  const handleDownloadUpdate = () => {
    void openUrl("https://github.com/void-soul/any-version/releases/latest");
  };

  return (
    <div className="flex-1 p-8 space-y-6 select-none max-w-3xl mx-auto">
      {/* Header */}

      <div className="glass-panel rounded-panel p-6 border border-white/5 space-y-6">
        <div className="flex items-center gap-2 pb-3 border-b border-white/5">
          <FolderKanban className="w-4 h-4 text-[var(--module-accent)]" />
          <h3 className="text-body font-semibold text-white">
            {t("settings.dataDirSection")}
          </h3>
        </div>

        {loading ? (
          <div className="text-body text-slate-400 py-6 flex items-center gap-2">
            <RefreshCw className="w-4 h-4 animate-spin text-[var(--module-accent)]" />
            {t("settings.loadingConfig")}
          </div>
        ) : (
          <div className="space-y-4">
            <div className="space-y-1.5">
              <label className="text-tiny text-slate-500 uppercase font-semibold">
                {t("settings.dataDirLabel")}
              </label>
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  value={dataDir}
                  onChange={(e) => setDataDir(e.target.value)}
                  className="flex-1 glass-input px-3.5 py-2.5 text-body font-mono"
                  placeholder="e.g. D:\Kira"
                />
                <button
                  onClick={() => handleBrowseFolder(setDataDir)}
                  className="p-2.5 bg-white/5 hover:bg-white/10 text-slate-400 hover:text-slate-200 rounded-ctl border border-white/5 cursor-pointer transition-all flex-shrink-0"
                  title={t("settings.chooseFolder")}
                >
                  <FolderOpen className="w-4 h-4" />
                </button>
              </div>
            </div>

            {/* 派生路径只读展示 */}
            <StoragePathPreview
              dataDir={dataDir}
            />

            {/* 路径变更确认弹窗 */}
            {showMigrateConfirm && (
              <StorageMigrationConfirm
                handleSave={handleSave}
                saving={saving}
                setShowMigrateConfirm={setShowMigrateConfirm}
              />
            )}

            {/* 迁移进度条 */}
            {progress && (
              <StorageMigrationProgress
                progress={progress}
              />
            )}

            {/* 迁移结果展示 */}
            {migrateResult && (
              <StorageMigrationResult
                migrateResult={migrateResult}
                deletedOldDirs={deletedOldDirs}
                handleDeleteOldDirs={handleDeleteOldDirs}
                deletingOldDirs={deletingOldDirs}
              />
            )}

            {/* Migration is applied before storage opens in the next process. */}
            {needRestart && (
              <div className="p-4 bg-[var(--module-accent-soft)] border border-[var(--module-accent-ring)] rounded-card space-y-2.5">
                <div className="flex items-start gap-1.5">
                  <Power className="w-3.5 h-3.5 mt-0.5 flex-shrink-0 text-[var(--module-accent)]" />
                  <div className="text-tiny text-slate-300 space-y-1">
                    <p className="font-semibold text-[var(--module-accent)]">{t("settings.restartRequired")}</p>
                    <p>{t("settings.migrationScheduled", { path: dataDir })}</p>
                  </div>
                </div>
                <button
                  onClick={restartApp}
                  className="px-3 py-1.5 rounded-ctl text-tiny font-semibold cursor-pointer transition-all flex items-center gap-1.5 bg-[var(--module-accent)] text-white hover:opacity-85"
                >
                  <RefreshCw className="w-3 h-3" />
                  {t("settings.restartNow")}
                </button>
              </div>
            )}

            <div className="flex items-center justify-between pt-4 border-t border-white/5">
              <div>
                {success && !needRestart &&
                  !migrateResult?.moved_versions &&
                  !migrateResult?.moved_links && (
                    <span className="text-body font-medium text-emerald-400 flex items-center gap-1.5">
                      <CheckCircle2 className="w-4 h-4" />
                      {t("settings.configSaved")}
                    </span>
                  )}
              </div>

              <button
                onClick={handleSaveClick}
                disabled={saving || !dataDir}
                className="px-6 py-2.5 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-white rounded-card text-body font-semibold shadow-lg shadow-[var(--module-accent-ring)] cursor-pointer transition-all flex items-center gap-1.5"
              >
                <Save className="w-3.5 h-3.5" />
                {saving ? t("settings.saving") : t("settings.saveConfig")}
              </button>
            </div>
          </div>
        )}
      </div>

      {/* 关于 Kira：名片 */}
      <div className="glass-panel rounded-panel p-6 border border-white/5 space-y-3">
        <div className="flex items-center gap-4">
          <VexGlowAvatar size={56} />
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <span className="text-base font-black tracking-wide text-white">Kira</span>
              <span className="text-micro px-1.5 py-0.5 rounded-full bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)]">v{appVersion || "1.0.0"}</span>
            </div>
            <p className="text-caption text-slate-300 mt-1 truncate">
              <VexGreeting />
            </p>
          </div>
        </div>
      </div>

      {/* 版本检查与升级 */}
      <div className="glass-panel rounded-panel p-6 border border-white/5 space-y-4">
        <div className="flex items-center justify-between pb-3 border-b border-white/5">
          <div className="flex items-center gap-2">
            <RefreshCw className="w-4 h-4 text-[var(--module-accent)]" />
            <h3 className="text-body font-semibold text-white">{t("settings.update")}</h3>
          </div>
          <button
            onClick={handleCheckUpdate}
            disabled={checkingUpdate}
            className="flex items-center gap-1.5 px-3 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded-ctl text-tiny border border-white/5 cursor-pointer"
          >
            <RefreshCw
              className={`w-3 h-3 ${checkingUpdate ? "animate-spin" : ""}`}
            />
            {checkingUpdate ? t("settings.checkingUpdate") : t("settings.checkUpdate")}
          </button>
        </div>

        <div className="flex items-center gap-3 text-body">
          <span className="text-slate-400">{t("settings.currentVersion")}</span>
          <span className="font-mono text-slate-200 bg-black/20 px-2 py-0.5 rounded">
            v{appVersion || "1.0.0"}
          </span>
        </div>

        {updateError && (
          <div className="p-3 bg-[var(--module-accent-soft)] border border-[var(--module-accent-ring)] rounded-card text-tiny text-[var(--module-accent)]">
            {updateError}
          </div>
        )}

        {latestVersion && (
          <ApplicationUpdateReady
            latestVersion={latestVersion}
            updateBody={updateBody}
            updateSource={updateSource}
            handleInstallUpdate={handleInstallUpdate}
            installing={installing}
            handleDownloadUpdate={handleDownloadUpdate}
          />
        )}

        {latestVersion === null && !checkingUpdate && !updateError && (
          <p className="text-tiny text-slate-500">
            {t("settings.updateHint")}
          </p>
        )}
      </div>

      {/* 应用行为 */}
      <ApplicationStartupSettings
        handleToggleAutostart={handleToggleAutostart}
        autostartBusy={autostartBusy}
        autostartOn={autostartOn}
        trayCfg={trayCfg}
        saveTrayCfg={saveTrayCfg}
      />

      {/* 外观 (Appearance) */}
      <div className="glass-panel rounded-panel p-6 border border-white/5 space-y-5">
        <div className="flex items-center justify-between pb-3 border-b border-white/5">
          <div className="flex items-center gap-2">
            <Sliders className="w-4 h-4 text-cyan-400" />
            <div>
              <h3 className="text-body font-semibold text-white">{t("settings.appearance")}</h3>
              <p className="text-micro text-slate-500 mt-0.5">
                {t("settings.appearanceHint")}
              </p>
            </div>
          </div>
        </div>

        {/* 0. 主题色 */}
        <ThemeAccentSettings
          themeAccent={themeAccent}
          handleResetThemeAccent={handleResetThemeAccent}
          handleSetThemeAccent={handleSetThemeAccent}
        />

        {/* 1. 全局字体 */}
        <div className="pt-3 border-t border-white/5 space-y-2.5">
          <p className="text-caption font-medium text-slate-200">{t("settings.globalFont")}</p>
          <div className="flex items-center gap-2">
            <div className="relative flex-1 min-w-0">
              <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3 h-3 text-slate-500 pointer-events-none" />
              <input
                type="text"
                value={fontSearch}
                onChange={(e) => setFontSearch(e.target.value)}
                placeholder={t("settings.searchFontsPh", { count: systemFonts.length })}
                className="w-full glass-input pl-7 pr-2.5 py-1.5 text-body bg-black/30 border border-white/10 rounded-ctl focus:outline-none focus:border-sky-400/50"
              />
            </div>
            <button
              onClick={refreshSystemFonts}
              disabled={fontRefreshing}
              className="p-1.5 rounded-ctl text-tiny text-slate-400 hover:text-slate-200 ui-btn transition cursor-pointer flex-shrink-0 disabled:opacity-50"
              title={t("settings.refreshFontsTitle")}
            >
              <RefreshCw className={`w-3.5 h-3.5 ${fontRefreshing ? "animate-spin" : ""}`} />
            </button>
          </div>
          <ApplicationFontSelector
            appearance={appearance}
            handleSetGlobalFont={handleSetGlobalFont}
            filteredFonts={filteredFonts}
            handleClearCustomFont={handleClearCustomFont}
            handleImportFont={handleImportFont}
            importingFont={importingFont}
          />
          {appearance.customFontPath && (
            <p className="text-micro text-emerald-400">
              {t("settings.usingCustomFont", { font: appearance.globalFont })}
            </p>
          )}
        </div>

        {/* 1.5 全局背景底图纹理 */}
        <BackgroundTextureSettings
          appearance={appearance}
          handleSetBackgroundTexture={handleSetBackgroundTexture}
        />

        {/* 1.6 界面语言 */}
        <div className="pt-3 border-t border-white/5 space-y-2.5">
          <p className="text-caption font-medium text-slate-200">{t("settings.language")}</p>
          <p className="text-micro text-slate-500">{t("settings.languageHint")}</p>
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
                  className={`px-4 py-1.5 rounded-ctl text-caption font-semibold transition cursor-pointer ${
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

        {/* 1.7 动效 */}
        <div className="pt-3 border-t border-white/5 space-y-2.5">
          <p className="text-caption font-medium text-slate-200">{t("settings.motion")}</p>
          <p className="text-micro text-slate-500">{t("settings.motionHint")}</p>
          <div className="flex items-center gap-2">
            {[
              { value: "", label: t("settings.motionSystem") },
              { value: "always", label: t("settings.motionAlways") },
            ].map((opt) => {
              const active = (appearance.motionPreference || "") === opt.value;
              return (
                <button
                  key={opt.value || "system"}
                  onClick={() => handleSetMotionPreference(opt.value)}
                  className={`px-4 py-1.5 rounded-ctl text-caption font-semibold transition cursor-pointer ${
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

        {/* 1.8 列表密度（试点：音乐 / 收藏 / 项目） */}
        <div className="pt-3 border-t border-white/5 space-y-2.5">
          <p className="text-caption font-medium text-slate-200">{t("settings.density")}</p>
          <p className="text-micro text-slate-500">{t("settings.densityHint")}</p>
          <div className="flex items-center gap-2">
            {[
              { value: "", label: t("settings.densityComfortable") },
              { value: "compact", label: t("settings.densityCompact") },
            ].map((opt) => {
              const active = (appearance.density || "") === opt.value;
              return (
                <button
                  key={opt.value || "comfortable"}
                  onClick={() => handleSetDensity(opt.value)}
                  className={`px-4 py-1.5 rounded-ctl text-caption font-semibold transition cursor-pointer ${
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
            <p className="text-caption font-medium text-slate-200">{t("settings.modules")}</p>
            <p className="text-micro text-slate-500">
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
                        handleSaveLauncherConfig({ moduleHotkeys: { [id]: null } });
                      }}
                    />
                  );
                })}
              </div>
            </SortableContext>
          </DndContext>
          <div className="flex items-center justify-between">
            <button
              onClick={handleResetModuleOrder}
              className="text-tiny text-slate-500 hover:text-slate-300 transition cursor-pointer flex items-center gap-1"
            >
              <RotateCcw className="w-3 h-3" /> {t("settings.resetOrder")}
            </button>
            <span className="text-micro text-slate-600">{t("settings.dragToOrder")}</span>
          </div>
        </div>
      </div>

      {/* 数据备份与同步（统一快照，原「数据同步」模块迁入设置） */}
      <DataSyncPanel />

      {/* 统一确认弹窗：删除迁移后残留的旧目录（不可逆，danger） */}
      {<ConfirmDialogHost request={confirmRequest} onClose={() => setConfirmRequest(null)} />}
    </div>
  );
}
