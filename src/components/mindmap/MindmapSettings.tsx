// 思维导图模块专属设置：节点速记热键、贴纸热键、外部编辑器、AI 探索参数。
//
// 数据源：
// - LauncherSetting（热键与外部编辑器，存 launcher 设置）
// - ExplorerSettings（AI 探索预算，存数据目录 mindmap_settings.json）
// 弹窗打开时才挂载，因此在这里按需拉取设置。
import { useEffect, useState } from "react";

import { invoke } from "@tauri-apps/api/core";
import { FolderOpen } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { LauncherSetting } from "../launcher/types";
import { HotkeyRecorder } from "../shared/HotkeyRecorder";
import { SettingsGroup, SettingsRow } from "../shared/ModuleSettings";

/** 与后端 mindmap::settings::ExplorerSettings 对应（camelCase 序列化） */
export interface ExplorerSettings {
  explorerRounds: number;
  explorerFilesPerRound: number;
  explorerCharsPerFile: number;
  explorerBatchChars: number;
  /** 右栏对话 Agent 的单轮最大交互次数（与 explorerRounds 是两件事） */
  agentRounds: number;
  /** 上次使用的 AI 供应商（思维导图专用记忆，无显式设置项） */
  lastProviderId?: string | null;
  /** 上次使用的 AI 模型 */
  lastModelId?: string | null;
}

/** 与后端钳制范围保持一致（前端先钳一次，后端保存时仍会硬钳制） */
const EXPLORER_LIMITS = {
  explorerRounds: { min: 1, max: 12 },
  explorerFilesPerRound: { min: 1, max: 24 },
  explorerCharsPerFile: { min: 500, max: 20000 },
  explorerBatchChars: { min: 4000, max: 60000 },
  agentRounds: { min: 1, max: 30 },
} as const;

