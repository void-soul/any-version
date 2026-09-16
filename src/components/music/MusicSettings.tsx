// 音乐播放器设置弹框内容（挂在本页右上角齿轮下）：
// - 音效：10 段均衡器 + 预设 + 总增益 + 声道平衡（EqDialog）
// - 快捷键：播放/暂停、上一首、下一首（全局热键，直接驱动后端播放器，
//   因此最小化到托盘后照样可用）
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

import type { LauncherSetting } from "../launcher/types";
import { HotkeyRecorder } from "../shared/HotkeyRecorder";
import { SettingsGroup, SettingsRow } from "../shared/ModuleSettings";
import { EqDialog } from "./EqDialog";
import type { EqParams, EqPresetInfo } from "./types";

interface Props {
  eq: EqParams;
  presets: EqPresetInfo[];
  onEqChange: (eq: EqParams) => void;
}

export function MusicSettingsDialog({ eq, presets, onEqChange }: Props) {
  const { t } = useTranslation();
  const [launcher, setLauncher] = useState<LauncherSetting | null>(null);

  // SharedModal 只在首次打开时挂载内容，因此这里按需拉取启动器设置（存放全局热键）
  useEffect(() => {
    invoke<LauncherSetting>("launcher_get_settings")
      .then(setLauncher)
      .catch(() => {});
  }, []);

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
    </div>
  );
}
