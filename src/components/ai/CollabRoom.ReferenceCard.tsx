import { useTranslation } from "react-i18next";
import { Trash2, Quote } from "lucide-react";
// ─── 共享组件：引用卡 / 文件徽章（输入区 + 消息渲染共用） ───

export function ReferenceCard({
  senderName,
  excerpt,
  onRemove,
  variant = "input",
}: {
  senderName: string;
  excerpt: string;
  onRemove?: () => void;
  variant?: "input" | "message";
}) {
  const { t } = useTranslation();
  const isInput = variant === "input";
  return (
    <div
      className={`flex items-start gap-2 p-1.5 rounded border-l-2 border-[var(--module-accent)] text-micro ${
        isInput ? "bg-slate-800/60 text-slate-300" : "bg-slate-800/50 text-slate-400"
      }`}
    >
      <Quote className="w-3 h-3 flex-shrink-0 mt-0.5 text-[var(--module-accent)]" />
      <div className="flex-1 min-w-0">
        <span className="font-semibold text-[var(--module-accent)]">
          {t("collab.fromSender", { name: senderName })}
        </span>
        <span className={`whitespace-pre-wrap ${isInput ? "line-clamp-2" : "line-clamp-3"}`}>
          {excerpt}
        </span>
      </div>
      {onRemove && (
        <button
          onClick={onRemove}
          className="text-slate-500 hover:text-red-300 cursor-pointer"
        >
          <Trash2 className="w-3 h-3" />
        </button>
      )}
    </div>
  );
}
