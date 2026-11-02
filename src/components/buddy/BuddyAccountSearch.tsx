import { useCallback, useEffect, useMemo, useState, type Dispatch, type SetStateAction } from "react";
import { Search, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { BuddyAccount } from "./BuddyPanel.model";
import { filterBuddyAccounts, toggleSearchResultSelection } from "./accountSearch";

export function useBuddyAccountSearch(accounts: BuddyAccount[], platform: string,
  selected: Set<string>, setSelected: Dispatch<SetStateAction<Set<string>>>) {
  const [query, setQuery] = useState("");
  const visibleAccounts = useMemo(() => filterBuddyAccounts(accounts, query), [accounts, query]);
  const changeQuery = useCallback((value: string) => {
    setQuery(value);
    setSelected(new Set());
  }, [setSelected]);
  useEffect(() => { changeQuery(""); }, [platform, changeQuery]);
  useEffect(() => {
    const visible = new Set(visibleAccounts.map((account) => account.id));
    setSelected((previous) => {
      const next = new Set([...previous].filter((id) => visible.has(id)));
      return next.size === previous.size ? previous : next;
    });
  }, [visibleAccounts, setSelected]);
  return { query, changeQuery, visibleAccounts,
    allSelected: visibleAccounts.length > 0 && visibleAccounts.every((account) => selected.has(account.id)),
    toggleAll: () => setSelected((previous) => toggleSearchResultSelection(visibleAccounts, previous)),
  };
}

export function BuddyAccountSearch({ query, onChange, shown, total }: {
  query: string; onChange: (query: string) => void; shown: number; total: number;
}) {
  const { t } = useTranslation();
  return <div className="flex items-center gap-3 px-4 py-2 border-b border-white/5 flex-shrink-0">
    <div className="relative flex-1 max-w-md">
      <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-slate-500 pointer-events-none" />
      <input type="search" value={query} onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => { if (event.key === "Escape") onChange(""); }}
        aria-label={t("buddy.accountSearch")} placeholder={t("buddy.accountSearchPlaceholder")}
        className="w-full bg-black/30 border border-white/10 rounded-ctl pl-8 pr-8 py-1.5 text-tiny text-slate-200 placeholder:text-slate-500 outline-none focus:border-[var(--module-accent)]" />
      {query && <button type="button" aria-label={t("buddy.clearAccountSearch")}
        onClick={() => onChange("")}
        className="absolute right-2 top-1/2 -translate-y-1/2 text-slate-400 hover:text-white cursor-pointer">
        <X className="w-3.5 h-3.5" />
      </button>}
    </div>
    <span aria-live="polite" className="text-tiny text-slate-500 tabular-nums">
      {t("buddy.accountSearchCount", { shown, total })}
    </span>
  </div>;
}
