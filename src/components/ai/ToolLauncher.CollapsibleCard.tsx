import React from "react";
import { ChevronRight } from "lucide-react";
/** 可折叠的设置卡片。
 *
 * 折叠后必须靠 `summary` 保留关键信息（例如「当前选了哪个模型」）——
 * 收起来就看不到设了什么，用户还得展开确认，等于没省事。
 * `action` 用于标题行右侧的常驻控件（如优化器开关）：它不能放进折叠按钮里，
 * 否则 button 套 button 是非法 HTML。 */
export function CollapsibleCard({
  title, hint, summary, open, onToggle, action, children,
}: {
  title: string;
  hint?: string;
  summary?: React.ReactNode;
  open: boolean;
  onToggle: () => void;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div>
      <div className="flex items-center gap-1.5 mb-1.5">
        <button onClick={onToggle} className="flex items-center gap-1.5 min-w-0 cursor-pointer text-left">
          <ChevronRight className={`w-3 h-3 text-slate-500 transition-transform flex-shrink-0 ${open ? "rotate-90" : ""}`} />
          <span className="text-body font-bold text-slate-300 truncate">{title}</span>
        </button>
        {hint && open && <span className="text-micro text-slate-500 truncate">{hint}</span>}
        {!open && summary && (
          <span className="text-micro text-slate-500 ml-auto truncate">{summary}</span>
        )}
        {action && <div className={!open && summary ? "" : "ml-auto"}>{action}</div>}
      </div>
      {open && children}
    </div>
  );
}
