import type { WebSearchSource } from "./types";
import type { SearchQuickAction } from "./LauncherPanel.model";
import { useState, useEffect, useMemo, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { Classification, FileHit, Item } from "./types";
import { matchPinyin } from "./pinyin";
import { looksLikeUrl, normalizeUrl } from "./LauncherPanel.model";
import type { Dispatch, SetStateAction, RefObject } from "react";

/** Owns the LauncherSearch feature, independently of page rendering. */
export function useLauncherSearch({ classificationMap, searchQuery, webSources, visibleItems, isSearchOpen, setIsSearchOpen, setSearchQuery, setSearchSelectedIndex, searchInputRef }: {
  classificationMap: Map<number, Classification>;
  searchQuery: string;
  webSources: WebSearchSource[];
  visibleItems: Item[];
  isSearchOpen: boolean;
  setIsSearchOpen: Dispatch<SetStateAction<boolean>>;
  setSearchQuery: Dispatch<SetStateAction<string>>;
  setSearchSelectedIndex: Dispatch<SetStateAction<number>>;
  searchInputRef: RefObject<HTMLInputElement | null>;
}) {
  const { t } = useTranslation();

  // 全盘文件索引搜索结果（与启动项一起参与统一搜索）
  const [fileHits, setFileHits] = useState<FileHit[]>([]);

  // 这批结果对应的关键词：与当前输入不一致时视为过期，避免回车打开错文件
  const [fileHitsQuery, setFileHitsQuery] = useState("");

  const fileSearchReqRef = useRef(0);


  /** 分类（含祖先）是否在「全局快速搜索」中被排除。 */
  const isExcludedFromSearch = useCallback(
    (classificationId: number) => {
      let node = classificationMap.get(classificationId);
      let guard = 0;
      while (node && guard++ < 100) {
        if (node.data?.excludeSearch) return true;
        node = node.parentId != null ? classificationMap.get(node.parentId) : undefined;
      }
      return false;
    },
    [classificationMap]
  );


  // 搜索快捷动作：网址直达 + 搜索引擎前缀（如「bd 关键词」）
  const searchQuickActions = useMemo<SearchQuickAction[]>(() => {
    const raw = searchQuery.trim();
    if (!raw) return [];
    const actions: SearchQuickAction[] = [];
    if (looksLikeUrl(raw)) {
      const url = normalizeUrl(raw);
      actions.push({
        key: `url:${url}`,
        title: url,
        subtitle: t("launcher.searchOpenUrl"),
        url,
        kind: "url",
      });
    }
    const space = raw.search(/\s/);
    if (space > 0) {
      const prefix = raw.slice(0, space).toLowerCase();
      const term = raw.slice(space + 1).trim();
      const source = webSources.find((s) => s.keyword.toLowerCase() === prefix);
      if (source && term) {
        const url = source.url.includes("%s")
          ? source.url.replace("%s", encodeURIComponent(term))
          : `${source.url}${encodeURIComponent(term)}`;
        actions.push({
          key: `engine:${source.id}:${term}`,
          title: t("launcher.searchWithEngine", { engine: source.name, query: term }),
          subtitle: source.url,
          url,
          kind: "engine",
        });
      }
    }
    return actions;
  }, [searchQuery, webSources, t]);


  // Unified Search Filtering (Figure 2)
  // 支持：子串匹配（名称/目标/备注）+ 拼音首字母匹配（如 vsc → Visual Studio Code）
  const searchResults = useMemo(() => {
    if (!searchQuery.trim()) return [];
    const q = searchQuery.trim().toLowerCase();
    return visibleItems.filter(
      (it) =>
        !isExcludedFromSearch(it.classificationId) &&
        (matchPinyin(it.name, q) ||
          (it.data.target || "").toLowerCase().includes(q) ||
          matchPinyin(it.data.remark || "", q))
    );
  }, [visibleItems, searchQuery, isExcludedFromSearch]);


  // 全盘文件索引搜索（与启动项统一展示）：200ms 防抖 + 请求代次校验。
  // 未建立索引时后端直接返回空数组，不影响原有快捷方式搜索体验。
  useEffect(() => {
    if (!isSearchOpen) return;
    const q = searchQuery.trim();
    if (!q) {
      fileSearchReqRef.current++;
      setFileHits([]);
      setFileHitsQuery("");
      return;
    }
    const req = ++fileSearchReqRef.current;
    const timer = window.setTimeout(() => {
      invoke<FileHit[]>("file_search_query", { query: q, limit: 30 })
        .then((list) => {
          if (fileSearchReqRef.current !== req) return;
          setFileHits(list);
          setFileHitsQuery(q);
        })
        .catch(() => {
          if (fileSearchReqRef.current !== req) return;
          setFileHits([]);
          setFileHitsQuery(q);
        });
    }, 200);
    return () => window.clearTimeout(timer);
  }, [searchQuery, isSearchOpen]);


  const openFileHit = (hit: FileHit) => {
    invoke("launcher_execute_raw", {
      file: hit.path,
      params: null,
      runAsAdmin: false,
      startLocation: null,
    }).catch(() => {});
  };


  // 只展示与当前关键词一致的结果（防抖期间不显示过期条目）
  const effectiveFileHits = fileHitsQuery === searchQuery.trim() ? fileHits : [];


  const searchTotal = searchQuickActions.length + searchResults.length + effectiveFileHits.length;


  // Open / Close Search
  const openSearch = () => {
    setIsSearchOpen(true);
    setSearchQuery("");
    setSearchSelectedIndex(0);
    setTimeout(() => {
      searchInputRef.current?.focus();
    }, 50);
  };


  const closeSearch = () => {
    setIsSearchOpen(false);
    setSearchQuery("");
  };
  return {
    searchQuickActions,
    searchResults,
    openFileHit,
    effectiveFileHits,
    searchTotal,
    openSearch,
    closeSearch,
  };
}
