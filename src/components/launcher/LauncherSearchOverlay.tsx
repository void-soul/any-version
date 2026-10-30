import type { WebSearchSource } from "./types";
import type { SearchQuickAction } from "./LauncherPanel.model";
import { Search, Folder, Globe, Shield, FileText, X, Link2, ArrowUpRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Classification, FileHit, Item } from "./types";
import VexEmptyState from "../VexEmptyState";
import type { Dispatch, SetStateAction, RefObject } from "react";

interface LauncherSearchOverlayProps {
  searchInputRef: RefObject<HTMLInputElement | null>;
  searchQuery: string;
  setSearchQuery: Dispatch<SetStateAction<string>>;
  setSearchSelectedIndex: Dispatch<SetStateAction<number>>;
  searchTotal: number;
  searchSelectedIndex: number;
  searchQuickActions: SearchQuickAction[];
  launchVirtual: (name: string, target: string, itemType: number, params?: string | null | undefined, icon?: string | null | undefined) => Promise<void>;
  closeSearch: () => void;
  searchResults: Item[];
  handleExecuteItem: (item: Item) => Promise<void>;
  effectiveFileHits: FileHit[];
  openFileHit: (hit: FileHit) => void;
  webSources: WebSearchSource[];
  classificationMap: Map<number, Classification>;
}


