import { useTranslation } from "react-i18next";
import { FolderOpen, Check } from "lucide-react";
import type { DetectedAiTool } from "../../contracts/ai";
import type { Dispatch, SetStateAction } from "react";

interface ToolCustomPathEditorProps {
  selectedTool: DetectedAiTool;
  pathInput: string;
  setPathInput: Dispatch<SetStateAction<string>>;
  browseCustomPath: () => Promise<void>;
  pathSaving: boolean;
  saveCustomPath: (value: string | null) => Promise<void>;
  pathMsg: { ok: boolean; msg: string; } | null;
}

/** Pure ToolCustomPathEditor view; its owning panel retains side effects and state. */
export function ToolCustomPathEditor({ selectedTool, pathInput, setPathInput, browseCustomPath, pathSaving, saveCustomPath, pathMsg }: ToolCustomPathEditorProps) {
  const { t } = useTranslation();
  return (
    <div className="p-3 rounded-card bg-slate-900/30 border border-white/5 space-y-2">
      <div className="flex items-center justify-between">
        <div className="text-body font-semibold text-slate-300 flex items-center gap-1.5">
          <FolderOpen className="w-3.5 h-3.5" /> {t("toollaunch.installPath")}
        </div>
        {selectedTool.custom_path && (
          <span className="text-micro px-1.5 py-px rounded bg-[var(--module-accent)]/20 text-[var(--module-accent)]">
            {t("toollaunch.pathManual")}
          </span>
        )}
      </div>
      <div className="flex items-center gap-1.5">
        <input
          value={pathInput}
          onChange={(e) => setPathInput(e.target.value)}
          placeholder={selectedTool.detected_path || t("toollaunch.pathPlaceholder")}
          className="flex-1 min-w-0 px-2 py-1 rounded-md bg-white/5 border border-white/10 text-caption text-slate-200 placeholder-slate-600 font-mono truncate focus:outline-none focus:border-[var(--module-accent)]/50"
        />
        <button onClick={() => void browseCustomPath()} disabled={pathSaving}
          className="px-2 py-1 rounded-md bg-white/5 hover:bg-white/10 text-tiny text-slate-300 flex items-center gap-1 cursor-pointer disabled:opacity-50"
          title={t("toollaunch.pathBrowse")}>
          <FolderOpen className="w-3 h-3" />
        </button>
        <button onClick={() => void saveCustomPath(pathInput.trim() || null)} disabled={pathSaving}
          className="px-2 py-1 rounded-md bg-emerald-500/15 hover:bg-emerald-500/25 text-tiny text-emerald-200 flex items-center gap-1 cursor-pointer disabled:opacity-50"
          title={t("toollaunch.pathSaveHint")}>
          <Check className="w-3 h-3" /> {t("toollaunch.save")}
        </button>
        <button onClick={() => { setPathInput(""); void saveCustomPath(null); }}
          disabled={pathSaving || !selectedTool.custom_path}
          className="px-2 py-1 rounded-md bg-white/5 hover:bg-white/10 text-tiny text-slate-300 cursor-pointer disabled:opacity-40"
          title={t("toollaunch.pathClearHint")}>
          {t("toollaunch.clear")}
        </button>
      </div>
      <div className="text-tiny text-slate-500 truncate">
        {selectedTool.detected_path ? (
          `${t("toollaunch.pathDetected")}: ${selectedTool.detected_path}`
        ) : selectedTool.custom_path ? (
          // 手动填了路径却在磁盘上找不到 → 明确说是「你填的那条不对」，
          // 而不是笼统的「未检测到」让用户以为自动检测失灵了
          <span className="text-amber-400/80">
            {t("toollaunch.pathInvalid", { path: selectedTool.custom_path })}
          </span>
        ) : selectedTool.tool_kind === "desktop" ? (
          // Store / 桌面应用本来就没有常规 exe 安装路径（装在系统包注册里），
          // 这不是检测失灵：启动走 `shell:AppsFolder\…` 包 URI，不依赖这条路径
          <span className="text-slate-500">{t("toollaunch.pathStoreApp")}</span>
        ) : (
          t("toollaunch.pathNotDetected")
        )}
      </div>
      {pathMsg && (
        <div className={`text-tiny ${pathMsg.ok ? "text-emerald-400" : "text-red-400"}`}>{pathMsg.msg}</div>
      )}
    </div>
  );
}
