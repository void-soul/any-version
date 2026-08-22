// Picky 模块面板：收藏 / 归档页面（与 Flutter 端 picky 同一数据接口 + S3 云同步）
import React, { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  Bookmark,
  Plus,
  Search,
  Trash2,
  Pencil,
  Tag,
  MessageSquare,
  ExternalLink,
  X,
  Check,
  Loader2,
  Globe,
  Archive,
  ArchiveRestore,
  Send,
  Reply,
  Cloud,
} from "lucide-react";

// ─── 类型（camelCase，与后端 / Flutter 端一致） ───

interface PickyBookmark {
  id: string;
  title: string;
  description?: string | null;
  url?: string | null;
  imageUrl?: string | null;
  faviconUrl?: string | null;
  createdAt: string;
  updatedAt: string;
  refined: boolean;
  metaFetched: boolean;
  [key: string]: unknown;
}

interface PickyComment {
  id: string;
  bookmarkId: string;
  content: string;
  createdAt: string;
  updatedAt: string;
  parentId?: string | null;
}

interface PickyTag {
  id: string;
  name: string;
  color: string;
  createdAt: string;
}

interface PickyState {
  bookmarks: PickyBookmark[];
  comments: PickyComment[];
  tags: PickyTag[];
  bookmarkTags: Record<string, string[]>;
}

interface PickySyncConfig {
  endpoint?: string | null;
  region: string;
  accessKeyId: string;
  secretAccessKey: string;
  bucketName: string;
  prefix?: string | null;
  enabled: boolean;
  lastSyncAt?: string | null;
  addressingStyle: string;
  tlsVerify: boolean;
  timeoutSeconds: number;
  concurrentReqs: number;
}

// ─── 工具 ───

