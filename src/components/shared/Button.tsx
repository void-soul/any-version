// 共享样式常量：跨模块统一按钮 / 输入框 / 卡片外观。
// 主题色相关均使用 --module-accent 系列 CSS 变量，随模块动态主题色联动。
// 各模块里的局部 btnXxx/inputCls/cardCls 可逐步迁移到这里。
import type { ButtonHTMLAttributes, ReactNode } from "react";

/* ---------- 可复用 className 常量（直接拼接在已有 className 处） ---------- */
export const btnBase =
  "inline-flex items-center justify-center gap-1.5 rounded-lg transition-all cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed select-none";

/** 次级按钮：中性底 / 描边 */
export const btnSecondary = `${btnBase} px-3 h-8 bg-white/5 hover:bg-white/10 border border-white/10 text-[11px] text-slate-300`;

/** 主按钮：用模块主题色 */
export const btnPrimary = `${btnBase} px-3 h-8 bg-[var(--module-accent)] hover:opacity-85 text-[11px] font-semibold text-white`;

/** 危险按钮：红 */
export const btnDanger = `${btnBase} px-3 h-8 bg-rose-500/15 hover:bg-rose-500/25 border border-rose-500/30 text-[11px] text-rose-300`;

/** 幽灵按钮：只有文字，无底 */
export const btnGhost = `${btnBase} px-3 h-8 text-[11px] text-slate-400 hover:text-white hover:bg-white/5`;

/* ---------- 输入框 / 卡片 ---------- */
export const inputCls =
  "w-full h-9 px-2.5 rounded-xl bg-white/5 border border-white/10 text-xs text-white placeholder-slate-500 focus:outline-none focus:border-[var(--module-accent)]";
export const labelCls = "text-[11px] text-slate-400 mb-1 block font-medium";
export const cardCls = "glass-panel rounded-2xl border border-white/10 bg-white/[0.02]";

/* ---------- 组件 ---------- */
type Variant = "primary" | "secondary" | "danger" | "ghost";

const variantCls: Record<Variant, string> = {
  primary: btnPrimary,
  secondary: btnSecondary,
  danger: btnDanger,
  ghost: btnGhost,
};

export function SharedButton({
  variant = "secondary",
  className = "",
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant }) {
  return (
    <button className={`${variantCls[variant]} ${className}`} {...rest}>
      {children as ReactNode}
    </button>
  );
}