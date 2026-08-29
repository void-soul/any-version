// mihomo 模块共享 UI（沿用 SystemTools 统一风格）

export const inputCls =
  "w-full h-9 px-2.5 rounded-xl bg-white/5 border border-white/10 text-xs text-white placeholder-slate-500 focus:outline-none focus:border-[var(--module-accent)]";
export const labelCls = "text-[11px] text-slate-400 mb-1 block font-medium";
export const btnSec =
  "px-3 py-1.5 rounded-lg bg-white/10 hover:bg-white/20 text-slate-200 text-[11px] font-semibold transition-all cursor-pointer";
export const btnDanger =
  "px-3 py-1.5 rounded-lg bg-rose-500/20 hover:bg-rose-500/30 text-rose-300 border border-rose-500/30 text-[11px] font-semibold transition-all cursor-pointer";
export const btnPrimary =
  "px-3 py-1.5 rounded-lg bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-[11px] font-semibold transition-all cursor-pointer";
export const tagCls =
  "text-[10px] px-2 py-0.5 rounded-md bg-white/5 text-slate-400 border border-white/10";
export const cardCls = "glass-panel rounded-2xl border border-white/10 bg-white/[0.02]";

export function Toggle({ label, v, onChange, disabled }: { label?: string; v: boolean; onChange: (b: boolean) => void; disabled?: boolean }) {
  return (
    <label className={`flex items-center gap-2 text-[12px] text-slate-300 select-none ${disabled ? "opacity-50" : "cursor-pointer"}`}>
      <button
        type="button"
        role="switch"
        aria-checked={v}
        disabled={disabled}
        onClick={() => !disabled && onChange(!v)}
        className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors cursor-pointer ${
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
  return (
    <div className={`flex items-center justify-between gap-3 py-2.5 ${divider ? "border-b border-white/5" : ""}`}>
      <div className="text-[12px] text-slate-300 flex items-center gap-1.5 flex-shrink-0">{title}</div>
      <div className="flex items-center gap-2 min-w-0">{children}</div>
    </div>
  );
}

/** 局部加载遮罩：父容器需为 relative */
export function BusyOverlay({ show, text }: { show: boolean; text?: string }) {
  if (!show) return null;
  return (
    <div className="absolute inset-0 z-40 flex flex-col items-center justify-center gap-2 rounded-2xl bg-[#0b0e15]/70 backdrop-blur-[2px]">
      <span className="w-6 h-6 rounded-full border-2 border-[var(--module-accent-ring)] border-t-[var(--module-accent)] animate-spin" />
      <span className="text-[11px] text-slate-300">{text || "处理中…"}</span>
    </div>
  );
}

export function Modal({ title, onClose, children, footer, wide, busy, busyText }: any) {
  const locked = !!busy;
  return (
    <div className="fixed inset-0 z-50 modal-mask flex items-center justify-center bg-black/60 p-4">
      <div
        className={`relative w-full ${wide ? "max-w-4xl" : "max-w-2xl"} max-h-[85vh] overflow-y-auto rounded-2xl border border-white/10 bg-[#11151f] shadow-2xl`}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-white/10 sticky top-0 bg-[#11151f] z-10">
          <h3 className="text-sm font-bold text-white">{title}</h3>
          <button
            className="text-slate-400 hover:text-white text-xl leading-none cursor-pointer disabled:opacity-40"
            disabled={locked}
            onClick={onClose}
          >
            ×
          </button>
        </div>
        <div className="p-4 space-y-3">{children}</div>
        {footer && <div className="px-4 py-3 border-t border-white/10 flex justify-end gap-2 sticky bottom-0 bg-[#11151f]">{footer}</div>}
        <BusyOverlay show={locked} text={busyText} />
      </div>
    </div>
  );
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
  if (delay === -1) return "未测试";
  if (delay === 0) return "超时";
  return `${delay} ms`;
}
