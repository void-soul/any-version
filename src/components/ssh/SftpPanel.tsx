import { useCallback, useEffect, useRef, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { Archive, ArrowUp, Download, FolderPlus, RefreshCw, Trash2, Upload, X } from "lucide-react";
import { ignoreInvokeError, reportInvokeError } from "../../utils/invokeOrReport";
import { confirmHostKey } from "./hostKeyPrompt";
import { SftpTree } from "./SftpTree";

interface SftpEntry {
  name: string; path: string; isDir: boolean; isSymlink: boolean; size: number; mtime: number | null;
}
interface SftpProgress {
  sessionId: string; direction: string; transferred: number; total: number; done: boolean;
  resumedFrom: number; stage?: string | null; message: string | null;
}

const btn = "inline-flex items-center gap-1.5 rounded px-2.5 py-1.5 text-tiny border border-white/10 bg-black/30 text-slate-200 hover:bg-white/10 disabled:opacity-40";
const field = "w-full rounded border border-white/10 bg-black/30 px-2 py-1.5 text-tiny text-slate-100 outline-none focus:border-sky-500/60";

function formatSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(1)} GB`;
}

/** 单个 SSH 连接的远程文件管理。卸载时必定断开会话。 */
export function SftpPanel({ connectionId, title, onClose }: {
  connectionId: string;
  title: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [sessionId, setSessionId] = useState("");
  const [cwd, setCwd] = useState("");
  const [pathInput, setPathInput] = useState("");
  const [entries, setEntries] = useState<SftpEntry[]>([]);
  const [selected, setSelected] = useState<SftpEntry | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [progress, setProgress] = useState<SftpProgress | null>(null);
  const sessionRef = useRef("");

  const refresh = useCallback(async (path: string) => {
    const sid = sessionRef.current;
    if (!sid) return;
    setBusy(true);
    try {
      const [full, list] = await invoke<[string, SftpEntry[]]>("sftp_list", { sessionId: sid, current: cwd, path });
      setCwd(full);
      setPathInput(full);
      setEntries(list);
      setSelected(null);
    } catch (e) { setError(reportInvokeError(e, "sftp_list")); }
    finally { setBusy(false); }
  }, [cwd]);

  useEffect(() => {
    let disposed = false;
    void (async () => {
      const open = async () => invoke<[string, string]>("sftp_open", { connectionId });
      try {
        let opened: [string, string];
        try {
          opened = await open();
        } catch (first) {
          if (disposed || !(await confirmHostKey(String(first), (msg) => window.confirm(msg)))) {
            throw first;
          }
          opened = await open();
        }
        const [sid, home] = opened;
        if (disposed) {
          await invoke("sftp_disconnect", { sessionId: sid })
            .catch((e) => ignoreInvokeError(e, "面板已卸载，会话由后端兜底释放"));
          return;
        }
        sessionRef.current = sid;
        setSessionId(sid);
        setCwd(home);
        setPathInput(home);
        const [, list] = await invoke<[string, SftpEntry[]]>("sftp_list", { sessionId: sid, current: home, path: "." });
        if (!disposed) setEntries(list);
      } catch (e) { if (!disposed) setError(reportInvokeError(e, "sftp_open")); }
    })();
    return () => {
      disposed = true;
      const sid = sessionRef.current;
      if (sid) void invoke("sftp_disconnect", { sessionId: sid })
        .catch((e) => ignoreInvokeError(e, "组件卸载时断开 SFTP，失败时后端会话随句柄释放"));
    };
  }, [connectionId]);

  const openEntry = (entry: SftpEntry) => {
    if (entry.isDir) void refresh(entry.path);
    else setSelected(entry);
  };

  const goUp = () => { void refresh(".."); };

  const makeDir = async () => {
    const name = window.prompt(t("sftp.mkdirPrompt"));
    if (!name?.trim()) return;
    try {
      await invoke("sftp_mkdir", { sessionId, current: cwd, path: name.trim() });
      await refresh(".");
    } catch (e) { setError(reportInvokeError(e, "sftp_mkdir")); }
  };

  const removeEntry = async () => {
    if (!selected || !window.confirm(t("sftp.confirmDelete", { name: selected.name }))) return;
    try {
      await invoke("sftp_remove", { sessionId, current: cwd, path: selected.path, isDir: selected.isDir });
      await refresh(".");
    } catch (e) { setError(reportInvokeError(e, "sftp_remove")); }
  };

  const transfer = async (direction: "upload" | "download") => {
    try {
      const channel = new Channel<SftpProgress>();
      channel.onmessage = (ev) => {
        setProgress(ev);
        if (ev.message) setError(ev.message);
      };
      if (direction === "download") {
        if (!selected) return;
        const local = await saveDialog({ title: t("sftp.download"), defaultPath: selected.name });
        if (!local) return;
        setBusy(true);
        await invoke("sftp_download", {
          sessionId, current: cwd, remotePath: selected.path, localPath: local,
          resume: true, onProgress: channel,
        });
      } else {
        const local = await openDialog({ multiple: false });
        if (!local || typeof local !== "string") return;
        const name = local.split(/[\\/]/).pop() || "upload";
        setBusy(true);
        await invoke("sftp_upload", {
          sessionId, current: cwd, localPath: local, remotePath: name,
          resume: true, onProgress: channel,
        });
      }
      await refresh(".");
    } catch (e) { setError(reportInvokeError(e, `sftp_${direction}`)); }
    finally { setBusy(false); setProgress(null); }
  };

  /** 单文件压缩下载：远端 gzip 后传输，本地解压；不支持续传。 */
  const downloadCompressed = async () => {
    if (!selected || selected.isDir) return;
    try {
      const local = await saveDialog({ title: t("sftp.downloadCompressed"), defaultPath: selected.name });
      if (!local) return;
      setBusy(true);
      const channel = new Channel<SftpProgress>();
      channel.onmessage = (ev) => {
        setProgress(ev);
        if (ev.message && ev.done) setError(ev.message);
      };
      await invoke("sftp_download_compressed", {
        sessionId, current: cwd, remotePath: selected.path, localPath: local, onProgress: channel,
      });
      await refresh(".");
    } catch (e) { setError(reportInvokeError(e, "sftp_download_compressed")); }
    finally { setBusy(false); setProgress(null); }
  };

  /** 目录打包下载：远端 tar.gz 后传输，本地解压出同名目录。 */
  const downloadDir = async () => {
    if (!selected?.isDir) return;
    try {
      const local = await openDialog({ directory: true, multiple: false });
      if (!local || typeof local !== "string") return;
      setBusy(true);
      const channel = new Channel<SftpProgress>();
      channel.onmessage = (ev) => {
        setProgress(ev);
        if (ev.message && ev.done) setError(ev.message);
      };
      await invoke("sftp_download_dir", {
        sessionId, current: cwd, remoteDir: selected.path, localDir: local, onProgress: channel,
      });
      await refresh(".");
    } catch (e) { setError(reportInvokeError(e, "sftp_download_dir")); }
    finally { setBusy(false); setProgress(null); }
  };

  const pct = progress && progress.total > 0
    ? Math.min(100, Math.round((progress.transferred / progress.total) * 100)) : 0;

  return (
    <div className="flex h-full min-h-0 flex-col gap-2 text-slate-200">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="text-tiny font-medium">{title}</span>
        <button className={btn} disabled={!sessionId || busy} onClick={() => void refresh(".")}><RefreshCw className="h-3.5 w-3.5" />{t("sftp.refresh")}</button>
        <button className={btn} disabled={!sessionId || busy} onClick={goUp}><ArrowUp className="h-3.5 w-3.5" />{t("sftp.up")}</button>
        <button className={btn} disabled={!sessionId || busy} onClick={() => void makeDir()}><FolderPlus className="h-3.5 w-3.5" />{t("sftp.mkdir")}</button>
        <button className={btn} disabled={!sessionId || busy} onClick={() => void transfer("upload")}><Upload className="h-3.5 w-3.5" />{t("sftp.upload")}</button>
        {selected?.isDir ? (
          <button className={btn} disabled={!selected || busy} onClick={() => void downloadDir()}><Archive className="h-3.5 w-3.5" />{t("sftp.downloadDir")}</button>
        ) : (
          <>
            <button className={btn} disabled={!selected || busy} onClick={() => void transfer("download")}><Download className="h-3.5 w-3.5" />{t("sftp.download")}</button>
            <button className={btn} disabled={!selected || busy} onClick={() => void downloadCompressed()}><Archive className="h-3.5 w-3.5" />{t("sftp.downloadCompressed")}</button>
          </>
        )}
        <button className={btn} disabled={!selected || busy} onClick={() => void removeEntry()}><Trash2 className="h-3.5 w-3.5" />{t("sftp.delete")}</button>
        <button className={`${btn} ml-auto`} onClick={onClose}><X className="h-3.5 w-3.5" />{t("ssh.close")}</button>
      </div>

      <form className="flex gap-1.5" onSubmit={(e) => { e.preventDefault(); void refresh(pathInput); }}>
        <input className={field} value={pathInput} onChange={(e) => setPathInput(e.target.value)} aria-label={t("sftp.path")} />
        <button className={btn} type="submit" disabled={!sessionId || busy}>{t("sftp.go")}</button>
      </form>

      {error && <div className="rounded border border-rose-400/30 bg-rose-400/10 px-3 py-2 text-tiny text-rose-200">{error}</div>}
      {progress && !progress.done && (
        <div className="text-tiny text-sky-300">
          {progress.stage === "compress"
            ? t("sftp.stageCompress")
            : progress.stage === "extract"
              ? t("sftp.stageExtract")
              : t(progress.direction === "upload" ? "sftp.uploading" : "sftp.downloading", { pct })}
          {progress.resumedFrom > 0 && (
            <span className="ml-2 text-slate-400">
              {t("sftp.resumedFrom", { bytes: formatSize(progress.resumedFrom) })}
            </span>
          )}
          <div className="mt-1 h-1 w-full overflow-hidden rounded bg-white/10"><div className="h-full bg-sky-400" style={{ width: `${pct}%` }} /></div>
        </div>
      )}

      <div className="flex min-h-0 flex-1 gap-2">
        <SftpTree sessionId={sessionId} cwd={cwd} onSelect={(path) => void refresh(path)} />
        <div className="min-h-0 flex-1 overflow-auto rounded border border-white/10">
        <table className="w-full text-tiny">
          <thead className="sticky top-0 bg-slate-900/90 text-slate-400">
            <tr>
              <th className="px-3 py-2 text-left">{t("sftp.name")}</th>
              <th className="px-3 py-2 text-left">{t("sftp.size")}</th>
              <th className="px-3 py-2 text-left">{t("sftp.mtime")}</th>
            </tr>
          </thead>
          <tbody>
            {!busy && entries.length === 0 && <tr><td colSpan={3} className="px-3 py-8 text-center text-slate-500">{t("sftp.empty")}</td></tr>}
            {entries.map((e) => (
              <tr key={e.path}
                className={`border-t border-white/5 hover:bg-white/5 cursor-pointer ${selected?.path === e.path ? "bg-sky-500/10" : ""}`}
                onDoubleClick={() => openEntry(e)} onClick={() => setSelected(e)}>
                <td className="px-3 py-2">{e.isDir ? "📁 " : "📄 "}{e.name}{e.isSymlink ? " ↪" : ""}</td>
                <td className="px-3 py-2 text-slate-400">{e.isDir ? "—" : formatSize(e.size)}</td>
                <td className="px-3 py-2 text-slate-400">{e.mtime ? new Date(e.mtime * 1000).toLocaleString() : "—"}</td>
              </tr>
            ))}
          </tbody>
        </table>
        </div>
      </div>
    </div>
  );
}