function fmtTime(iso?: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  return d.toLocaleString("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function hostOf(url?: string | null): string {
  if (!url) return "";
  try {
    return new URL(url).hostname;
  } catch {
    return url;
  }
}

// ─── 面板 ───

type Tab = "all" | "active" | "archived";

export default function PickyPanel() {
  const [state, setState] = useState<PickyState>({
    bookmarks: [],
    comments: [],
    tags: [],
    bookmarkTags: {},
  });
  const [loading, setLoading] = useState(true);
  const [tab, setTab] = useState<Tab>("active");
  const [search, setSearch] = useState("");
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [showAdd, setShowAdd] = useState(false);
  const [editing, setEditing] = useState<PickyBookmark | null>(null);
  const [tagFor, setTagFor] = useState<PickyBookmark | null>(null);
  const [showSync, setShowSync] = useState(false);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const [confirm, setConfirm] = useState<{ title: string; message: string; onConfirm: () => void } | null>(null);

  const load = async () => {
    try {
      const s = await invoke<PickyState>("picky_get_state");
      setState(s);
    } catch (e) {
      console.error("加载 Picky 失败", e);
      setNotice(`加载失败：${e}`);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const flash = (msg: string) => {
    setNotice(msg);
    setTimeout(() => setNotice(""), 3000);
  };

  const toggleExpand = (id: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const commentsOf = (id: string) =>
    state.comments.filter((c) => c.bookmarkId === id).sort((a, b) => a.createdAt.localeCompare(b.createdAt));

  const tagsOf = (id: string): PickyTag[] =>
    (state.bookmarkTags[id] || [])
      .map((tid) => state.tags.find((t) => t.id === tid))
      .filter((t): t is PickyTag => !!t);

  // 筛选
  const filtered = useMemo(() => {
    const kw = search.trim().toLowerCase();
    return state.bookmarks
      .filter((b) => {
        if (tab === "active" && b.refined) return false;
        if (tab === "archived" && !b.refined) return false;
        if (!kw) return true;
        const tags = tagsOf(b.id).map((t) => t.name.toLowerCase());
        return (
          b.title.toLowerCase().includes(kw) ||
          (b.description || "").toLowerCase().includes(kw) ||
          (b.url || "").toLowerCase().includes(kw) ||
          tags.some((t) => t.includes(kw))
        );
      })
      .sort((a, b) => b.createdAt.localeCompare(a.createdAt));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state, tab, search]);

  const counts = useMemo(() => {
    const active = state.bookmarks.filter((b) => !b.refined).length;
    return { all: state.bookmarks.length, active, archived: state.bookmarks.length - active };
  }, [state.bookmarks]);

  // 打开链接
  const openLink = (url?: string | null) => {
    if (url) openUrl(url).catch((e) => flash(`打开链接失败：${e}`));
  };

  // 添加 / 编辑
  const saveBookmark = async (input: {
    id?: string;
    title: string;
    url: string;
    description: string;
    imageUrl?: string;
    faviconUrl?: string;
  }) => {
    setBusy(true);
    try {
      if (input.id) {
        const existing = state.bookmarks.find((b) => b.id === input.id);
        if (!existing) return;
        const updated: PickyBookmark = {
          ...existing,
          title: input.title.trim() || existing.title,
          url: input.url.trim() || existing.url,
          description: input.description.trim() || null,
          imageUrl: input.imageUrl || existing.imageUrl,
          faviconUrl: input.faviconUrl || existing.faviconUrl,
        };
        await invoke("picky_update_bookmark", { bookmark: updated });
      } else {
        await invoke("picky_add_bookmark", {
          title: input.title.trim(),
          url: input.url.trim(),
          description: input.description.trim(),
          imageUrl: input.imageUrl || null,
          faviconUrl: input.faviconUrl || null,
        });
      }
      setShowAdd(false);
      setEditing(null);
      await load();
    } catch (e) {
      flash(`保存失败：${e}`);
    } finally {
      setBusy(false);
    }
  };

  const setRefined = async (b: PickyBookmark, refined: boolean) => {
    try {
      await invoke("picky_set_refined", { id: b.id, refined });
      await load();
    } catch (e) {
      flash(`操作失败：${e}`);
    }
  };

  const removeBookmark = (b: PickyBookmark) => {
    setConfirm({
      title: "删除收藏",
      message: `确定删除「${b.title}」？其评论与标签关联也会一并删除。`,
      onConfirm: async () => {
        await invoke("picky_delete_bookmark", { id: b.id });
        await load();
        flash("已删除");
      },
    });
  };

  // 评论
  const addComment = async (bookmarkId: string, content: string, parentId?: string) => {
    if (!content.trim()) return;
    try {
      await invoke("picky_add_comment", { bookmarkId, content, parentId: parentId || null });
      await load();
    } catch (e) {
      flash(`评论失败：${e}`);
    }
  };

  const removeComment = (c: PickyComment) => {
    setConfirm({
      title: "删除评论",
      message: "确定删除这条评论？（其回复会一并删除）",
      onConfirm: async () => {
        await invoke("picky_delete_comment", { id: c.id });
        await load();
      },
    });
  };

  // 标签
  const toggleTag = async (bookmarkId: string, tagId: string) => {
    await invoke("picky_toggle_bookmark_tag", { bookmarkId, tagId }).catch((e) => flash(`标签操作失败：${e}`));
    await load();
  };

  const createTag = async (name: string) => {
    try {
      await invoke("picky_add_tag", { name });
      await load();
    } catch (e) {
      flash(`新建标签失败：${e}`);
    }
  };

  const removeTag = (t: PickyTag) => {
    setConfirm({
      title: "删除标签",
      message: `确定删除标签「#${t.name}」？`,
      onConfirm: async () => {
        await invoke("picky_delete_tag", { id: t.id });
        await load();
      },
    });
  };

  return (
    <div className="h-full w-full flex flex-col overflow-hidden select-none text-slate-200">
      {/* 头部 */}
      <div className="flex items-center gap-2 px-4 py-3 border-b border-white/5 flex-shrink-0">
        <Bookmark className="w-4 h-4 text-[var(--module-accent)]" />
        <span className="text-sm font-bold text-white">Picky 收藏</span>
        <span className="text-[10px] text-slate-500">{state.bookmarks.length} 条</span>
        <div className="flex-1" />
        <div className="relative">
          <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3 h-3 text-slate-500 pointer-events-none" />
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="搜索标题 / 链接 / 标签"
            className="glass-input pl-7 pr-2 py-1.5 text-xs bg-black/30 border border-white/10 rounded-lg w-52 focus:outline-none focus:border-[var(--module-accent)]/50"
          />
        </div>
        <button
          onClick={() => setShowSync(true)}
          className="px-2.5 py-1.5 rounded-lg text-[11px] bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition"
          title="S3 云同步设置与操作"
        >
          <Cloud className="w-3 h-3" /> 同步
        </button>
        <button
          onClick={() => {
            setEditing(null);
            setShowAdd(true);
          }}
          className="px-2.5 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] hover:opacity-85 text-white font-semibold flex items-center gap-1 cursor-pointer transition"
        >
          <Plus className="w-3 h-3" /> 添加收藏
        </button>
      </div>

      {/* 标签页：全部 / 收藏中 / 已归档 */}
      <div className="flex items-center gap-1 px-4 py-2 border-b border-white/5 flex-shrink-0">
        {(
          [
            ["all", `全部 (${counts.all})`],
            ["active", `收藏中 (${counts.active})`],
            ["archived", `已归档 (${counts.archived})`],
          ] as [Tab, string][]
        ).map(([t, label]) => (
          <button
            key={t}
            onClick={() => setTab(t)}
            className={`px-3 py-1 rounded-lg text-[11px] transition cursor-pointer ${
              tab === t
                ? "bg-[var(--module-accent)]/20 text-white border border-[var(--module-accent)]/30"
                : "text-slate-400 hover:bg-white/5 border border-transparent"
            }`}
          >
            {label}
          </button>
        ))}
      </div>

      {/* 提示条 */}
      {notice && (
        <div className="px-4 py-1.5 bg-[var(--module-accent)]/10 border-b border-[var(--module-accent)]/20 text-[11px] text-[var(--module-accent)] flex-shrink-0">
          {notice}
        </div>
      )}

      {/* 列表 */}
      <div className="flex-1 overflow-y-auto p-4">
        {loading ? (
          <div className="h-full flex items-center justify-center text-slate-500 text-xs gap-2">
            <Loader2 className="w-4 h-4 animate-spin" /> 加载中…
          </div>
        ) : filtered.length === 0 ? (
          <div className="h-full flex flex-col items-center justify-center text-slate-500 gap-3">
            <Globe className="w-10 h-10 text-slate-600" />
            <p className="text-xs">
              {state.bookmarks.length === 0 ? "还没有收藏，点击「添加收藏」开始，或从云端同步" : "没有匹配的收藏"}
            </p>
          </div>
        ) : (
          <div className="max-w-[1000px] mx-auto space-y-2.5">
            {filtered.map((b) => (
              <BookmarkCard
                key={b.id}
                bookmark={b}
                tags={tagsOf(b.id)}
                comments={commentsOf(b.id)}
                expanded={expanded.has(b.id)}
                onToggleExpand={() => toggleExpand(b.id)}
                onOpen={() => openLink(b.url)}
                onEdit={() => {
                  setEditing(b);
                  setShowAdd(true);
                }}
                onArchive={() => setRefined(b, !b.refined)}
                onDelete={() => removeBookmark(b)}
                onTags={() => setTagFor(b)}
                onAddComment={(content, parentId) => addComment(b.id, content, parentId)}
                onDeleteComment={removeComment}
              />
            ))}
          </div>
        )}
      </div>

      {/* 弹窗 */}
      {showAdd && (
        <BookmarkModal
          bookmark={editing}
          busy={busy}
          onSave={saveBookmark}
          onClose={() => {
            setShowAdd(false);
            setEditing(null);
          }}
        />
      )}

      {tagFor && (
        <TagModal
          bookmark={tagFor}
          allTags={state.tags}
          selectedIds={state.bookmarkTags[tagFor.id] || []}
          onToggle={(tid) => toggleTag(tagFor.id, tid)}
          onCreate={createTag}
          onDelete={removeTag}
          onClose={() => setTagFor(null)}
        />
      )}

      {showSync && (
        <SyncModal
          onClose={() => setShowSync(false)}
          onDone={(msg) => {
            // 推送/拉取都会改动本地数据（合并），同步后刷新列表；弹窗保留结果信息
            flash(msg);
            load();
          }}
        />
      )}

      {confirm && (
        <div
          className="fixed inset-0 z-[110] flex items-center justify-center bg-black/70 backdrop-blur-sm p-4"
          onClick={() => setConfirm(null)}
        >
          <div
            className="w-[380px] max-w-[95vw] rounded-2xl border border-white/10 bg-slate-900/95 shadow-2xl p-5"
            onClick={(e) => e.stopPropagation()}
          >
            <h3 className="text-sm font-bold text-white mb-2">{confirm.title}</h3>
            <p className="text-xs text-slate-400 mb-4">{confirm.message}</p>
            <div className="flex justify-end gap-2">
              <button
                onClick={() => setConfirm(null)}
                className="px-3 py-1.5 rounded-lg text-[11px] text-slate-400 hover:bg-white/5 cursor-pointer"
              >
                取消
              </button>
              <button
                onClick={() => {
                  const cb = confirm.onConfirm;
                  setConfirm(null);
                  cb();
                }}
                className="px-3 py-1.5 rounded-lg text-[11px] bg-red-500/90 text-white font-semibold cursor-pointer hover:bg-red-500"
              >
                确定
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

// ─── 收藏卡片 ───

function BookmarkCard({
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
}) {
  const [commentText, setCommentText] = useState("");
  const [replyTo, setReplyTo] = useState<PickyComment | null>(null);

  const submitComment = () => {
    if (!commentText.trim()) return;
    onAddComment(commentText.trim(), replyTo?.id);
    setCommentText("");
    setReplyTo(null);
  };

  const topLevel = comments.filter((c) => !c.parentId);
  const repliesOf = (id: string) => comments.filter((c) => c.parentId === id);

  return (
    <div
      className={`rounded-xl border p-3 transition ${
        b.refined ? "bg-white/[0.02] border-white/5" : "bg-white/[0.03] border-white/10 hover:border-white/20"
      }`}
    >
      <div className="flex items-start gap-3">
        {/* 图标 */}
        <div className="w-9 h-9 rounded-lg bg-[var(--module-accent)]/10 border border-[var(--module-accent)]/20 flex items-center justify-center flex-shrink-0 overflow-hidden">
          {b.faviconUrl ? (
            <img src={b.faviconUrl} alt="" className="w-5 h-5 object-contain" />
          ) : (
            <Globe className="w-4 h-4 text-[var(--module-accent)]" />
          )}
        </div>

        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-[13px] font-semibold text-white truncate">{b.title || "未命名"}</span>
            {b.refined && (
              <span className="text-[8px] px-1.5 py-0.5 rounded bg-white/10 text-slate-400 flex-shrink-0">已归档</span>
            )}
          </div>
          {b.url && (
            <button
              onClick={onOpen}
              className="text-[10px] text-sky-400/80 hover:text-sky-300 truncate flex items-center gap-1 cursor-pointer max-w-full"
              title={b.url}
            >
              <ExternalLink className="w-2.5 h-2.5 flex-shrink-0" />
              <span className="truncate">{hostOf(b.url)}</span>
            </button>
          )}
          {b.description && <p className="text-[10px] text-slate-500 line-clamp-2 mt-0.5">{b.description}</p>}
          <div className="flex items-center gap-2 mt-1 text-[9px] text-slate-600">
            <span>收藏于 {fmtTime(b.createdAt)}</span>
            {b.updatedAt !== b.createdAt && <span>· 更新于 {fmtTime(b.updatedAt)}</span>}
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
          <IconBtn title="打开链接" onClick={onOpen}>
            <ExternalLink className="w-3 h-3" />
          </IconBtn>
          <IconBtn title={expanded ? "收起评论" : `评论 (${comments.length})`} onClick={onToggleExpand} active={expanded}>
            <MessageSquare className="w-3 h-3" />
            <span className="text-[9px]">{comments.length > 0 ? comments.length : ""}</span>
          </IconBtn>
          <IconBtn title="标签" onClick={onTags}>
            <Tag className="w-3 h-3" />
          </IconBtn>
          <IconBtn title={b.refined ? "取消归档（移回收藏）" : "归档（炼化）"} onClick={onArchive}>
            {b.refined ? <ArchiveRestore className="w-3 h-3" /> : <Archive className="w-3 h-3" />}
          </IconBtn>
          <IconBtn title="编辑" onClick={onEdit}>
            <Pencil className="w-3 h-3" />
          </IconBtn>
          <IconBtn title="删除" onClick={onDelete} danger>
            <Trash2 className="w-3 h-3" />
          </IconBtn>
        </div>
      </div>

      {/* 评论区 */}
      {expanded && (
        <div className="mt-3 pt-3 border-t border-white/5 space-y-2">
          {comments.length === 0 && <p className="text-[10px] text-slate-600">还没有评论</p>}
          {topLevel.map((c) => (
            <div key={c.id} className="space-y-1.5">
              <CommentRow comment={c} depth={0} onReply={() => setReplyTo(replyTo?.id === c.id ? null : c)} onDelete={() => onDeleteComment(c)} />
              {repliesOf(c.id).map((r) => (
                <CommentRow key={r.id} comment={r} depth={1} onDelete={() => onDeleteComment(r)} />
              ))}
            </div>
          ))}
          <div className="flex items-center gap-2 pt-1">
            <input
              value={commentText}
              onChange={(e) => setCommentText(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && submitComment()}
              placeholder={replyTo ? `回复「${replyTo.content.slice(0, 20)}」…` : "写评论…"}
              className="flex-1 glass-input px-2.5 py-1.5 text-[11px] bg-black/30 border border-white/10 rounded-lg focus:outline-none focus:border-[var(--module-accent)]/50"
            />
            {replyTo && (
              <button
                onClick={() => setReplyTo(null)}
                className="text-[10px] text-slate-500 hover:text-slate-300 cursor-pointer"
              >
                取消
              </button>
            )}
            <button
              onClick={submitComment}
              disabled={!commentText.trim()}
              className="p-1.5 rounded-lg bg-[var(--module-accent)]/80 text-white cursor-pointer hover:opacity-85 disabled:opacity-40 flex items-center"
            >
              <Send className="w-3 h-3" />
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

function CommentRow({
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
  return (
    <div
      className="rounded-lg bg-white/[0.02] border border-white/5 p-2"
      style={{ marginLeft: depth > 0 ? 20 : 0 }}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="flex-1 min-w-0">
          <p className="text-[11px] text-slate-200 leading-relaxed break-words whitespace-pre-wrap">{comment.content}</p>
          <div className="flex items-center gap-2 mt-1 text-[9px] text-slate-600">
            <span>{fmtTime(comment.createdAt)}</span>
            {depth === 0 && onReply && (
              <button onClick={onReply} className="flex items-center gap-0.5 hover:text-slate-300 cursor-pointer">
                <Reply className="w-2.5 h-2.5" /> 回复
              </button>
            )}
          </div>
        </div>
        <button
          onClick={onDelete}
          className="p-1 rounded text-slate-600 hover:text-red-400 hover:bg-red-500/10 cursor-pointer flex-shrink-0"
          title="删除评论"
        >
          <Trash2 className="w-2.5 h-2.5" />
        </button>
      </div>
    </div>
  );
}

function IconBtn({
  title,
  onClick,
  children,
  danger,
  active,
}: {
  title: string;
  onClick: () => void;
  children: React.ReactNode;
  danger?: boolean;
  active?: boolean;
}) {
  return (
    <button
      title={title}
      onClick={onClick}
      className={`p-1.5 rounded-md transition cursor-pointer flex items-center gap-0.5 ${
        active ? "text-[var(--module-accent)] bg-[var(--module-accent)]/10" : danger ? "text-slate-500 hover:text-red-400 hover:bg-red-500/10" : "text-slate-500 hover:text-white hover:bg-white/10"
      }`}
    >
      {children}
    </button>
  );
}

// ─── 添加 / 编辑收藏 ───

function BookmarkModal({
  bookmark,
  busy,
  onSave,
  onClose,
}: {
  bookmark: PickyBookmark | null;
  busy: boolean;
  onSave: (input: { id?: string; title: string; url: string; description: string }) => void;
  onClose: () => void;
}) {
  const [title, setTitle] = useState(bookmark?.title || "");
  const [url, setUrl] = useState(bookmark?.url || "");
  const [description, setDescription] = useState(bookmark?.description || "");
  const [fetching, setFetching] = useState(false);

  // 抓取网页元数据（标题）
  const fetchMeta = async () => {
    if (!url.trim()) return;
    setFetching(true);
    try {
      const meta = await invoke<{ title?: string; icon?: string; url?: string }>("launcher_fetch_url_info", {
        url: url.trim(),
      });
      if (meta?.title && !title.trim()) setTitle(meta.title);
      if (!meta?.title) setTitle(url.trim());
    } catch {
      if (!title.trim()) setTitle(url.trim());
    } finally {
      setFetching(false);
    }
  };

  const submit = () => {
    if (!title.trim() && !url.trim()) return;
    onSave({ id: bookmark?.id, title: title || url, url, description });
  };

  return (
    <div className="fixed inset-0 z-[110] flex items-center justify-center bg-black/70 backdrop-blur-sm p-4" onClick={onClose}>
      <div
        className="w-[460px] max-w-[95vw] rounded-2xl border border-white/10 bg-slate-900/95 shadow-2xl p-5"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-sm font-bold text-white">{bookmark ? "编辑收藏" : "添加收藏"}</h3>
          <button onClick={onClose} className="p-1 rounded hover:bg-white/10 text-slate-400 cursor-pointer">
            <X className="w-4 h-4" />
          </button>
        </div>
        <div className="space-y-3">
          <div>
            <label className="text-[10px] text-slate-400 mb-1 block">标题</label>
            <input
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="页面标题（留空则自动抓取或使用 URL）"
              className="w-full glass-input px-3 py-2 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none focus:border-[var(--module-accent)]/50"
            />
          </div>
          <div>
            <label className="text-[10px] text-slate-400 mb-1 block">URL</label>
            <div className="flex gap-2">
              <input
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                placeholder="https://example.com/article"
                className="flex-1 glass-input px-3 py-2 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none focus:border-[var(--module-accent)]/50"
              />
              {!bookmark && (
                <button
                  onClick={fetchMeta}
                  disabled={fetching || !url.trim()}
                  className="px-3 py-2 rounded-lg text-[11px] bg-white/5 hover:bg-white/10 border border-white/10 text-slate-300 cursor-pointer disabled:opacity-50 flex items-center gap-1 flex-shrink-0"
                >
                  {fetching ? <Loader2 className="w-3 h-3 animate-spin" /> : <Globe className="w-3 h-3" />} 抓取
                </button>
              )}
            </div>
          </div>
          <div>
            <label className="text-[10px] text-slate-400 mb-1 block">描述（可选）</label>
            <textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="备注 / 摘要"
              rows={3}
              className="w-full glass-input px-3 py-2 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none focus:border-[var(--module-accent)]/50 resize-none"
            />
          </div>
        </div>
        <div className="flex justify-end gap-2 mt-4">
          <button onClick={onClose} className="px-3 py-1.5 rounded-lg text-[11px] text-slate-400 hover:bg-white/5 cursor-pointer">
            取消
          </button>
          <button
            onClick={submit}
            disabled={busy || (!title.trim() && !url.trim())}
            className="px-4 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] text-white font-semibold cursor-pointer hover:opacity-85 disabled:opacity-50 flex items-center gap-1"
          >
            {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />} 保存
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── 标签管理 ───

function TagModal({
  bookmark,
  allTags,
  selectedIds,
  onToggle,
  onCreate,
  onDelete,
  onClose,
}: {
  bookmark: PickyBookmark;
  allTags: PickyTag[];
  selectedIds: string[];
  onToggle: (tagId: string) => void;
  onCreate: (name: string) => void;
  onDelete: (t: PickyTag) => void;
  onClose: () => void;
}) {
  const [newName, setNewName] = useState("");

  const submit = () => {
    if (!newName.trim()) return;
    onCreate(newName.trim());
    setNewName("");
  };

  return (
    <div className="fixed inset-0 z-[110] flex items-center justify-center bg-black/70 backdrop-blur-sm p-4" onClick={onClose}>
      <div
        className="w-[420px] max-w-[95vw] rounded-2xl border border-white/10 bg-slate-900/95 shadow-2xl p-5"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between mb-1">
          <h3 className="text-sm font-bold text-white">标签</h3>
          <button onClick={onClose} className="p-1 rounded hover:bg-white/10 text-slate-400 cursor-pointer">
            <X className="w-4 h-4" />
          </button>
        </div>
        <p className="text-[10px] text-slate-500 mb-3 truncate">「{bookmark.title}」 · 点击标签切换关联</p>

        <div className="flex flex-wrap gap-1.5 min-h-[40px] max-h-[200px] overflow-y-auto mb-3">
          {allTags.length === 0 && <p className="text-[10px] text-slate-600">还没有标签，在下方新建</p>}
          {allTags.map((t) => {
            const on = selectedIds.includes(t.id);
            return (
              <span key={t.id} className="inline-flex items-center gap-1">
                <button
                  onClick={() => onToggle(t.id)}
                  className={`px-2.5 py-1 rounded-full text-[10px] border transition-colors cursor-pointer ${
                    on ? "bg-white/10 border-white/30 text-white" : "bg-white/[0.03] border-white/10 text-slate-400 hover:border-white/25"
                  }`}
                  style={on ? { background: `${t.color}33`, borderColor: `${t.color}66`, color: t.color } : undefined}
                >
                  {on ? "✓ " : ""}#{t.name}
                </button>
                <button
                  onClick={() => onDelete(t)}
                  className="p-0.5 text-slate-600 hover:text-red-400 cursor-pointer"
                  title={`删除标签 #${t.name}`}
                >
                  <Trash2 className="w-2.5 h-2.5" />
                </button>
              </span>
            );
          })}
        </div>

        <div className="flex gap-2">
          <input
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && submit()}
            placeholder="新建标签名"
            className="flex-1 glass-input px-3 py-1.5 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none focus:border-[var(--module-accent)]/50"
          />
          <button
            onClick={submit}
            disabled={!newName.trim()}
            className="px-3 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] text-white font-semibold cursor-pointer hover:opacity-85 disabled:opacity-50 flex items-center gap-1"
          >
            <Plus className="w-3 h-3" /> 新建
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── 云同步设置 ───

function SyncModal({ onClose, onDone }: { onClose: () => void; onDone: (msg: string) => void }) {
  const [cfg, setCfg] = useState<PickySyncConfig | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");

  useEffect(() => {
    invoke<PickySyncConfig>("picky_get_sync_config")
      .then(setCfg)
      .catch((e) => setMsg(`读取配置失败：${e}`));
  }, []);

  if (!cfg) {
    return (
      <ModalShell onClose={onClose} title="云同步（S3）">
        <div className="flex items-center gap-2 text-xs text-slate-500">
          <Loader2 className="w-4 h-4 animate-spin" /> 加载配置…
        </div>
      </ModalShell>
    );
  }

  const set = (k: keyof PickySyncConfig, v: unknown) => setCfg((c) => (c ? { ...c, [k]: v } : c));

  const save = async () => {
    if (!cfg.enabled) {
      setMsg("请先勾选「启用云同步」才能保存参数");
      return;
    }
    setBusy(true);
    setMsg("");
    try {
      await invoke("picky_save_sync_config", { config: cfg });
      setMsg("配置已保存");
    } catch (e) {
      setMsg(`保存失败：${e}`);
    } finally {
      setBusy(false);
    }
  };

  // 一个「同步」按钮 = 内部双向合并：先拉取云端合并到本地，再上传合并结果。
  const syncNow = async () => {
    if (!cfg.enabled) {
      setMsg("请先勾选「启用云同步」再执行同步");
      return;
    }
    setBusy(true);
    setMsg("");
    try {
      await invoke("picky_save_sync_config", { config: cfg });
      const res = await invoke<string>("picky_sync_now");
      setMsg(res);
      onDone(res);
    } catch (e) {
      setMsg(`同步失败：${e}`);
    } finally {
      setBusy(false);
    }
  };

  const field = "w-full glass-input px-3 py-2 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none focus:border-[var(--module-accent)]/50";

  return (
    <ModalShell onClose={onClose} title="云同步（S3 兼容存储）">
      <div className="space-y-3">
        <label className="flex items-center gap-2 text-[11px] text-slate-300 cursor-pointer">
          <input
            type="checkbox"
            checked={cfg.enabled}
            onChange={(e) => set("enabled", e.target.checked)}
            className="accent-[var(--module-accent)]"
          />
          启用云同步
        </label>
        {!cfg.enabled && (
          <p className="text-[10px] text-amber-400/90 bg-amber-400/10 border border-amber-400/20 rounded-lg px-3 py-2">
            ⚠ 云同步未启用：先勾选上方「启用云同步」，才能编辑参数、保存配置和执行同步
          </p>
        )}
        {cfg.lastSyncAt && <p className="text-[10px] text-slate-500">上次同步：{fmtTime(cfg.lastSyncAt)}</p>}

        <div>
          <label className="text-[10px] text-slate-400 mb-1 block">Endpoint（如 https://s3.amazonaws.com 或 MinIO 地址）</label>
          <input value={cfg.endpoint || ""} onChange={(e) => set("endpoint", e.target.value)} placeholder="https://s3.example.com" disabled={!cfg.enabled} className={field} />
        </div>
        <div className="grid grid-cols-2 gap-2">
          <div>
            <label className="text-[10px] text-slate-400 mb-1 block">Region</label>
            <input value={cfg.region} onChange={(e) => set("region", e.target.value)} disabled={!cfg.enabled} className={field} />
          </div>
          <div>
            <label className="text-[10px] text-slate-400 mb-1 block">Bucket</label>
            <input value={cfg.bucketName} onChange={(e) => set("bucketName", e.target.value)} disabled={!cfg.enabled} className={field} />
          </div>
        </div>
        <div>
          <label className="text-[10px] text-slate-400 mb-1 block">AccessKey ID</label>
          <input value={cfg.accessKeyId} onChange={(e) => set("accessKeyId", e.target.value)} disabled={!cfg.enabled} className={field} />
        </div>
        <div>
          <label className="text-[10px] text-slate-400 mb-1 block">SecretKey（加密存储）</label>
          <input
            type="password"
            value={cfg.secretAccessKey}
            onChange={(e) => set("secretAccessKey", e.target.value)}
            disabled={!cfg.enabled}
            className={field}
          />
        </div>
        <div className="grid grid-cols-2 gap-2">
          <div>
            <label className="text-[10px] text-slate-400 mb-1 block">前缀（可选）</label>
            <input value={cfg.prefix || ""} onChange={(e) => set("prefix", e.target.value)} placeholder="picky/" disabled={!cfg.enabled} className={field} />
          </div>
          <div>
            <label className="text-[10px] text-slate-400 mb-1 block">寻址风格</label>
            <select value={cfg.addressingStyle} onChange={(e) => set("addressingStyle", e.target.value)} disabled={!cfg.enabled} className={field}>
              <option value="auto">auto（自动）</option>
              <option value="path">path（路径式）</option>
              <option value="virtual-host">virtual-host（虚拟主机式）</option>
            </select>
          </div>
        </div>
        <label className="flex items-center gap-2 text-[11px] text-slate-300 cursor-pointer">
          <input
            type="checkbox"
            checked={cfg.tlsVerify}
            onChange={(e) => set("tlsVerify", e.target.checked)}
            disabled={!cfg.enabled}
            className="accent-[var(--module-accent)]"
          />
          校验 TLS 证书（自签名 / 内网 http 可关闭）
        </label>

        <p className="text-[10px] text-slate-600 leading-relaxed">
          点击「同步」即完成双向合并：先拉取云端数据合并到本地（按更新时间后写覆盖），再上传合并后的
          全量状态。两端各自增删改后点同步即可，不会互相覆盖、不丢数据。
        </p>

        {msg && <p className="text-[11px] text-[var(--module-accent)] break-words">{msg}</p>}

        <div className="flex justify-end gap-2 pt-1">
          <button
            onClick={save}
            disabled={!cfg.enabled || busy}
            className="px-3 py-1.5 rounded-lg text-[11px] bg-white/5 hover:bg-white/10 border border-white/10 text-slate-300 cursor-pointer disabled:opacity-50"
          >
            保存配置
          </button>
          <button
            onClick={syncNow}
            disabled={!cfg.enabled || busy}
            className="px-4 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] text-white font-semibold cursor-pointer hover:opacity-85 disabled:opacity-50 flex items-center gap-1"
          >
            {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Cloud className="w-3 h-3" />} 同步
          </button>
        </div>
      </div>
    </ModalShell>
  );
}

function ModalShell({ title, onClose, children }: { title: string; onClose: () => void; children: React.ReactNode }) {
  return (
    <div className="fixed inset-0 z-[110] flex items-center justify-center bg-black/70 backdrop-blur-sm p-4" onClick={onClose}>
      <div
        className="w-[460px] max-w-[95vw] max-h-[90vh] overflow-y-auto rounded-2xl border border-white/10 bg-slate-900/95 shadow-2xl p-5"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-sm font-bold text-white flex items-center gap-2">
            <Cloud className="w-4 h-4 text-[var(--module-accent)]" /> {title}
          </h3>
          <button onClick={onClose} className="p-1 rounded hover:bg-white/10 text-slate-400 cursor-pointer">
            <X className="w-4 h-4" />
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}
