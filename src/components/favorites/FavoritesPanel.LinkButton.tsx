import { RefreshCw } from "lucide-react";
/**
 * 工具栏的文字链接按钮。
 *
 * 刻意做成纯文字：这一排全是同级动作，用按钮样式会把「导入」这种高频操作
 * 和其它动作拉成一样的视觉重量，反而看不出主次，一行里还会挤满色块。
 * `busy` 时显示转圈并禁用点击（避免重复触发）。
 */
export function LinkButton({
  label,
  onClick,
  disabled,
  busy,
  danger,
  active,
  title,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  busy?: boolean;
  danger?: boolean;
  /** 当前处于「已开启」状态（如 AI 检索右栏展开中）：高亮，让用户看出这是开关 */
  active?: boolean;
  title?: string;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled || busy}
      title={title}
      className={`inline-flex items-center gap-1 text-caption cursor-pointer transition-colors disabled:opacity-40 disabled:cursor-not-allowed ${
        danger
          ? "text-rose-400/80 hover:text-rose-300"
          : active
            ? "text-[var(--module-accent)]"
            : "text-slate-400 hover:text-[var(--module-accent)]"
      }`}
    >
      {busy && <RefreshCw className="w-2.5 h-2.5 animate-spin" />}
      {label}
    </button>
  );
}
