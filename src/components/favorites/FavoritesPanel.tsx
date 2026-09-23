// 收藏 / 星标聚合：只读导入各平台的收藏 → 本地库 → AI 多标签归类 → 失效检测。
//
// 设计要点：
// - 导入是幂等的（后端按平台原生 id 去重），重复点「导入」只会得到 added=0；
// - 归类只处理未归类且未被人工改过的条目，人工改标签后该条目被锁定；
// - 一个条目可以属于多个分类（多标签），所以同一条目会在多个分类下出现。
import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";
import {
  ChevronDown,
  ChevronRight,
  ExternalLink,
  Pencil,
  RefreshCw,
  Search,
  Tag,
  Trash2,
} from "lucide-react";

import { SharedButton } from "../shared/Button";
import { ConfirmDialogHost, type ConfirmRequest } from "../shared/ConfirmDialog";
import { toast } from "../shared/Toast";
import { GithubTokenDialog } from "../project/GithubTokenDialog";
import { MarkdownRenderer } from "../ai/MarkdownRenderer";
import { CredentialDialog } from "./CredentialDialog";
import type { AiConfig, AiProvider } from "../ai/types";
import {
  SOURCE_LABELS,
  expiringInDays,
  statusBadge,
  type CachedContent,
  type CheckResult,
  type ClassifyResult,
  type CredentialStatus,
  type FavoriteRow,
  type FavoriteStats,
  type FavoritesProgress,
  type ImportResult,
} from "./types";

