import { GripVertical, X } from "lucide-react";
import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { useTranslation } from "react-i18next";
// 可拖拽的模块配置行：主题色 + 位置 + 启用 + 快捷键（拖拽手柄独立，避免与控件冲突）
export function ModuleConfigRow({
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
      className={`flex items-center gap-2 px-2.5 py-2 rounded-card bg-white/[0.03] border border-white/5 ${
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
      <span className="text-caption text-slate-300 w-14 truncate flex-shrink-0">{label}</span>

      {/* 位置：顶栏/更多 */}
      {pinned ? (
        <span className="text-micro text-slate-600 w-9 text-center flex-shrink-0">{t("settings.pinned")}</span>
      ) : (
        <button
          onClick={onToggleToolbar}
          disabled={disabled}
          className={`px-1.5 py-0.5 rounded-md text-micro font-medium border transition cursor-pointer disabled:opacity-40 w-9 flex-shrink-0 ${
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
          className={`px-2 py-0.5 rounded-md text-micro font-mono border transition cursor-pointer ${
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
