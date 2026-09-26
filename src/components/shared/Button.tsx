// 共享样式常量：跨模块统一按钮 / 输入框 / 卡片外观。
//
// 外观一律走 App.css 里的 `ui-*` 通用层（圆角 / 字号 / 底色 / 描边取自统一令牌），
// 这里只负责尺寸与间距（px-3 h-8 …）——「外观归通用层、布局归调用处」，
// 各模块自造的 btnXxx/inputCls/cardCls 逐步迁移到这里的常量即可。
import type { ButtonHTMLAttributes, ReactNode } from "react";

/* ---------- 可复用 className 常量（直接拼接在已有 className 处） ---------- */
export const btnBase = "ui-btn select-none";

/** 次级按钮：中性底 / 描边（`ui-btn` 的默认态） */
export const btnSecondary = `${btnBase} px-3 h-8`;

/** 主按钮：用主题色（实心 accent） */
export const btnPrimary = `${btnBase} ui-btn-primary px-3 h-8`;

/** 危险按钮：红（与「确定」必须长得不一样，避免误点） */
export const btnDanger = `${btnBase} ui-btn-danger px-3 h-8`;

/** 幽灵按钮：只有文字，无底 */
export const btnGhost = `${btnBase} ui-btn-ghost px-3 h-8`;

/* ---------- 输入框 / 卡片 ---------- */
export const inputCls = "ui-input w-full h-9 px-2.5 placeholder-slate-500";
export const labelCls = "text-caption text-slate-400 mb-1 block font-medium";
export const cardCls = "ui-card";

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