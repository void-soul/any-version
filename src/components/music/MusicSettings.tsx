// 音乐播放器设置弹框内容（挂在本页右上角齿轮下）：
// - 音效：10 段均衡器 + 预设 + 总增益 + 声道平衡（EqDialog）
// - 快捷键：播放/暂停、上一首、下一首（全局热键，直接驱动后端播放器，
//   因此最小化到托盘后照样可用）
// - 下载与存储：在线音源的下载目录 + 播放缓存占用/清理
//
// 「下载路径」刻意放在音乐设置里（而不是插件页）：它是**播放器的**设置，
// 与「装了哪些插件」无关；插件页只管插件本身。
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";

import type { LauncherSetting } from "../launcher/types";
import { SharedButton } from "../shared/Button";
import { HotkeyRecorder } from "../shared/HotkeyRecorder";
import { SettingsGroup, SettingsRow } from "../shared/ModuleSettings";
import { toast } from "../shared/Toast";
import { EqDialog } from "./EqDialog";
import { formatBytes, type EqParams, type EqPresetInfo, type PluginStorageInfo } from "./types";

interface Props {
  eq: EqParams;
  presets: EqPresetInfo[];
  onEqChange: (eq: EqParams) => void;
}

export function MusicSettingsDialog({ eq, presets, onEqChange }: Props) {
  const { t } = useTranslation();
  const [launcher, setLauncher] = useState<LauncherSetting | null>(null);
  const [storage, setStorage] = useState<PluginStorageInfo | null>(null);

  // SharedModal 只在首次打开时挂载内容，因此这里按需拉取启动器设置（存放全局热键）
  useEffect(() => {
    invoke<LauncherSetting>("launcher_get_settings")
      .then(setLauncher)
      .catch(() => {});
  }, []);

  const reloadStorage = useCallback(async () => {
    try {
      setStorage(await invoke<PluginStorageInfo>("music_plugin_storage_info"));
    } catch {
      /* 存储信息只是展示，取不到就不显示 */
    }
  }, []);

  useEffect(() => {
    void reloadStorage();
  }, [reloadStorage]);

  // 必须整体回传（含 moduleHotkeys 等字段），否则会把其它模块的热键覆盖成空
  const saveLauncher = async (patch: Partial<LauncherSetting>) => {
    if (!launcher) return;
    const next = { ...launcher, ...patch };
    setLauncher(next);
    try {
      await invoke("launcher_save_settings", { settings: next });
    } catch {
      /* 保存失败时保持界面值，用户可重试 */
    }
  };

  const chooseDownloadDir = async () => {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked !== "string") return;
    try {
      await invoke("music_plugin_set_download_dir", { dir: picked });
      await reloadStorage();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const clearCache = async () => {
    try {
      const freed = await invoke<number>("music_plugin_clear_cache");
      toast(t("music.storageCacheCleared", { size: formatBytes(freed) }));
      await reloadStorage();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  return (
    <div className="space-y-5">
      <SettingsGroup title={t("music.eqSection")}>
        <EqDialog eq={eq} presets={presets} onChange={onEqChange} />
      </SettingsGroup>

      <SettingsGroup title={t("music.hotkeySection")}>
        <SettingsRow
          label={t("music.hotkeyPlayPause")}
          hint={t("music.hotkeyHint")}
        >
          <HotkeyRecorder
            value={launcher?.musicPlayPauseHotkey ?? ""}
            onChange={(hotkey) => void saveLauncher({ musicPlayPauseHotkey: hotkey })}
            clearTitle={t("settings.clearHotkey")}
          />
        </SettingsRow>
        <SettingsRow label={t("music.hotkeyPrev")} hint={t("music.hotkeyHint")}>
          <HotkeyRecorder
            value={launcher?.musicPrevHotkey ?? ""}
            onChange={(hotkey) => void saveLauncher({ musicPrevHotkey: hotkey })}
            clearTitle={t("settings.clearHotkey")}
          />
        </SettingsRow>
        <SettingsRow label={t("music.hotkeyNext")} hint={t("music.hotkeyHint")}>
          <HotkeyRecorder
            value={launcher?.musicNextHotkey ?? ""}
            onChange={(hotkey) => void saveLauncher({ musicNextHotkey: hotkey })}
            clearTitle={t("settings.clearHotkey")}
          />
        </SettingsRow>
      </SettingsGroup>

      {/* 在线音源的落盘位置与缓存 */}
      <SettingsGroup title={t("music.storageSection")}>
        <SettingsRow label={t("music.storageDownloadDir")} hint={t("music.storageDownloadHint")}>
          <div className="flex items-center gap-2">
            <span
              className="text-caption text-slate-300 max-w-[240px] truncate"
              title={storage?.download_dir ?? ""}
            >
              {storage?.download_dir || t("music.storageDefaultDir")}
            </span>
            <SharedButton variant="secondary" onClick={() => void chooseDownloadDir()}>
              {t("music.storageChooseDir")}
            </SharedButton>
          </div>
        </SettingsRow>
        <SettingsRow
          label={t("music.storageCacheUsed", { size: formatBytes(storage?.cache_bytes ?? 0) })}
          hint={t("music.storageCacheHint")}
        >
          <SharedButton
            variant="secondary"
            onClick={() => void clearCache()}
            disabled={(storage?.cache_bytes ?? 0) === 0}
          >
            {t("music.storageClearCache")}
          </SharedButton>
        </SettingsRow>
      </SettingsGroup>
    </div>
  );
}
