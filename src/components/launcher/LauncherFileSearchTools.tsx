import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { Database, RefreshCw } from "lucide-react";
import { SharedButton } from "../shared/Button";
import { FILTER_PRESETS, type IndexProgress } from "../FileSearchPopup.model";
import type { DriveIndexInfo } from "./types";
import type { FileSearchScope } from "./fileSearchScope";

/** Filters and index setup belong to the current search, without another search input/window. */
export function LauncherFileSearchTools({ scope, onScopeChange, onIndexChanged }: {
  scope: FileSearchScope; onScopeChange: (scope: FileSearchScope) => void; onIndexChanged: () => void;
}) {
  const { t } = useTranslation();
  const [drives, setDrives] = useState<DriveIndexInfo[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [managerOpen, setManagerOpen] = useState(false);
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
  return <section className="mb-3 space-y-2 shrink-0">
    <div className="flex items-center gap-2 flex-wrap text-caption">
      <span className="text-slate-400">{t("launcher.searchFiles")}</span>
      <select aria-label={t("fileSearch.driveFilter")} value={scope.drive}
        onChange={(event) => onScopeChange({ ...scope, drive: event.target.value })}
        className="ui-input h-8 px-2">
        <option value="">{t("fileSearch.allDrives")}</option>
        {drives.map((drive) => <option key={drive.letter} value={drive.letter}>{drive.letter}</option>)}
      </select>
      <select aria-label={t("launcher.fileType")} value={scope.filter}
        onChange={(event) => onScopeChange({ ...scope, filter: event.target.value })}
        className="ui-input h-8 px-2">
        {FILTER_PRESETS.map((preset) => <option key={preset.id} value={preset.id}>{t(preset.labelKey)}</option>)}
      </select>
      <SharedButton aria-expanded={managerOpen} onClick={() => setManagerOpen(!managerOpen)}>
        <Database className="w-3 h-3" />{t("fileSearch.manage")}
      </SharedButton>
    </div>
    {loaded && !indexed && !building && <p className="ui-note text-caption px-3 py-2">{t("launcher.unifiedIndexHint")}</p>}
    {error && <p role="alert" className="ui-note ui-note-error text-caption px-3 py-2 break-words">{error}</p>}
    {building && <div role="status" className="flex gap-2 items-center text-caption text-slate-400">
      <span className="min-w-0 truncate">{progress ? `${progress.drive} · ${t("fileSearch.scanning", { count: progress.scanned })}` : t("fileSearch.statusBuilding")}</span>
      <SharedButton disabled={busy} onClick={() => void stop()}>{t("fileSearch.stop")}</SharedButton>
    </div>}
    {managerOpen && <div className="ui-card p-3 space-y-2 max-h-48 overflow-auto">
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
