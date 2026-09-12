// 模块设置入口：统一的「齿轮按钮 + 设置弹窗」容器。
//
// 设计背景：App 是多模块并列的工具集合，各模块的专属设置不应堆在全局设置页。
// 每个模块用本组件在自己的 header 放一个齿轮按钮，把专属设置作为 children 传入，
// 保证跨模块的入口位置、弹窗样式与交互完全一致。
//
// 注意：SharedModal 在未打开时返回 null，因此 children 只在首次打开时才挂载——
// 内容组件可以放心在自身 useEffect 里按需拉取设置数据，无需外部预加载。
import { useState } from "react";

import { Settings } from "lucide-react";

import { SharedModal } from "./Modal";

interface ModuleSettingsButtonProps {
  /** 按钮 tooltip 与弹窗标题 */
  title: string;
  /** 弹窗内容（该模块的专属设置项） */
  children: React.ReactNode;
  /** 提供后按钮显示「图标 + 文字」，样式贴近各模块工具栏的既有按钮；
   *  不提供则是图标按钮（适配置于 header 右侧的操作区）。 */
  label?: string;
  /** 追加到齿轮按钮的 class（用于适配各模块 header 的布局与尺寸） */
  buttonClassName?: string;
  /** 弹窗宽度 */
  width?: number;
}

/** 齿轮按钮 + 模块设置弹窗。 */
export function ModuleSettingsButton({
  title,
  children,
  label,
  buttonClassName = "",
  width = 560,
}: ModuleSettingsButtonProps) {
  const [open, setOpen] = useState(false);
  const baseCls = label
    ? "inline-flex items-center gap-1.5 rounded px-2 py-1.5 text-[10px]"
    : "p-2 rounded-lg";
  const iconCls = label ? "h-3 w-3" : "w-4 h-4";
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        title={title}
        aria-label={title}
        className={`${baseCls} text-slate-400 hover:text-white hover:bg-white/10 transition-all cursor-pointer ${buttonClassName}`}
      >
        <Settings className={iconCls} />
        {label}
      </button>
      <SharedModal open={open} onClose={() => setOpen(false)} title={title} width={width}>
        {children}
      </SharedModal>
    </>
  );
}

/** 设置分组：弹窗内需要多个小节时用于区隔。 */
export function SettingsGroup({
  title,
  children,
}: {
  title?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-2.5">
      {title ? (
        <div className="text-[10px] font-semibold uppercase tracking-wider text-slate-500">
          {title}
        </div>
      ) : null}
      <div className="space-y-3">{children}</div>
    </div>
  );
}

/** 单行设置项：左侧标题与说明，右侧控件。 */
export function SettingsRow({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-start justify-between gap-3">
      <div className="min-w-0 flex-1">
        <div className="text-[12px] text-slate-200">{label}</div>
        {hint ? (
          <div className="text-[10px] text-slate-500 mt-0.5 leading-relaxed">{hint}</div>
        ) : null}
      </div>
      <div className="flex-shrink-0">{children}</div>
    </div>
  );
}

/** 开关控件：模块设置弹窗内统一使用（与全局设置观感一致）。 */
export function SettingsSwitch({
  checked,
  onChange,
  disabled,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors cursor-pointer ${
        checked ? "bg-[var(--module-accent)]" : "bg-white/15"
      } ${disabled ? "opacity-50 cursor-not-allowed" : ""}`}
    >
      <span
        className={`inline-block h-3.5 w-3.5 rounded-full bg-white shadow-sm transition-transform ${
          checked ? "translate-x-4" : "translate-x-0.5"
        }`}
      />
    </button>
  );
}
