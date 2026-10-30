import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Send } from "lucide-react";
import { type PickyComment } from "./PickyPanel.model";
import { CommentRow } from "./PickyPanel.CommentRow";
// ─── 评论区块（列表 / 卡片视图共用） ───

export function CommentBlock({
  comments,
  onSubmit,
  onDelete,
}: {
  comments: PickyComment[];
  onSubmit: (content: string, parentId?: string) => void;
  onDelete: (c: PickyComment) => void;
}) {
  const { t } = useTranslation();
  const [commentText, setCommentText] = useState("");
  const [replyTo, setReplyTo] = useState<PickyComment | null>(null);

  const submitComment = () => {
    if (!commentText.trim()) return;
    onSubmit(commentText.trim(), replyTo?.id);
    setCommentText("");
    setReplyTo(null);
  };

  const topLevel = comments.filter((c) => !c.parentId);
  const repliesOf = (id: string) => comments.filter((c) => c.parentId === id);

  return (
    <div className="space-y-2">
      {comments.length === 0 && <p className="text-tiny text-slate-600">{t("picky.noComments")}</p>}
      {topLevel.map((c) => (
        <div key={c.id} className="space-y-1.5">
          <CommentRow comment={c} depth={0} onReply={() => setReplyTo(replyTo?.id === c.id ? null : c)} onDelete={() => onDelete(c)} />
          {repliesOf(c.id).map((r) => (
            <CommentRow key={r.id} comment={r} depth={1} onDelete={() => onDelete(r)} />
          ))}
        </div>
      ))}
      <div className="flex items-center gap-2 pt-0.5">
        <input
          value={commentText}
          onChange={(e) => setCommentText(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && submitComment()}
          placeholder={replyTo ? t("picky.replyPh", { text: replyTo.content.slice(0, 20) }) : t("picky.commentPh")}
          className="flex-1 glass-input px-2.5 py-1.5 text-caption bg-black/30 border border-white/10 rounded-ctl focus:outline-none focus:border-[var(--module-accent)]/50"
        />
        {replyTo && (
          <button onClick={() => setReplyTo(null)} className="text-tiny text-slate-500 hover:text-slate-300 cursor-pointer">
            {t("picky.cancel")}
          </button>
        )}
        <button
          onClick={submitComment}
          disabled={!commentText.trim()}
          className="p-1.5 rounded-ctl bg-[var(--module-accent)]/80 text-white cursor-pointer hover:opacity-85 disabled:opacity-40 flex items-center"
        >
          <Send className="w-3 h-3" />
        </button>
      </div>
    </div>
  );
}
