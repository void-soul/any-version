import { useTranslation } from "react-i18next";
import { Trash2, Pencil, Tag, MessageSquare, ExternalLink, Globe, Archive, ArchiveRestore, RefreshCw, BookOpen } from "lucide-react";
import { fmtTime, hostOf, type PickyBookmark, type PickyComment, type PickyTag } from "./PickyPanel.model";
import { IconBtn } from "./PickyPanel.IconBtn";
import { CommentBlock } from "./PickyPanel.CommentBlock";
// ─── 收藏卡片 ───

export function BookmarkCard({
  bookmark: b,
  tags,
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
}: {
  bookmark: PickyBookmark;
  tags: PickyTag[];
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
  return (
    <div
      className={`rounded-card border p-3 transition ${
        b.refined ? "bg-white/[0.02] border-white/5" : "bg-white/[0.03] border-white/10 hover:border-white/20"
      }`}
    >
      <div className="flex items-start gap-3">
        {/* 图标 */}
        <div className="w-9 h-9 rounded-ctl bg-[var(--module-accent)]/10 border border-[var(--module-accent)]/20 flex items-center justify-center flex-shrink-0 overflow-hidden">
          {b.faviconUrl ? (
            <img src={b.faviconUrl} alt="" className="w-5 h-5 object-contain" />
          ) : (
            <Globe className="w-4 h-4 text-[var(--module-accent)]" />
          )}
        </div>

        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-title font-semibold text-white truncate">{b.title || t("picky.unnamed")}</span>
            {b.refined && (
              <span className="text-[8px] px-1.5 py-0.5 rounded bg-white/10 text-slate-400 flex-shrink-0">{t("picky.archived")}</span>
            )}
          </div>
          {b.url && (
            <button
              onClick={onOpen}
              className="text-tiny text-sky-400/80 hover:text-sky-300 truncate flex items-center gap-1 cursor-pointer max-w-full"
              title={b.url}
            >
              <ExternalLink className="w-2.5 h-2.5 flex-shrink-0" />
              <span className="truncate">{hostOf(b.url)}</span>
            </button>
          )}
          {b.description && <p className="text-tiny text-slate-500 line-clamp-2 mt-0.5">{b.description}</p>}
          <div className="flex items-center gap-2 mt-1 text-micro text-slate-600">
            <span>{t("picky.savedAt", { time: fmtTime(b.createdAt) })}</span>
            {b.updatedAt !== b.createdAt && <span>{t("picky.updatedAt", { time: fmtTime(b.updatedAt) })}</span>}
          </div>

          {/* 标签 */}
          {tags.length > 0 && (
            <div className="flex flex-wrap gap-1 mt-1.5">
              {tags.map((t) => (
                <span
                  key={t.id}
                  className="inline-flex items-center gap-1 text-[8px] px-1.5 py-0.5 rounded"
                  style={{ background: `${t.color}22`, color: t.color, border: `1px solid ${t.color}44` }}
                >
                  #{t.name}
                </span>
              ))}
            </div>
          )}
        </div>

        {/* 操作 */}
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
        <div className="mt-3 pt-3 border-t border-white/5">
          <CommentBlock comments={comments} onSubmit={onAddComment} onDelete={onDeleteComment} />
        </div>
      )}
    </div>
  );
}
