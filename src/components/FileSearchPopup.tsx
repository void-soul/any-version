import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";
import {
  Check,
  Copy,
  Database,
  FileText,
  Folder,
  FolderOpen,
  HardDrive,
  Loader2,
  RefreshCw,
  Search,
  Square,
  Trash2,
  X,
} from "lucide-react";
import ShortcutRecorder from "./launcher/ShortcutRecorder";
import type { DriveIndexInfo, FileHit, LauncherSetting } from "./launcher/types";

interface IndexProgress {
  drive: string;
  scanned: number;
  inserted: number;
  current: string;
  elapsedMs: number;
  done: boolean;
  canceled: boolean;
  error: string | null;
}

function formatSize(bytes: number): string {
  if (!bytes || bytes < 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

function formatTime(sec: number): string {
  if (!sec) return "";
  return new Date(sec * 1000).toLocaleString();
}

/** 目录部分（结果行显示所在位置）。 */
function dirOf(path: string, name: string): string {
  const cut = path.length - name.length;
  return cut > 0 ? path.slice(0, cut) : path;
}

/** 首个关键词高亮。 */
function highlight(text: string, query: string) {
  const first = query.trim().split(/\s+/)[0];
  if (!first) return text;
  const idx = text.toLowerCase().indexOf(first.toLowerCase());
  if (idx < 0) return text;
  return (
    <>
      {text.slice(0, idx)}
      <span className="font-semibold text-[var(--module-accent)]">
        {text.slice(idx, idx + first.length)}
      </span>
      {text.slice(idx + first.length)}
    </>
  );
}

export default function FileSearchPopup() {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<FileHit[]>([]);
  const [hitsQuery, setHitsQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const [searching, setSearching] = useState(false);
  const [searched, setSearched] = useState(false);

  const [drives, setDrives] = useState<DriveIndexInfo[]>([]);
  const [drivesLoading, setDrivesLoading] = useState(true);
  const [managerOpen, setManagerOpen] = useState(false);
  const [progress, setProgress] = useState<IndexProgress | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [hotkey, setHotkey] = useState("");
  const [error, setError] = useState("");
  const [info, setInfo] = useState("");
  const [copiedPath, setCopiedPath] = useState("");

  const inputRef = useRef<HTMLInputElement>(null);
  const requestRef = useRef(0);
  const managerRef = useRef(false);
  const queryRef = useRef("");
  managerRef.current = managerOpen;
  queryRef.current = query;

  const indexedDrives = useMemo(() => drives.filter((d) => d.status !== "idle"), [drives]);
  const anyBuilding = useMemo(() => drives.some((d) => d.status === "building"), [drives]);

  const hide = useCallback(() => {
    void invoke("hide_file_search_popup");
  }, []);

  const loadDrives = useCallback(async () => {
    setDrivesLoading(true);
    try {
      const list = await invoke<DriveIndexInfo[]>("file_search_list_drives");
      setDrives(list);
    } catch (e) {
      setError(String(e));
    } finally {
      setDrivesLoading(false);
    }
  }, []);

  const loadHotkey = useCallback(async () => {
    try {
      const s = await invoke<LauncherSetting>("launcher_get_settings");
      setHotkey(s.fileSearchHotkey ?? "");
    } catch {
      /* 读取失败时保留空值 */
    }
  }, []);

  // 首次挂载：加载驱动器状态与快捷键，聚焦输入框
  useEffect(() => {
    void loadDrives();
    void loadHotkey();
    const timer = window.setTimeout(() => inputRef.current?.focus(), 60);
    let unlisten: (() => void) | undefined;
    void listen("file-search-focus", () => {
      setManagerOpen(false);
      window.setTimeout(() => {
        inputRef.current?.focus();
        inputRef.current?.select();
      }, 30);
    }).then((u) => {
      unlisten = u;
    });
    return () => {
      window.clearTimeout(timer);
      unlisten?.();
    };
  }, [loadDrives, loadHotkey]);

  // 索引进度
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<IndexProgress>("file-search-index-progress", (event) => {
      const p = event.payload;
      setProgress(p.done ? null : p);
      if (p.done) {
        if (p.error) setError(p.error);
        else if (p.canceled) setInfo(t("fileSearch.canceled"));
        void loadDrives();
      }
      if (p.done && !p.error && !p.canceled) {
        setInfo(t("fileSearch.indexDone", { count: p.inserted }));
        // 索引刚更新完，若正在搜索则刷新一次结果
        if (queryRef.current.trim()) void runSearch(queryRef.current);
      }
    }).then((u) => {
      unlisten = u;
    });
    return () => unlisten?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadDrives, t]);

  // 搜索结果（150ms 防抖 + 请求代次校验）
  const runSearch = useCallback(async (raw: string) => {
    const q = raw.trim();
    if (!q) {
      setHits([]);
      setHitsQuery("");
      setSearched(false);
      setSearching(false);
      return;
    }
    const req = ++requestRef.current;
    setSearching(true);
    try {
      const list = await invoke<FileHit[]>("file_search_query", { query: q, limit: 100 });
      if (requestRef.current !== req) return;
      setHits(list);
      setHitsQuery(q);
      setSelected(0);
      setSearched(true);
    } catch (e) {
      if (requestRef.current !== req) return;
      setHits([]);
      setHitsQuery(q);
      setSearched(true);
      setError(String(e));
    } finally {
      if (requestRef.current === req) setSearching(false);
    }
  }, []);

  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setHits([]);
      setHitsQuery("");
      setSearched(false);
      setSearching(false);
      return;
    }
    const timer = window.setTimeout(() => void runSearch(q), 150);
    return () => window.clearTimeout(timer);
  }, [query, runSearch]);

  // 只展示与当前输入一致的结果（防抖期间隐藏过期条目，避免回车打开错文件）
  const effectiveHits = hitsQuery === query.trim() ? hits : [];
  const effectiveSearched = hitsQuery === query.trim() && searched;

  // 提示信息自动消失
  useEffect(() => {
    if (!info) return;
    const timer = window.setTimeout(() => setInfo(""), 3500);
    return () => window.clearTimeout(timer);
  }, [info]);
  useEffect(() => {
    if (!error) return;
    const timer = window.setTimeout(() => setError(""), 6000);
    return () => window.clearTimeout(timer);
  }, [error]);

  // Esc 关闭：录制快捷键时 ShortcutRecorder 会 stopPropagation，不会误触
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (managerRef.current) {
        setManagerOpen(false);
      } else {
        hide();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [hide]);

  const openHit = useCallback(
    async (hit: FileHit) => {
      try {
        await invoke("launcher_execute_raw", {
          file: hit.path,
          params: null,
          runAsAdmin: false,
          startLocation: null,
        });
        hide();
      } catch (e) {
        setError(String(e));
      }
    },
    [hide]
  );

  const revealHit = useCallback(async (hit: FileHit) => {
    try {
      await invoke("launcher_reveal_file", { path: hit.path });
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const copyHit = useCallback(async (hit: FileHit) => {
    try {
      await navigator.clipboard.writeText(hit.path);
      setCopiedPath(hit.path);
      window.setTimeout(() => setCopiedPath(""), 1500);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const onInputKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (effectiveHits.length) setSelected((i) => (i + 1) % effectiveHits.length);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (effectiveHits.length) setSelected((i) => (i - 1 + effectiveHits.length) % effectiveHits.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const hit = effectiveHits[Math.min(selected, Math.max(0, effectiveHits.length - 1))];
      if (!hit) return;
      if (e.ctrlKey || e.metaKey) void revealHit(hit);
      else void openHit(hit);
    }
  };

  const startIndex = async (letter: string, mode: "rebuild" | "update") => {
    setError("");
    setInfo("");
    setProgress({
      drive: letter,
      scanned: 0,
      inserted: 0,
      current: "",
      elapsedMs: 0,
      done: false,
      canceled: false,
      error: null,
    });
    try {
      await invoke("file_search_start_index", { drive: letter, mode });
      await loadDrives();
    } catch (e) {
      setProgress(null);
      setError(String(e));
      await loadDrives();
    }
  };

  const stopIndex = async () => {
    try {
      await invoke("file_search_cancel_index");
    } catch (e) {
      setError(String(e));
    }
  };

  const removeIndex = async (letter: string) => {
    setConfirmDelete(null);
    try {
      await invoke("file_search_delete_index", { drive: letter });
      setInfo(t("fileSearch.indexDeleted", { drive: letter }));
      await loadDrives();
    } catch (e) {
      setError(String(e));
    }
  };

  const saveHotkey = async (combo: string | null) => {
    try {
      const s = await invoke<LauncherSetting>("launcher_get_settings");
      const next: LauncherSetting = { ...s, fileSearchHotkey: combo ?? "" };
      await invoke("launcher_save_settings", { settings: next });
      setHotkey(combo ?? "");
      setInfo(t("fileSearch.hotkeySaved"));
    } catch (e) {
      setError(String(e));
    }
  };

  const onTitleMouseDown = (e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest("button, input")) return;
    void getCurrentWindow().startDragging().catch(() => {});
  };

  const driveTypeLabel = (type: string) => {
    switch (type) {
      case "fixed":
        return t("fileSearch.driveFixed");
      case "removable":
        return t("fileSearch.driveRemovable");
      case "network":
        return t("fileSearch.driveNetwork");
      case "cdrom":
        return t("fileSearch.driveCdrom");
      case "ramdisk":
        return t("fileSearch.driveRamdisk");
      default:
        return t("fileSearch.driveUnknown");
    }
  };

  const statusLabel = (d: DriveIndexInfo) => {
    if (d.status === "building") return t("fileSearch.statusBuilding");
    if (d.status === "ready") {
      return t("fileSearch.statusReady", { count: d.itemCount.toLocaleString() });
    }
    return t("fileSearch.statusIdle");
  };

  const noIndex = indexedDrives.length === 0;

  return (
    <div
      className="flex h-screen w-screen select-none flex-col overflow-hidden rounded-card border border-white/10 bg-surface-panel text-slate-200 shadow-2xl"
    >
      {/* 标题栏（拖动窗口） */}
      <div
        className="flex shrink-0 cursor-grab items-center gap-2 border-b border-white/10 px-3 py-2 active:cursor-grabbing"
        onMouseDown={onTitleMouseDown}
      >
        <Search className="h-4 w-4" style={{ color: "var(--module-accent)" }} />
        <span className="text-body font-semibold text-white">{t("fileSearch.title")}</span>
        <span className="rounded-md bg-white/5 px-1.5 py-0.5 font-mono text-micro text-slate-500">
          {hotkey || t("fileSearch.hotkeyEmpty")}
        </span>
        <div className="flex-1" />
        <button
          type="button"
          onClick={() => setManagerOpen((v) => !v)}
          title={t("fileSearch.manage")}
          className={`rounded p-1 transition hover:bg-white/10 hover:text-white ${
            managerOpen ? "text-[var(--module-accent)]" : "text-slate-400"
          }`}
        >
          <Database className="h-4 w-4" />
        </button>
        <button
          type="button"
          onClick={hide}
          title={t("fileSearch.close")}
          className="rounded p-1 text-slate-400 transition hover:bg-white/10 hover:text-white"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      {/* 搜索框 */}
      <div className="shrink-0 px-3 pb-2 pt-3">
        <div className="relative flex items-center">
          <Search className="pointer-events-none absolute left-3 h-4 w-4 text-[var(--module-accent)]" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onInputKeyDown}
            placeholder={t("fileSearch.placeholder")}
            spellCheck={false}
            className="w-full rounded-card border border-[var(--module-accent-ring)] bg-white/5 py-2.5 pl-10 pr-10 text-sm text-white placeholder-slate-500 outline-none transition focus:border-[var(--module-accent)]"
          />
          <div className="absolute right-3 flex items-center gap-1">
            {searching ? (
              <Loader2 className="h-4 w-4 animate-spin text-slate-500" />
            ) : (
              query && (
                <button
                  type="button"
                  onClick={() => {
                    setQuery("");
                    inputRef.current?.focus();
                  }}
                  className="rounded p-0.5 text-slate-500 transition hover:text-white"
                  title={t("fileSearch.clear")}
                >
                  <X className="h-4 w-4" />
                </button>
              )
            )}
          </div>
        </div>
      </div>

      {/* 主体 */}
      {managerOpen ? (
        <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto px-3 pb-3">
          <div className="flex items-center justify-between px-0.5">
            <span className="text-caption font-medium text-slate-200">{t("fileSearch.drives")}</span>
            <button
              type="button"
              onClick={() => void loadDrives()}
              className="flex items-center gap-1 rounded px-1.5 py-0.5 text-micro text-slate-500 transition hover:bg-white/5 hover:text-slate-300"
            >
              <RefreshCw className={`h-3 w-3 ${drivesLoading ? "animate-spin" : ""}`} />
              {t("fileSearch.refresh")}
            </button>
          </div>

          {drivesLoading && drives.length === 0 ? (
            <div className="flex flex-1 items-center justify-center text-caption text-slate-500">
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              {t("fileSearch.loading")}
            </div>
          ) : drives.length === 0 ? (
            <div className="py-10 text-center text-caption text-slate-500">{t("fileSearch.noDrives")}</div>
          ) : (
            <div className="space-y-1.5">
              {drives.map((d) => {
                const active = progress && !progress.done && progress.drive === d.letter ? progress : null;
                const building = d.status === "building";
                return (
                  <div
                    key={d.letter}
                    className="rounded-card border border-white/10 bg-white/[0.03] px-3 py-2.5"
                  >
                    <div className="flex items-center gap-2.5">
                      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-ctl bg-white/5">
                        <HardDrive className="h-4 w-4 text-slate-400" />
                      </div>
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <span className="font-mono text-body font-semibold text-white">{d.letter}</span>
                          <span className="truncate text-micro text-slate-500">{driveTypeLabel(d.driveType)}</span>
                          <span
                            className={`rounded px-1.5 py-0.5 text-micro ${
                              building
                                ? "bg-[var(--module-accent-soft)] text-[var(--module-accent)]"
                                : d.status === "ready"
                                ? "bg-emerald-400/10 text-emerald-300"
                                : "bg-white/5 text-slate-500"
                            }`}
                          >
                            {statusLabel(d)}
                          </span>
                        </div>
                        <div className="mt-0.5 truncate text-micro text-slate-500">
                          {t("fileSearch.driveCapacity", {
                            free: formatSize(d.freeBytes),
                            total: formatSize(d.totalBytes),
                          })}
                          {d.status === "ready" && d.indexedAt > 0 && (
                            <span className="ml-2 text-slate-600">
                              {t("fileSearch.indexedAt", { time: formatTime(d.indexedAt) })}
                            </span>
                          )}
                          {d.status === "ready" && d.elapsedMs > 0 && (
                            <span className="ml-2 text-slate-600">
                              {t("fileSearch.elapsed", { sec: (d.elapsedMs / 1000).toFixed(1) })}
                            </span>
                          )}
                        </div>
                      </div>
                      <div className="flex shrink-0 items-center gap-1">
                        {building ? (
                          <button
                            type="button"
                            onClick={() => void stopIndex()}
                            className="flex items-center gap-1 rounded-ctl border border-red-400/30 bg-red-500/10 px-2 py-1 text-tiny text-red-300 transition hover:bg-red-500/20"
                          >
                            <Square className="h-3 w-3" />
                            {t("fileSearch.stop")}
                          </button>
                        ) : (
                          <>
                            <button
                              type="button"
                              disabled={anyBuilding}
                              onClick={() => void startIndex(d.letter, d.status === "ready" ? "update" : "rebuild")}
                              className="rounded-ctl border border-[var(--module-accent-ring)] bg-[var(--module-accent-soft)] px-2 py-1 text-tiny text-[var(--module-accent)] transition hover:brightness-125 disabled:cursor-not-allowed disabled:opacity-40"
                            >
                              {d.status === "ready" ? t("fileSearch.update") : t("fileSearch.build")}
                            </button>
                            {d.status === "ready" && (
                              <button
                                type="button"
                                disabled={anyBuilding}
                                onClick={() =>
                                  confirmDelete === d.letter ? void removeIndex(d.letter) : setConfirmDelete(d.letter)
                                }
                                onBlur={() => setConfirmDelete((cur) => (cur === d.letter ? null : cur))}
                                className={`flex items-center gap-1 rounded-ctl border px-2 py-1 text-tiny transition disabled:cursor-not-allowed disabled:opacity-40 ${
                                  confirmDelete === d.letter
                                    ? "border-red-400/40 bg-red-500/20 text-red-200"
                                    : "border-white/10 text-slate-400 hover:border-red-400/30 hover:text-red-300"
                                }`}
                                title={t("fileSearch.deleteIndex")}
                              >
                                <Trash2 className="h-3 w-3" />
                                {confirmDelete === d.letter ? t("fileSearch.deleteConfirm") : ""}
                              </button>
                            )}
                          </>
                        )}
                      </div>
                    </div>
                    {active && (
                      <div className="mt-2">
                        <div className="h-1 overflow-hidden rounded-full bg-white/10">
                          <div className="h-full w-1/3 animate-pulse rounded-full bg-[var(--module-accent)]" />
                        </div>
                        <div className="mt-1 flex items-center gap-2 text-micro text-slate-500">
                          <span>{t("fileSearch.scanning", { count: active.scanned.toLocaleString() })}</span>
                          <span className="min-w-0 flex-1 truncate text-slate-600">{active.current}</span>
                        </div>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}

          <p className="px-0.5 text-micro leading-relaxed text-slate-600">{t("fileSearch.scanningHint")}</p>

          {/* 快捷键 */}
          <div className="mt-1 rounded-card border border-white/10 bg-white/[0.03] px-3 py-2.5">
            <div className="mb-1.5 text-caption font-medium text-slate-200">{t("fileSearch.hotkey")}</div>
            <ShortcutRecorder value={hotkey} onChange={(v) => void saveHotkey(v)} />
            <p className="mt-1.5 text-micro text-slate-600">{t("fileSearch.hotkeyHint")}</p>
          </div>
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-2">
          {!query.trim() ? (
            <div className="flex h-full flex-col items-center justify-center gap-2 text-center">
              <div className="flex h-12 w-12 items-center justify-center rounded-full bg-white/5">
                <Search className="h-5 w-5 text-slate-500" />
              </div>
              <div className="text-body text-slate-300">
                {noIndex ? t("fileSearch.emptyNoIndexTitle") : t("fileSearch.emptyTitle")}
              </div>
              <div className="max-w-sm text-micro leading-relaxed text-slate-600">
                {noIndex ? t("fileSearch.emptyNoIndexHint") : t("fileSearch.emptyHint")}
              </div>
            </div>
          ) : effectiveSearched && effectiveHits.length === 0 ? (
            <div className="flex h-full flex-col items-center justify-center gap-2 text-center">
              <div className="flex h-12 w-12 items-center justify-center rounded-full bg-white/5">
                <FileText className="h-5 w-5 text-slate-500" />
              </div>
              <div className="text-body text-slate-300">{t("fileSearch.noResult", { query })}</div>
              <div className="max-w-sm text-micro leading-relaxed text-slate-600">
                {noIndex ? t("fileSearch.emptyNoIndexHint") : t("fileSearch.noResultHint")}
              </div>
            </div>
          ) : (
            <div className="space-y-0.5">
              {effectiveHits.map((hit, idx) => {
                const isSelected = idx === selected;
                return (
                  <div
                    key={hit.path}
                    role="button"
                    tabIndex={-1}
                    onMouseEnter={() => setSelected(idx)}
                    onClick={() => setSelected(idx)}
                    onDoubleClick={() => void openHit(hit)}
                    className={`group flex cursor-pointer items-center gap-2.5 rounded-ctl px-2.5 py-1.5 text-left transition ${
                      isSelected
                        ? "bg-[var(--module-accent)] text-white"
                        : "text-slate-200 hover:bg-white/5"
                    }`}
                  >
                    <div
                      className={`flex h-6 w-6 shrink-0 items-center justify-center overflow-hidden rounded ${
                        isSelected ? "bg-black/20" : "bg-white/10"
                      }`}
                    >
                      {hit.isDir ? (
                        <Folder className={`h-4 w-4 ${isSelected ? "text-white" : "text-amber-400"}`} />
                      ) : (
                        <FileText className={`h-4 w-4 ${isSelected ? "text-white" : "text-[var(--module-accent)]"}`} />
                      )}
                    </div>
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-body">{highlight(hit.name, query)}</div>
                      <div
                        className={`truncate text-micro ${isSelected ? "text-white/70" : "text-slate-500"}`}
                      >
                        {dirOf(hit.path, hit.name)}
                      </div>
                    </div>
                    <div
                      className={`shrink-0 text-micro ${isSelected ? "text-white/70" : "text-slate-600"}`}
                    >
                      {hit.isDir ? "" : formatSize(hit.size)}
                    </div>
                    <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition group-hover:opacity-100">
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          void revealHit(hit);
                        }}
                        title={t("fileSearch.reveal")}
                        className={`rounded p-1 ${isSelected ? "hover:bg-black/20" : "hover:bg-white/10"}`}
                      >
                        <FolderOpen className="h-3.5 w-3.5" />
                      </button>
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          void copyHit(hit);
                        }}
                        title={t("fileSearch.copyPath")}
                        className={`rounded p-1 ${isSelected ? "hover:bg-black/20" : "hover:bg-white/10"}`}
                      >
                        {copiedPath === hit.path ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
                      </button>
                    </div>
                  </div>
                );
              })}
              {effectiveHits.length >= 100 && (
                <p className="px-2 py-1.5 text-micro text-slate-600">
                  {t("fileSearch.resultLimitHint", { count: effectiveHits.length })}
                </p>
              )}
            </div>
          )}
        </div>
      )}

      {/* 底部状态栏 */}
      <div className="flex shrink-0 items-center justify-between gap-2 border-t border-white/10 px-3 py-1.5 text-micro text-slate-600">
        <span className="min-w-0 truncate">
          {error ? (
            <span className="text-red-400">{error}</span>
          ) : info ? (
            <span className="text-emerald-400">{info}</span>
          ) : query.trim() && effectiveSearched ? (
            t("fileSearch.resultCount", { count: effectiveHits.length })
          ) : (
            noIndex
              ? t("fileSearch.emptyNoIndexTitle")
              : t("fileSearch.footerHint")
          )}
        </span>
        <span className="shrink-0">{t("fileSearch.keyHint")}</span>
      </div>
    </div>
  );
}
