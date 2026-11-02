import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Check, Loader2, Globe } from "lucide-react";
import { ModalShell } from "./PickyPanel.ModalShell";
import { btnPrimary, btnSecondary, inputCls, labelCls } from "./shared/Button";
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

  const request = useRef(0);
  const fetchInFlight = useRef(false);
  useEffect(() => () => { request.current++; }, []);
  // 抓取网页元数据（完整链路：应用内隐藏 WebView → 无头渲染 → HTTP）。
  // 原先只走单次 HTTP 请求（launcher_fetch_url_info），JS 渲染页面拿不到真实标题；
  // 现与书签「刷新元数据」共用同一链路（picky_fetch_url_meta）。
  const fetchMeta = async () => {
    if (!url.trim() || busy || fetchInFlight.current) return;
    const generation = ++request.current;
    const fetchUrl = url.trim();
    fetchInFlight.current = true;
    setFetching(true);
    try {
      const meta = await invoke<{ title?: string; description?: string; image_url?: string; favicon_url?: string; source?: string }>("picky_fetch_url_meta", {
        url: fetchUrl,
      });
      if (generation !== request.current) return;
      setTitle((previous) => previous.trim() ? previous : (meta?.title || fetchUrl));
      if (meta?.description) setDescription((previous) => previous.trim() ? previous : meta.description!);
      setFetched({ imageUrl: meta?.image_url, faviconUrl: meta?.favicon_url });
    } catch {
      if (generation === request.current) setTitle((previous) => previous.trim() ? previous : fetchUrl);
    } finally {
      fetchInFlight.current = false;
      if (generation === request.current) setFetching(false);
    }
  };

  const submit = () => {
    if (busy || fetching || (!title.trim() && !url.trim())) return;
    onSave({ id: bookmark?.id, title: title || url, url, description, imageUrl: fetched?.imageUrl, faviconUrl: fetched?.faviconUrl });
  };

  return (
    <ModalShell title={bookmark ? t("picky.editBookmark") : t("picky.addBookmark")} onClose={onClose} busy={busy}>
      <fieldset disabled={busy} className="min-w-0" aria-busy={busy}>
        <div className="space-y-3">
          <div>
            <label className={labelCls}>{t("picky.titleLabel")}</label>
            <input
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder={t("picky.titlePh")}
              className={inputCls}
            />
          </div>
          <div>
            <label className={labelCls}>URL</label>
            <div className="flex gap-2">
              <input
                value={url}
                disabled={fetching}
                onChange={(e) => { setFetched(null); setUrl(e.target.value); }}
                placeholder="https://example.com/article"
                className={`${inputCls} flex-1 min-w-0`}
              />
              {!bookmark && (
                <button
                  onClick={fetchMeta}
                  disabled={fetching || !url.trim()}
                  className={btnSecondary}
                >
                  {fetching ? <Loader2 className="w-3 h-3 animate-spin" /> : <Globe className="w-3 h-3" />} {t("picky.fetch")}
                </button>
              )}
            </div>
          </div>
          <div>
            <label className={labelCls}>{t("picky.descLabel")}</label>
            <textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder={t("picky.descPh")}
              rows={3}
              className="ui-input w-full px-2.5 py-2 resize-none"
            />
          </div>
        </div>
        <div className="flex justify-end gap-2 mt-4">
          <button onClick={onClose} className={btnSecondary}>
            {t("picky.cancel")}
          </button>
          <button
            onClick={submit}
            disabled={busy || fetching || (!title.trim() && !url.trim())}
            className={btnPrimary}
          >
            {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />} {t("picky.save")}
          </button>
        </div>
      </fieldset>
    </ModalShell>
  );
}
