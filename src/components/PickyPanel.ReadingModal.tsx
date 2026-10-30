import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { X, Loader2, RefreshCw, BookOpen } from "lucide-react";
import { hostOf, type PickyBookmark } from "./PickyPanel.model";
// ─── 阅读正文弹窗：展示书签抓取到的正文（纯文本），支持加载/空态/刷新 ───

export function ReadingModal({
  bookmark: b,
  loading,
  onClose,
  onRefetch,
}: {
  bookmark: PickyBookmark;
  loading: boolean;
  onClose: () => void;
  onRefetch: () => void;
}) {
  const { t } = useTranslation();
  const content = (b.content ?? "").trim();
  const wordCount = content ? content.replace(/\s+/g, " ").trim().split(" ").length : 0;
  return (
    <div className="fixed inset-0 z-[110] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
      <div
        className="w-[720px] max-w-[95vw] max-h-[90vh] flex flex-col rounded-panel border border-white/10 bg-slate-900/95 shadow-2xl overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 头部：标题 + 操作 */}
        <div className="flex items-center gap-2 border-b border-white/10 px-4 py-3 flex-shrink-0">
          <BookOpen className="w-4 h-4 text-[var(--module-accent)] flex-shrink-0" />
          <div className="min-w-0 flex-1">
            <div className="text-sm font-bold text-white truncate" title={b.title}>{b.title || t("picky.unnamed")}</div>
            {b.url && (
              <button onClick={() => void openUrl(b.url!)} className="text-tiny text-sky-400/80 hover:text-sky-300 truncate block max-w-full cursor-pointer" title={b.url}>
                {hostOf(b.url)}
              </button>
            )}
          </div>
          {content && !loading && (
            <span className="text-micro text-slate-500 flex-shrink-0">{t("picky.contentWords", { count: wordCount })}</span>
          )}
          <button onClick={onRefetch} disabled={loading}
            className="p-1.5 rounded-ctl hover:bg-white/10 text-slate-400 hover:text-white disabled:opacity-40 cursor-pointer flex-shrink-0"
            title={t("picky.readRefetch")}>
            <RefreshCw className={`w-4 h-4 ${loading ? "animate-spin" : ""}`} />
          </button>
          <button onClick={onClose} className="p-1.5 rounded-ctl hover:bg-white/10 text-slate-400 hover:text-white cursor-pointer flex-shrink-0">
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* 正文区 */}
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          {loading ? (
            <div className="flex flex-col items-center justify-center gap-3 py-16 text-slate-500">
              <Loader2 className="w-6 h-6 animate-spin text-[var(--module-accent)]" />
              <span className="text-body">{t("picky.contentLoading")}</span>
            </div>
          ) : content ? (
            <article className="text-title leading-7 text-slate-200 whitespace-pre-wrap break-words font-[system-ui]">
              {content}
            </article>
          ) : (
            <div className="flex flex-col items-center justify-center gap-3 py-16 text-slate-500">
              <BookOpen className="w-8 h-8 opacity-40" />
              <span className="text-body text-center">{t("picky.contentEmpty")}</span>
              {b.url && (
                <button onClick={onRefetch}
                  className="mt-1 px-3 py-1.5 rounded-ctl text-caption bg-[var(--module-accent)]/15 border border-[var(--module-accent)]/30 text-[var(--module-accent)] hover:bg-[var(--module-accent)]/25 cursor-pointer">
                  {t("picky.readRefetch")}
                </button>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
