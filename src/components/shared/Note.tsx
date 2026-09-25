import type { ReactNode } from "react";
import { AlertTriangle, CheckCircle2, Info, XCircle } from "lucide-react";

/**
 * 统一样式的提示块（图标 + 可选标题 + 正文）。
 *
 * 原型是 Skill 页「软链接与工具集成」里的那块琥珀色提示——观感好、信息层级清楚，
 * 但它是内联 div，全软件各处提示块各写各的（有的琥珀、有的蓝、有的红，圆角和内边距
 * 也不统一）。这里把它抽成组件，供所有提示类区块复用：**同一个意思就应该是同一张脸**。
 *
 * 用法：
 * ```tsx
 * <Note tone="warn" title={t("xxx.title")}>{t("xxx.body")}</Note>
 * <Note tone="success">{msg}</Note>   // 结果提示：只给正文
 * ```
 */
export type NoteTone = "warn" | "info" | "success" | "error";

const TONE_STYLE: Record<NoteTone, { box: string; iconWrap: string; title: string; body: string }> = {
  warn: {
    box: "bg-amber-500/10 border-amber-500/20",
    iconWrap: "text-amber-300",
    title: "text-amber-200",
    body: "text-slate-300",
  },
  info: {
    box: "bg-sky-500/10 border-sky-500/20",
    iconWrap: "text-sky-300",
    title: "text-sky-200",
    body: "text-slate-300",
  },
  success: {
    box: "bg-emerald-500/10 border-emerald-500/20",
    iconWrap: "text-emerald-300",
    title: "text-emerald-200",
    body: "text-slate-300",
  },
  error: {
    box: "bg-rose-500/10 border-rose-500/20",
    iconWrap: "text-rose-300",
    title: "text-rose-200",
    body: "text-slate-300",
  },
};

function toneIcon(tone: NoteTone, className: string) {
  switch (tone) {
    case "success":
      return <CheckCircle2 className={className} />;
    case "error":
      return <XCircle className={className} />;
    case "info":
      return <Info className={className} />;
    default:
      return <AlertTriangle className={className} />;
  }
}

export function Note({
  tone = "warn",
  title,
  action,
  children,
  className = "",
}: {
  tone?: NoteTone;
  /** 标题（可省：结果提示通常只要一行正文） */
  title?: ReactNode;
  /** 标题行右侧的操作（如「清除告警」），没有就不渲染 */
  action?: ReactNode;
  children?: ReactNode;
  className?: string;
}) {
  const style = TONE_STYLE[tone];
  return (
    <div className={`rounded-xl border p-3.5 space-y-1.5 ${style.box} ${className}`}>
      {(title || tone !== "success") && (
        <div className={`flex items-center gap-2 text-xs font-bold ${style.title}`}>
          <span className={`flex-shrink-0 ${style.iconWrap}`}>{toneIcon(tone, "w-4 h-4")}</span>
          {title && <span>{title}</span>}
          {action && <span className="ml-auto">{action}</span>}
        </div>
      )}
      {children && (
        <div className={`text-[11px] leading-relaxed ${style.body} space-y-1`}>{children}</div>
      )}
    </div>
  );
}

/** 单行结果提示（成功 / 失败）：结果区全站同构，收成一个组件最省事。 */
export function ResultNote({ ok, message }: { ok: boolean; message: string }) {
  return (
    <Note tone={ok ? "success" : "error"}>
      <span className="whitespace-pre-line break-all">{message}</span>
    </Note>
  );
}
