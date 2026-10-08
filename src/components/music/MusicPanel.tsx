// 音乐播放器（纯本地）：
// - 只能导入本地文件夹；曲库持久化在 data_dir/music/library.json，**启动不自动重扫**，
//   只有点「重新扫描」才重新读盘；
// - 顺序 / 随机（洗牌袋）/ 单曲循环三种播放模式；
// - 音效为 10 段均衡器 + 预设 + 总增益 + 声道平衡（改动即时生效并自动保存）。
//
// 播放/解码/队列推进全部在 Rust（rodio + symphonia）：
// 窗口隐藏到托盘后 WebView2 会节流前端定时器，切歌必须由后端负责
// （见 src-tauri/src/commands/music/{player,queue}.rs）；本组件只负责
// 把曲库顺序同步给后端、驱动用户操作、以及轮询展示状态。
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import {
  ArrowRight,
  Download,
  FolderOpen,
  FolderPlus,
  ListMusic,
  Loader2,
  Package,
  Pause,
  Pencil,
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
  X,
} from "lucide-react";

import { SharedButton } from "../shared/Button";
import { ConfirmDialogHost, type ConfirmRequest } from "../shared/ConfirmDialog";
import { ModuleSettingsButton } from "../shared/ModuleSettings";
import { toast } from "../shared/Toast";
import VexEmptyState from "../VexEmptyState";
import { MusicSettingsDialog } from "./MusicSettings";
import OnlineSearch from "./OnlineSearch";
import PluginManager from "./PluginManager";
import {
  folderName,
  formatTime,
  MUSIC_QUALITIES,
  type AddFolderResult,
  type DeleteTracksResult,
  type EqParams,
  type EqPresetInfo,
  type MusicLibrary,
  type MusicQuality,
  type MusicSettings,
  type MusicTrack,
  type PlayMode,
  type PlayerState,
  type PluginDownloadOutcome,
  type RenameTrackResult,
  type TrackNameSuggestion,
} from "./types";

/** 播放状态轮询间隔（ms） */
const POLL_MS = 500;
/**
 * 连续轮询失败多少次才弹提示。
 *
 * 单次失败常是切歌/启动瞬间的竞态，立刻弹会刷屏；但一直不弹又成了静默失败
 * （界面停在旧状态，用户以为"没变化"）。取 3 次（约 1.5s）作为折中。
 */
const POLL_FAIL_ALERT_AFTER = 3;
/** 设置（音量/音效）落盘防抖（ms） */
const SAVE_DEBOUNCE_MS = 500;

const MODE_ICONS = { sequence: ArrowRight, shuffle: Shuffle, single: Repeat1 } as const;

/**
 * 曲目列表列宽（百分比，合计 100%）：序号 / 标题 / 作者 / 专辑 / 时长 / 操作。
 * 用 colgroup + table-fixed 让六列按比例随容器缩放，列之间不留 gap。
 */
const TRACK_COL_WIDTHS = ["5%", "34%", "18%", "16%", "10%", "17%"];

/**
 * 音乐模块内的三个视图。
 *
 * 刻意做成**模块内视图**而不是三个顶级模块：它们是同一件事的三个面，
 * 在线播放要落回同一个播放条，独立模块拿不到播放器状态，会变成「搜到歌不能直接播」。
 */
type MusicView = "library" | "online" | "plugins";

const VIEW_TABS: { id: MusicView; icon: typeof ListMusic; label: string }[] = [
  { id: "library", icon: ListMusic, label: "tabLibrary" },
  { id: "online", icon: Search, label: "tabOnline" },
  { id: "plugins", icon: Package, label: "tabPlugins" },
];

