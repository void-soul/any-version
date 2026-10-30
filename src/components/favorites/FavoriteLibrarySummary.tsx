import { useTranslation } from "react-i18next";
import { type FavoriteDeletedRow, type FavoriteStats } from "./types";
import type { Dispatch, SetStateAction } from "react";

interface FavoriteLibrarySummaryProps {
  stats: FavoriteStats | null;
  goneCount: number;
  categoryId: number | null;
  categoryName: string | null;
  setCategoryId: Dispatch<SetStateAction<number | null>>;
  setCategoryName: Dispatch<SetStateAction<string | null>>;
  deletedRows: FavoriteDeletedRow[];
  setDeletedOpen: Dispatch<SetStateAction<boolean>>;
}

/** Pure FavoriteLibrarySummary view; its owning panel retains side effects and state. */
export function FavoriteLibrarySummary({ stats, goneCount, categoryId, categoryName, setCategoryId, setCategoryName, deletedRows, setDeletedOpen }: FavoriteLibrarySummaryProps) {
  const { t } = useTranslation();
  return (
    <div className="flex items-center gap-3 text-tiny text-slate-500">
      <span>
        {t("favorites.total")}: {stats?.total ?? 0}
      </span>
      <span>
        {t("favorites.unclassified")}: {stats?.unclassified ?? 0}
      </span>
      {goneCount > 0 && (
        <span className="text-rose-400/80">
          {t("favorites.goneCount")}: {goneCount}
        </span>
      )}
      {/* 当前分类筛选（含子分类）：点了分类树就得让用户知道自己「在哪一层」 */}
      {categoryId != null && categoryName && (
        <button
          onClick={() => { setCategoryId(null); setCategoryName(null); }}
          className="px-1.5 py-0.5 rounded-full bg-[var(--module-accent-soft)] text-[var(--module-accent)] cursor-pointer"
          title={t("favorites.clearCategoryFilter")}
        >
          {categoryName} ✕
        </button>
      )}
      {/* 删除墓碑：让用户知道删掉的条目被记住了，且可以反悔 */}
      {deletedRows.length > 0 && (
        <button
          onClick={() => setDeletedOpen(true)}
          className="px-1.5 py-0.5 rounded-full border border-white/10 text-slate-400 hover:text-[var(--module-accent)] cursor-pointer"
          title={t("favorites.deletedHint")}
        >
          {t("favorites.deletedCount", { count: deletedRows.length })}
        </button>
      )}
      <span className="ml-auto">{t("favorites.readonlyHint")}</span>
    </div>
  );
}
