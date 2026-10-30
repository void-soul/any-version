import type { FavoritesProgress } from "./types";
import { useTranslation } from "react-i18next";
import { RefreshCw } from "lucide-react";
import { ThinkingBlock } from "../shared/ThinkingBlock";
import { SOURCE_LABELS } from "./types";

interface FavoriteTaskProgressProps {
  progressRows: [string, FavoritesProgress][];
}

/** Pure FavoriteTaskProgress view; its owning panel retains side effects and state. */
export function FavoriteTaskProgress({ progressRows }: FavoriteTaskProgressProps) {
  const { t } = useTranslation();
  return (
    <div className="space-y-1">
      {progressRows.map(([key, p]) => (
        <div key={key} className="glass-panel px-3 py-2 space-y-1.5">
          <div className="flex items-center gap-2 text-tiny text-slate-300 flex-wrap">
            <RefreshCw className="w-3 h-3 animate-spin text-[var(--module-accent)]" />
            {p.stage === "check" ? (
              <>
                <span>{t("favorites.checkProgress")}</span>
                <span>
                  {t("favorites.progressChecked", {
                    checked: p.checked ?? 0,
                    total: p.checkTotal ?? 0,
                  })}
                </span>
                <span className="text-slate-500 truncate max-w-[260px]">
                  {p.message ?? ""}
                </span>
              </>
            ) : p.stage === "import" ? (
              <>
                <span>
                  {SOURCE_LABELS[p.source ?? ""] ?? p.source ?? ""}
                  {p.folder ? ` · ${p.folder}` : ""}
                  {p.message ? ` · ${p.message}` : ""}
                </span>
                <span>{t("favorites.progressFetched", { fetched: p.fetched ?? 0 })}</span>
                <span className="text-emerald-400/80">
                  {t("favorites.progressAdded", { added: p.added ?? 0 })}
                </span>
                <span className="text-amber-400/80">
                  {t("favorites.progressUpdated", { updated: p.updated ?? 0 })}
                </span>
                <span className="text-slate-500">
                  {t("favorites.progressSkipped", { skipped: p.skipped ?? 0 })}
                </span>
              </>
            ) : (
              <>
                <span>{p.message}</span>
                <span>
                  {t("favorites.progressClassified", {
                    classified: p.classified ?? 0,
                    remaining: p.remaining ?? 0,
                  })}
                </span>
                <span className="text-slate-400">
                  {t("favorites.progressTags", { tags: p.tagsWritten ?? 0 })}
                </span>
              </>
            )}
          </div>
          {/* 归类过程中模型对这批的真实思考。原来完全没有上报，前端只看到进度数字；
              分类错了也无从查它当时是怎么判断的。
              注意它**不能另起一行**：progressKeyOf 用 task 作行键，reasoning 事件
              与归类事件同 key，另起一个 stage 会让两边互相覆盖。
              所以后端在 reasoning 帧里一并带上分类进度，这里渲染在同一行。 */}
          {p.stage === "reasoning" && !!p.message && (
            <ThinkingBlock text={p.message} pending />
          )}
          <div className="h-1 rounded bg-white/5 overflow-hidden">
            {p.stage === "check" && (p.checkTotal ?? 0) > 0 ? (
              <div
                className="h-full bg-[var(--module-accent)] transition-all duration-300"
                style={{
                  width: `${Math.min(
                    100,
                    ((p.checked ?? 0) / (p.checkTotal ?? 1)) * 100,
                  )}%`,
                }}
              />
            ) : p.stage === "classify" &&
              (p.classified ?? 0) + (p.remaining ?? 0) > 0 ? (
              <div
                className="h-full bg-[var(--module-accent)] transition-all duration-300"
                style={{
                  width: `${Math.min(
                    100,
                    ((p.classified ?? 0) /
                      ((p.classified ?? 0) + (p.remaining ?? 0))) *
                      100,
                  )}%`,
                }}
              />
            ) : p.stage === "import" &&
              (p.folderTotal ?? 0) > 0 &&
              (p.folderFetched ?? 0) > 0 ? (
              // 知乎：服务端给了收藏夹总数（Totals），可以显示真实百分比
              <div
                className="h-full bg-[var(--module-accent)] transition-all duration-300"
                style={{
                  width: `${Math.min(
                    100,
                    ((p.folderFetched ?? 0) / (p.folderTotal ?? 1)) * 100,
                  )}%`,
                }}
              />
            ) : (
              <div className="h-full w-1/3 bg-[var(--module-accent)] animate-pulse" />
            )}
          </div>
        </div>
      ))}
    </div>
  );
}