export default function MusicPanel() {
  const { t } = useTranslation();

  const [view, setView] = useState<MusicView>("library");
  const [library, setLibrary] = useState<MusicLibrary>({ folders: [], tracks: [] });
  const [search, setSearch] = useState("");
  /** 文件夹筛选：null = 全部；否则只显示该导入目录下的曲目（并同步收窄播放队列） */
  const [folderFilter, setFolderFilter] = useState<string | null>(null);
  const [player, setPlayer] = useState<PlayerState | null>(null);
  const [settings, setSettings] = useState<MusicSettings | null>(null);
  const [presets, setPresets] = useState<EqPresetInfo[]>([]);
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  const [seeking, setSeeking] = useState<number | null>(null);
  const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null);
  const [busy, setBusy] = useState(false);
  // 重命名弹窗（null = 关闭）：名字由后端按音频标签给的推荐名预填，可手改
  const [renameTrack, setRenameTrack] = useState<MusicTrack | null>(null);
  const [renameName, setRenameName] = useState("");
  const [renameFromTags, setRenameFromTags] = useState(false);
  const [renaming, setRenaming] = useState(false);
  // 「下载当前曲目」：null = 跟随当前缓存的音质，用户选过就以选择为准
  const [dlQuality, setDlQuality] = useState<MusicQuality | null>(null);
  const [dlBusy, setDlBusy] = useState(false);

  const tracksRef = useRef<MusicTrack[]>([]);
  /** path -> 曲库下标（后端自行切歌时用于同步选中行） */
  const pathIndexRef = useRef<Map<string, number>>(new Map());
  const saveTimerRef = useRef<number | null>(null);
  /** 连续轮询失败计数（成功即清零，用于失败可感知提示） */
  const pollFailRef = useRef(0);

  // —— 过滤后的曲目列表（文件夹筛选 + 搜索 + 保持原始索引，播放索引以完整列表为准）——
  /**
   * `folderFilter` 非空 = 只看该导入目录下的曲目。
   *
   * 它同时管两件事（这是本需求的关键）：列表**显示**范围，以及交给后端的
   * **播放队列**范围 —— 点文件夹名筛选后，播放列表里就只有这个文件夹的歌。
   * 只筛显示、不筛队列的话，点「下一首」会跳到别的文件夹去，与界面所见不一致。
   */
  const filteredTracks = useMemo(() => {
    const keyword = search.trim().toLowerCase();
    const folder = folderFilter;
    return library.tracks
      .map((track, index) => ({ track, index }))
      .filter(({ track }) => (folder ? track.folder === folder : true))
      .filter(({ track }) =>
        keyword
          ? [track.title, track.artist, track.album, track.path]
              .join(" ")
              .toLowerCase()
              .includes(keyword)
          : true,
      );
  }, [library.tracks, search, folderFilter]);

  /** 筛选后的路径序列：既是列表可见范围，也是后端队列 */
  const visiblePaths = useMemo(
    () => filteredTracks.map(({ track }) => track.path),
    [filteredTracks],
  );

  useEffect(() => {
    tracksRef.current = library.tracks;
    pathIndexRef.current = new Map(library.tracks.map((track, index) => [track.path, index]));
  }, [library.tracks]);

  // 被筛掉的目录若正好是当前筛选源，先解除筛选，否则界面会停在空列表上
  useEffect(() => {
    if (folderFilter && !library.folders.includes(folderFilter)) {
      setFolderFilter(null);
    }
  }, [library.folders, folderFilter]);

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

  // —— 队列同步：可见曲目顺序或播放模式变化时交给后端（后端据此自动续播）——
  //
  // 用 `visiblePaths`（含文件夹筛选）而非全曲库：筛选到某个文件夹后，
  // 播放列表应当只含这个文件夹的歌，「下一首」才不会跳到别的文件夹。
  useEffect(() => {
    if (!settings || visiblePaths.length === 0) return;
    invoke("music_set_queue", {
      paths: visiblePaths,
      mode: settings.play_mode,
    }).catch(() => {});
  }, [visiblePaths, settings?.play_mode]);

  /** 把后端状态同步到界面；后端可能自行切歌（含托盘模式），因此要同步选中行 */
  const syncState = useCallback((snapshot: PlayerState) => {
    setPlayer(snapshot);
    if (snapshot.path) {
      const index = pathIndexRef.current.get(snapshot.path);
      if (index != null) setSelectedIndex(index);
    }
  }, []);

  // —— 播放状态轮询：只做 UI 同步，切歌由后端负责 ——
  useEffect(() => {
    const refresh = () => {
      invoke<PlayerState>("music_get_state")
        .then((snapshot) => {
          pollFailRef.current = 0;
          syncState(snapshot);
        })
        .catch((err) => {
          // 静默失败会让界面停在旧状态，用户会以为是「没变化」而不是「取数失败」。
          // 但要连续失败几次才提示：单次失败常是切歌/启动瞬间的竞态，每次都弹会刷屏。
          pollFailRef.current += 1;
          if (pollFailRef.current === POLL_FAIL_ALERT_AFTER) {
            toast(t("music.statePollFailed", { err: String(err) }), "err");
          }
        });
    };
    const timer = window.setInterval(refresh, POLL_MS);
    // 从托盘/最小化返回时立即对齐一次（隐藏期间 WebView2 会节流定时器）
    const onVisibility = () => {
      if (!document.hidden) refresh();
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [syncState, t]);

  const playIndex = useCallback(
    async (index: number | null) => {
      const list = tracksRef.current;
      if (index == null || index < 0 || index >= list.length) return;
      try {
        const snapshot = await invoke<PlayerState>("music_play", { path: list[index].path });
        syncState(snapshot);
        setSelectedIndex(index);
      } catch (e) {
        toast(t("music.playFail", { err: String(e) }), "err");
      }
    },
    [syncState, t]
  );

  /** 上一首 / 下一首：交给后端队列（保持与托盘模式一致的推进规则） */
  const advance = useCallback(
    async (direction: "next" | "prev") => {
      try {
        const snapshot = await invoke<PlayerState>(
          direction === "next" ? "music_next" : "music_prev"
        );
        syncState(snapshot);
      } catch (e) {
        toast(t("music.playFail", { err: String(e) }), "err");
      }
    },
    [syncState, t]
  );

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
        // 队列会随 play_mode 变化重新同步给后端（见上方 set_queue effect），
        // 随机模式由后端重新洗牌，无需前端干预。
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
          toast(t("music.removedFolder"), "ok");
        } catch (e) {
          toast(t("music.importFail", { err: String(e) }), "err");
        }
      },
    });
  };

  // —— 单曲文件操作：重命名 / 删除 ——

  /// 打开重命名弹窗：先向后端要一个按音频标签生成的推荐名
  const openRename = async (track: MusicTrack) => {
    setRenameTrack(track);
    setRenameName(track.path.split(/[\\/]/).pop() ?? track.title);
    setRenameFromTags(false);
    try {
      const suggestion = await invoke<TrackNameSuggestion>("music_track_name_suggestion", {
        path: track.path,
      });
      setRenameName(suggestion.suggested_name);
      setRenameFromTags(suggestion.from_tags);
    } catch {
      // 拿不到建议就沿用现名，用户仍可手动改
    }
  };

  const submitRename = async () => {
    if (!renameTrack || renaming) return;
    const name = renameName.trim();
    if (!name) return;
    setRenaming(true);
    try {
      const result = await invoke<RenameTrackResult>("music_rename_track", {
        path: renameTrack.path,
        newName: name,
      });
      setLibrary(result.library);
      setRenameTrack(null);
      toast(t(result.renamed ? "music.renamed" : "music.renameUnchanged"), "ok");
    } catch (e) {
      toast(t("music.renameFail", { err: String(e) }), "err");
    } finally {
      setRenaming(false);
    }
  };

  /// 定位文件：在资源管理器里**选中**该曲目文件。
  ///
  /// 复用 launcher 的 `launcher_reveal_file`，不另起一个 music 命令：定位这事与音乐无关，
  /// 而那条实现已经踩过坑（管理员降权时 `/select,"path"` 会被已运行的 explorer 误判，
  /// 退化成打开「我的文档」），重复实现只会把坑再踩一遍。
  /// 文件在曲库之外被移走/删掉时后端会报错 —— 照实提示，让用户知道曲库已经过期。
  const revealTrack = async (track: MusicTrack) => {
    try {
      await invoke("launcher_reveal_file", { path: track.path });
    } catch (e) {
      toast(t("music.revealFail", { err: String(e) }), "err");
    }
  };

  const removeTrack = (track: MusicTrack) => {
    setConfirmRequest({
      title: t("music.deleteTrack"),
      desc: t("music.deleteTrackConfirm", { name: track.title }),
      danger: true,
      onConfirm: async () => {
        setConfirmRequest(null);
        try {
          const result = await invoke<DeleteTracksResult>("music_delete_tracks", {
            paths: [track.path],
          });
          setLibrary(result.library);
          // 删到正在播放那首时后端已切歌，这里同步播放状态
          syncState(result.player);
          if (result.failed.length > 0) {
            toast(t("music.deleteFail", { err: result.failed.join("; ") }), "err");
          } else {
            toast(t("music.deleted"), "ok");
          }
        } catch (e) {
          toast(t("music.deleteFail", { err: String(e) }), "err");
        }
      },
    });
  };

  // —— 播放控制 ——
  const togglePlay = async () => {
    // 空闲/已播完：优先播当前选中行（没选则从头开始）；否则交给后端切换播放态
    if (!player || player.status === "idle" || player.status === "ended") {
      await playIndex(selectedIndex ?? 0);
      return;
    }
    try {
      syncState(await invoke<PlayerState>("music_toggle"));
    } catch (e) {
      toast(t("music.playFail", { err: String(e) }), "err");
    }
  };

  const stop = async () => {
    try {
      syncState(await invoke<PlayerState>("music_stop"));
    } catch {
      /* 停止失败无副作用 */
    }
  };

  // 当前播的是在线音源时，把这首按选定音质存进下载目录。
  // 播放器记着它的来源（插件 + 原始曲目），所以搜索结果清掉之后也照样能下。
  const downloadCurrent = async () => {
    if (!player?.online) return;
    setDlBusy(true);
    try {
      const outcome = await invoke<PluginDownloadOutcome>("music_plugin_download_current", {
        quality: dlQuality ?? player.online.quality,
      });
      // 下载会自动登记进曲库，直接采用后端回传的最新曲库
      setLibrary(outcome.library);
      toast(t("music.onlineDownloaded", { dir: outcome.dir }), "ok");
    } catch (e) {
      toast(t("music.onlineDownloadFail", { err: String(e) }), "err");
    } finally {
      setDlBusy(false);
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

  /**
   * 只让当前视图显示。
   *
   * 用类名而不是条件卸载：曲库视图切走再切回时，搜索词 / 选中行 / 滚动位置都还在。
   * （Tailwind 的 `hidden` 定义在 `flex` 之后，能覆盖 `flex` 的 display。）
   */
  const showing = (target: MusicView) => (view === target ? "" : "hidden");

  return (
    <div className="h-full flex flex-col overflow-hidden">
      {/* ── 视图切换：曲库 / 在线搜索 / 插件 ── */}
      <div className="flex items-center gap-0.5 px-3 pt-2 flex-shrink-0">
        {VIEW_TABS.map(({ id, icon: Icon, label }) => {
          const active = view === id;
          return (
            <button
              key={id}
              onClick={() => setView(id)}
              className={`flex items-center gap-1.5 px-2.5 py-1 rounded-t-ctl text-caption cursor-pointer transition-colors border-b-2 ${
                active
                  ? "text-slate-100 border-[var(--module-accent)] bg-white/[0.05]"
                  : "text-slate-500 border-transparent hover:text-slate-300"
              }`}
            >
              <Icon className="w-3 h-3" />
              {t(`music.${label}`)}
            </button>
          );
        })}
      </div>

      {/* ── 工具条 ── */}
      <div
        className={`flex items-center gap-2 flex-wrap p-3 border-b border-white/5 bg-white/[0.02] flex-shrink-0 ${showing(
          "library"
        )}`}
      >
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
            className="glass-input w-full pl-7 pr-2 h-8 text-caption"
          />
        </div>
        <div className="flex items-center gap-0.5 rounded-ctl bg-white/5 p-0.5">
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
        <ModuleSettingsButton title={t("music.settingsTitle")} width={560}>
          {settings && (
            <MusicSettingsDialog eq={settings.eq} presets={presets} onEqChange={applyEq} />
          )}
        </ModuleSettingsButton>
      </div>

      {/* ── 文件夹与统计 ── */}
      {view === "library" && library.folders.length > 0 && (
        <div className="flex items-center gap-1.5 flex-wrap px-3 py-2 border-b border-white/5 flex-shrink-0">
          <ListMusic className="w-3 h-3 text-slate-500 flex-shrink-0" />
          {/*
            文件夹名本身是筛选开关：点一下只看这个文件夹的文件（播放队列同步收窄），
            再点一下取消。选中态用 accent 描边，和旁边的「✕ 删除」区分开——
            一个是切视图，一个是移出曲库，按钮挨在一起很容易误点。
          */}
          {library.folders.map((folder) => {
            const active = folderFilter === folder;
            return (
              <span
                key={folder}
                title={active ? t("music.filterAllHint") : `${t("music.filterFolder")}: ${folderName(folder)}`}
                className={`group inline-flex items-center gap-1 pl-1.5 pr-0.5 py-0.5 rounded border max-w-[220px] cursor-pointer transition-colors text-tiny ${
                  active
                    ? "border-[var(--module-accent)]/60 bg-[var(--module-accent)]/15 text-white"
                    : "bg-white/5 border-white/10 text-slate-300 hover:bg-white/10"
                }`}
              >
                <span
                  className="truncate"
                  onClick={() => setFolderFilter(active ? null : folder)}
                >
                  {folderName(folder)}
                </span>
                {active && (
                  <X
                    className="w-2.5 h-2.5 flex-shrink-0 text-[var(--module-accent)] cursor-pointer"
                    onClick={() => setFolderFilter(null)}
                  />
                )}
                <button
                  onClick={() => removeFolder(folder)}
                  className="text-slate-500 hover:text-rose-400 cursor-pointer flex-shrink-0"
                  title={t("music.removeFolder")}
                >
                  <Trash2 className="w-2.5 h-2.5" />
                </button>
              </span>
            );
          })}
          <span className="text-tiny text-slate-500 ml-auto">
            {/* 筛选时同时给出「当前可见 / 曲库总数」，否则筛选后数字不变会让人以为没生效 */}
            {folderFilter
              ? t("music.filteredTracks", {
                  shown: filteredTracks.length,
                  total: library.tracks.length,
                })
              : t("music.totalTracks", { count: library.tracks.length })}
          </span>
        </div>
      )}

      {/* ── 曲目列表 ── */}
      <div className={`flex-1 overflow-auto ${showing("library")}`}>
        {library.tracks.length === 0 ? (
          <VexEmptyState
            title={t("music.empty")}
            desc={t("music.emptyHint")}
            tick={t("music.emptyTick")}
            avatarSize={44}
            className="h-full"
            action={{ label: t("music.importFolder"), onClick: importFolder }}
          />
        ) : filteredTracks.length === 0 ? (
          <VexEmptyState
            title={t("music.noMatch")}
            tick={t("music.noMatchTick")}
            avatarSize={36}
            className="!py-10"
          />
        ) : (
          /*
            列宽：按百分比分配（colgroup + table-fixed），列与列之间不留 gap——
            宽度全部随容器缩放，「序号 / 标题 / 作者 / 专辑 / 时长」五列合计 100%。
          */
          <table className="w-full table-fixed border-collapse text-caption text-left">
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
                <th className="py-2 text-center font-medium border-b border-white/5">
                  {t("music.thActions")}
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
                    {/* 单曲操作：播放 / 定位文件 / 重命名（改磁盘文件名）/ 删除（移入回收站）。
                        stopPropagation：避免触发行级选中与双击播放。 */}
                    <td className="py-1.5 text-center">
                      <div className="flex items-center justify-center gap-0.5">
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            void playIndex(index);
                          }}
                          title={t("music.play")}
                          className="p-1 rounded text-slate-500 hover:text-[var(--module-accent)] hover:bg-white/10 cursor-pointer transition-all"
                        >
                          <Play className="w-3 h-3" />
                        </button>
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            void revealTrack(track);
                          }}
                          title={t("music.revealTrack")}
                          className="p-1 rounded text-slate-500 hover:text-[var(--module-accent)] hover:bg-amber-500/10 cursor-pointer transition-all"
                        >
                          <FolderOpen className="w-3 h-3" />
                        </button>
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            void openRename(track);
                          }}
                          title={t("music.rename")}
                          className="p-1 rounded text-slate-500 hover:text-[var(--module-accent)] hover:bg-blue-500/10 cursor-pointer transition-all"
                        >
                          <Pencil className="w-3 h-3" />
                        </button>
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            removeTrack(track);
                          }}
                          title={t("music.deleteTrack")}
                          className="p-1 rounded text-slate-500 hover:text-red-400 hover:bg-red-500/10 cursor-pointer transition-all"
                        >
                          <Trash2 className="w-3 h-3" />
                        </button>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>

      {/* ── 其它视图 ──
          在线播放要落回同一个播放条（下面那条常驻），所以它们是同一模块内的视图，
          而不是各做一个顶级模块 —— 独立模块拿不到播放器状态，搜到歌也播不了。

          和曲库一样用**隐藏**而不是条件卸载：卸载会丢掉搜索结果 / 页码 / 滚动位置，
          切走再切回就变成一片空白，用户得重新搜一遍。 */}
      <div className={`flex-1 flex flex-col min-h-0 ${showing("online")}`}>
        <OnlineSearch onPlayer={syncState} onLibrary={setLibrary} />
      </div>
      <div className={`flex-1 flex flex-col min-h-0 ${showing("plugins")}`}>
        <PluginManager />
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
            className="p-2 rounded-ctl bg-[var(--module-accent)] text-white hover:opacity-85 cursor-pointer disabled:opacity-40"
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
          <span className="text-tiny font-mono text-slate-400 w-10 text-right">
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
          <span className="text-tiny font-mono text-slate-400 w-10">{formatTime(duration)}</span>
        </div>

        <div className="min-w-0 w-48 flex-shrink-0">
          <p className="text-caption text-slate-200 truncate">
            {player?.title ?? t("music.notPlaying")}
          </p>
          <p className="text-tiny text-slate-500 truncate">{player?.artist ?? ""}</p>
        </div>

        {/* 在线音源才出现：下载 + 音质。本地曲目不显示，免得挤占播放条 */}
        {player?.online && (
          <div className="flex items-center gap-1 flex-shrink-0">
            <select
              value={dlQuality ?? player.online.quality}
              onChange={(e) => setDlQuality(e.target.value as MusicQuality)}
              title={t("music.qualityLabel")}
              className="glass-input h-7 text-tiny cursor-pointer"
            >
              {MUSIC_QUALITIES.map((item) => (
                <option key={item} value={item}>
                  {t(`music.quality.${item}`)}
                </option>
              ))}
            </select>
            <button
              onClick={() => void downloadCurrent()}
              disabled={dlBusy}
              className="p-1.5 rounded text-slate-300 hover:text-white hover:bg-white/10 cursor-pointer disabled:opacity-40"
              title={t("music.onlineDownloadFrom", { platform: player.online.platform })}
            >
              {dlBusy ? (
                <Loader2 className="w-4 h-4 animate-spin" />
              ) : (
                <Download className="w-4 h-4" />
              )}
            </button>
          </div>
        )}

        {/* 音量：右侧留出边距，避免贴着窗口边缘 */}
        <div className="flex items-center gap-1.5 flex-shrink-0 w-32 mr-3">
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

      {/* 重命名弹窗：默认填后端按标签生成的推荐名，可手改；只改文件名与扩展名，不动标签 */}
      {renameTrack && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
          onClick={() => {
            if (!renaming) setRenameTrack(null);
          }}
        >
          <div
            className="w-[440px] max-w-full rounded-panel border border-white/10 bg-slate-900 p-4 space-y-3"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="text-title font-bold text-white">{t("music.renameTitle")}</div>
            <div className="text-caption text-slate-400 break-all">
              {renameTrack.path.split(/[\\/]/).pop()}
            </div>
            <input
              value={renameName}
              onChange={(e) => setRenameName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void submitRename();
                if (e.key === "Escape" && !renaming) setRenameTrack(null);
              }}
              autoFocus
              spellCheck={false}
              className="w-full bg-black/30 border border-white/10 rounded-ctl px-2.5 py-2 text-body text-slate-100 outline-none focus:border-[var(--module-accent)]"
            />
            <div className="text-tiny text-slate-500">
              {renameFromTags ? t("music.renameFromTags") : t("music.renameNoTags")}
            </div>
            <div className="flex justify-end gap-2 pt-1">
              <SharedButton variant="secondary" onClick={() => setRenameTrack(null)} disabled={renaming}>
                {t("common.cancel")}
              </SharedButton>
              <SharedButton
                onClick={() => void submitRename()}
                disabled={renaming || !renameName.trim()}
              >
                {renaming ? t("music.renaming") : t("music.renameConfirm")}
              </SharedButton>
            </div>
          </div>
        </div>
      )}

      <ConfirmDialogHost request={confirmRequest} onClose={() => setConfirmRequest(null)} />
    </div>
  );
}
