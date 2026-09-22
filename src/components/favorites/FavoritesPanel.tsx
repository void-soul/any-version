// 收藏 / 星标聚合：只读导入各平台的收藏 → 本地库 → AI 多标签归类 → 失效检测。
//
// 设计要点：
// - 导入是幂等的（后端按平台原生 id 去重），重复点「导入」只会得到 added=0；
// - 归类只处理未归类且未被人工改过的条目，人工改标签后该条目被锁定；
// - 一个条目可以属于多个分类（多标签），所以同一条目会在多个分类下出现。
import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  BookMarked,
  Download,
  ExternalLink,
  KeyRound,
  Lock,
  LogIn,
  Pencil,
  RefreshCw,
  Search,
  Sparkles,
  Square,
  Tag,
  Trash2,
  Tv,
  X,
} from "lucide-react";

import { SharedButton } from "../shared/Button";
import { ConfirmDialogHost, type ConfirmRequest } from "../shared/ConfirmDialog";
import { toast } from "../shared/Toast";
import { GithubTokenDialog } from "../project/GithubTokenDialog";
import type { AiConfig, AiProvider } from "../ai/types";
import {
  SOURCE_LABELS,
  statusBadge,
  type CheckResult,
  type ClassifyResult,
  type FavoriteRow,
  type FavoriteStats,
  type ImportResult,
  type ZhihuStatus,
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

  // AI 归类的模型选择：配置里没存模型列表的供应商，现拉一次并按 provider 缓存
  const [fetchedModels, setFetchedModels] = useState<Record<string, string[]>>({});
  const [modelsLoading, setModelsLoading] = useState(false);

  // B站：需要 Cookie（含 SESSDATA）才能读自己的收藏；配过就不再每次问
  const [biliConfigured, setBiliConfigured] = useState(false);
  const [cookieOpen, setCookieOpen] = useState(false);
  const [cookieText, setCookieText] = useState("");

  // GitHub Token：**收藏模块自己的一份**，与 SDK 模块的 token 互不共享
  const [tokenConfigured, setTokenConfigured] = useState(false);
  const [tokenOpen, setTokenOpen] = useState(false);

  const refresh = useCallback(async () => {
    const [list, overview] = await Promise.all([
      invoke<FavoriteRow[]>("fav_list", {
        source,
        tag,
        status: null,
        keyword: keyword.trim() || null,
        limit: 0,
      }),
      invoke<FavoriteStats>("fav_stats"),
    ]);
    setItems(list);
    setStats(overview);
  }, [source, tag, keyword]);

  useEffect(() => {
    void refresh().catch((e) => toast(String(e), "err"));
  }, [refresh]);

  useEffect(() => {
    invoke<boolean>("fav_has_credential", { source: "bilibili" })
      .then(setBiliConfigured)
      .catch(() => setBiliConfigured(false));
    invoke<string>("fav_get_github_token")
      .then((token) => setTokenConfigured(!!token.trim()))
      .catch(() => setTokenConfigured(false));
  }, []);

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
    }
  };

  const saveCookie = async () => {
    if (!cookieText.trim()) return;
    try {
      await invoke("fav_set_credential", { source: "bilibili", cookie: cookieText.trim() });
      setBiliConfigured(true);
      setCookieOpen(false);
      setCookieText("");
      toast(t("favorites.cookieSaved"), "ok");
    } catch (e) {
      toast(t("favorites.cookieFail", { err: String(e) }), "err");
    }
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
    }
  };

  // 知乎：登录态活在隐藏窗口的 Cookie 里，所以先探测再导入；
  // 不在面板打开时就探测——那会在后台加载一次知乎首页，没必要。
  const runImportZhihu = async () => {
    setBusy("zhihu");
    try {
      const status = await invoke<ZhihuStatus>("fav_zhihu_status");
      if (!status.loggedIn) {
        await invoke("fav_zhihu_open_login");
        toast(t("favorites.zhihuNeedLogin"), "err");
        return;
      }
      const result = await invoke<ImportResult>("fav_import_zhihu");
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
    } finally {
      setBusy(null);
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
      toast(
        t("favorites.classifyDone", {
          classified: result.classified,
          tags: result.tagsWritten,
          remaining: result.remaining,
        }),
        "ok",
      );
    } catch (e) {
      toast(t("favorites.classifyFail", { err: String(e) }), "err");
    } finally {
      setBusy(null);
    }
  };

  const runCheck = async () => {
    setBusy("check");
    try {
      const result = await invoke<CheckResult>("fav_check_gone", { all: false });
      await refresh();
      if (result.aborted) {
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

  return (
    <div className="h-full flex flex-col gap-2 p-3 text-slate-200">
      {/* 顶部操作区 */}
      <div className="flex items-center gap-2 flex-wrap">
        <SharedButton className="!h-7 !px-2" onClick={() => void runImport()} disabled={busy !== null}>
          {busy === "import" ? (
            <RefreshCw className="w-3 h-3 animate-spin" />
          ) : (
            <Download className="w-3 h-3" />
          )}
          {t("favorites.import")}
        </SharedButton>
        {busy === "import" && (
          <SharedButton variant="secondary" className="!h-7 !px-2" onClick={() => void invoke("fav_cancel_import")}>
            <Square className="w-3 h-3" />
            {t("favorites.cancel")}
          </SharedButton>
        )}

        {/* 收藏模块自己的 GitHub Token（与 SDK 模块相互独立） */}
        <button
          onClick={() => setTokenOpen(true)}
          className={`p-1 rounded cursor-pointer transition-colors ${
            tokenConfigured ? "text-emerald-400" : "text-slate-500 hover:text-slate-200"
          }`}
          title={
            tokenConfigured
              ? t("favorites.githubTokenSetTip")
              : t("favorites.githubTokenNeedTip")
          }
        >
          <Lock className="w-3.5 h-3.5" />
        </button>

        <div className="flex items-center gap-1">
          <SharedButton
            variant="secondary"
            className="!h-7 !px-2"
            onClick={() => (biliConfigured ? void runImportBili() : setCookieOpen(true))}
            disabled={busy !== null}
          >
            {busy === "bili" ? (
              <RefreshCw className="w-3 h-3 animate-spin" />
            ) : (
              <Tv className="w-3 h-3" />
            )}
            {t("favorites.importBili")}
          </SharedButton>
          <button
            onClick={() => setCookieOpen(true)}
            className="p-1 rounded text-slate-500 hover:text-slate-200 cursor-pointer"
            title={t("favorites.biliCookieTitle")}
          >
            <KeyRound className="w-3 h-3" />
          </button>
        </div>

        <div className="flex items-center gap-1">
          <SharedButton
            variant="secondary"
            className="!h-7 !px-2"
            onClick={() => void runImportZhihu()}
            disabled={busy !== null}
            title={t("favorites.zhihuExperimental")}
          >
            {busy === "zhihu" ? (
              <RefreshCw className="w-3 h-3 animate-spin" />
            ) : (
              <BookMarked className="w-3 h-3" />
            )}
            {t("favorites.importZhihu")}
          </SharedButton>
          <button
            onClick={() => void invoke("fav_zhihu_open_login")}
            className="p-1 rounded text-slate-500 hover:text-slate-200 cursor-pointer"
            title={t("favorites.zhihuLogin")}
          >
            <LogIn className="w-3 h-3" />
          </button>
          {/* 页面卡住/白屏时重置窗口（不动登录 Cookie） */}
          <button
            onClick={() =>
              void invoke("fav_zhihu_close")
                .then(() => toast(t("favorites.zhihuClosed"), "ok"))
                .catch((e) => toast(String(e), "err"))
            }
            className="p-1 rounded text-slate-500 hover:text-rose-300 cursor-pointer"
            title={t("favorites.zhihuClose")}
          >
            <X className="w-3 h-3" />
          </button>
        </div>

        <div className="flex items-center gap-1">
          <select
            value={providerId}
            onChange={(e) => {
              setProviderId(e.target.value);
              const p = providers.find((x) => x.id === e.target.value);
              setModelId(p?.active_model_id || p?.models[0]?.id || "");
            }}
            className="glass-input px-2 h-7 text-[11px] cursor-pointer max-w-[140px]"
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
            className="glass-input px-2 h-7 text-[11px] cursor-pointer max-w-[180px]"
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
          <SharedButton
            className="!h-7 !px-2"
            onClick={() => void runClassify()}
            disabled={busy !== null || providers.length === 0}
          >
            {busy === "classify" ? (
              <RefreshCw className="w-3 h-3 animate-spin" />
            ) : (
              <Sparkles className="w-3 h-3" />
            )}
            {t("favorites.classify")}
          </SharedButton>
        </div>

        <SharedButton
          variant="secondary"
          className="!h-7 !px-2"
          onClick={() => void runCheck()}
          disabled={busy !== null}
        >
          {busy === "check" ? (
            <RefreshCw className="w-3 h-3 animate-spin" />
          ) : (
            <AlertTriangle className="w-3 h-3" />
          )}
          {t("favorites.check")}
        </SharedButton>

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

                  <button
                    onClick={() => removeItem(item)}
                    className="p-1 rounded text-slate-600 hover:text-rose-400 cursor-pointer opacity-0 group-hover:opacity-100"
                    title={t("favorites.delete")}
                  >
                    <Trash2 className="w-3 h-3" />
                  </button>
                </div>
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

      {/* B站 Cookie：登录后从浏览器开发者工具复制整条 Cookie（需含 SESSDATA） */}
      {cookieOpen && (
        <div
          className="fixed inset-0 z-[250] flex items-center justify-center bg-black/60 backdrop-blur-sm p-4"
          onClick={() => setCookieOpen(false)}
        >
          <div
            className="w-[460px] max-w-full rounded-2xl border border-white/10 bg-slate-900 p-4 space-y-3"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="text-[13px] font-bold text-white">{t("favorites.biliCookieTitle")}</div>
            <p className="text-[10px] text-slate-400 leading-snug">
              {t("favorites.biliCookieHint")}
            </p>
            <textarea
              value={cookieText}
              onChange={(e) => setCookieText(e.target.value)}
              placeholder={t("favorites.biliCookiePlaceholder")}
              spellCheck={false}
              className="w-full h-24 glass-input p-2 text-[10px] font-mono resize-y"
            />
            <p className="text-[10px] text-amber-400/80 leading-snug">
              {t("favorites.biliExperimental")}
            </p>
            <div className="flex justify-end gap-2">
              <SharedButton variant="secondary" onClick={() => setCookieOpen(false)}>
                {t("common.cancel")}
              </SharedButton>
              <SharedButton onClick={() => void saveCookie()} disabled={!cookieText.trim()}>
                {t("favorites.biliCookieSave")}
              </SharedButton>
            </div>
          </div>
        </div>
      )}

      <ConfirmDialogHost request={confirmRequest} onClose={() => setConfirmRequest(null)} />
    </div>
  );
}
