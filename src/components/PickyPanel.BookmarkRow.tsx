import { useTranslation } from "react-i18next";
import { Trash2, Pencil, Tag, MessageSquare, ExternalLink, Globe, Archive, ArchiveRestore, RefreshCw, BookOpen } from "lucide-react";
import { type PickyBookmark, type PickyComment } from "./PickyPanel.model";
import { IconBtn } from "./PickyPanel.IconBtn";
import { CommentBlock } from "./PickyPanel.CommentBlock";
// ─── 列表视图：一行一条，仅标题 + 操作按钮（简洁紧凑） ───

export function BookmarkRow(props: {
  bookmark: PickyBookmark;
  comments: PickyComment[];
  expanded: boolean;
  onToggleExpand: () => void;
  onOpen: () => void;
  onEdit: () => void;
  onArchive: () => void;
  onDelete: () => void;
  onTags: () => void;
  onAddComment: (content: string, parentId?: string) => void;
  onDeleteComment: (c: PickyComment) => void;
  onRefetch: () => void;
  onRead: () => void;
}) {
  const { t } = useTranslation();
  const {
    bookmark: b,
    comments,
    expanded,
    onToggleExpand,
    onOpen,
    onEdit,
    onArchive,
    onDelete,
    onTags,
    onAddComment,
    onDeleteComment,
    onRefetch,
    onRead,
  } = props;

  return (
    <div
      className={`rounded-ctl border px-2.5 py-1.5 transition ${
        b.refined ? "bg-white/[0.02] border-white/5" : "bg-white/[0.03] border-white/10 hover:border-white/20"
      }`}
    >
      <div className="flex items-center gap-2 min-w-0">
        <div className="w-5 h-5 rounded bg-[var(--module-accent)]/10 border border-[var(--module-accent)]/20 flex items-center justify-center flex-shrink-0 overflow-hidden">
          {b.faviconUrl ? (
            <img src={b.faviconUrl} alt="" className="w-3.5 h-3.5 object-contain" />
          ) : (
            <Globe className="w-3 h-3 text-[var(--module-accent)]" />
          )}
        </div>
        <span className="text-body font-semibold text-white truncate flex-1 min-w-0" title={b.title}>
          {b.title || t("picky.unnamed")}
        </span>
        {b.refined && (
          <span className="text-[8px] px-1.5 py-0.5 rounded bg-white/10 text-slate-400 flex-shrink-0">{t("picky.archived")}</span>
        )}
        <div className="flex items-center gap-0.5 flex-shrink-0">
          <IconBtn title={t("picky.openLink")} onClick={onOpen}>
            <ExternalLink className="w-3 h-3" />
          </IconBtn>
          <IconBtn title={t("picky.readContent")} onClick={onRead} active={Boolean((b.content ?? "").trim())}>
            <BookOpen className="w-3 h-3" />
          </IconBtn>
          <IconBtn title={expanded ? t("picky.collapseComments") : t("picky.commentsCollapsed", { count: comments.length })} onClick={onToggleExpand} active={expanded}>
            <MessageSquare className="w-3 h-3" />
            <span className="text-micro">{comments.length > 0 ? comments.length : ""}</span>
          </IconBtn>
          <IconBtn title={t("picky.refetch")} onClick={onRefetch}>
            <RefreshCw className="w-3 h-3" />
          </IconBtn>
          <IconBtn title={t("picky.tags")} onClick={onTags}>
            <Tag className="w-3 h-3" />
          </IconBtn>
          <IconBtn title={b.refined ? t("picky.unarchive") : t("picky.archive")} onClick={onArchive}>
            {b.refined ? <ArchiveRestore className="w-3 h-3" /> : <Archive className="w-3 h-3" />}
          </IconBtn>
          <IconBtn title={t("picky.edit")} onClick={onEdit}>
            <Pencil className="w-3 h-3" />
          </IconBtn>
          <IconBtn title={t("picky.delete")} onClick={onDelete} danger>
            <Trash2 className="w-3 h-3" />
          </IconBtn>
        </div>
      </div>

      {/* 评论区 */}
      {expanded && (
        <div className="mt-2 pt-2 border-t border-white/5">
          <CommentBlock comments={comments} onSubmit={onAddComment} onDelete={onDeleteComment} />
        </div>
      )}
    </div>


  );
}
