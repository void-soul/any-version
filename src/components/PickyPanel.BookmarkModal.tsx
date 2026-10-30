import { useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { X, Check, Loader2, Globe } from "lucide-react";
import { type PickyBookmark } from "./PickyPanel.model";
// ─── 添加 / 编辑收藏 ───

export function BookmarkModal({
  bookmark,
  busy,
  onSave,
  onClose,
}: {
  bookmark: PickyBookmark | null;
  busy: boolean;
  onSave: (input: { id?: string; title: string; url: string; description: string; imageUrl?: string; faviconUrl?: string }) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [title, setTitle] = useState(bookmark?.title || "");
  const [url, setUrl] = useState(bookmark?.url || "");
  const [description, setDescription] = useState(bookmark?.description || "");
  const [fetching, setFetching] = useState(false);
  // 抓取到的富元数据（保存时随表单传给后端，用于补全图片/favicon）
  const [fetched, setFetched] = useState<{ imageUrl?: string; faviconUrl?: string } | null>(null);

  // 抓取网页元数据（完整链路：应用内隐藏 WebView → 无头渲染 → HTTP）。
  // 原先只走单次 HTTP 请求（launcher_fetch_url_info），JS 渲染页面拿不到真实标题；
  // 现与书签「刷新元数据」共用同一链路（picky_fetch_url_meta）。
  const fetchMeta = async () => {
    if (!url.trim()) return;
    setFetching(true);
    try {
      const meta = await invoke<{ title?: string; description?: string; image_url?: string; favicon_url?: string; source?: string }>("picky_fetch_url_meta", {
        url: url.trim(),
      });
      if (meta?.title && !title.trim()) setTitle(meta.title);
      if (!meta?.title) setTitle(url.trim());
      if (meta?.description && !description.trim()) setDescription(meta.description);
      setFetched({ imageUrl: meta?.image_url, faviconUrl: meta?.favicon_url });
    } catch {
      if (!title.trim()) setTitle(url.trim());
    } finally {
      setFetching(false);
    }
  };

  const submit = () => {
    if (!title.trim() && !url.trim()) return;
    onSave({ id: bookmark?.id, title: title || url, url, description, imageUrl: fetched?.imageUrl, faviconUrl: fetched?.faviconUrl });
  };

  return (
    <div className="fixed inset-0 z-[110] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
      <div
        className="w-[460px] max-w-[95vw] rounded-panel border border-white/10 bg-slate-900/95 shadow-2xl p-5"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-sm font-bold text-white">{bookmark ? t("picky.editBookmark") : t("picky.addBookmark")}</h3>
          <button onClick={onClose} className="p-1 rounded hover:bg-white/10 text-slate-400 cursor-pointer">
            <X className="w-4 h-4" />
          </button>
        </div>
        <div className="space-y-3">
          <div>
            <label className="text-tiny text-slate-400 mb-1 block">{t("picky.titleLabel")}</label>
            <input
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder={t("picky.titlePh")}
              className="w-full glass-input px-3 py-2 text-body bg-black/30 border border-white/10 rounded-ctl focus:outline-none focus:border-[var(--module-accent)]/50"
            />
          </div>
          <div>
            <label className="text-tiny text-slate-400 mb-1 block">URL</label>
            <div className="flex gap-2">
              <input
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                placeholder="https://example.com/article"
                className="flex-1 glass-input px-3 py-2 text-body bg-black/30 border border-white/10 rounded-ctl focus:outline-none focus:border-[var(--module-accent)]/50"
              />
              {!bookmark && (
                <button
                  onClick={fetchMeta}
                  disabled={fetching || !url.trim()}
                  className="px-3 py-2 rounded-ctl text-caption ui-btn text-slate-300 cursor-pointer disabled:opacity-50 flex items-center gap-1 flex-shrink-0"
                >
                  {fetching ? <Loader2 className="w-3 h-3 animate-spin" /> : <Globe className="w-3 h-3" />} {t("picky.fetch")}
                </button>
              )}
            </div>
          </div>
          <div>
            <label className="text-tiny text-slate-400 mb-1 block">{t("picky.descLabel")}</label>
            <textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder={t("picky.descPh")}
              rows={3}
              className="w-full glass-input px-3 py-2 text-body bg-black/30 border border-white/10 rounded-ctl focus:outline-none focus:border-[var(--module-accent)]/50 resize-none"
            />
          </div>
        </div>
        <div className="flex justify-end gap-2 mt-4">
          <button onClick={onClose} className="px-3 py-1.5 rounded-ctl text-caption text-slate-400 hover:bg-white/5 cursor-pointer">
            {t("picky.cancel")}
          </button>
          <button
            onClick={submit}
            disabled={busy || (!title.trim() && !url.trim())}
            className="px-4 py-1.5 rounded-ctl text-caption bg-[var(--module-accent)] text-white font-semibold cursor-pointer hover:opacity-85 disabled:opacity-50 flex items-center gap-1"
          >
            {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />} {t("picky.save")}
          </button>
        </div>
      </div>
    </div>
  );
}
