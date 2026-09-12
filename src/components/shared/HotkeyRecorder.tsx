// 共享热键录制控件：供各模块设置弹窗使用（翻译划词热键、思维导图速记/贴纸热键等）。
//
// 录制基于 e.code 解析，支持 F1-F12、Shift/Ctrl/Alt/Win + 组合键与单键；
// 录制期间 Esc 取消、点击窗口任意处取消。
import { useEffect, useRef, useState } from "react";

import { X } from "lucide-react";
import { useTranslation } from "react-i18next";

interface HotkeyRecorderProps {
  /** 当前热键值；空串表示未设置 */
  value: string;
  /** 录制完成或清除时回调（清除回调传空串） */
  onChange: (hotkey: string) => void;
  /** 清除按钮的 tooltip */
  clearTitle?: string;
  disabled?: boolean;
}

/** 把 KeyboardEvent 解析为应用统一的热键字符串（如 "Ctrl+Shift+A" / "F6"）。 */
function formatHotkey(e: KeyboardEvent): string {
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Win");

  const code = e.code;
  let key = "";
  if (code.startsWith("Key")) {
    key = code.slice(3); // KeyA -> A
  } else if (code.startsWith("Digit")) {
    key = code.slice(5); // Digit1 -> 1
  } else if (/^(F[1-9]|F1[0-2])$/.test(code)) {
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
  if (!key) return "";

  const formatted = key.length === 1 ? key.toUpperCase() : key;
  if (!parts.includes(formatted)) parts.push(formatted);
  return parts.join("+");
}

export function HotkeyRecorder({
  value,
  onChange,
  clearTitle,
  disabled,
}: HotkeyRecorderProps) {
  const { t } = useTranslation();
  const [recording, setRecording] = useState(false);

  // 用 ref 持有最新回调，避免父组件每次渲染产生的新函数导致监听反复重装
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  useEffect(() => {
    if (!recording) return;

    const handleKeyCapture = (e: KeyboardEvent) => {
      if (e.code === "Escape") {
        e.preventDefault();
        setRecording(false);
        return;
      }
      // 忽略单独按下的修饰键，等待最终按键
      if (
        [
          "ShiftLeft",
          "ShiftRight",
          "ControlLeft",
          "ControlRight",
          "AltLeft",
          "AltRight",
          "MetaLeft",
          "MetaRight",
        ].includes(e.code)
      ) {
        return;
      }

      e.preventDefault();
      e.stopPropagation();

      const hotkey = formatHotkey(e);
      if (!hotkey) return; // 无法识别的按键，继续等待

      onChangeRef.current(hotkey);
      setRecording(false);
    };

    const handleWindowClick = () => setRecording(false);

    window.addEventListener("keydown", handleKeyCapture, true);
    window.addEventListener("mousedown", handleWindowClick);
    return () => {
      window.removeEventListener("keydown", handleKeyCapture, true);
      window.removeEventListener("mousedown", handleWindowClick);
    };
  }, [recording]);

  return (
    <div className="flex items-center gap-1.5 shrink-0">
      {value && !recording && (
        <button
          type="button"
          disabled={disabled}
          onClick={() => onChange("")}
          className="p-1.5 rounded-md text-slate-500 hover:text-red-400 hover:bg-red-500/10 cursor-pointer"
          title={clearTitle ?? t("settings.clearTranslateHotkey")}
        >
          <X className="w-3 h-3" />
        </button>
      )}
      <button
        type="button"
        disabled={disabled}
        onClick={() => setRecording((r) => !r)}
        className={`min-w-[86px] px-2.5 py-1 rounded-md border text-[11px] text-center transition cursor-pointer ${
          recording
            ? "border-emerald-500/50 bg-emerald-500/15 text-emerald-300"
            : "border-white/10 bg-white/5 text-slate-300 hover:bg-white/10 hover:text-white"
        } ${disabled ? "opacity-50 cursor-not-allowed" : ""}`}
      >
        {recording ? t("settings.pressKeys") : value || t("settings.clickToRecord")}
      </button>
    </div>
  );
}
