// 音乐播放器（纯本地）：
// - 只能导入本地文件夹；曲库持久化在 data_dir/music/library.json，**启动不自动重扫**，
//   只有点「重新扫描」才重新读盘；
// - 顺序 / 随机（洗牌袋）/ 单曲循环三种播放模式；
// - 音效为 10 段均衡器 + 预设 + 总增益 + 声道平衡（改动即时生效并自动保存）。
//
// 播放/解码全部在 Rust（rodio + symphonia），本组件只负责状态、轮询与交互。
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import {
  ArrowRight,
  FolderPlus,
  ListMusic,
  Pause,
  Play,
  RefreshCw,
  Repeat1,
  Search,
  Shuffle,
  SkipBack,
  SkipForward,
  Square,
  Trash2,
  Volume2,
  VolumeX,
} from "lucide-react";

import { SharedButton } from "../shared/Button";
import { ConfirmDialogHost, type ConfirmRequest } from "../shared/ConfirmDialog";
import { ModuleSettingsButton } from "../shared/ModuleSettings";
import { toast } from "../shared/Toast";
import { EqDialog } from "./EqDialog";
import { createCursor, nextTrack, prevTrack, shuffleIndices, type PlaylistCursor } from "./playlist";
import {
  folderName,
  formatTime,
  type AddFolderResult,
  type EqParams,
  type EqPresetInfo,
  type MusicLibrary,
  type MusicSettings,
  type MusicTrack,
  type PlayMode,
  type PlayerState,
} from "./types";

/** 播放状态轮询间隔（ms） */
const POLL_MS = 500;
/** 设置（音量/音效）落盘防抖（ms） */
const SAVE_DEBOUNCE_MS = 500;

const MODE_ICONS = { sequence: ArrowRight, shuffle: Shuffle, single: Repeat1 } as const;

/**
 * 曲目列表列宽（百分比，合计 100%）：序号 / 标题 / 作者 / 专辑 / 时长。
 * 用 colgroup + table-fixed 让五列按比例随容器缩放，列之间不留 gap。
 */
const TRACK_COL_WIDTHS = ["6%", "42%", "20%", "20%", "12%"];