export default function FavoritesPanel() {
  const { t } = useTranslation();

  const [items, setItems] = useState<FavoriteRow[]>([]);
  const [stats, setStats] = useState<FavoriteStats | null>(null);
  const [tag, setTag] = useState<string | null>(null);
  const [source, setSource] = useState<string | null>(null);
  const [keyword, setKeyword] = useState("");
  const [busy, setBusy] = useState<string | null>(null);

  // AI 归类的模型选择：直接复用 AI 模块的配置，不另设一套
  const [providers, setProviders] = useState<AiProvider[]>([]);
  const [providerId, setProviderId] = useState<string>("");
  const [modelId, setModelId] = useState<string>("");

  const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null);
  const [editingId, setEditingId] = useState<number | null>(null);
  const [editingTags, setEditingTags] = useState("");

  // 导入 / 归类的实时进度（后端 favorites-progress 事件）
  const [progress, setProgress] = useState<FavoritesProgress | null>(null);

  useEffect(() => {
    const unlisten = listen<FavoritesProgress>("favorites-progress", (event) => {
      setProgress(event.payload);
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  // AI 归类的模型选择：配置里没存模型列表的供应商，现拉一次并按 provider 缓存
  const [fetchedModels, setFetchedModels] = useState<Record<string, string[]>>({});
  const [modelsLoading, setModelsLoading] = useState(false);

  // B站：需要 Cookie（含 SESSDATA）才能读自己的收藏；配过就不再每次问
  const [biliConfigured, setBiliConfigured] = useState(false);
  const [cookieOpen, setCookieOpen] = useState(false);

  // 知乎：只走 Cookie 直连（官方接口拿不到私有收藏夹，已下线）
  const [zhihuCookieConfigured, setZhihuCookieConfigured] = useState(false);
  const [zhihuCookieOpen, setZhihuCookieOpen] = useState(false);

  // 凭证健康状态：Cookie 快过期/已失效时主动提醒
  const [credStatus, setCredStatus] = useState<CredentialStatus[]>([]);
  const [credWarned, setCredWarned] = useState(false);

  // 展开看正文：知乎收藏内容 / GitHub README（懒抓，抓过就缓存在库里）
  const [expandedId, setExpandedId] = useState<number | null>(null);
  const [content, setContent] = useState<Record<number, CachedContent | null>>({});
  const [contentBusy, setContentBusy] = useState<number | null>(null);

  // GitHub Token：**收藏模块自己的一份**，与 SDK 模块的 token 互不共享
  const [tokenConfigured, setTokenConfigured] = useState(false);
  const [tokenOpen, setTokenOpen] = useState(false);

  const refresh = useCallback(async () => {
    const [list, overview, creds] = await Promise.all([
      invoke<FavoriteRow[]>("fav_list", {
        source,
        tag,
        status: null,
        keyword: keyword.trim() || null,
        limit: 0,
      }),
      invoke<FavoriteStats>("fav_stats"),
      invoke<CredentialStatus[]>("fav_credential_status"),
    ]);
    setItems(list);
    setStats(overview);
    setCredStatus(creds);
  }, [source, tag, keyword]);

  useEffect(() => {
    void refresh().catch((e) => toast(String(e), "err"));
  }, [refresh]);

  useEffect(() => {
    invoke<boolean>("fav_has_credential", { source: "bilibili" })
      .then(setBiliConfigured)
      .catch(() => setBiliConfigured(false));
    invoke<boolean>("fav_has_credential", { source: "zhihu-cookie" })
      .then(setZhihuCookieConfigured)
      .catch(() => setZhihuCookieConfigured(false));
    invoke<string>("fav_get_github_token")
      .then((token) => setTokenConfigured(!!token.trim()))
      .catch(() => setTokenConfigured(false));
  }, []);

  /** 某个凭证的告警级别：expired > soon > ok/unknown（未配置则 null）。 */
  const credAlert = useCallback(
    (sourceKey: string): "expired" | "soon" | null => {
      const found = credStatus.find((c) => c.source === sourceKey);
      if (!found?.configured) return null;
      if (found.status === "expired") return "expired";
      if (expiringInDays(found.expiresAt) !== null) return "soon";
      return null;
    },
    [credStatus],
  );

  // 失效/快过期只主动提醒一次，别每次重渲染都弹
  useEffect(() => {
    if (credWarned || credStatus.length === 0) return;
    const expired = credStatus.filter((c) => c.configured && c.status === "expired");
    const soon = credStatus.filter(
      (c) => c.configured && c.status !== "expired" && expiringInDays(c.expiresAt) !== null,
    );
    if (expired.length === 0 && soon.length === 0) return;
    setCredWarned(true);
    if (expired.length > 0) {
      toast(
        t("favorites.credExpiredHint", {
          sources: expired.map((c) => t(`favorites.credName.${c.source}`)).join("、"),
        }),
        "err",
      );
    } else {
      const days = expiringInDays(soon[0].expiresAt) ?? 0;
      toast(
        t("favorites.credSoonHint", {
          source: t(`favorites.credName.${soon[0].source}`),
          days,
        }),
        "info",
      );
    }
  }, [credStatus, credWarned, t]);

  useEffect(() => {
    invoke<AiConfig>("get_ai_config")
      .then((cfg) => {
        const usable = (cfg.providers || []).filter((p) => p.openai_url && p.api_key);
        setProviders(usable);
        const first = usable[0];
        if (first) {
          setProviderId(first.id);
          setModelId(first.active_model_id || first.models[0]?.id || "");
        }
      })
      .catch(() => setProviders([]));
  }, []);

  const activeProvider = useMemo(
    () => providers.find((p) => p.id === providerId) ?? null,
    [providers, providerId],
  );

  /** 当前可选的模型：优先用配置里存的，没有就现拉（有些供应商配置里 models 是空的） */
  const modelOptions = useMemo<string[]>(() => {
    if (!activeProvider) return [];
    if (activeProvider.models.length > 0) {
      return activeProvider.models.map((m) => m.id);
    }
    return fetchedModels[activeProvider.id] ?? [];
  }, [activeProvider, fetchedModels]);

  // 模型列表为空时自动补拉一次（失败静默：用户仍可用「默认」让后端自己选）
  useEffect(() => {
    if (!activeProvider || activeProvider.models.length > 0) return;
    const pid = activeProvider.id;
    if (fetchedModels[pid]) return;
    setModelsLoading(true);
    invoke<string[]>("fetch_provider_models", {
      baseUrl: activeProvider.openai_url,
      apiKey: activeProvider.api_key,
    })
      .then((models) => setFetchedModels((prev) => ({ ...prev, [pid]: models })))
      .catch(() => setFetchedModels((prev) => ({ ...prev, [pid]: [] })))
      .finally(() => setModelsLoading(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeProvider?.id]);

  const runImport = async () => {
    // 没配 Token 就直接把配置弹窗递上去，别让用户吃一个报错再自己找入口
    if (!tokenConfigured) {
      setTokenOpen(true);
      return;
    }
    setBusy("import");
    try {
      const result = await invoke<ImportResult>("fav_import_github", { maxPages: null });
      await refresh();
      if (result.cancelled) {
        toast(t("favorites.importCancelled"), "ok");
      } else {
        toast(
          t("favorites.importDone", {
            added: result.added,
            updated: result.updated,
            skipped: result.skipped,
          }),
          "ok",
        );
      }
    } catch (e) {
      toast(t("favorites.importFail", { err: String(e) }), "err");
    } finally {
      setBusy(null);
      setProgress(null);
    }
  };

  /**
   * 展开正文：知乎用已缓存的内容，GitHub 懒抓 README（优先中文版）。
   *
   * `refresh` 为 true 时强制重新抓（GitHub 用），否则命中缓存直接显示。
   */
  const openContent = async (item: FavoriteRow, force = false) => {
    setContentBusy(item.id);
    try {
      const data =
        item.source === "github"
          ? await invoke<CachedContent>("fav_github_readme", { id: item.id, refresh: force })
          : await invoke<CachedContent | null>("fav_get_content", { id: item.id });
      setContent((prev) => ({ ...prev, [item.id]: data }));
    } catch (e) {
      toast(t("favorites.contentFail", { err: String(e) }), "err");
      // 失败也记一笔，避免每次收起再展开都重试一遍
      setContent((prev) => ({ ...prev, [item.id]: null }));
    } finally {
      setContentBusy(null);
    }
  };

  /** 能否展开看正文：知乎有正文缓存，GitHub 能抓 README，B站暂不支持。 */
  const canPreview = (item: FavoriteRow) =>
    item.source === "zhihu" || item.source === "github";

  const toggleContent = (item: FavoriteRow) => {
    if (expandedId === item.id) {
      setExpandedId(null);
      return;
    }
    setExpandedId(item.id);
    if (!(item.id in content)) void openContent(item);
  };

  const runImportBili = async () => {
    setBusy("bili");
    try {
      const result = await invoke<ImportResult>("fav_import_bilibili");
      await refresh();
      toast(
        t("favorites.importDone", {
          added: result.added,
          updated: result.updated,
          skipped: result.skipped,
        }),
        "ok",
      );
    } catch (e) {
      toast(t("favorites.importFail", { err: String(e) }), "err");
      // Cookie 失效是最常见原因：直接把配置弹窗递上去
      if (!biliConfigured) setCookieOpen(true);
    } finally {
      setBusy(null);
      setProgress(null);
    }
  };

  // 知乎：只走 Cookie 直连（含私有收藏夹；官方接口拿不到全量，已下线）
  const runImportZhihu = async () => {
    if (!zhihuCookieConfigured) {
      setZhihuCookieOpen(true);
      return;
    }
    setBusy("zhihu");
    try {
      const result = await invoke<ImportResult>("fav_import_zhihu");
      await refresh();
      // 部分收藏夹失败（限流等）：主体照常导入，但要把失败明细露出来
      if (result.failed && result.failed.length > 0) {
        toast(
          t("favorites.importPartial", {
            added: result.added,
            failed: result.failed.length,
            detail: result.failed.join("；"),
          }),
          "err",
        );
      } else if (result.cancelled) {
        toast(t("favorites.importCancelled"), "ok");
      } else {
        toast(
          t("favorites.importDone", {
            added: result.added,
            updated: result.updated,
            skipped: result.skipped,
          }),
          "ok",
        );
      }
    } catch (e) {
      toast(t("favorites.importFail", { err: String(e) }), "err");
      // Cookie 失效是最常见原因：把配置弹窗递上去
      setZhihuCookieOpen(true);
    } finally {
      setBusy(null);
      setProgress(null);
    }
  };

  const runClassify = async () => {
    setBusy("classify");
    try {
      const result = await invoke<ClassifyResult>("fav_classify", {
        providerId: providerId || null,
        modelId: modelId || null,
        limit: null,
      });
      await refresh();
      if (result.cancelled) {
        // 停掉了也要说清「已归类多少条被保留」，否则用户不知道白干了没有
        toast(
          t("favorites.classifyCancelled", {
            classified: result.classified,
            remaining: result.remaining,
          }),
          "info",
        );
      } else {
        toast(
          t("favorites.classifyDone", {
            classified: result.classified,
            tags: result.tagsWritten,
            remaining: result.remaining,
          }),
          "ok",
        );
      }
    } catch (e) {
      toast(t("favorites.classifyFail", { err: String(e) }), "err");
    } finally {
      setBusy(null);
      setProgress(null);
    }
  };

  const runCheck = async () => {
    setBusy("check");
    try {
      const result = await invoke<CheckResult>("fav_check_gone", { all: false });
      await refresh();
      if (result.cancelled) {
        toast(t("favorites.checkCancelled", { checked: result.checked }), "info");
      } else if (result.aborted) {
        toast(t("favorites.checkAborted", { checked: result.checked }), "err");
      } else {
        toast(
          t("favorites.checkDone", {
            checked: result.checked,
            gone: result.gone,
            redirect: result.redirect,
          }),
          "ok",
        );
      }
    } catch (e) {
      toast(t("favorites.checkFail", { err: String(e) }), "err");
    } finally {
      setBusy(null);
      setProgress(null);
    }
  };

  const submitTags = async (id: number) => {
    const tags = editingTags
      .split(/[,，]/)
      .map((s) => s.trim())
      .filter(Boolean);
    try {
      await invoke("fav_set_tags", { id, tags });
      setEditingId(null);
      await refresh();
    } catch (e) {
      toast(t("favorites.tagFail", { err: String(e) }), "err");
    }
  };

  const removeItem = (item: FavoriteRow) => {
    setConfirmRequest({
      title: t("favorites.deleteTitle"),
      desc: t("favorites.deleteConfirm", { name: item.title }),
      danger: true,
      onConfirm: async () => {
        setConfirmRequest(null);
        try {
          await invoke("fav_delete", { id: item.id });
          await refresh();
        } catch (e) {
          toast(t("favorites.deleteFail", { err: String(e) }), "err");
        }
      },
    });
  };

  const goneCount = stats?.gone ?? 0;

  /**
   * 停止当前长任务。
   *
   * 后端在**循环的下一轮开始前**检查标志：进行中的那次请求会跑完
   * （导入的一页 / 归类的一批 / 检测的一条），所以提示语要说清是「下一轮才停」，
   * 否则用户看界面没立刻反应会以为按钮坏了、再点几次。
   */
  const requestStop = async () => {
    try {
      await invoke("fav_cancel");
      toast(t("favorites.stopRequested"), "info");
    } catch (e) {
      toast(String(e), "err");
    }
  };

  /** 只在对应任务运行时出现的「停止」链接。 */
  const stopLink = (active: boolean) =>
    active ? <LinkButton label={t("favorites.stop")} danger onClick={() => void requestStop()} /> : null;

  /** 凭证按钮的悬停提示：有告警时把原因说清楚，别让用户猜角标是什么意思。 */
  const credTitle = (sourceKey: string, fallback: string) => {
    const level = credAlert(sourceKey);
    if (level === "expired") return t("favorites.credExpired");
    if (level === "soon") {
      const days =
        expiringInDays(credStatus.find((c) => c.source === sourceKey)?.expiresAt) ?? 0;
      return t("favorites.credSoon", { days });
    }
    return fallback;
  };

  return (
    <div className="h-full flex flex-col gap-2 p-3 text-slate-200">
      {/* 顶部操作区：站点名只出现一次，动作一律用文字链接表达（细分隔线分组）。
          三个站点的两个动作语义一致（导入 / 密钥），凭证键不同所以互不覆盖。 */}
      <div className="flex items-center gap-x-3 gap-y-1 flex-wrap">
        {/* GitHub star */}
        <div className="flex items-center gap-1.5">
          <span className="text-[11px] text-slate-200 font-medium">GitHub</span>
          <LinkButton
            label={t("favorites.import")}
            busy={busy === "import"}
            disabled={busy !== null}
            onClick={() => void runImport()}
          />
          {stopLink(busy === "import")}
          <KeyLink
            label={t("favorites.key")}
            configured={tokenConfigured}
            title={
              tokenConfigured
                ? t("favorites.githubTokenSetTip")
                : t("favorites.githubTokenNeedTip")
            }
            onClick={() => setTokenOpen(true)}
          />
        </div>

        <Divider />

        {/* B站收藏 */}
        <div className="flex items-center gap-1.5">
          <span className="text-[11px] text-slate-200 font-medium">B站</span>
          <LinkButton
            label={t("favorites.import")}
            busy={busy === "bili"}
            disabled={busy !== null}
            onClick={() => (biliConfigured ? void runImportBili() : setCookieOpen(true))}
          />
          {stopLink(busy === "bili")}
          <KeyLink
            label={t("favorites.key")}
            configured={biliConfigured}
            alert={credAlert("bilibili")}
            title={credTitle(
              "bilibili",
              biliConfigured
                ? t("favorites.biliCookieSetTip")
                : t("favorites.biliCookieNeedTip"),
            )}
            onClick={() => setCookieOpen(true)}
          />
        </div>

        <Divider />

        {/* 知乎收藏 */}
        <div className="flex items-center gap-1.5">
          <span className="text-[11px] text-slate-200 font-medium">知乎</span>
          <LinkButton
            label={t("favorites.import")}
            busy={busy === "zhihu"}
            disabled={busy !== null}
            title={t("favorites.zhihuImportHint")}
            onClick={() => void runImportZhihu()}
          />
          {stopLink(busy === "zhihu")}
          {/* 密钥**永远打开弹窗**（回显已存的 Cookie）：之前配过就直接跑的写法
              让用户再也进不去弹窗，改不了一份过期 Cookie。 */}
          <KeyLink
            label={t("favorites.key")}
            configured={zhihuCookieConfigured}
            alert={credAlert("zhihu-cookie")}
            title={credTitle(
              "zhihu-cookie",
              zhihuCookieConfigured
                ? t("favorites.zhihuCookieSetTip")
                : t("favorites.zhihuCookieNeedTip"),
            )}
            onClick={() => setZhihuCookieOpen(true)}
          />
        </div>

        <Divider />

        {/* AI 归类 + 失效检测：都是对已有的本地库做加工 */}
        <div className="flex items-center gap-1.5">
          <span className="text-[11px] text-slate-200 font-medium">
            {t("favorites.toolsLabel")}
          </span>
          <select
            value={providerId}
            onChange={(e) => {
              setProviderId(e.target.value);
              const p = providers.find((x) => x.id === e.target.value);
              setModelId(p?.active_model_id || p?.models[0]?.id || "");
            }}
            className="glass-input px-2 h-6 text-[11px] cursor-pointer max-w-[130px]"
            title={t("favorites.providerHint")}
          >
            <option value="">{t("favorites.providerDefault")}</option>
            {providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
          <select
            value={modelId}
            onChange={(e) => setModelId(e.target.value)}
            className="glass-input px-2 h-6 text-[11px] cursor-pointer max-w-[170px]"
            title={t("favorites.modelHint")}
            disabled={!activeProvider}
          >
            <option value="">{t("favorites.modelDefault")}</option>
            {modelsLoading && <option disabled>{t("favorites.modelLoading")}</option>}
            {modelOptions.map((id) => (
              <option key={id} value={id}>
                {id}
              </option>
            ))}
          </select>
          <LinkButton
            label={t("favorites.classify")}
            busy={busy === "classify"}
            disabled={busy !== null || providers.length === 0}
            onClick={() => void runClassify()}
          />
          <LinkButton
            label={t("favorites.check")}
            busy={busy === "check"}
            disabled={busy !== null}
            onClick={() => void runCheck()}
          />
          {stopLink(busy === "classify" || busy === "check")}
        </div>

        <div className="ml-auto flex items-center gap-2">
          <div className="flex items-center gap-1 glass-input px-2 h-7">
            <Search className="w-3 h-3 text-slate-500" />
            <input
              value={keyword}
              onChange={(e) => setKeyword(e.target.value)}
              placeholder={t("favorites.searchPlaceholder")}
              className="bg-transparent outline-none text-[11px] w-36"
            />
          </div>
          <select
            value={source ?? ""}
            onChange={(e) => setSource(e.target.value || null)}
            className="glass-input px-2 h-7 text-[11px] cursor-pointer"
          >
            <option value="">{t("favorites.allSources")}</option>
            {(stats?.by_source || []).map(([key]) => (
              <option key={key} value={key}>
                {SOURCE_LABELS[key] ?? key}
              </option>
            ))}
          </select>
        </div>
      </div>

      {/* 实时进度：导入 / 归类 / 失效检测三类共用一条进度条。
          只按 stage 过滤**不**看 busy：检测失效在 picker 那侧没有 busy 之外的信号，
          而导入/归类结束时后端会发 done=true，前端在 finally 里清掉即可。 */}
      {progress && (
        <div className="glass-panel px-3 py-2 space-y-1.5">
          <div className="flex items-center gap-2 text-[10px] text-slate-300 flex-wrap">
            <RefreshCw className="w-3 h-3 animate-spin text-[var(--module-accent)]" />
            {progress.stage === "check" ? (
              <>
                <span>{t("favorites.checkProgress")}</span>
                <span>
                  {t("favorites.progressChecked", {
                    checked: progress.checked ?? 0,
                    total: progress.checkTotal ?? 0,
                  })}
                </span>
                <span className="text-slate-500 truncate max-w-[260px]">
                  {progress.message ?? ""}
                </span>
              </>
            ) : progress.stage === "import" ? (
              <>
                <span>
                  {SOURCE_LABELS[progress.source ?? ""] ?? progress.source ?? ""}
                  {progress.folder ? ` · ${progress.folder}` : ""}
                  {progress.message ? ` · ${progress.message}` : ""}
                </span>
                <span>{t("favorites.progressFetched", { fetched: progress.fetched ?? 0 })}</span>
                <span className="text-emerald-400/80">
                  {t("favorites.progressAdded", { added: progress.added ?? 0 })}
                </span>
                <span className="text-amber-400/80">
                  {t("favorites.progressUpdated", { updated: progress.updated ?? 0 })}
                </span>
                <span className="text-slate-500">
                  {t("favorites.progressSkipped", { skipped: progress.skipped ?? 0 })}
                </span>
              </>
            ) : (
              <>
                <span>{progress.message}</span>
                <span>
                  {t("favorites.progressClassified", {
                    classified: progress.classified ?? 0,
                    remaining: progress.remaining ?? 0,
                  })}
                </span>
                <span className="text-slate-400">
                  {t("favorites.progressTags", { tags: progress.tagsWritten ?? 0 })}
                </span>
              </>
            )}
          </div>
          <div className="h-1 rounded bg-white/5 overflow-hidden">
            {progress.stage === "check" && (progress.checkTotal ?? 0) > 0 ? (
              <div
                className="h-full bg-[var(--module-accent)] transition-all duration-300"
                style={{
                  width: `${Math.min(
                    100,
                    ((progress.checked ?? 0) / (progress.checkTotal ?? 1)) * 100,
                  )}%`,
                }}
              />
            ) : progress.stage === "classify" &&
            (progress.classified ?? 0) + (progress.remaining ?? 0) > 0 ? (
              <div
                className="h-full bg-[var(--module-accent)] transition-all duration-300"
                style={{
                  width: `${Math.min(
                    100,
                    ((progress.classified ?? 0) /
                      ((progress.classified ?? 0) + (progress.remaining ?? 0))) *
                      100,
                  )}%`,
                }}
              />
            ) : progress.stage === "import" &&
              (progress.folderTotal ?? 0) > 0 &&
              (progress.folderFetched ?? 0) > 0 ? (
              // 知乎：服务端给了收藏夹总数（Totals），可以显示真实百分比
              <div
                className="h-full bg-[var(--module-accent)] transition-all duration-300"
                style={{
                  width: `${Math.min(
                    100,
                    ((progress.folderFetched ?? 0) / (progress.folderTotal ?? 1)) * 100,
                  )}%`,
                }}
              />
            ) : (
              <div className="h-full w-1/3 bg-[var(--module-accent)] animate-pulse" />
            )}
          </div>
        </div>
      )}

      <div className="flex-1 flex gap-2 min-h-0">
        {/* 左侧分类树 */}
        <div className="w-40 shrink-0 overflow-y-auto glass-panel p-2 space-y-0.5">
          <button
            onClick={() => setTag(null)}
            className={`w-full text-left px-2 py-1 rounded text-[11px] cursor-pointer transition-colors ${
              tag === null
                ? "bg-[var(--module-accent-soft)] text-white"
                : "text-slate-400 hover:bg-white/5"
            }`}
          >
            {t("favorites.allTags")}
            <span className="float-right text-slate-500">{stats?.total ?? 0}</span>
          </button>
          {(stats?.tags || []).map(([name, count]) => (
            <button
              key={name}
              onClick={() => setTag(name)}
              className={`w-full text-left px-2 py-1 rounded text-[11px] cursor-pointer transition-colors truncate ${
                tag === name
                  ? "bg-[var(--module-accent-soft)] text-white"
                  : "text-slate-400 hover:bg-white/5"
              }`}
              title={name}
            >
              <Tag className="w-3 h-3 inline mr-1 opacity-70" />
              {name}
              <span className="float-right text-slate-500">{count}</span>
            </button>
          ))}
          {(stats?.tags || []).length === 0 && (
            <p className="text-[10px] text-slate-500 px-2 py-1 leading-snug">
              {t("favorites.noTagsHint")}
            </p>
          )}
        </div>

        {/* 右侧条目列表 */}
        <div className="flex-1 overflow-y-auto glass-panel divide-y divide-white/5">
          {items.length === 0 && (
            <div className="h-full flex items-center justify-center text-[11px] text-slate-500">
              {t("favorites.empty")}
            </div>
          )}
          {items.map((item) => {
            const badge = statusBadge(item.status);
            return (
              <div key={item.id} className="px-3 py-2 hover:bg-white/5 group">
                <div className="flex items-start gap-2">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5">
                      <button
                        onClick={() => void openUrl(item.url).catch(() => toast(item.url, "err"))}
                        className="text-[12px] font-medium text-slate-100 hover:text-[var(--module-accent)] cursor-pointer truncate flex items-center gap-1"
                        title={item.url}
                      >
                        {item.title}
                        <ExternalLink className="w-2.5 h-2.5 opacity-0 group-hover:opacity-60" />
                      </button>
                      <span className="text-[9px] px-1 rounded bg-white/5 text-slate-500">
                        {SOURCE_LABELS[item.source] ?? item.source}
                      </span>
                      {badge && (
                        <span className={`text-[9px] px-1 rounded ${badge.className}`}>
                          {t(badge.text)}
                        </span>
                      )}
                      {item.ai_locked && (
                        <span
                          className="text-[9px] px-1 rounded bg-emerald-500/10 text-emerald-400/80"
                          title={t("favorites.lockedHint")}
                        >
                          {t("favorites.locked")}
                        </span>
                      )}
                    </div>
                    {item.description && (
                      <p className="text-[10px] text-slate-400 line-clamp-2 mt-0.5">
                        {item.description}
                      </p>
                    )}

                    {editingId === item.id ? (
                      <div className="flex items-center gap-1 mt-1">
                        <input
                          value={editingTags}
                          onChange={(e) => setEditingTags(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") void submitTags(item.id);
                            if (e.key === "Escape") setEditingId(null);
                          }}
                          autoFocus
                          placeholder={t("favorites.tagPlaceholder")}
                          className="bg-black/30 border border-white/10 rounded px-1.5 py-0.5 text-[10px] text-slate-100 outline-none w-64"
                        />
                        <SharedButton className="!h-6 !px-2 !text-[10px]" onClick={() => void submitTags(item.id)}>
                          {t("common.save")}
                        </SharedButton>
                      </div>
                    ) : (
                      <div className="flex items-center gap-1 mt-1 flex-wrap">
                        {item.tags.map((name) => (
                          <button
                            key={name}
                            onClick={() => setTag(name)}
                            className="text-[9px] px-1.5 py-0.5 rounded-full bg-white/5 text-slate-300 hover:bg-white/10 cursor-pointer"
                          >
                            {name}
                          </button>
                        ))}
                        <button
                          onClick={() => {
                            setEditingId(item.id);
                            setEditingTags(item.tags.join(", "));
                          }}
                          className="p-0.5 rounded text-slate-600 hover:text-slate-300 cursor-pointer opacity-0 group-hover:opacity-100"
                          title={t("favorites.editTags")}
                        >
                          <Pencil className="w-2.5 h-2.5" />
                        </button>
                      </div>
                    )}
                  </div>

                  <div className="flex items-center gap-0.5">
                    {canPreview(item) && (
                      <button
                        onClick={() => toggleContent(item)}
                        className="p-1 rounded text-slate-600 hover:text-slate-200 cursor-pointer"
                        title={
                          expandedId === item.id
                            ? t("favorites.contentHide")
                            : t("favorites.contentShow")
                        }
                      >
                        {expandedId === item.id ? (
                          <ChevronDown className="w-3 h-3" />
                        ) : (
                          <ChevronRight className="w-3 h-3" />
                        )}
                      </button>
                    )}
                    <button
                      onClick={() => removeItem(item)}
                      className="p-1 rounded text-slate-600 hover:text-rose-400 cursor-pointer opacity-0 group-hover:opacity-100"
                      title={t("favorites.delete")}
                    >
                      <Trash2 className="w-3 h-3" />
                    </button>
                  </div>
                </div>

                {/* 展开的正文：知乎是导入时顺手缓存的内容，GitHub 是懒抓的 README（优先中文版） */}
                {expandedId === item.id && (
                  <div className="mt-1.5 rounded-lg border border-white/10 bg-black/25 p-2">
                    <div className="flex items-center gap-2 mb-1">
                      <span className="text-[9px] text-slate-500 truncate">
                        {contentBusy === item.id
                          ? t("favorites.contentLoading")
                          : content[item.id]?.label ??
                            (item.source === "github"
                              ? t("favorites.contentReadme")
                              : t("favorites.contentEmpty"))}
                        {content[item.id]?.fetchedAt
                          ? ` · ${content[item.id]?.fetchedAt}`
                          : ""}
                      </span>
                      {item.source === "github" && (
                        <button
                          onClick={() => void openContent(item, true)}
                          disabled={contentBusy === item.id}
                          className="ml-auto text-[9px] text-slate-500 hover:text-slate-200 cursor-pointer disabled:opacity-40"
                          title={t("favorites.contentRefresh")}
                        >
                          {t("favorites.contentRefresh")}
                        </button>
                      )}
                    </div>
                    {contentBusy === item.id ? (
                      <div className="flex items-center gap-2 text-[10px] text-slate-500 py-2">
                        <RefreshCw className="w-3 h-3 animate-spin" />
                        {t("favorites.contentLoading")}
                      </div>
                    ) : content[item.id]?.text ? (
                      <div className="max-h-72 overflow-y-auto">
                        {item.source === "github" ? (
                          <MarkdownRenderer content={content[item.id]!.text} />
                        ) : (
                          // 知乎正文只渲染纯文本：远端 HTML 直接进 DOM 等于把注入面交给知乎
                          <p className="text-[11px] text-slate-300 leading-relaxed whitespace-pre-wrap">
                            {content[item.id]!.text}
                          </p>
                        )}
                      </div>
                    ) : (
                      <p className="text-[10px] text-slate-500 py-1">
                        {t("favorites.contentEmpty")}
                      </p>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>

      <div className="flex items-center gap-3 text-[10px] text-slate-500">
        <span>
          {t("favorites.total")}: {stats?.total ?? 0}
        </span>
        <span>
          {t("favorites.unclassified")}: {stats?.unclassified ?? 0}
        </span>
        {goneCount > 0 && (
          <span className="text-rose-400/80">
            {t("favorites.goneCount")}: {goneCount}
          </span>
        )}
        <span className="ml-auto">{t("favorites.readonlyHint")}</span>
      </div>

      {/* GitHub Token（收藏模块专属）：与 SDK 模块的 Token 各存各的 */}
      <GithubTokenDialog
        open={tokenOpen}
        onClose={() => setTokenOpen(false)}
        onSaved={setTokenConfigured}
        getCommand="fav_get_github_token"
        setCommand="fav_set_github_token"
        titleKey="favorites.githubTokenTitle"
        hintKey="favorites.githubTokenHint"
        noteKey="favorites.githubTokenLocalNote"
      />

      {/* 两类凭证各用一个弹窗，**存储键不同所以互不覆盖**：
          - zhihu-cookie = 知乎 Cookie（多文本框，需含 z_c0 + d_c0）
          - bilibili     = B站 Cookie（多文本框，需含 SESSDATA） */}
      <CredentialDialog
        open={zhihuCookieOpen}
        onClose={() => setZhihuCookieOpen(false)}
        source="zhihu-cookie"
        title={t("favorites.zhihuCookieTitle")}
        hint={t("favorites.zhihuCookieHint")}
        placeholder={t("favorites.zhihuCookiePlaceholder")}
        note={t("favorites.zhihuCookieNote")}
        multiline
        onSaved={(configured) => {
          setZhihuCookieConfigured(configured);
          // 换了 Cookie 就把健康状态刷新一下：后端会把 status 重置为「未验证」
          void refresh().catch(() => {});
        }}
      />

      <CredentialDialog
        open={cookieOpen}
        onClose={() => setCookieOpen(false)}
        source="bilibili"
        title={t("favorites.biliCookieTitle")}
        hint={t("favorites.biliCookieHint")}
        placeholder={t("favorites.biliCookiePlaceholder")}
        note={t("favorites.biliExperimental")}
        multiline
        onSaved={setBiliConfigured}
      />

      <ConfirmDialogHost request={confirmRequest} onClose={() => setConfirmRequest(null)} />
    </div>
  );
}

/** 分组之间的细分隔线（比留白更明确地「断开」，又不至于像卡片那样围起来）。 */
function Divider() {
  return <span className="w-px h-3.5 bg-white/10" />;
}

/**
 * 工具栏的文字链接按钮。
 *
 * 刻意做成纯文字：这一排全是同级动作，用按钮样式会把「导入」这种高频操作
 * 和其它动作拉成一样的视觉重量，反而看不出主次，一行里还会挤满色块。
 * `busy` 时显示转圈并禁用点击（避免重复触发）。
 */
function LinkButton({
  label,
  onClick,
  disabled,
  busy,
  danger,
  title,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  busy?: boolean;
  danger?: boolean;
  title?: string;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled || busy}
      title={title}
      className={`inline-flex items-center gap-1 text-[11px] cursor-pointer transition-colors disabled:opacity-40 disabled:cursor-not-allowed ${
        danger
          ? "text-rose-400/80 hover:text-rose-300"
          : "text-slate-400 hover:text-[var(--module-accent)]"
      }`}
    >
      {busy && <RefreshCw className="w-2.5 h-2.5 animate-spin" />}
      {label}
    </button>
  );
}

/** 「密钥」链接：已配置为正常色，未配置更暗；有告警时带一个点。 */
function KeyLink({
  label,
  configured,
  title,
  alert,
  onClick,
}: {
  label: string;
  configured: boolean;
  title: string;
  alert?: "expired" | "soon" | null;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      className={`relative text-[11px] cursor-pointer transition-colors ${
        configured
          ? "text-slate-400 hover:text-[var(--module-accent)]"
          : "text-slate-600 hover:text-slate-300"
      }`}
    >
      {label}
      <CredDot level={alert ?? null} />
    </button>
  );
}

/**
 * 凭证链接右上角的告警点。
 *
 * 只做「一眼看出有事」，具体原因交给 `title`——因为这里能承载的信息量太小，
 * 把「3 天后过期」塞进一个点里只会让人困惑。
 */
function CredDot({ level }: { level: "expired" | "soon" | null }) {
  if (!level) return null;
  return (
    <span
      className={`absolute -top-0.5 -right-0.5 w-1.5 h-1.5 rounded-full ${
        level === "expired" ? "bg-rose-400" : "bg-amber-400"
      }`}
    />
  );
}
