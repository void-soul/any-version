import { useEffect, useState } from "react";
import { Keyboard, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { comboFromKeyboardEvent, normalizeCombo } from "./hotkey";

interface ShortcutRecorderProps {
  value?: string | null;
  onChange: (value: string | null) => void;
  disabled?: boolean;
  className?: string;
}

/**
 * 快捷键录制器：点击后进入录制态，按下组合键即写入；Esc 取消，Backspace/Delete 清除。
 * 输出格式与后端 parse_hotkey 一致（如 "Ctrl+Shift+L"）。
 */
export default function ShortcutRecorder({
  value,
  onChange,
  disabled = false,
  className = "",
}: ShortcutRecorderProps) {
  const { t } = useTranslation();
  const [recording, setRecording] = useState(false);

  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setRecording(false);
        return;
      }
      // 无修饰键的 Backspace / Delete：清除已录制的快捷键
      if (
        (e.key === "Backspace" || e.key === "Delete") &&
        !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey
      ) {
        onChange(null);
        setRecording(false);
        return;
      }
      const combo = comboFromKeyboardEvent(e);
      if (!combo) return;
      onChange(combo);
      setRecording(false);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [recording, onChange]);

  const display = normalizeCombo(value);

  return (
    <div className={`flex items-center gap-1.5 ${className}`}>
      <button
        type="button"
        disabled={disabled}
        onClick={() => setRecording((v) => !v)}
        onBlur={() => setRecording(false)}
        title={t("launcher.hotkeyHint")}
        className={`flex-1 min-w-0 px-2.5 py-1.5 rounded-ctl border text-left text-body transition cursor-pointer flex items-center gap-1.5 disabled:opacity-40 disabled:cursor-not-allowed ${
          recording
            ? "border-[var(--module-accent)] bg-[var(--module-accent-soft)] text-[var(--module-accent)]"
            : "bg-white/5 border-white/10 text-slate-200 hover:bg-white/10"
        }`}
      >
        <Keyboard className="w-3.5 h-3.5 shrink-0 opacity-70" />
        <span className={`truncate font-mono ${display ? "" : "font-sans text-slate-500"}`}>
          {recording ? t("launcher.hotkeyRecording") : display || t("launcher.hotkeyEmpty")}
        </span>
      </button>
      {display && !disabled && (
        <button
          type="button"
          onClick={() => onChange(null)}
          title={t("launcher.hotkeyClear")}
          className="p-1.5 rounded-ctl text-slate-500 hover:text-red-400 hover:bg-white/5 transition cursor-pointer"
        >
          <X className="w-3.5 h-3.5" />
        </button>
      )}
    </div>
  );
}
