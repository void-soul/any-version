// mihomo 模块共享 UI（沿用 SystemTools 统一风格）
import { useTranslation } from "react-i18next";

import { createContext, useContext, useId, type ReactNode } from "react";
import { SharedModal } from "../../shared/Modal";
import { inputCls, labelCls, btnSecondary as btnSec, btnDanger, btnPrimary } from "../../shared/Button";
export { inputCls, labelCls, btnSec, btnDanger, btnPrimary };
const SettingLabel = createContext<string | undefined>(undefined);
export const tagCls = "ui-chip px-2 py-0.5";
export const cardCls = "ui-card";

export function Toggle({ label, v, onChange, disabled }: { label?: string; v: boolean; onChange: (b: boolean) => void; disabled?: boolean }) {
  const settingLabel = useContext(SettingLabel);
  return (
    <label className={`flex items-center gap-2 text-body text-slate-300 select-none ${disabled ? "opacity-50" : "cursor-pointer"}`}>
      <button
        type="button"
        role="switch"
        aria-checked={v}
        aria-label={label}
        aria-labelledby={label ? undefined : settingLabel}
        disabled={disabled}
        onClick={() => !disabled && onChange(!v)}
        className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors cursor-pointer disabled:cursor-not-allowed ${
          v ? "bg-[var(--module-accent)]" : "bg-white/15"
        }`}
      >
        <span
          className={`inline-block h-3.5 w-3.5 rounded-full bg-white shadow-sm transition-transform ${
            v ? "translate-x-4" : "translate-x-0.5"
          }`}
        />
      </button>
      {label}
    </label>
  );
}

// 设置行：左标题右控件（对齐 clash-party SettingItem）
export function SettingItem({ title, children, divider = true }: { title: React.ReactNode; children?: React.ReactNode; divider?: boolean }) {
  const titleId = useId();
  return (
    <SettingLabel.Provider value={titleId}>
    <div className={`flex flex-wrap items-center justify-between gap-3 py-2.5 ${divider ? "border-b border-white/5" : ""}`}>
      <div id={titleId} className="text-body text-slate-300 flex items-center gap-1.5 min-w-0">{title}</div>
      <div className="flex flex-wrap items-center justify-end gap-2 min-w-0 max-w-full">{children}</div>
    </div>
    </SettingLabel.Provider>
  );
}

/** 局部加载遮罩：父容器需为 relative */
export function BusyOverlay({ show, text }: { show: boolean; text?: string }) {
  const { t } = useTranslation();
  if (!show) return null;
  return (
    <div role="status" aria-live="polite" className="absolute inset-0 z-40 flex flex-col items-center justify-center gap-2 rounded-panel bg-surface-deep/70 backdrop-blur-[2px]">
      <span className="w-6 h-6 rounded-full border-2 border-[var(--module-accent-ring)] border-t-[var(--module-accent)] animate-spin" />
      <span className="text-caption text-slate-300">{text || t("mihomoui.processing")}</span>
    </div>
  );
}

interface ModalProps {
  title: ReactNode; onClose: () => void; children?: ReactNode; footer?: ReactNode;
  wide?: boolean; busy?: boolean; busyText?: string;
}
export function Modal({ title, onClose, children, footer, wide, busy, busyText }: ModalProps) {
  return <SharedModal open title={title} width={wide ? 896 : 672} onClose={() => { if (!busy) onClose(); }}
    footer={footer ? <fieldset disabled={busy} className="contents">{footer}</fieldset> : undefined}>
    <fieldset disabled={busy} className="contents">{children}</fieldset>
    <BusyOverlay show={!!busy} text={busyText} />
  </SharedModal>;
}

// 流量格式化（对齐 clash-party calcTraffic）
export function calcTraffic(bytes: number): string {
  if (!bytes || bytes < 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  const s = v >= 100 || v % 1 === 0 ? String(Math.round(v)) : v.toFixed(2);
  return `${s} ${units[i]}`;
}

// 延迟颜色（对齐 clash-party proxy-item：<=500 绿 / <=800 黄 / 其它红；0 = 超时）
export function delayColor(delay: number): string {
  if (delay === -1) return "text-slate-500"; // 未测试
  if (delay === 0) return "text-rose-400"; // 超时
  if (delay <= 500) return "text-emerald-400";
  if (delay <= 800) return "text-amber-400";
  return "text-rose-400";
}
export function delayText(delay: number): string {
  if (delay === -1) return "mihomoui.untested";
  if (delay === 0) return "mihomoui.timeout";
  return `${delay} ms`;
}
