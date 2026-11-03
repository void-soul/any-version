import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { Settings, RefreshCw } from "lucide-react";
import { SharedButton } from "../shared/Button";
import { FILTER_PRESETS, type IndexProgress } from "../FileSearchPopup.model";
import ShortcutRecorder from "./ShortcutRecorder";
import { loadAppSettings, patchAppSettings } from "../../services/appSettings";
import { searchesFiles } from "./fileSearchScope";
import type { WebSearchSource } from "./types";
import type { DriveIndexInfo } from "./types";
import type { FileSearchScope } from "./fileSearchScope";

/** Filters and index setup belong to the current search, without another search input/window. */
export function LauncherFileSearchTools({ scope, onScopeChange, onIndexChanged, webSources, input, closeButton }: {
  webSources: WebSearchSource[]; input: ReactNode; closeButton: ReactNode;
  scope: FileSearchScope; onScopeChange: (scope: FileSearchScope) => void; onIndexChanged: () => void;
}) {
  const { t } = useTranslation();
  const [drives, setDrives] = useState<DriveIndexInfo[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [managerOpen, setManagerOpen] = useState(false);
  const [hotkey, setHotkey] = useState("");
  const [hotkeyLoaded, setHotkeyLoaded] = useState(false);
  const [hotkeyBusy, setHotkeyBusy] = useState(false);
  const hotkeyGate = useRef(false);
  const [respectGitignore, setRespectGitignore] = useState(true);
  const [progress, setProgress] = useState<IndexProgress | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const mounted = useRef(false);
  const actionGate = useRef(false);
  const loadEpoch = useRef(0);
  const loadDrives = useCallback(async () => {
    const epoch = ++loadEpoch.current;
    try {
      const list = await invoke<DriveIndexInfo[]>("file_search_list_drives");
      if (mounted.current && epoch === loadEpoch.current) { setDrives(list); setLoaded(true); }
    } catch (err) {
      if (mounted.current && epoch === loadEpoch.current) setError(String(err));
    }
  }, []);
  useEffect(() => {
    mounted.current = true;
    let disposed = false;
    let off: (() => void) | undefined;
    void loadDrives();
    void loadAppSettings().then((settings) => {
      if (!disposed) { setHotkey(settings.fileSearchHotkey ?? ""); setHotkeyLoaded(true); }
    }).catch((err) => { if (!disposed) setError(String(err)); });
    void listen<IndexProgress>("file-search-index-progress", ({ payload }) => {
      if (disposed) return;
      setProgress(payload.done ? null : payload);
      if (payload.done) {
        if (payload.error) setError(payload.error);
        void loadDrives();
        onIndexChanged();
      }
    }).then((unlisten) => { if (disposed) unlisten(); else off = unlisten; })
      .catch((err) => { if (!disposed) setError(String(err)); });
    return () => { disposed = true; mounted.current = false; loadEpoch.current += 1; off?.(); };
  }, [loadDrives, onIndexChanged]);

  const building = !!progress || drives.some((drive) => drive.status === "building");
  const indexed = drives.some((drive) => drive.status === "ready" || drive.itemCount > 0);
  const start = async (drive: DriveIndexInfo) => {
    if (actionGate.current || building) return;
    actionGate.current = true;
    setBusy(true); setError("");
    try {
      await invoke("file_search_start_index", { drive: drive.letter,
        mode: drive.status === "ready" ? "update" : "rebuild", respectGitignore });
      await loadDrives();
      if (mounted.current) onIndexChanged();
    } catch (err) { if (mounted.current) setError(String(err)); }
    finally { actionGate.current = false; if (mounted.current) setBusy(false); }
  };
  const stop = async () => {
    if (actionGate.current) return;
    actionGate.current = true; setBusy(true);
    try { await invoke("file_search_cancel_index"); await loadDrives(); }
    catch (err) { if (mounted.current) setError(String(err)); }
    finally { actionGate.current = false; if (mounted.current) setBusy(false); }
  };
  const saveHotkey = async (value: string | null) => {
    if (!hotkeyLoaded || hotkeyGate.current) return;
    hotkeyGate.current = true; setHotkeyBusy(true); setError("");
    try {
      const saved = await patchAppSettings({ fileSearchHotkey: value ?? "" });
      if (mounted.current) setHotkey(saved.fileSearchHotkey ?? "");
    } catch (err) { if (mounted.current) setError(String(err)); }
    finally { hotkeyGate.current = false; if (mounted.current) setHotkeyBusy(false); }
  };
  const source = scope.source ?? "all";
  const selectedSource = source === "files" && scope.drive ? `drive:${scope.drive}` : source;
  return <section className="mb-3 space-y-2 shrink-0">
    <div className="ui-input flex items-center gap-2 h-11 px-2 focus-within:border-[var(--module-accent)]">
      <select aria-label={t("launcher.searchSource")} value={selectedSource}
        onChange={(event) => {
          const value = event.target.value;
          onScopeChange({ ...scope, source: value.startsWith("drive:") ? "files" : value,
            drive: value.startsWith("drive:") ? value.slice(6) : "" });
        }} className="bg-transparent text-caption text-slate-300 max-w-[30%] h-8 outline-none shrink-0">
        <option value="all">{t("launcher.sourceAll")}</option>
        <option value="launcher">{t("launcher.sourceLauncher")}</option>
        <optgroup label={t("launcher.sourceComputer")}>
          <option value="files">{t("launcher.sourceComputer")}</option>
          {drives.map((drive) => <option key={drive.letter} value={`drive:${drive.letter}`}>{drive.letter}</option>)}
        </optgroup>
        {webSources.length > 0 && <optgroup label={t("launcher.sourceWeb")}>
          {webSources.map((site) => <option key={site.id} value={`web:${site.id}`}>{site.name}</option>)}
        </optgroup>}
      </select>
      <div className="flex-1 min-w-0 border-l border-white/10 pl-2">{input}</div>
      <select aria-label={t("launcher.fileType")} value={scope.filter} disabled={!searchesFiles(scope)}
        onChange={(event) => onScopeChange({ ...scope, filter: event.target.value })}
        className="bg-transparent h-8 text-caption text-slate-300 outline-none max-w-[23%] disabled:opacity-30 shrink-0">
        {FILTER_PRESETS.map((preset) => <option key={preset.id} value={preset.id}>{t(preset.labelKey)}</option>)}
      </select>
      <button type="button" aria-label={t("launcher.searchSettings")} title={t("launcher.searchSettings")}
        aria-expanded={managerOpen} onClick={() => setManagerOpen(!managerOpen)}
        className="ui-btn ui-btn-ghost p-1.5 shrink-0"><Settings className="w-3.5 h-3.5" /></button>
      {closeButton}
    </div>
    {searchesFiles(scope) && loaded && !indexed && !building && <p className="ui-note text-caption px-3 py-2">{t("launcher.unifiedIndexHint")}</p>}
    {error && <p role="alert" className="ui-note ui-note-error text-caption px-3 py-2 break-words">{error}</p>}
    {building && <div role="status" className="flex gap-2 items-center text-caption text-slate-400">
      <span className="min-w-0 truncate">{progress ? `${progress.drive} · ${t("fileSearch.scanning", { count: progress.scanned })}` : t("fileSearch.statusBuilding")}</span>
      <SharedButton disabled={busy} onClick={() => void stop()}>{t("fileSearch.stop")}</SharedButton>
    </div>}
    {managerOpen && <div className="ui-card modal-mask p-3 space-y-3 max-h-64 overflow-auto">
      <div className="space-y-1">
        <label className="text-caption text-slate-300 block">{t("launcher.searchPopupHotkey")}</label>
        <ShortcutRecorder value={hotkey} disabled={!hotkeyLoaded || hotkeyBusy} onChange={(value) => void saveHotkey(value)} />
      </div>
      <h3 className="text-caption font-medium text-slate-300">{t("fileSearch.manage")}</h3>
      <div className="flex gap-2 items-center justify-between">
        <label className="flex items-center gap-2 text-caption text-slate-400">
          <input type="checkbox" checked={respectGitignore} disabled={busy || building}
            onChange={(event) => setRespectGitignore(event.target.checked)} />{t("fileSearch.respectGitignore")}
        </label>
        <SharedButton onClick={() => void loadDrives()} aria-label={t("fileSearch.refresh")}>
          <RefreshCw className="h-3 w-3" />
        </SharedButton>
      </div>
      {drives.map((drive) => <div key={drive.letter} className="flex items-center gap-2 text-caption">
        <span className="font-mono text-slate-200">{drive.letter}</span>
        <span className="text-slate-500 flex-1">{drive.status === "ready" ? t("fileSearch.statusReady", { count: drive.itemCount }) : t(drive.status === "building" ? "fileSearch.statusBuilding" : "fileSearch.statusIdle")}</span>
        <SharedButton disabled={busy || building} onClick={() => void start(drive)}>
          {t(drive.status === "ready" ? "fileSearch.update" : "fileSearch.build")}
        </SharedButton>
      </div>)}
      {loaded && drives.length === 0 && <p className="text-caption text-slate-500">{t("fileSearch.noDrives")}</p>}
    </div>}
  </section>;
}
