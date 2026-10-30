import { useTranslation } from "react-i18next";
import { Trash2, Reply } from "lucide-react";
import { fmtTime, type PickyComment } from "./PickyPanel.model";
export function CommentRow({
  comment,
  depth,
  onReply,
  onDelete,
}: {
  comment: PickyComment;
  depth: number;
  onReply?: () => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div
      className="rounded-ctl bg-white/[0.02] border border-white/5 p-2"
      style={{ marginLeft: depth > 0 ? 20 : 0 }}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="flex-1 min-w-0">
          <p className="text-caption text-slate-200 leading-relaxed break-words whitespace-pre-wrap">{comment.content}</p>
          <div className="flex items-center gap-2 mt-1 text-micro text-slate-600">
            <span>{fmtTime(comment.createdAt)}</span>
            {depth === 0 && onReply && (
              <button onClick={onReply} className="flex items-center gap-0.5 hover:text-slate-300 cursor-pointer">
                <Reply className="w-2.5 h-2.5" /> {t("picky.reply")}
              </button>
            )}
          </div>
        </div>
        <button
          onClick={onDelete}
          className="p-1 rounded text-slate-600 hover:text-red-400 hover:bg-red-500/10 cursor-pointer flex-shrink-0"
          title={t("picky.delComment")}
        >
          <Trash2 className="w-2.5 h-2.5" />
        </button>
      </div>
    </div>
  );
}
