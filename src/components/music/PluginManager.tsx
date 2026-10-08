// 插件管理视图（音乐模块内的一个视图，不做成独立顶级模块）。
//
// 三件事：依赖状态与安装、插件列表（导入 / 启停 / 排序 / 删除 / 刷新）、存储与缓存。
//
// 依赖是**功能级前置**：一次安装、所有插件共享，与 SDK 托管体系无关
// （Node 只从用户自己的 PATH 上找，见后端 plugin_host）。
import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import {
  ChevronDown,
  ChevronUp,
  CircleAlert,
  CircleCheck,
  FolderOpen,
  Loader2,
  Package,
  RefreshCw,
  ShieldAlert,
  ShieldCheck,
  Trash2,
  Upload,
  X,
} from "lucide-react";

import { SharedButton } from "../shared/Button";
import { ConfirmDialogHost, type ConfirmRequest } from "../shared/ConfirmDialog";
import { toast } from "../shared/Toast";
import VexEmptyState from "../VexEmptyState";
import {
  type PluginEntry,
  type PluginImportResult,
  type PluginListResult,
} from "./types";

/** 依赖安装日志事件名（与后端 plugin_commands::DEPS_LOG_EVENT 一致） */
const DEPS_LOG_EVENT = "music-plugin-deps-log";

/**
 * 导入状态。
 *
 * 用**常驻**状态条而不是只弹 toast：订阅导入要逐个下载几十个插件、可能持续几十秒，
 * 期间必须让用户看得见「在跑」，结束后结果也不该一闪而过（失败还要能逐条读）。
 */
type ImportStatus = { kind: "busy" | "ok" | "err"; text: string };