export function MindmapModuleSettings() {
  const { t } = useTranslation();
  const [launcher, setLauncher] = useState<LauncherSetting | null>(null);
  const [explorer, setExplorer] = useState<ExplorerSettings | null>(null);
  const [editorCmd, setEditorCmd] = useState("");

  useEffect(() => {
    invoke<LauncherSetting>("launcher_get_settings")
      .then((c) => {
        setLauncher(c);
        setEditorCmd(c.externalEditor ?? "");
      })
      .catch((e) => console.error("读取启动器设置失败:", e));
    invoke<ExplorerSettings>("mm_get_explorer_settings")
      .then(setExplorer)
      .catch((e) => console.error("读取思维导图设置失败:", e));
  }, []);

  const saveLauncher = async (patch: Partial<LauncherSetting>) => {
    if (!launcher) return;
    const next = { ...launcher, ...patch };
    setLauncher(next);
    try {
      await invoke("launcher_save_settings", { settings: next });
    } catch (e) {
      console.error("保存启动器设置失败:", e);
    }
  };

  // 注意：必须整体回传（含 last* 记忆字段），否则会把「上次使用的模型」覆盖成空
  const saveExplorer = async (patch: Partial<ExplorerSettings>) => {
    if (!explorer) return;
    const next = { ...explorer, ...patch };
    setExplorer(next);
    try {
      await invoke("mm_save_explorer_settings", { settings: next });
    } catch (e) {
      console.error("保存思维导图设置失败:", e);
    }
  };

  const browseEditorExe = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ multiple: false, directory: false });
      if (selected) {
        setEditorCmd(selected as string);
        await saveLauncher({ externalEditor: selected as string });
      }
    } catch {
      alert(t("settings.folderPickerUnavailable"));
    }
  };

  if (!launcher || !explorer) {
    return <div className="text-[11px] text-slate-500">…</div>;
  }

  return (
    <div className="space-y-5">
      <SettingsGroup title={t("mindmap.settingsHotkeys")}>
        <SettingsRow
          label={t("settings.mindmapNodeHotkey")}
          hint={t("settings.mindmapNodeHotkeyHint")}
        >
          <HotkeyRecorder
            value={launcher.mindmapQuickHotkey ?? ""}
            onChange={(hotkey) => saveLauncher({ mindmapQuickHotkey: hotkey })}
            clearTitle={t("settings.clearHotkey")}
          />
        </SettingsRow>
        <SettingsRow
          label={t("settings.mindmapStickerHotkey")}
          hint={t("settings.mindmapStickerHotkeyHint")}
        >
          <HotkeyRecorder
            value={launcher.mindmapStickerHotkey ?? ""}
            onChange={(hotkey) => saveLauncher({ mindmapStickerHotkey: hotkey })}
            clearTitle={t("settings.clearHotkey")}
          />
        </SettingsRow>
      </SettingsGroup>

      <SettingsGroup title={t("settings.externalEditor")}>
        <div className="flex items-center gap-1.5">
          <input
            value={editorCmd}
            onChange={(e) => setEditorCmd(e.target.value)}
            onBlur={() => {
              if (editorCmd !== (launcher.externalEditor ?? "")) {
                void saveLauncher({ externalEditor: editorCmd });
              }
            }}
            placeholder={t("settings.externalEditorPh")}
            className="flex-1 h-9 px-2.5 rounded-xl bg-white/5 border border-white/10 text-xs text-white placeholder-slate-500 focus:outline-none focus:border-[var(--module-accent)]"
          />
          <button
            type="button"
            onClick={browseEditorExe}
            className="h-9 px-3 rounded-xl bg-white/10 hover:bg-white/20 text-white transition-colors cursor-pointer flex items-center justify-center"
            title={t("settings.chooseFolder")}
          >
            <FolderOpen className="w-4 h-4" />
          </button>
        </div>
        <p className="text-[10px] text-slate-500 leading-relaxed">
          {t("settings.externalEditorHint")}
        </p>
        <p className="text-[10px] text-slate-600 leading-relaxed">
          {t("settings.externalEditorExample")}
        </p>
      </SettingsGroup>

      <SettingsGroup title={t("settings.explorerParams")}>
        <p className="text-[10px] text-slate-500 leading-relaxed">
          {t("settings.explorerParamsHint")}
        </p>
        <div className="grid grid-cols-2 gap-2">
          {(
            [
              ["explorerRounds", "settings.explorerRounds", "settings.explorerRoundsHint"],
              ["explorerFilesPerRound", "settings.explorerFilesHint", ""],
              ["explorerCharsPerFile", "settings.explorerCharsFile", ""],
              ["explorerBatchChars", "settings.explorerCharsBatch", ""],
              ["agentRounds", "settings.agentRounds", "settings.agentRoundsHint"],
            ] as const
          ).map(([key, label, hint]) => {
            const lim = EXPLORER_LIMITS[key];
            return (
              <div key={key}>
                <label className="text-[9px] text-slate-400">{t(label)}</label>
                <input
                  type="number"
                  min={lim.min}
                  max={lim.max}
                  value={explorer[key]}
                  onChange={(e) => {
                    const n = Number(e.target.value);
                    if (Number.isNaN(n)) return;
                    setExplorer({ ...explorer, [key]: n } as ExplorerSettings);
                  }}
                  onBlur={(e) => {
                    // 失焦时钳制到合法范围（后端保存时还会再硬钳制一次）
                    const n = Math.min(lim.max, Math.max(lim.min, Number(e.target.value) || lim.min));
                    void saveExplorer({ [key]: n } as Partial<ExplorerSettings>);
                  }}
                  className="w-full h-9 px-2.5 rounded-xl bg-white/5 border border-white/10 text-xs text-white font-mono focus:outline-none focus:border-[var(--module-accent)]"
                />
                {hint ? <p className="text-[8px] text-slate-600 mt-0.5">{t(hint)}</p> : null}
              </div>
            );
          })}
        </div>
      </SettingsGroup>
    </div>
  );
}
