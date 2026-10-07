// Suno 下载：粘贴用户主页 URL，解析该主页所有歌曲，勾选批量下载为 mp3（内置 ffmpeg 转码）。
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import {
  Music2,
  Search,
  Download,
  CheckSquare,
  Square,
  Loader2,
  AlertTriangle,
  Star,
  X,
} from "lucide-react";

interface SunoSong {
  id: string;
  title: string;
  audioUrl: string;
  imageUrl?: string | null;
  playCount: number;
  downloaded: boolean;
}

interface DownloadReport {
  succeeded: number;
  failed: string[];
}

interface Progress {
  current: number;
  total: number;
  title: string;
  stage: "download" | "transcode";
}

export default function SunoPanel() {
  const { t } = useTranslation();
  const [url, setUrl] = useState("");
  const [songs, setSongs] = useState<SunoSong[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [parsing, setParsing] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);
  const [profiles, setProfiles] = useState<string[]>([]);
  const unlistenRef = useRef<UnlistenFn | null>(null);

  // 监听下载进度
  useEffect(() => {
    let disposed = false;
    listen<Progress>("suno-download-progress", (e) => {
      if (!disposed) setProgress(e.payload);
    }).then((un) => {
      if (disposed) un();
      else unlistenRef.current = un;
    });
    return () => {
      disposed = true;
      unlistenRef.current?.();
      unlistenRef.current = null;
    };
  }, []);

  const showMsg = (ok: boolean, text: string) => setMessage({ ok, text });

  // 加载收藏的主页
  const loadProfiles = async () => {
    try {
      setProfiles(await invoke<string[]>("suno_list_profiles"));
    } catch {
      /* 忽略 */
    }
  };
  useEffect(() => {
    void loadProfiles();
  }, []);

  const saveProfile = async () => {
    const trimmed = url.trim();
    if (!trimmed) {
      showMsg(false, t("suno.emptyUrl"));
      return;
    }
    try {
      setProfiles(await invoke<string[]>("suno_save_profile", { url: trimmed }));
      showMsg(true, t("suno.profileSaved"));
    } catch (e) {
      showMsg(false, String(e));
    }
  };

  const removeProfile = async (u: string) => {
    try {
      setProfiles(await invoke<string[]>("suno_remove_profile", { url: u }));
    } catch (e) {
      showMsg(false, String(e));
    }
  };

  const parse = async (target?: string) => {
    const trimmed = (target ?? url).trim();
    if (!trimmed) {
      showMsg(false, t("suno.emptyUrl"));
      return;
    }
    setParsing(true);
    setMessage(null);
    setSongs([]);
    setSelected(new Set());
    try {
      const list = await invoke<SunoSong[]>("suno_parse_profile", { url: trimmed });
      setSongs(list);
      showMsg(true, t("suno.parsed", { count: list.length }));
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setParsing(false);
    }
  };

  const toggle = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  // 全选 = 只勾选「未下载过」的歌曲（已下载的跳过）；已全选未下载则清空。
  const toggleAll = () => {
    setSelected((prev) => {
      const unDownloaded = songs.filter((s) => !s.downloaded);
      if (unDownloaded.length === 0) return new Set();
      if (prev.size === unDownloaded.length) return new Set();
      return new Set(unDownloaded.map((s) => s.id));
    });
  };

  const download = async () => {
    if (selected.size === 0) {
      showMsg(false, t("suno.noneSelected"));
      return;
    }
    let dir: string | null = null;
    try {
      const picked = await openDialog({ directory: true, multiple: false, title: t("suno.pickDirTitle") });
      dir = typeof picked === "string" ? picked : null;
    } catch {
      dir = null;
    }
    if (!dir) return;

    const refs = songs
      .filter((s) => selected.has(s.id))
      .map((s) => ({ id: s.id, title: s.title, audioUrl: s.audioUrl }));

    setDownloading(true);
    setProgress({ current: 0, total: refs.length, title: "", stage: "download" });
    setMessage(null);
    try {
      const report = await invoke<DownloadReport>("suno_download_songs", {
        songs: refs,
        dir,
      });
      if (report.failed.length === 0) {
        showMsg(true, t("suno.done", { count: report.succeeded }));
      } else {
        showMsg(
          false,
          t("suno.donePartial", { ok: report.succeeded, fail: report.failed.length }) +
            "\n" +
            report.failed.join("\n"),
        );
      }
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setDownloading(false);
    }
  };

  const selectedCount = selected.size;

  return (
    <div className="h-full w-full flex flex-col p-4 gap-3 overflow-hidden">
      {/* 输入行 */}
      <div className="flex items-center gap-2 flex-shrink-0">
        <div className="flex items-center gap-1.5 px-2 h-8 rounded-md border border-white/10 bg-black/30 flex-1">
          <Music2 className="w-3.5 h-3.5 text-[var(--module-accent)]" />
          <input
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && !parsing && void parse()}
            placeholder={t("suno.urlPlaceholder")}
            className="flex-1 bg-transparent text-body text-slate-100 outline-none placeholder:text-slate-600"
          />
        </div>
        <button
          onClick={() => void saveProfile()}
          disabled={!url.trim()}
          className="inline-flex items-center px-2 h-8 rounded-md border border-white/10 text-slate-400 hover:text-amber-300 hover:border-amber-300/40 disabled:opacity-40 cursor-pointer"
          title={t("suno.saveProfile")}
        >
          <Star className="w-3.5 h-3.5" />
        </button>
        <button
          onClick={() => void parse()}
          disabled={parsing}
          className="inline-flex items-center gap-1 px-3 h-8 rounded-md bg-[var(--module-accent)] text-white text-caption font-medium hover:opacity-90 disabled:opacity-40 cursor-pointer"
        >
          {parsing ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Search className="w-3.5 h-3.5" />}
          {t("suno.parse")}
        </button>
      </div>

      {/* 收藏的主页 */}
      {profiles.length > 0 && (
        <div className="flex items-center gap-1.5 flex-wrap flex-shrink-0">
          {profiles.map((p) => (
            <span
              key={p}
              className="inline-flex items-center gap-1 pl-2 pr-1 py-0.5 rounded-md bg-white/5 border border-white/10 text-tiny text-slate-300 max-w-full"
            >
              <button
                onClick={() => {
                  setUrl(p);
                  void parse(p);
                }}
                className="hover:text-[var(--module-accent)] truncate max-w-52"
                title={p}
              >
                {p.replace(/^https?:\/\//, "").replace(/^www\./, "")}
              </button>
              <button
                onClick={() => void removeProfile(p)}
                className="text-slate-500 hover:text-red-400 flex-shrink-0"
                title={t("suno.removeProfile")}
              >
                <X className="w-3 h-3" />
              </button>
            </span>
          ))}
        </div>
      )}

      {/* 提示 */}
      {message && (
        <div
          className={`flex-shrink-0 px-3 py-2 rounded-md text-tiny whitespace-pre-wrap ${
            message.ok ? "bg-emerald-500/10 text-emerald-300" : "bg-red-500/10 text-red-300"
          }`}
        >
          {message.text}
        </div>
      )}

      {/* 列表头：全选 + 下载 */}
      {songs.length > 0 && (
        <div className="flex items-center gap-2 flex-shrink-0">
          <button
            onClick={toggleAll}
            className="inline-flex items-center gap-1 px-2 py-1 rounded-md border border-white/10 text-slate-400 hover:text-white hover:border-white/25 cursor-pointer text-tiny"
          >
            {selectedCount > 0 &&
            selectedCount === songs.filter((s) => !s.downloaded).length ? (
              <CheckSquare className="w-3.5 h-3.5" />
            ) : (
              <Square className="w-3.5 h-3.5" />
            )}
            {t("suno.selectAllUnDownloaded")}
          </button>
          <span className="text-tiny text-slate-500">
            {t("suno.selectedCount", { count: selectedCount, total: songs.filter((s) => !s.downloaded).length })}
          </span>
          <div className="flex-1" />
          <button
            onClick={() => void download()}
            disabled={selectedCount === 0 || downloading}
            className="inline-flex items-center gap-1 px-3 h-8 rounded-md bg-[var(--module-accent)] text-white text-caption font-medium hover:opacity-90 disabled:opacity-40 cursor-pointer"
          >
            {downloading ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Download className="w-3.5 h-3.5" />}
            {t("suno.download")}
          </button>
        </div>
      )}

      {/* 进度条：整体 current/total + 单曲阶段 */}
      {downloading && progress && progress.total > 0 && (
        <div className="flex-shrink-0 px-3 py-2 rounded-md bg-white/5 space-y-1.5">
          <div className="flex items-center justify-between text-tiny text-slate-300 gap-2">
            <span className="truncate">
              {t(
                progress.stage === "download" ? "suno.progressDownload" : "suno.progressTranscode",
                { current: progress.current, total: progress.total, title: progress.title },
              )}
            </span>
            <span className="text-slate-500 shrink-0">
              {Math.round((progress.current / progress.total) * 100)}%
            </span>
          </div>
          <div className="h-1.5 rounded-full bg-white/10 overflow-hidden">
            <div
              className={`h-full rounded-full transition-all duration-300 ${
                progress.stage === "download" ? "bg-sky-400" : "bg-[var(--module-accent)]"
              }`}
              style={{ width: `${(progress.current / progress.total) * 100}%` }}
            />
          </div>
        </div>
      )}

      {/* 歌曲列表 */}
      <div className="flex-1 overflow-y-auto">
        {songs.length === 0 ? (
          <div className="h-full flex flex-col items-center justify-center text-slate-500 gap-2">
            <Music2 className="w-8 h-8 opacity-40" />
            <span className="text-caption">{t("suno.empty")}</span>
          </div>
        ) : (
          <ul className="space-y-1.5">
            {songs.map((s) => {
              const checked = selected.has(s.id);
              return (
                <li
                  key={s.id}
                  onClick={() => toggle(s.id)}
                  className={`flex items-center gap-2 px-2 py-1.5 rounded-md border cursor-pointer transition ${
                    checked
                      ? "border-[var(--module-accent)]/50 bg-[var(--module-accent)]/10"
                      : "border-white/5 hover:border-white/15 bg-black/20"
                  }`}
                >
                  {checked ? (
                    <CheckSquare className="w-4 h-4 text-[var(--module-accent)] flex-shrink-0" />
                  ) : (
                    <Square className="w-4 h-4 text-slate-500 flex-shrink-0" />
                  )}
                  {s.imageUrl ? (
                    <img
                      src={s.imageUrl}
                      alt=""
                      className="w-9 h-9 rounded object-cover flex-shrink-0 bg-white/5"
                      loading="lazy"
                      onError={(e) => {
                        (e.currentTarget as HTMLImageElement).style.display = "none";
                      }}
                    />
                  ) : (
                    <div className="w-9 h-9 rounded bg-white/5 flex items-center justify-center flex-shrink-0">
                      <Music2 className="w-4 h-4 text-slate-500" />
                    </div>
                  )}
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-1.5">
                      <span className="text-body text-slate-100 truncate">{s.title}</span>
                      {s.downloaded && (
                        <span className="flex-shrink-0 text-micro px-1 rounded bg-emerald-500/15 text-emerald-400">
                          {t("suno.downloaded")}
                        </span>
                      )}
                    </div>
                    <div className="text-micro text-slate-500">{t("suno.playCount", { count: s.playCount })}</div>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </div>

      {/* 下载中遮罩提示（转码可能较慢） */}
      {downloading && (
        <div className="flex-shrink-0 flex items-center gap-2 px-3 py-2 rounded-md bg-amber-500/10 text-amber-300 text-tiny">
          <AlertTriangle className="w-3.5 h-3.5" />
          {t("suno.transcodeHint")}
        </div>
      )}
    </div>
  );
}