export default function PluginManager() {
  const { t } = useTranslation();

  const [deps, setDeps] = useState<PluginListResult["deps"] | null>(null);
  const [plugins, setPlugins] = useState<PluginEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [source, setSource] = useState("");
  const [installing, setInstalling] = useState(false);
  // 订阅导入要逐个下载几十个插件，可能持续很久 —— 期间禁掉按钮并转圈
  const [importing, setImporting] = useState(false);
  const [importStatus, setImportStatus] = useState<ImportStatus | null>(null);
  const [depsLog, setDepsLog] = useState<string[]>([]);
  const [showLog, setShowLog] = useState(false);
  const [busyFile, setBusyFile] = useState<string | null>(null);
  const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null);
  const logEndRef = useRef<HTMLDivElement | null>(null);

  const apply = useCallback((result: PluginListResult) => {
    setDeps(result.deps);
    setPlugins(result.plugins);
  }, []);

  const reload = useCallback(async () => {
    try {
      apply(await invoke<PluginListResult>("music_plugin_list"));
    } catch (e) {
      toast(t("music.pluginImportFail", { err: String(e) }), "err");
    } finally {
      setLoading(false);
    }
  }, [apply, t]);

  useEffect(() => {
    void reload();
  }, [reload]);

  // npm 输出逐行推送：先订阅再触发安装，避免漏掉前几行
  useEffect(() => {
    const unlisten = listen<string>(DEPS_LOG_EVENT, (event) => {
      setDepsLog((prev) => [...prev, event.payload]);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  useEffect(() => {
    if (showLog) logEndRef.current?.scrollIntoView({ block: "end" });
  }, [depsLog, showLog]);

  const installDeps = async () => {
    setInstalling(true);
    setDepsLog([]);
    setShowLog(true);
    try {
      apply(await invoke<PluginListResult>("music_plugin_install_deps"));
      toast(t("music.pluginDepsReady"));
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setInstalling(false);
    }
  };

  /**
   * 导入：单个插件与订阅清单走**同一条命令**，由后端按内容识别。
   *
   * 所以这里不按扩展名分派 —— 用户手里的「一个 JSON 文件」和「一个订阅 URL」
   * 最终都是同一件事，界面只该有一个入口。
   */
  const importSource = async (value: string) => {
    const trimmed = value.trim();
    if (!trimmed) return;
    setImporting(true);
    // 三态里的「处理中」：先立起来，用户点完立刻有反馈（订阅导入可能几十秒）
    setImportStatus({ kind: "busy", text: t("music.pluginImportBusy") });
    try {
      const outcome = await invoke<PluginImportResult>("music_plugin_import", { source: trimmed });
      if (outcome.kind === "subscription") {
        // 订阅里单个插件失败很常见（下架 / 网络），不该当成整体失败：
        // 成功数照报，失败**逐个点名**（能换行展示，这也正是用状态条而不是 toast 的原因）
        setImportStatus({
          kind: outcome.failures.length > 0 ? "err" : "ok",
          text: [
            t("music.pluginSubscriptionDone", {
              ok: outcome.imported,
              fail: outcome.failures.length,
            }),
            ...outcome.failures.map((item) => `· ${item.name}：${item.error}`),
          ].join("\n"),
        });
      } else if (outcome.probe_error) {
        // 落盘成功但读不到插件信息：插件仍可用，提示清楚即可
        setImportStatus({
          kind: "err",
          text: t("music.pluginProbeFail", { name: outcome.name, err: outcome.probe_error }),
        });
      } else {
        setImportStatus({ kind: "ok", text: t("music.pluginImported", { name: outcome.name }) });
      }
      setSource("");
      await reload();
    } catch (e) {
      setImportStatus({ kind: "err", text: t("music.pluginImportFail", { err: String(e) }) });
    } finally {
      setImporting(false);
    }
  };

  const pickFile = async () => {
    const picked = await open({
      multiple: false,
      filters: [{ name: "MusicFree", extensions: ["js", "json"] }],
    });
    if (typeof picked === "string") await importSource(picked);
  };

  const toggleEnabled = async (entry: PluginEntry) => {
    setBusyFile(entry.file);
    try {
      apply(
        await invoke<PluginListResult>("music_plugin_set_enabled", {
          file: entry.file,
          enabled: !entry.enabled,
        })
      );
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusyFile(null);
    }
  };

  const removePlugin = (entry: PluginEntry) => {
    const name = displayName(entry);
    setConfirmRequest({
      title: t("music.pluginRemove"),
      desc: t("music.pluginRemoveConfirm", { name }),
      danger: true,
      onConfirm: () => {
        setConfirmRequest(null);
        void (async () => {
          try {
            apply(await invoke<PluginListResult>("music_plugin_remove", { file: entry.file }));
            toast(t("music.pluginRemoved", { name }));
          } catch (e) {
            toast(String(e), "err");
          }
        })();
      },
    });
  };

  /** 上移 / 下移：把整份顺序交给后端（后端按下标重排）。 */
  const move = async (index: number, delta: number) => {
    const target = index + delta;
    if (target < 0 || target >= plugins.length) return;
    const files = plugins.map((entry) => entry.file);
    [files[index], files[target]] = [files[target], files[index]];
    try {
      apply(await invoke<PluginListResult>("music_plugin_reorder", { files }));
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const refreshMeta = async (file?: string) => {
    setBusyFile(file ?? "*");
    try {
      apply(await invoke<PluginListResult>("music_plugin_refresh_meta", { file: file ?? null }));
      toast(t("music.pluginRefreshed"));
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusyFile(null);
    }
  };

  const openDir = async (file?: string) => {
    try {
      await invoke("music_plugin_open_dir", { file: file ?? null });
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const displayName = (entry: PluginEntry) =>
    entry.name || entry.meta.platform || entry.file.replace(/\.js$/i, "");

  const depsProblem = deps?.problem ?? null;
  const depsReady = deps?.ready ?? false;

  return (
    <div className="flex-1 overflow-auto p-3 space-y-3">
      {/* ── 依赖状态 ── */}
      <div className="rounded-ctl border border-white/10 bg-white/[0.02] p-2.5 space-y-2">
        <div className="flex items-center gap-2 flex-wrap">
          <Package className="w-3.5 h-3.5 text-slate-400 flex-shrink-0" />
          <span className={`text-caption ${depsReady ? "text-emerald-400" : "text-amber-400"}`}>
            {depsProblem
              ? depsProblem
              : depsReady
                ? t("music.pluginDepsReady")
                : t("music.pluginDepsMissing", {
                    names: (deps?.missing_packages ?? []).join(", "),
                  })}
          </span>
          {!depsReady && (
            <SharedButton
              variant="primary"
              onClick={() => void installDeps()}
              disabled={installing || depsProblem !== null}
            >
              {installing ? (
                <Loader2 className="w-3.5 h-3.5 animate-spin" />
              ) : (
                <Package className="w-3.5 h-3.5" />
              )}
              {installing ? t("music.pluginDepsInstalling") : t("music.pluginDepsInstall")}
            </SharedButton>
          )}
          {depsLog.length > 0 && (
            <button
              onClick={() => setShowLog((prev) => !prev)}
              className="text-tiny text-slate-400 hover:text-slate-200 cursor-pointer ml-auto"
            >
              {t("music.pluginDepsLog")}
            </button>
          )}
        </div>
        {deps?.node_path && (
          <p className="text-tiny text-slate-500 break-all">Node: {deps.node_path}</p>
        )}
        {/* 沙箱状态要显式可见：它静默失效是最坏的情况（插件照常能跑，用户以为有保护） */}
        {deps?.sandbox && (
          <p
            className={`text-tiny flex items-start gap-1 ${
              deps.sandbox.enabled ? "text-emerald-400/80" : "text-amber-400/90"
            }`}
          >
            {deps.sandbox.enabled ? (
              <ShieldCheck className="w-3 h-3 mt-[1px] flex-shrink-0" />
            ) : (
              <ShieldAlert className="w-3 h-3 mt-[1px] flex-shrink-0" />
            )}
            {deps.sandbox.enabled ? t("music.pluginSandboxOn") : deps.sandbox.reason}
          </p>
        )}
        <p className="text-tiny text-slate-500">{t("music.pluginDepsHint")}</p>
        {showLog && depsLog.length > 0 && (
          <pre className="max-h-40 overflow-auto rounded bg-black/30 p-2 text-tiny text-slate-400 whitespace-pre-wrap break-all">
            {depsLog.join("\n")}
            <div ref={logEndRef} />
          </pre>
        )}
      </div>

      {/* ── 导入 ── */}
      <div className="flex items-center gap-2 flex-wrap">
        <input
          value={source}
          onChange={(e) => setSource(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void importSource(source);
          }}
          placeholder={t("music.pluginImportPh")}
          className="glass-input flex-1 min-w-[200px] h-8 text-caption"
        />
        <SharedButton
          variant="primary"
          onClick={() => void importSource(source)}
          disabled={!source.trim() || importing}
        >
          {importing ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
          ) : (
            <Upload className="w-3.5 h-3.5" />
          )}
          {t("music.pluginImport")}
        </SharedButton>
        <SharedButton variant="secondary" onClick={() => void pickFile()} disabled={importing}>
          {t("music.pluginPickFile")}
        </SharedButton>
      </div>

      {/* ── 导入状态（处理中 / 成功 / 失败）──
          常驻而不是 toast：导入可能要几十秒，结果（尤其失败明细）不该一闪而过。 */}
      {importStatus && (
        <div
          className={`flex items-start gap-1.5 rounded-ctl border px-2 py-1.5 text-caption whitespace-pre-line ${
            importStatus.kind === "busy"
              ? "border-white/10 bg-white/[0.02] text-slate-300"
              : importStatus.kind === "ok"
                ? "border-emerald-500/20 bg-emerald-500/[0.07] text-emerald-300"
                : "border-rose-500/20 bg-rose-500/[0.07] text-rose-300"
          }`}
        >
          {importStatus.kind === "busy" ? (
            <Loader2 className="w-3.5 h-3.5 mt-[1px] animate-spin flex-shrink-0" />
          ) : importStatus.kind === "ok" ? (
            <CircleCheck className="w-3.5 h-3.5 mt-[1px] flex-shrink-0" />
          ) : (
            <CircleAlert className="w-3.5 h-3.5 mt-[1px] flex-shrink-0" />
          )}
          <span className="flex-1 break-all">{importStatus.text}</span>
          {importStatus.kind !== "busy" && (
            <button
              onClick={() => setImportStatus(null)}
              title={t("music.pluginImportDismiss")}
              className="opacity-60 hover:opacity-100 cursor-pointer flex-shrink-0"
            >
              <X className="w-3 h-3" />
            </button>
          )}
        </div>
      )}

      {/* ── 列表 ── */}
      {loading ? (
        <div className="py-10 flex justify-center text-slate-500">
          <Loader2 className="w-5 h-5 animate-spin" />
        </div>
      ) : plugins.length === 0 ? (
        <VexEmptyState
          title={t("music.pluginEmpty")}
          desc={t("music.pluginEmptyHint")}
          tick={t("music.pluginEmptyTick")}
          avatarSize={40}
          className="!py-10"
          action={{ label: t("music.pluginPickFile"), onClick: () => void pickFile() }}
        />
      ) : (
        <div className="space-y-1.5">
          {plugins.map((entry, index) => {
            const probed = entry.meta.platform !== "" || entry.meta.has_search;
            const usable = entry.meta.has_search && entry.meta.has_media_source;
            const busy = busyFile === entry.file;
            return (
              <div
                key={entry.file}
                className={`rounded-ctl border p-2 flex items-start gap-2 transition-colors ${
                  entry.enabled
                    ? "border-white/10 bg-white/[0.02]"
                    : "border-white/5 bg-white/[0.01] opacity-60"
                }`}
              >
                <input
                  type="checkbox"
                  checked={entry.enabled}
                  disabled={busy}
                  onChange={() => void toggleEnabled(entry)}
                  title={entry.enabled ? t("music.pluginDisable") : t("music.pluginEnable")}
                  className="mt-1 accent-[var(--module-accent)] cursor-pointer"
                />
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-1.5 flex-wrap">
                    <span className="text-caption text-slate-200 font-medium truncate">
                      {displayName(entry)}
                    </span>
                    {entry.meta.version && (
                      <span className="text-tiny text-slate-500">
                        {t("music.pluginVersion")} {entry.meta.version}
                      </span>
                    )}
                    {entry.meta.author && (
                      <span className="text-tiny text-slate-600">
                        {t("music.pluginAuthor")} {entry.meta.author}
                      </span>
                    )}
                    {/* 能力标记：缺「取流」的插件点了也播不了，提前说清楚 */}
                    {!probed ? (
                      <span
                        className="text-tiny px-1 rounded bg-amber-500/15 text-amber-300"
                        title={t("music.pluginUnknownHint")}
                      >
                        {t("music.pluginUnknown")}
                      </span>
                    ) : (
                      <>
                        {!entry.meta.has_search && (
                          <span className="text-tiny px-1 rounded bg-rose-500/15 text-rose-300">
                            {t("music.pluginNoSearch")}
                          </span>
                        )}
                        {!entry.meta.has_media_source && (
                          <span className="text-tiny px-1 rounded bg-rose-500/15 text-rose-300">
                            {t("music.pluginNoMedia")}
                          </span>
                        )}
                      </>
                    )}
                  </div>
                  <p className="text-tiny text-slate-600 truncate mt-0.5" title={entry.source || entry.file}>
                    {entry.source || t("music.pluginFromDisk")} · {entry.file}
                  </p>
                </div>
                <div className="flex items-center gap-0.5 flex-shrink-0">
                  {/* 「未识别」或能力缺失的插件，给一个明显的刷新入口 */}
                  {(!probed || !usable) && (
                    <button
                      onClick={() => void refreshMeta(entry.file)}
                      disabled={busy}
                      title={t("music.pluginRefresh")}
                      className="p-1 rounded text-slate-500 hover:text-[var(--module-accent)] hover:bg-white/10 cursor-pointer disabled:opacity-40"
                    >
                      {busy ? (
                        <Loader2 className="w-3 h-3 animate-spin" />
                      ) : (
                        <RefreshCw className="w-3 h-3" />
                      )}
                    </button>
                  )}
                  <button
                    onClick={() => void move(index, -1)}
                    disabled={index === 0}
                    title={t("music.pluginMoveUp")}
                    className="p-1 rounded text-slate-500 hover:text-slate-200 hover:bg-white/10 cursor-pointer disabled:opacity-30"
                  >
                    <ChevronUp className="w-3 h-3" />
                  </button>
                  <button
                    onClick={() => void move(index, 1)}
                    disabled={index === plugins.length - 1}
                    title={t("music.pluginMoveDown")}
                    className="p-1 rounded text-slate-500 hover:text-slate-200 hover:bg-white/10 cursor-pointer disabled:opacity-30"
                  >
                    <ChevronDown className="w-3 h-3" />
                  </button>
                  <button
                    onClick={() => void openDir(entry.file)}
                    title={t("music.pluginOpenDir")}
                    className="p-1 rounded text-slate-500 hover:text-[var(--module-accent)] hover:bg-white/10 cursor-pointer"
                  >
                    <FolderOpen className="w-3 h-3" />
                  </button>
                  <button
                    onClick={() => removePlugin(entry)}
                    title={t("music.pluginRemove")}
                    className="p-1 rounded text-slate-500 hover:text-rose-400 hover:bg-rose-500/10 cursor-pointer"
                  >
                    <Trash2 className="w-3 h-3" />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* 下载目录与播放缓存已移到「音乐设置」——那是播放器的设置，与装了哪些插件无关；
          这里只留一个直达插件目录的入口。 */}
      <div className="flex items-center gap-2">
        <SharedButton variant="secondary" onClick={() => void openDir()}>
          <FolderOpen className="w-3.5 h-3.5" />
          {t("music.pluginOpenDir")}
        </SharedButton>
      </div>

      <ConfirmDialogHost request={confirmRequest} onClose={() => setConfirmRequest(null)} />
    </div>
  );
}
