import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Bookmark, Plus, Search, Loader2, Globe, Cloud, List, LayoutGrid } from "lucide-react";
import { type PickyBookmark, type PickyComment, type PickyState, type PickyTag, type Tab } from "./PickyPanel.model";
import { BookmarkRow } from "./PickyPanel.BookmarkRow";
import { BookmarkCard } from "./PickyPanel.BookmarkCard";
import { BookmarkModal } from "./PickyPanel.BookmarkModal";
import { TagModal } from "./PickyPanel.TagModal";
import { SyncModal } from "./PickyPanel.SyncModal";
import { ReadingModal } from "./PickyPanel.ReadingModal";
export default function PickyPanel() {
  const { t } = useTranslation();
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
  const [viewMode, setViewMode] = useState<"row" | "grid">("row");
  const [showAdd, setShowAdd] = useState(false);
  const [editing, setEditing] = useState<PickyBookmark | null>(null);
  const [tagFor, setTagFor] = useState<PickyBookmark | null>(null);
  const [showSync, setShowSync] = useState(false);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const [confirm, setConfirm] = useState<{ title: string; message: string; onConfirm: () => void } | null>(null);
  // 阅读正文弹窗：reading 为当前打开的书签；readingLoading 表示正文正在后台抓取
  const [reading, setReading] = useState<PickyBookmark | null>(null);
  const [readingLoading, setReadingLoading] = useState(false);

  // 后端为兼容 Flutter 云同步（state.json 用 0/1），序列化 refined/metaFetched 为整数；
  // 前端归一化为真实布尔，否则 {b.refined && <jsx/>} 会把数字 0 直接渲染成 "0"。
  const asBool = (v: unknown): boolean => v === true || v === 1 || v === "1" || v === "true";

  const load = async () => {
    try {
      const s = await invoke<PickyState>("picky_get_state");
      setState({
        ...s,
        bookmarks: s.bookmarks.map((b) => ({ ...b, refined: asBool(b.refined), metaFetched: asBool(b.metaFetched) })),
      });
    } catch (e) {
      console.error("加载 Picky 失败", e);
      setNotice(t("picky.loadFail", { err: String(e) }));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  // 后台自动补全元数据完成 → 刷新列表（新增收藏后无需手动点「刷新元数据」）
  const loadRef = useRef(load);
  useEffect(() => { loadRef.current = load; });
  useEffect(() => {
    const un = listen<string>("picky-bookmark-updated", () => { loadRef.current(); });
    return () => { void un.then((f) => f()); };
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
    if (url) openUrl(url).catch((e) => flash(t("picky.openLinkFail", { err: String(e) })));
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
      flash(t("picky.saveFail", { err: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  const setRefined = async (b: PickyBookmark, refined: boolean) => {
    try {
      await invoke("picky_set_refined", { id: b.id, refined });
      await load();
    } catch (e) {
      flash(t("picky.opFail", { err: String(e) }));
    }
  };

  const refetchBookmark = async (b: PickyBookmark) => {
    try {
      flash(t("picky.refetching", { title: b.title }));
      await invoke("picky_refetch_metadata", { id: b.id });
      await load();
      flash(t("picky.refetched", { title: b.title }));
    } catch (e) {
      flash(t("picky.refetchFail", { err: String(e) }));
    }
  };

  // 打开「阅读正文」：已抓过正文直接显示；否则后台走抓取链路（与刷新元数据同源，
  // 会一并补全标题/描述/图片/正文）拉取并保存，完成后把最新正文回填到弹窗。
  const openReading = async (b: PickyBookmark) => {
    setReading(b);
    if ((b.content ?? "").trim()) return; // 已有正文，直接展示
    if (!b.url) {
      // 无 URL 无法抓取：保持弹窗展示「无正文」空态
      return;
    }
    setReadingLoading(true);
    try {
      const updated = await invoke<PickyBookmark>("picky_refetch_metadata", { id: b.id });
      await load();
      // 用最新抓取结果回填弹窗（content 可能仍为空 = 站点未渲染出正文）
      setReading({ ...updated, refined: asBool(updated.refined), metaFetched: asBool(updated.metaFetched) });
    } catch (e) {
      flash(t("picky.contentFetchFail", { err: String(e) }));
    } finally {
      setReadingLoading(false);
    }
  };

  const removeBookmark = (b: PickyBookmark) => {
    setConfirm({
      title: t("picky.delBookmarkTitle"),
      message: t("picky.delBookmarkMsg", { title: b.title }),
      onConfirm: async () => {
        await invoke("picky_delete_bookmark", { id: b.id });
        await load();
        flash(t("picky.deleted"));
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
      flash(t("picky.commentFail", { err: String(e) }));
    }
  };

  const removeComment = (c: PickyComment) => {
    setConfirm({
      title: t("picky.delCommentTitle"),
      message: t("picky.delCommentMsg"),
      onConfirm: async () => {
        await invoke("picky_delete_comment", { id: c.id });
        await load();
      },
    });
  };

  // 标签
  const toggleTag = async (bookmarkId: string, tagId: string) => {
    await invoke("picky_toggle_bookmark_tag", { bookmarkId, tagId }).catch((e) => flash(t("picky.tagOpFail", { err: String(e) })));
    await load();
  };

  const createTag = async (name: string) => {
    try {
      await invoke("picky_add_tag", { name });
      await load();
    } catch (e) {
      flash(t("picky.newTagFail", { err: String(e) }));
    }
  };

  const removeTag = (tag: PickyTag) => {
    setConfirm({
      title: t("picky.delTagTitle"),
      message: t("picky.delTagMsg", { name: tag.name }),
      onConfirm: async () => {
        await invoke("picky_delete_tag", { id: tag.id });
        await load();
      },
    });
  };

  return (
    <div className="h-full w-full flex flex-col overflow-hidden select-none text-slate-200">
      {/* 头部 */}
      <div className="flex items-center gap-2 px-4 py-3 border-b border-white/5 flex-shrink-0">
        <Bookmark className="w-4 h-4 text-[var(--module-accent)]" />
        <span className="text-sm font-bold text-white">{t("picky.title")}</span>
        <span className="text-tiny text-slate-500">{t("picky.count", { count: state.bookmarks.length })}</span>
        <div className="flex-1" />
        <div className="relative">
          <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3 h-3 text-slate-500 pointer-events-none" />
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t("picky.searchPh")}
            className="glass-input pl-7 pr-2 py-1.5 text-body bg-black/30 border border-white/10 rounded-ctl w-52 focus:outline-none focus:border-[var(--module-accent)]/50"
          />
        </div>
        {/* 视图切换：列表（一行一条简略） / 块状（一行多条信息块） */}
        <div className="flex items-center gap-0.5 border border-white/10 rounded-ctl p-0.5 flex-shrink-0">
          <button
            onClick={() => setViewMode("row")}
            title={t("picky.listViewTitle")}
            className={`p-1.5 rounded-md transition cursor-pointer flex items-center ${
              viewMode === "row" ? "bg-[var(--module-accent)]/20 text-[var(--module-accent)]" : "text-slate-500 hover:text-white hover:bg-white/10"
            }`}
          >
            <List className="w-3.5 h-3.5" />
          </button>
          <button
            onClick={() => setViewMode("grid")}
            title={t("picky.gridViewTitle")}
            className={`p-1.5 rounded-md transition cursor-pointer flex items-center ${
              viewMode === "grid" ? "bg-[var(--module-accent)]/20 text-[var(--module-accent)]" : "text-slate-500 hover:text-white hover:bg-white/10"
            }`}
          >
            <LayoutGrid className="w-3.5 h-3.5" />
          </button>
        </div>
        <button
          onClick={() => setShowSync(true)}
          className="px-2.5 py-1.5 rounded-ctl text-caption bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition"
          title={t("picky.syncTitle")}
        >
          <Cloud className="w-3 h-3" /> {t("picky.sync")}
        </button>
        <button
          onClick={() => {
            setEditing(null);
            setShowAdd(true);
          }}
          className="px-2.5 py-1.5 rounded-ctl text-caption ui-btn-primary text-white font-semibold flex items-center gap-1 cursor-pointer transition"
        >
          <Plus className="w-3 h-3" /> {t("picky.add")}
        </button>
      </div>

      {/* 标签页：全部 / 收藏中 / 已归档 */}
      <div className="flex items-center gap-1 px-4 py-2 border-b border-white/5 flex-shrink-0">
        {(
          [
            ["all", t("picky.tabAll", { count: counts.all })],
            ["active", t("picky.tabActive", { count: counts.active })],
            ["archived", t("picky.tabArchived", { count: counts.archived })],
          ] as [Tab, string][]
        ).map(([t, label]) => (
          <button
            key={t}
            onClick={() => setTab(t)}
            className={`px-3 py-1 rounded-ctl text-caption transition cursor-pointer ${
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
        <div className="px-4 py-1.5 bg-[var(--module-accent)]/10 border-b border-[var(--module-accent)]/20 text-caption text-[var(--module-accent)] flex-shrink-0">
          {notice}
        </div>
      )}

      {/* 列表 */}
      <div className="flex-1 overflow-y-auto p-4">
        {loading ? (
          <div className="h-full flex items-center justify-center text-slate-500 text-body gap-2">
            <Loader2 className="w-4 h-4 animate-spin" /> {t("picky.loading")}
          </div>
        ) : filtered.length === 0 ? (
          <div className="h-full flex flex-col items-center justify-center text-slate-500 gap-3">
            <Globe className="w-10 h-10 text-slate-600" />
            <p className="text-body">
              {state.bookmarks.length === 0 ? t("picky.emptyAll") : t("picky.emptyFiltered")}
            </p>
          </div>
        ) : (
          <div
            className={
              viewMode === "row"
                ? "flex flex-col gap-1.5"
                : "grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4 gap-3"
            }
          >
            {filtered.map((b) =>
              viewMode === "row" ? (
                <BookmarkRow
                  key={b.id}
                  bookmark={b}
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
                  onRefetch={() => refetchBookmark(b)}
                  onRead={() => void openReading(b)}
                />
              ) : (
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
                  onRefetch={() => refetchBookmark(b)}
                  onRead={() => void openReading(b)}
                />
              )
            )}
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

      {reading && (
        <ReadingModal
          bookmark={reading}
          loading={readingLoading}
          onClose={() => setReading(null)}
          onRefetch={() => void openReading(reading)}
        />
      )}

      {confirm && (
        <div
          className="fixed inset-0 z-[110] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4"
        >
          <div
            className="w-[380px] max-w-[95vw] rounded-panel border border-white/10 bg-slate-900/95 shadow-2xl p-5"
            onClick={(e) => e.stopPropagation()}
          >
            <h3 className="text-sm font-bold text-white mb-2">{confirm.title}</h3>
            <p className="text-body text-slate-400 mb-4">{confirm.message}</p>
            <div className="flex justify-end gap-2">
              <button
                onClick={() => setConfirm(null)}
                className="px-3 py-1.5 rounded-ctl text-caption text-slate-400 hover:bg-white/5 cursor-pointer"
              >
                {t("picky.cancel")}
              </button>
              <button
                onClick={() => {
                  const cb = confirm.onConfirm;
                  setConfirm(null);
                  cb();
                }}
                className="px-3 py-1.5 rounded-ctl text-caption bg-red-500/90 text-white font-semibold cursor-pointer hover:bg-red-500"
              >
                {t("picky.confirm")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
