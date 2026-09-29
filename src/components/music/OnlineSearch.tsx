// 在线搜索视图（音乐模块内的一个视图）。
//
// 搜索 → 结果列表 → 每行「播放 / 下载」。
// - 播放：后端取流后落到缓存再交给同一个播放引擎（因此队列 / 均衡器照旧生效）
// - 下载：落到用户设置的下载目录，并自动登记进曲库
//
// 结果**按来源插件聚合展示**（同一首歌在多个音源里都有），每行带来源标记；
// 某个音源出错只在顶部提示，不影响其它音源的结果。
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Download, Loader2, Play, Search } from "lucide-react";

import { SharedButton } from "../shared/Button";
import { toast } from "../shared/Toast";
import {
  formatSeconds,
  type DownloadProgress,
  type MusicLibrary,
  type MusicQuality,
  type OnlineTrack,
  type PlayerState,
  type PluginDownloadOutcome,
  type PluginPlayOutcome,
  type SearchHit,
  type SearchOutcome,
} from "./types";

/** 下载进度事件名（与后端 plugin_playback::DOWNLOAD_PROGRESS_EVENT 一致） */
const DOWNLOAD_PROGRESS_EVENT = "music-plugin-download-progress";

const QUALITIES: MusicQuality[] = ["low", "standard", "high", "super"];

interface Props {
  /** 播放后把播放器状态同步回面板（否则要等下一次轮询才更新） */
  onPlayer: (state: PlayerState) => void;
  /** 下载会改曲库，回传给面板一次刷新 */
  onLibrary: (library: MusicLibrary) => void;
}

/** 结果的稳定键：插件 + 曲目 id（没有 id 时退回标题），用于逐行忙态 */
function hitKey(hit: SearchHit): string {
  const id = hit.item.id ?? hit.item.title ?? "";
  return `${hit.file}::${String(id)}`;
}

