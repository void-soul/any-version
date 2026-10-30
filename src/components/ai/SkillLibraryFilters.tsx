import { useTranslation } from 'react-i18next';
import { Search, X, ChevronDown, Filter } from 'lucide-react';
import type { Dispatch, SetStateAction } from "react";

interface SkillLibraryFiltersProps {
  search: string;
  setSearch: Dispatch<SetStateAction<string>>;
  selCat: string | null;
  setSelCat: Dispatch<SetStateAction<string | null>>;
  allCats: string[];
  hasFilter: string | boolean;
  clearFilter: () => void;
}

/** Pure SkillLibraryFilters view; its owning panel retains side effects and state. */
export function SkillLibraryFilters({ search, setSearch, selCat, setSelCat, allCats, hasFilter, clearFilter }: SkillLibraryFiltersProps) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-wrap items-center gap-2">
      <div className="relative flex-1 min-w-[200px]">
        <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-slate-500" />
        <input
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder={t("skillmgr.searchPh")}
          className="w-full pl-8 pr-2 py-1.5 rounded-ctl bg-white/5 border border-white/10 text-body text-slate-200 placeholder-slate-500 focus:outline-none focus:border-[var(--module-accent-ring)]"
        />
      </div>
      <div className="relative">
        <select
          value={selCat ?? ''}
          onChange={(e) => setSelCat(e.target.value || null)}
          className="appearance-none pl-7 pr-7 py-1.5 rounded-ctl bg-slate-800 border border-white/10 text-body text-slate-200 focus:outline-none focus:border-[var(--module-accent-ring)] cursor-pointer"
        >
          <option value="">{t("skillmgr.allCategories")}</option>
          {allCats.map((c) => <option key={c} value={c}>{c}</option>)}
        </select>
        <Filter className="absolute left-2 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-slate-500" />
        <ChevronDown className="absolute right-2 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-slate-500 pointer-events-none" />
      </div>
      {hasFilter && (
        <button onClick={clearFilter} className="px-2 py-1.5 rounded-ctl bg-white/5 hover:bg-white/10 text-tiny text-slate-400 flex items-center gap-1 cursor-pointer">
          <X className="w-3 h-3" /> {t("skillmgr.clearFilter")}
        </button>
      )}
    </div>
  );
}