/** Pure LauncherSearchOverlay view; its owning panel retains side effects and state. */
export function LauncherSearchOverlay({ searchInputRef, searchQuery, setSearchQuery, setSearchSelectedIndex, searchTotal, searchSelectedIndex, searchQuickActions, launchVirtual, closeSearch, searchResults, handleExecuteItem, effectiveFileHits, openFileHit, webSources, classificationMap }: LauncherSearchOverlayProps) {
  const { t } = useTranslation();
  return (
<div className="absolute inset-0 z-40 bg-surface-splash/80 backdrop-blur-md flex flex-col items-center p-6 animate-in fade-in zoom-in-95 duration-100">
            <div className="w-full max-w-[600px] h-full flex flex-col">
              {/* Search Input Box */}
              <div className="relative flex items-center mb-3">
                <Search className="w-4 h-4 absolute left-3.5 text-[var(--module-accent)] pointer-events-none" />
                <input
                  ref={searchInputRef}
                  type="text"
                  value={searchQuery}
                  onChange={(e) => {
                    setSearchQuery(e.target.value);
                    setSearchSelectedIndex(0);
                  }}
                  onKeyDown={(e) => {
                    e.stopPropagation();
                    if (e.key === "ArrowDown") {
                      e.preventDefault();
                      setSearchSelectedIndex((prev) => (prev + 1) % (searchTotal || 1));
                    } else if (e.key === "ArrowUp") {
                      e.preventDefault();
                      setSearchSelectedIndex(
                        (prev) => (prev - 1 + (searchTotal || 1)) % (searchTotal || 1)
                      );
                    } else if (e.key === "Enter") {
                      e.preventDefault();
                      const selected = Math.min(
                        searchSelectedIndex,
                        Math.max(0, searchTotal - 1)
                      );
                      const action = searchQuickActions[selected];
                      if (action) {
                        launchVirtual(action.title, action.url, 2);
                        closeSearch();
                      } else {
                        const target = searchResults[selected - searchQuickActions.length];
                        if (target) {
                          handleExecuteItem(target);
                          closeSearch();
                        } else {
                          const hit = effectiveFileHits[selected - searchQuickActions.length - searchResults.length];
                          if (hit) {
                            openFileHit(hit);
                            closeSearch();
                          }
                        }
                      }
                    } else if (e.key === "Escape") {
                      e.preventDefault();
                      closeSearch();
                    }
                  }}
                  placeholder={t("launcher.searchPlaceholder")}
                  className="w-full bg-white/5 border border-[var(--module-accent-ring)] rounded-card pl-10 pr-10 py-2.5 text-sm text-white placeholder-slate-500 focus:outline-none focus:border-[var(--module-accent)] transition shadow-lg shadow-[var(--module-accent-ring)]"
                />
                <button
                  onClick={closeSearch}
                  className="absolute right-3 p-1 text-slate-400 hover:text-white rounded-ctl hover:bg-white/10 transition cursor-pointer"
                  title={t("launcher.closeSearch")}
                >
                  <X className="w-4 h-4" />
                </button>
              </div>

              {/* 搜索引擎快捷入口（输入「关键词 + 空格 + 词」直接搜索） */}
              <div className="flex flex-wrap items-center gap-1.5 mb-2 px-0.5">
                {webSources.map((src) => (
                  <button
                    key={src.id}
                    type="button"
                    onClick={() => {
                      setSearchQuery(`${src.keyword} `);
                      setSearchSelectedIndex(0);
                      searchInputRef.current?.focus();
                    }}
                    title={src.description || src.url}
                    className="px-2 py-0.5 rounded-ctl text-tiny border border-white/10 bg-white/[0.03] text-slate-400 hover:text-white hover:border-[var(--module-accent-ring)] transition cursor-pointer"
                  >
                    <span className="font-mono text-[var(--module-accent)]">{src.keyword}</span>
                    <span className="mx-1 opacity-40">·</span>
                    <span>{src.name}</span>
                  </button>
                ))}
              </div>

              {/* Search Results List */}
              <div className="flex-1 overflow-y-auto space-y-1 pr-1">
              {searchQuickActions.map((action, idx) => {
                const isSelected = idx === searchSelectedIndex;
                return (
                  <button
                    type="button"
                    key={action.key}
                    onClick={() => {
                      launchVirtual(action.title, action.url, 2);
                      closeSearch();
                    }}
                    className={`w-full flex items-center gap-3 px-3.5 py-2.5 rounded-card text-body text-left transition cursor-pointer ${
                      isSelected
                        ? "bg-[var(--module-accent)] text-white font-medium shadow-md shadow-[var(--module-accent-ring)]"
                        : "bg-white/[0.03] text-slate-200 hover:bg-white/5"
                    }`}
                  >
                    <div className={`w-6 h-6 rounded-ctl flex items-center justify-center flex-shrink-0 overflow-hidden ${
                      isSelected ? "bg-black/20 text-white" : "bg-white/10 text-blue-400"
                    }`}>
                      {action.kind === "engine" ? (
                        <Search className="w-3.5 h-3.5" />
                      ) : (
                        <Link2 className="w-3.5 h-3.5" />
                      )}
                    </div>
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-sm">{action.title}</div>
                      <div className={`truncate text-tiny ${isSelected ? "text-white/70" : "text-slate-500"}`}>
                        {action.subtitle}
                      </div>
                    </div>
                    <ArrowUpRight className={`w-3.5 h-3.5 flex-shrink-0 ${isSelected ? "text-white" : "text-slate-500"}`} />
                  </button>
                );
              })}
              {(searchResults.length > 0 || effectiveFileHits.length > 0) ? (
                <>
              {searchResults.length > 0 && searchResults.map((item, idx) => {
                  const isSelected = searchQuickActions.length + idx === searchSelectedIndex;
                  const parentCategory = classificationMap.get(item.classificationId);
                  const parentName = parentCategory ? parentCategory.name : "";

                  return (
                    <div
                      key={item.id}
                      onClick={() => {
                        handleExecuteItem(item);
                        closeSearch();
                      }}
                      className={`flex items-center gap-3 px-3.5 py-2.5 rounded-card text-body transition cursor-pointer ${
                        isSelected
                          ? "bg-[var(--module-accent)] text-white font-medium shadow-md shadow-[var(--module-accent-ring)]"
                          : "text-slate-200 hover:bg-white/5"
                      }`}
                    >
                      {/* Icon */}
                      <div className="w-6 h-6 rounded-ctl bg-white/10 flex items-center justify-center flex-shrink-0 overflow-hidden">
                        {item.data.icon ? (
                          <img src={item.data.icon} className="w-5 h-5 object-contain" alt="" />
                        ) : item.data.htmlIcon ? (
                          <span className="text-body">{item.data.htmlIcon}</span>
                        ) : item.itemType === 1 ? (
                          <Folder className="w-4 h-4 text-amber-400" />
                        ) : item.itemType === 2 ? (
                          <Globe className="w-4 h-4 text-blue-400" />
                        ) : (
                          <FileText className="w-4 h-4 text-[var(--module-accent)]" />
                        )}
                      </div>

                      {/* Name + Parent Category Tag */}
                      <div className="flex items-center gap-2 min-w-0 flex-1">
                        <span className="truncate text-sm">{item.name}</span>
                        {parentName && (
                          <span
                            className={`text-body px-1.5 py-0.5 rounded ${
                              isSelected ? "bg-black/20 text-[var(--module-accent)]" : "bg-white/5 text-slate-400"
                            }`}
                          >
                            ({parentName})
                          </span>
                        )}
                      </div>

                      {item.data.runAsAdmin && (
                        <Shield className={`w-3.5 h-3.5 ${isSelected ? "text-amber-300" : "text-amber-400"}`} />
                      )}
                    </div>
                  );
                })}
              {effectiveFileHits.length > 0 && (
                <>
                  <div className="px-3.5 pb-1 pt-2 text-micro font-medium text-slate-500">
                    {t("launcher.searchFiles")}
                  </div>
                  {effectiveFileHits.map((hit, idx) => {
                    const fileIndex = searchQuickActions.length + searchResults.length + idx;
                    const isSelected = fileIndex === searchSelectedIndex;
                    const dir = hit.path.slice(0, Math.max(0, hit.path.length - hit.name.length));
                    return (
                      <div
                        key={hit.path}
                        onClick={() => {
                          openFileHit(hit);
                          closeSearch();
                        }}
                        className={`flex items-center gap-3 px-3.5 py-2 rounded-card text-body transition cursor-pointer ${
                          isSelected
                            ? "bg-[var(--module-accent)] text-white font-medium shadow-md shadow-[var(--module-accent-ring)]"
                            : "text-slate-200 hover:bg-white/5"
                        }`}
                      >
                        <div className={`w-6 h-6 rounded-ctl flex items-center justify-center flex-shrink-0 overflow-hidden ${isSelected ? "bg-black/20" : "bg-white/10"}`}>
                          {hit.isDir ? (
                            <Folder className={`w-4 h-4 ${isSelected ? "text-white" : "text-amber-400"}`} />
                          ) : (
                            <FileText className={`w-4 h-4 ${isSelected ? "text-white" : "text-[var(--module-accent)]"}`} />
                          )}
                        </div>
                        <div className="min-w-0 flex-1">
                          <div className="truncate text-sm">{hit.name}</div>
                          <div className={`truncate text-micro ${isSelected ? "text-white/70" : "text-slate-500"}`}>
                            {dir}
                          </div>
                        </div>
                        <span className={`shrink-0 rounded px-1.5 py-0.5 font-mono text-micro ${isSelected ? "bg-black/20 text-white/80" : "bg-white/5 text-slate-500"}`}>
                          {hit.drive}
                        </span>
                      </div>
                    );
                  })}
                </>
              )}
                </>
              ) : searchQuery.trim() && searchTotal === 0 ? (
                <VexEmptyState
                  title={t("launcher.searchNoResult", { query: searchQuery })}
                  desc={t("launcher.searchNoResultHint")}
                  avatarSize={44}
                  className="!py-14"
                />
              ) : (
                <div className="py-12 text-center text-slate-500 text-body space-y-1">
                  <p>{t("launcher.searchHint")}</p>
                  <p className="text-caption text-slate-600">{t("launcher.searchHintKeys")}</p>
                </div>
              )}
            </div>
          </div>
        </div>
  );
}