export default function OnlineSearch({ onPlayer, onLibrary }: Props) {
  const { t } = useTranslation();

  const [keyword, setKeyword] = useState("");
  const [submitted, setSubmitted] = useState("");
  const [quality, setQuality] = useState<MusicQuality>("standard");
  const [hits, setHits] = useState<SearchHit[]>([]);
  /** 当前选中的来源插件文件名；空串 = 全部（只在结果来自多个音源时才出现选项卡） */
  const [activeSource, setActiveSource] = useState("");
  const [exhausted, setExhausted] = useState<string[]>([]);
  const [failures, setFailures] = useState<SearchOutcome["failures"]>([]);
  const [searched, setSearched] = useState(false);
  const [loading, setLoading] = useState(false);
  const pageRef = useRef(1);
  const [playingKey, setPlayingKey] = useState<string | null>(null);
  const [downloadingKey, setDownloadingKey] = useState<string | null>(null);
  const [progress, setProgress] = useState<DownloadProgress | null>(null);

  // 下载进度：按 label 匹配当前下载的那首，避免串台
  useEffect(() => {
    const unlisten = listen<DownloadProgress>(DOWNLOAD_PROGRESS_EVENT, (event) => {
      setProgress(event.payload);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  const runSearch = useCallback(
    async (nextPage: number, replace: boolean) => {
      const target = submitted.trim();
      if (!target) return;
      setLoading(true);
      try {
        const outcome = await invoke<SearchOutcome>("music_plugin_search", {
          keyword: target,
          page: nextPage,
        });
        setFailures(outcome.failures);
        setExhausted(outcome.exhausted);
        pageRef.current = nextPage;
        setHits((prev) => (replace ? outcome.hits : [...prev, ...outcome.hits]));
        setSearched(true);
      } catch (e) {
        toast(t("music.onlineSearchFail", { err: String(e) }), "err");
      } finally {
        setLoading(false);
      }
    },
    [submitted, t]
  );

  const submit = () => {
    const trimmed = keyword.trim();
    if (!trimmed) return;
    setSubmitted(trimmed);
    setHits([]);
    // 换了关键词，上一次选中的音源大概率没有结果了，回到「全部」
    setActiveSource("");
    void runSearch(1, true);
  };

  const play = async (hit: SearchHit) => {
    const key = hitKey(hit);
    setPlayingKey(key);
    try {
      // 整份结果一起交给后端：播放列表换成这次搜到的歌，并从点中的这首开始往后播。
      // 传的是**当前可见**的那份（按音源筛选时，播放列表就该是筛选后的结果）。
      const index = visibleHits.indexOf(hit);
      const outcome = await invoke<PluginPlayOutcome>("music_plugin_play", {
        file: hit.file,
        item: hit.item,
        quality,
        hits: visibleHits.map((item) => ({ file: item.file, item: item.item })),
        index: index < 0 ? 0 : index,
      });
      onPlayer(outcome.player);
      if (outcome.from_cache) toast(t("music.onlinePlayCached"));
    } catch (e) {
      // 不加「播放失败:」前缀：后端消息本身已说清是什么失败、为什么、怎么办
      toast(String(e), "err");
    } finally {
      setPlayingKey(null);
    }
  };

  const download = async (hit: SearchHit) => {
    const key = hitKey(hit);
    setDownloadingKey(key);
    setProgress(null);
    try {
      const outcome = await invoke<PluginDownloadOutcome>("music_plugin_download", {
        file: hit.file,
        item: hit.item,
        quality,
      });
      onLibrary(outcome.library);
      toast(t("music.onlineDownloaded", { dir: outcome.dir }));
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setDownloadingKey(null);
      setProgress(null);
    }
  };

  const track = (hit: SearchHit): OnlineTrack => hit.item;

  /**
   * 按来源插件分组（同一首歌常常多个音源都有）。
   *
   * 一次遍历同时得到「有哪些音源」与「各有多少条」，供选项卡展示。
   */
  const sourceGroups = useMemo(() => {
    const groups = new Map<string, { label: string; count: number }>();
    for (const hit of hits) {
      const existing = groups.get(hit.file);
      if (existing) existing.count += 1;
      else groups.set(hit.file, { label: hit.platform || hit.file, count: 1 });
    }
    return groups;
  }, [hits]);

  // 选中的来源若已不在结果里（换了关键词、该音源这次没结果），退回「全部」，
  // 否则会呈现一个空列表，看起来像「搜索没结果」
  const effectiveSource = activeSource && sourceGroups.has(activeSource) ? activeSource : "";
  const visibleHits = effectiveSource
    ? hits.filter((hit) => hit.file === effectiveSource)
    : hits;
  const canLoadMore = hits.length > 0 && exhausted.length === 0;

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      {/* ── 搜索条 ── */}
      <div className="flex items-center gap-2 flex-wrap p-3 border-b border-white/5 bg-white/[0.02] flex-shrink-0">
        <div className="relative flex-1 min-w-[200px]">
          <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3 h-3 text-slate-500" />
          <input
            value={keyword}
            onChange={(e) => setKeyword(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") submit();
            }}
            placeholder={t("music.onlineSearchPh")}
            className="glass-input w-full pl-7 pr-2 h-8 text-caption"
          />
        </div>
        <select
          value={quality}
          onChange={(e) => setQuality(e.target.value as MusicQuality)}
          title={t("music.qualityLabel")}
          className="glass-input h-8 text-caption cursor-pointer"
        >
          {QUALITIES.map((item) => (
            <option key={item} value={item}>
              {t(`music.quality.${item}`)}
            </option>
          ))}
        </select>
        <SharedButton variant="primary" onClick={submit} disabled={loading || !keyword.trim()}>
          {loading ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Search className="w-3.5 h-3.5" />}
          {loading ? t("music.onlineSearching") : t("music.onlineSearchGo")}
        </SharedButton>
      </div>

      {/* ── 按来源分选项卡 ──
          只在结果来自**多个**音源时出现：只有一个音源时选项卡是纯噪音。 */}
      {sourceGroups.size > 1 && (
        <div className="flex items-center gap-0.5 px-3 py-1.5 border-b border-white/5 overflow-x-auto flex-shrink-0">
          <button
            onClick={() => setActiveSource("")}
            className={`px-2.5 py-1 rounded-ctl text-caption whitespace-nowrap cursor-pointer transition-colors ${
              effectiveSource === ""
                ? "bg-[var(--module-accent)] text-white"
                : "text-slate-400 hover:text-slate-200 hover:bg-white/9"
            }`}
          >
            {t("music.onlineSourceAll")}
            <span className="text-tiny opacity-70 ml-1">{hits.length}</span>
          </button>
          {[...sourceGroups.entries()].map(([file, info]) => (
            <button
              key={file}
              onClick={() => setActiveSource(file)}
              title={file}
              className={`px-2.5 py-1 rounded-ctl text-caption whitespace-nowrap max-w-[180px] cursor-pointer transition-colors ${
                effectiveSource === file
                  ? "bg-[var(--module-accent)] text-white"
                  : "text-slate-400 hover:text-slate-200 hover:bg-white/9"
              }`}
            >
              <span className="inline-block align-middle truncate max-w-[140px]">
                {info.label}
              </span>
              <span className="text-tiny opacity-70 ml-1">{info.count}</span>
            </button>
          ))}
        </div>
      )}

      {/* ── 汇总 / 失败提示 ── */}
      {(hits.length > 0 || failures.length > 0) && (
        <div className="px-3 py-1.5 border-b border-white/5 flex items-center gap-2 flex-wrap flex-shrink-0">
          <span className="text-tiny text-slate-500">
            {t("music.onlineSearchSummary", { count: hits.length, sources: sourceGroups.size })}
          </span>
          {failures.length > 0 && (
            <span
              className="text-tiny text-amber-400 flex items-center gap-1"
              title={failures.map((item) => `${item.name}: ${item.error}`).join("\n")}
            >
              <AlertTriangle className="w-2.5 h-2.5" />
              {t("music.onlineSearchPartial", { count: failures.length })}
            </span>
          )}
        </div>
      )}

      {/* ── 结果 ── */}
      <div className="flex-1 overflow-auto">
        {!searched ? (
          <div className="h-full flex flex-col items-center justify-center gap-2 text-slate-500">
            <Search className="w-9 h-9 opacity-40" />
            <p className="text-sm">{t("music.onlineSearchIdle")}</p>
            <p className="text-caption text-slate-600">{t("music.onlineSearchIdleHint")}</p>
          </div>
        ) : hits.length === 0 && !loading ? (
          <div className="p-6 text-center text-caption text-slate-500">
            {t("music.onlineSearchNone")}
          </div>
        ) : (
          <div className="divide-y divide-white/[0.03]">
            {visibleHits.map((hit) => {
              const item = track(hit);
              const key = hitKey(hit);
              const isPlaying = playingKey === key;
              const isDownloading = downloadingKey === key;
              const percent =
                isDownloading && progress && progress.total > 0
                  ? Math.min(100, Math.round((progress.received / progress.total) * 100))
                  : null;
              return (
                <div key={key} className="flex items-center gap-2 px-3 py-1.5 hover:bg-white/[0.03]">
                  {item.artwork ? (
                    <img
                      src={String(item.artwork)}
                      alt=""
                      loading="lazy"
                      className="w-8 h-8 rounded object-cover bg-white/5 flex-shrink-0"
                    />
                  ) : (
                    <div className="w-8 h-8 rounded bg-white/5 flex-shrink-0" />
                  )}
                  <div className="flex-1 min-w-0">
                    <p className="text-caption text-slate-200 truncate" title={item.title}>
                      {item.title || t("music.pluginUnknown")}
                    </p>
                    <p className="text-tiny text-slate-500 truncate">
                      {[item.artist, item.album].filter(Boolean).join(" · ")}
                    </p>
                  </div>
                  <span className="text-tiny text-slate-600 flex-shrink-0 max-w-[120px] truncate" title={hit.platform}>
                    {hit.platform || hit.file}
                  </span>
                  <span className="text-tiny font-mono text-slate-500 flex-shrink-0 w-10 text-right">
                    {formatSeconds(item.duration)}
                  </span>
                  {/* 下载中把百分比显示在按钮位置，避免再多一行 */}
                  {isDownloading && percent != null && (
                    <span className="text-tiny text-[var(--module-accent)] w-12 text-right flex-shrink-0">
                      {t("music.onlineDownloading", { percent })}
                    </span>
                  )}
                  <div className="flex items-center gap-0.5 flex-shrink-0">
                    <button
                      onClick={() => void play(hit)}
                      disabled={isPlaying}
                      title={t("music.onlinePlay")}
                      className="p-1 rounded text-slate-500 hover:text-[var(--module-accent)] hover:bg-white/10 cursor-pointer disabled:opacity-40"
                    >
                      {isPlaying ? (
                        <Loader2 className="w-3 h-3 animate-spin" />
                      ) : (
                        <Play className="w-3 h-3" />
                      )}
                    </button>
                    <button
                      onClick={() => void download(hit)}
                      disabled={isDownloading}
                      title={t("music.onlineDownload")}
                      className="p-1 rounded text-slate-500 hover:text-emerald-400 hover:bg-emerald-500/10 cursor-pointer disabled:opacity-40"
                    >
                      {isDownloading ? (
                        <Loader2 className="w-3 h-3 animate-spin" />
                      ) : (
                        <Download className="w-3 h-3" />
                      )}
                    </button>
                  </div>
                </div>
              );
            })}
            {canLoadMore && (
              <div className="p-3 flex justify-center">
                <SharedButton
                  variant="secondary"
                  onClick={() => void runSearch(pageRef.current + 1, false)}
                  disabled={loading}
                >
                  {loading ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : null}
                  {t("music.onlineSearchMore")}
                </SharedButton>
              </div>
            )}
            {hits.length > 0 && exhausted.length > 0 && (
              <div className="p-3 text-center text-tiny text-slate-600">
                {t("music.onlineSearchEnd")}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