export default function MusicPanel() {
  const { t } = useTranslation();

  const [library, setLibrary] = useState<MusicLibrary>({ folders: [], tracks: [] });
  const [search, setSearch] = useState("");
  const [player, setPlayer] = useState<PlayerState | null>(null);
  const [settings, setSettings] = useState<MusicSettings | null>(null);
  const [presets, setPresets] = useState<EqPresetInfo[]>([]);
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  const [seeking, setSeeking] = useState<number | null>(null);
  const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null);
  const [busy, setBusy] = useState(false);

  const cursorRef = useRef<PlaylistCursor>(createCursor());
  const modeRef = useRef<PlayMode>("sequence");
  const tracksRef = useRef<MusicTrack[]>([]);
  const currentIndexRef = useRef<number | null>(null);
  const handledEndedRef = useRef<string | null>(null);
  const saveTimerRef = useRef<number | null>(null);

  // —— 过滤后的曲目列表（搜索 + 保持原始索引，播放索引以完整列表为准）——
  const filteredTracks = useMemo(() => {
    const keyword = search.trim().toLowerCase();
    const items = library.tracks.map((track, index) => ({ track, index }));
    if (!keyword) return items;
    return items.filter(({ track }) =>
      [track.title, track.artist, track.album, track.path]
        .join(" ")
        .toLowerCase()
        .includes(keyword)
    );
  }, [library.tracks, search]);

  useEffect(() => {
    tracksRef.current = library.tracks;
  }, [library.tracks]);

  useEffect(() => {
    modeRef.current = settings?.play_mode ?? "sequence";
  }, [settings?.play_mode]);

  // —— 初始化：曲库 / 设置 / 预设 / 播放状态 ——
  useEffect(() => {
    invoke<MusicLibrary>("music_get_library")
      .then(setLibrary)
      .catch((e) => toast(t("music.loadFail", { err: String(e) }), "err"));
    invoke<MusicSettings>("music_get_settings")
      .then(setSettings)
      .catch(() => {});
    invoke<EqPresetInfo[]>("music_list_presets")
      .then(setPresets)
      .catch(() => {});
    invoke<PlayerState>("music_get_state")
      .then(setPlayer)
      .catch(() => {});
  }, [t]);

  // —— 轮询播放状态；发现播完则按模式推进 ——
  const playIndex = useCallback(
    async (index: number | null) => {
      const list = tracksRef.current;
      if (index == null || index < 0 || index >= list.length) return;
      const track = list[index];
      try {
        const snapshot = await invoke<PlayerState>("music_play", { path: track.path });
        setPlayer(snapshot);
        currentIndexRef.current = index;
        setSelectedIndex(index);
        handledEndedRef.current = null;
      } catch (e) {
        toast(t("music.playFail", { err: String(e) }), "err");
      }
    },
    [t]
  );

  const advance = useCallback(
    async (direction: "next" | "prev") => {
      const list = tracksRef.current;
      const mode = modeRef.current;
      const options = {
        current: currentIndexRef.current,
        total: list.length,
        mode,
        cursor: cursorRef.current,
      };
      const result =
        direction === "next"
          ? nextTrack({ ...options })
          : prevTrack({ ...options });
      cursorRef.current = result.cursor;
      await playIndex(result.index);
    },
    [playIndex]
  );

  useEffect(() => {
    const timer = window.setInterval(() => {
      invoke<PlayerState>("music_get_state")
        .then((snapshot) => {
          setPlayer(snapshot);
          if (
            snapshot.status === "ended" &&
            snapshot.path &&
            handledEndedRef.current !== snapshot.path
          ) {
            // 同一首只推进一次，避免轮询重复触发
            handledEndedRef.current = snapshot.path;
            const list = tracksRef.current;
            const mode = modeRef.current;
            const result = nextTrack({
              current: currentIndexRef.current,
              total: list.length,
              mode,
              cursor: cursorRef.current,
            });
            cursorRef.current = result.cursor;
            if (result.index != null) void playIndex(result.index);
          }
        })
        .catch(() => {});
    }, POLL_MS);
    return () => window.clearInterval(timer);
  }, [playIndex]);

  // —— 设置持久化（防抖；音效改动即时预览）——
  const persistSettings = useCallback((next: MusicSettings) => {
    if (saveTimerRef.current) window.clearTimeout(saveTimerRef.current);
    saveTimerRef.current = window.setTimeout(() => {
      invoke<MusicSettings>("music_update_settings", { settings: next }).catch(() => {});
    }, SAVE_DEBOUNCE_MS);
  }, []);

  const applyEq = useCallback(
    (eq: EqParams) => {
      setSettings((prev) => {
        if (!prev) return prev;
        const next = { ...prev, eq };
        invoke<EqParams>("music_preview_eq", { eq }).catch(() => {});
        persistSettings(next);
        return next;
      });
    },
    [persistSettings]
  );

  const changeMode = useCallback(
    (mode: PlayMode) => {
      setSettings((prev) => {
        if (!prev) return prev;
        const next = { ...prev, play_mode: mode };
        if (mode === "shuffle") {
          // 切到随机模式：立刻重排洗牌袋，避免第一首就是当前曲
          cursorRef.current = {
            bag: shuffleIndices(tracksRef.current.length, Math.random, currentIndexRef.current),
            history: cursorRef.current.history,
          };
        }
        persistSettings(next);
        return next;
      });
    },
    [persistSettings]
  );

  const changeVolume = useCallback(
    (volume: number) => {
      setSettings((prev) => (prev ? { ...prev, volume } : prev));
      invoke<PlayerState>("music_set_volume", { volume })
        .then(setPlayer)
        .catch(() => {});
    },
    []
  );

  // —— 曲库操作 ——
  const importFolder = async () => {
    const picked = await open({ directory: true, title: t("music.importFolder") });
    if (typeof picked !== "string") return;
    setBusy(true);
    try {
      const result = await invoke<AddFolderResult>("music_add_folder", { path: picked });
      setLibrary(result.library);
      cursorRef.current = createCursor();
      toast(t("music.imported", { count: result.added }), "ok");
    } catch (e) {
      toast(t("music.importFail", { err: String(e) }), "err");
    } finally {
      setBusy(false);
    }
  };

  const rescan = async () => {
    setBusy(true);
    try {
      const result = await invoke<{ added: number; removed: number; library: MusicLibrary }>(
        "music_refresh_library"
      );
      setLibrary(result.library);
      cursorRef.current = createCursor();
      toast(t("music.rescanned", { added: result.added, removed: result.removed }), "ok");
    } catch (e) {
      toast(t("music.importFail", { err: String(e) }), "err");
    } finally {
      setBusy(false);
    }
  };

  const removeFolder = (folder: string) => {
    setConfirmRequest({
      title: t("music.removeFolder"),
      desc: t("music.removeFolderConfirm", { name: folderName(folder) }),
      danger: true,
      onConfirm: async () => {
        setConfirmRequest(null);
        try {
          const next = await invoke<MusicLibrary>("music_remove_folder", { path: folder });
          setLibrary(next);
          cursorRef.current = createCursor();
          currentIndexRef.current = null;
          toast(t("music.removedFolder"), "ok");
        } catch (e) {
          toast(t("music.importFail", { err: String(e) }), "err");
        }
      },
    });
  };

  // —— 播放控制 ——
  const togglePlay = async () => {
    if (!player || player.status === "idle") {
      await playIndex(selectedIndex ?? 0);
      return;
    }
    const command = player.status === "playing" ? "music_pause" : "music_resume";
    try {
      setPlayer(await invoke<PlayerState>(command));
    } catch (e) {
      toast(t("music.playFail", { err: String(e) }), "err");
    }
  };

  const stop = async () => {
    try {
      setPlayer(await invoke<PlayerState>("music_stop"));
      handledEndedRef.current = null;
    } catch {
      /* 停止失败无副作用 */
    }
  };

  const commitSeek = async () => {
    if (seeking == null) return;
    const target = seeking;
    setSeeking(null);
    try {
      setPlayer(await invoke<PlayerState>("music_seek", { positionMs: target }));
    } catch (e) {
      toast(t("music.seekFail", { err: String(e) }), "err");
    }
  };

  const mode = settings?.play_mode ?? "sequence";
  const volume = settings?.volume ?? 0.8;
  const muted = volume <= 0.001;
  const position = seeking ?? player?.position_ms ?? 0;
  const duration = player?.duration_ms ?? 0;
  const playingPath = player?.path ?? null;

  return (
    <div className="h-full flex flex-col overflow-hidden">
      {/* ── 工具条 ── */}
      <div className="flex items-center gap-2 flex-wrap p-3 border-b border-white/5 bg-white/[0.02] flex-shrink-0">
        <SharedButton variant="primary" onClick={importFolder} disabled={busy}>
          <FolderPlus className="w-3.5 h-3.5" />
          {t("music.importFolder")}
        </SharedButton>
        <SharedButton variant="secondary" onClick={rescan} disabled={busy || library.folders.length === 0}>
          <RefreshCw className={`w-3.5 h-3.5 ${busy ? "animate-spin" : ""}`} />
          {t("music.rescan")}
        </SharedButton>
        <div className="relative flex-1 min-w-[160px]">
          <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3 h-3 text-slate-500" />
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t("music.searchPh")}
            className="glass-input w-full pl-7 pr-2 h-8 text-[11px]"
          />
        </div>
        <div className="flex items-center gap-0.5 rounded-lg bg-white/5 p-0.5">
          {(["sequence", "shuffle", "single"] as PlayMode[]).map((item) => {
            const Icon = MODE_ICONS[item];
            return (
              <button
                key={item}
                onClick={() => changeMode(item)}
                title={t(`music.mode.${item}`)}
                className={`p-1.5 rounded cursor-pointer transition-all ${
                  mode === item
                    ? "bg-[var(--module-accent)] text-white"
                    : "text-slate-400 hover:text-slate-200"
                }`}
              >
                <Icon className="w-3.5 h-3.5" />
              </button>
            );
          })}
        </div>
        <ModuleSettingsButton title={t("music.eqTitle")} width={520}>
          {settings && (
            <EqDialog eq={settings.eq} presets={presets} onChange={applyEq} />
          )}
        </ModuleSettingsButton>
      </div>

      {/* ── 文件夹与统计 ── */}
      {library.folders.length > 0 && (
        <div className="flex items-center gap-1.5 flex-wrap px-3 py-2 border-b border-white/5 flex-shrink-0">
          <ListMusic className="w-3 h-3 text-slate-500 flex-shrink-0" />
          {library.folders.map((folder) => (
            <span
              key={folder}
              title={folder}
              className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded bg-white/5 border border-white/10 text-[10px] text-slate-300 max-w-[220px]"
            >
              <span className="truncate">{folderName(folder)}</span>
              <button
                onClick={() => removeFolder(folder)}
                className="text-slate-500 hover:text-rose-400 cursor-pointer flex-shrink-0"
                title={t("music.removeFolder")}
              >
                <Trash2 className="w-2.5 h-2.5" />
              </button>
            </span>
          ))}
          <span className="text-[10px] text-slate-500 ml-auto">
            {t("music.totalTracks", { count: library.tracks.length })}
          </span>
        </div>
      )}

      {/* ── 曲目列表 ── */}
      <div className="flex-1 overflow-auto">
        {library.tracks.length === 0 ? (
          <div className="h-full flex flex-col items-center justify-center gap-3 text-slate-500">
            <ListMusic className="w-10 h-10 opacity-40" />
            <p className="text-sm">{t("music.empty")}</p>
            <p className="text-[11px] text-slate-600">{t("music.emptyHint")}</p>
            <SharedButton variant="primary" onClick={importFolder}>
              <FolderPlus className="w-3.5 h-3.5" />
              {t("music.importFolder")}
            </SharedButton>
          </div>
        ) : filteredTracks.length === 0 ? (
          <div className="p-6 text-center text-slate-500 text-[11px]">{t("music.noMatch")}</div>
        ) : (
          /*
            列宽：按百分比分配（colgroup + table-fixed），列与列之间不留 gap——
            宽度全部随容器缩放，「序号 / 标题 / 作者 / 专辑 / 时长」五列合计 100%。
          */
          <table className="w-full table-fixed border-collapse text-[11px] text-left">
            <colgroup>
              {TRACK_COL_WIDTHS.map((width) => (
                <col key={width} style={{ width }} />
              ))}
            </colgroup>
            <thead className="sticky top-0 z-10 bg-slate-900/95 backdrop-blur">
              <tr className="text-slate-500">
                <th className="py-2 text-center font-medium border-b border-white/5">
                  {t("music.thIndex")}
                </th>
                <th className="py-2 font-medium border-b border-white/5">{t("music.thTitle")}</th>
                <th className="py-2 font-medium border-b border-white/5">{t("music.thArtist")}</th>
                <th className="py-2 font-medium border-b border-white/5">{t("music.thAlbum")}</th>
                <th className="py-2 text-right font-medium border-b border-white/5">
                  {t("music.thDuration")}
                </th>
              </tr>
            </thead>
            <tbody>
              {filteredTracks.map(({ track, index }) => {
                const isPlaying = playingPath === track.path;
                const isSelected = selectedIndex === index;
                return (
                  <tr
                    key={track.path}
                    onClick={() => setSelectedIndex(index)}
                    onDoubleClick={() => void playIndex(index)}
                    className={`cursor-pointer transition-colors border-b border-white/[0.03] ${
                      isSelected ? "bg-[var(--module-accent-soft)]" : "hover:bg-white/[0.03]"
                    }`}
                  >
                    <td className="py-2 text-center font-mono text-slate-500" title={track.path}>
                      {isPlaying ? <span className="text-[var(--module-accent)]">♪</span> : index + 1}
                    </td>
                    <td
                      className={`py-2 truncate ${
                        isPlaying ? "text-[var(--module-accent)] font-semibold" : "text-slate-200"
                      }`}
                      title={track.title}
                    >
                      {track.title}
                    </td>
                    <td className="py-2 truncate text-slate-400" title={track.artist}>
                      {track.artist || t("music.unknownArtist")}
                    </td>
                    <td className="py-2 truncate text-slate-500" title={track.album}>
                      {track.album}
                    </td>
                    <td className="py-2 text-right font-mono text-slate-400">
                      {formatTime(track.duration_ms)}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>

      {/* ── 播放条 ── */}
      <div className="border-t border-white/10 bg-white/[0.02] px-3 py-2.5 flex items-center gap-3 flex-shrink-0">
        <div className="flex items-center gap-1 flex-shrink-0">
          <button
            onClick={() => void advance("prev")}
            disabled={library.tracks.length === 0}
            className="p-1.5 rounded text-slate-300 hover:text-white hover:bg-white/10 cursor-pointer disabled:opacity-40"
            title={t("music.prev")}
          >
            <SkipBack className="w-4 h-4" />
          </button>
          <button
            onClick={() => void togglePlay()}
            disabled={library.tracks.length === 0}
            className="p-2 rounded-lg bg-[var(--module-accent)] text-white hover:opacity-85 cursor-pointer disabled:opacity-40"
            title={player?.status === "playing" ? t("music.pause") : t("music.play")}
          >
            {player?.status === "playing" ? <Pause className="w-4 h-4" /> : <Play className="w-4 h-4" />}
          </button>
          <button
            onClick={() => void advance("next")}
            disabled={library.tracks.length === 0}
            className="p-1.5 rounded text-slate-300 hover:text-white hover:bg-white/10 cursor-pointer disabled:opacity-40"
            title={t("music.next")}
          >
            <SkipForward className="w-4 h-4" />
          </button>
          <button
            onClick={() => void stop()}
            disabled={!player || player.status === "idle"}
            className="p-1.5 rounded text-slate-400 hover:text-white hover:bg-white/10 cursor-pointer disabled:opacity-40"
            title={t("music.stop")}
          >
            <Square className="w-3.5 h-3.5" />
          </button>
        </div>

        <div className="flex items-center gap-2 flex-1 min-w-0">
          <span className="text-[10px] font-mono text-slate-400 w-10 text-right">
            {formatTime(position)}
          </span>
          <input
            type="range"
            min={0}
            max={Math.max(duration, 1)}
            step={1000}
            value={position}
            disabled={!duration}
            onChange={(e) => setSeeking(Number(e.target.value))}
            onMouseUp={() => void commitSeek()}
            onKeyUp={() => void commitSeek()}
            className="flex-1 accent-[var(--module-accent)] cursor-pointer disabled:cursor-default"
          />
          <span className="text-[10px] font-mono text-slate-400 w-10">{formatTime(duration)}</span>
        </div>

        <div className="min-w-0 w-48 flex-shrink-0">
          <p className="text-[11px] text-slate-200 truncate">
            {player?.title ?? t("music.notPlaying")}
          </p>
          <p className="text-[10px] text-slate-500 truncate">{player?.artist ?? ""}</p>
        </div>

        <div className="flex items-center gap-1.5 flex-shrink-0 w-32">
          <button
            onClick={() => changeVolume(muted ? 0.8 : 0)}
            className="text-slate-400 hover:text-slate-200 cursor-pointer flex-shrink-0"
            title={t("music.mute")}
          >
            {muted ? <VolumeX className="w-3.5 h-3.5" /> : <Volume2 className="w-3.5 h-3.5" />}
          </button>
          <input
            type="range"
            min={0}
            max={1}
            step={0.01}
            value={volume}
            onChange={(e) => changeVolume(Number(e.target.value))}
            className="flex-1 accent-[var(--module-accent)] cursor-pointer"
            title={`${t("music.volume")} ${Math.round(volume * 100)}%`}
          />
        </div>
      </div>

      <ConfirmDialogHost request={confirmRequest} onClose={() => setConfirmRequest(null)} />
    </div>
  );
}
