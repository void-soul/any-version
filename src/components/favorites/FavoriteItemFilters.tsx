import { useTranslation } from "react-i18next";
import { Search } from "lucide-react";
import { type SincePreset } from "./favoritedTime";
import { SOURCE_LABELS, type FavoriteStats } from "./types";
import type { Dispatch, SetStateAction } from "react";

interface FavoriteItemFiltersProps {
  keyword: string;
  setKeyword: Dispatch<SetStateAction<string>>;
  source: string | null;
  setSource: Dispatch<SetStateAction<string | null>>;
  stats: FavoriteStats | null;
  since: SincePreset;
  setSince: Dispatch<SetStateAction<SincePreset>>;
}

/** Pure FavoriteItemFilters view; its owning panel retains side effects and state. */
export function FavoriteItemFilters({ keyword, setKeyword, source, setSource, stats, since, setSince }: FavoriteItemFiltersProps) {
  const { t } = useTranslation();
  return (
    <div className="ml-auto flex items-center gap-2">
      <div className="flex items-center gap-1 glass-input px-2 h-7">
        <Search className="w-3 h-3 text-slate-500" />
        <input
          value={keyword}
          onChange={(e) => setKeyword(e.target.value)}
          placeholder={t("favorites.searchPlaceholder")}
          className="bg-transparent outline-none text-caption w-36"
        />
      </div>
      <select
        value={source ?? ""}
        onChange={(e) => setSource(e.target.value || null)}
        className="glass-input px-2 h-7 text-caption cursor-pointer"
      >
        <option value="">{t("favorites.allSources")}</option>
        {(stats?.bySource || []).map(([key]) => (
          <option key={key} value={key}>
            {SOURCE_LABELS[key] ?? key}
          </option>
        ))}
      </select>
      {/* 收藏时间过滤 */}
      <select
        value={since}
        onChange={(e) => setSince(e.target.value as SincePreset)}
        className="glass-input px-2 h-7 text-caption cursor-pointer"
        title={t("favorites.sinceTip")}
      >
        <option value="all">{t("favorites.sinceAll")}</option>
        <option value="7d">{t("favorites.since7d")}</option>
        <option value="30d">{t("favorites.since30d")}</option>
        <option value="365d">{t("favorites.since365d")}</option>
      </select>
    </div>
  );
}
