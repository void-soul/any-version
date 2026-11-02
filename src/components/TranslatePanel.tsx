import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Note } from "./shared/Note";
import {
  Languages,
  ArrowRightLeft,
  ArrowUp,
  Copy,
  Check,
  Trash2,
  Loader2,
  Pin,
  Search,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { HotkeyRecorder } from "./shared/HotkeyRecorder";
import { ModuleSettingsButton, SettingsGroup, SettingsRow } from "./shared/ModuleSettings";
import type { AppSettings } from "../contracts/appSettings";
import { loadAppSettings, patchAppSettings } from "../services/appSettings";
import { ModelSelector } from "./shared/ai/ModelSelector";
import { toast } from "./shared/Toast";
import { btnPrimary, labelCls } from "./shared/Button";

// ─── 类型 ───

interface AiProvider {
  id: string;
  name: string;
  category: string;
  api_key: string;
  openai_url: string;
  anthropic_url: string;
  google_url: string;
  models: { id: string; name: string }[];
  active_model_id: string | null;
}

interface AiConfig {
  providers: AiProvider[];
  proxy_port: number;
  default_project_path: string;
  skills_dir: string;
}

interface TranslateConfig {
  providerId: string | null;
  modelId: string | null;
  targetLang: string | null;
}

interface HistoryEntry {
  id: string;
  source: string;
  result: string;
  target: string;
  provider: string;
  model: string;
  ts: number;
  pinned?: boolean;
}

// ─── API ───

function getTranslateConfig(): Promise<TranslateConfig> {
  return invoke("get_translate_config");
}
function saveTranslateConfig(cfg: TranslateConfig): Promise<void> {
  return invoke("save_translate_config", { config: cfg });
}
function doTranslate(text: string, providerId: string | null, modelId: string | null, target: string): Promise<string> {
  return invoke("translate_text", {
    text,
    providerId,
    modelId,
    targetLang: target,
  });
}

// ─── 面板 ───

/** 翻译模块专属设置：划词翻译热键（数据源为 AppSettings）。
 *  弹窗打开时才挂载，因此可以在这里按需拉取设置。 */
function TranslateModuleSettings() {
  const { t } = useTranslation();
  const [cfg, setCfg] = useState<AppSettings | null>(null);

  const [loadError, setLoadError] = useState("");
  useEffect(() => {
    let disposed = false;
    loadAppSettings()
      .then((c) => { if (!disposed) setCfg(c); })
      .catch((e) => { if (!disposed) setLoadError(String(e)); });
    return () => { disposed = true; };
  }, []);

  const save = async (patch: Partial<AppSettings>) => {
    if (!cfg) return;
    try {
      setCfg(await patchAppSettings(patch));
    } catch (e) {
      console.error("保存翻译设置失败:", e);
      toast(t("settings.saveFailedTryAgain"), "err");
    }
  };

  if (loadError) return <div role="alert" className="ui-note-error">{loadError}</div>;
  if (!cfg) {
    return <div className="text-caption text-slate-500">…</div>;
  }

  return (
    <SettingsGroup>
      <SettingsRow
        label={t("settings.translateHotkey")}
        hint={t("settings.translateHotkeyHint")}
      >
        <HotkeyRecorder
          value={cfg.selectionTranslateHotkey ?? ""}
          onChange={(hotkey) => save({ selectionTranslateHotkey: hotkey })}
          clearTitle={t("settings.clearTranslateHotkey")}
        />
      </SettingsRow>
    </SettingsGroup>
  );
}

export default function TranslatePanel() {
  const { t } = useTranslation();
  const [providers, setProviders] = useState<AiProvider[]>([]);
  const [source, setSource] = useState("");
  const [target, setTarget] = useState("中文");
  const [result, setResult] = useState("");
  const [translating, setTranslating] = useState(false);
  const [error, setError] = useState("");
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  // 搜索：输入过程只更新 searchInput，点击「搜索」/回车后才同步到 keyword（与剪贴板一致，避免输入卡顿）
  const [searchInput, setSearchInput] = useState("");
  const [keyword, setKeyword] = useState("");
  const [copied, setCopied] = useState(false);
  // 历史条目内复制反馈："id:source" / "id:result"
  const [copiedEntry, setCopiedEntry] = useState<string | null>(null);

  // 划词翻译独立模型选择（默认继承 AI 当前选中）
  const [providerId, setProviderId] = useState<string>("");
  const [modelId, setModelId] = useState<string>("");
  const [modelInitialized, setModelInitialized] = useState(false);

  const textRef = useRef<HTMLTextAreaElement>(null);
  const [actionError, setActionError] = useState("");
  const alive = useRef(false);
  const historyRequest = useRef(0);
  const sending = useRef(false);
  const preferenceQueue = useRef<Promise<void>>(Promise.resolve());
  const preferenceVersion = useRef(0);
  const copyTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const entryTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const refreshHistory = async () => {
    if (!alive.current) return;
    const request = ++historyRequest.current;
    try {
      const list = await invoke<HistoryEntry[]>("translate_history_list");
      if (alive.current && request === historyRequest.current) setHistory(list);
    } catch (e) {
      if (alive.current && request === historyRequest.current) setActionError(String(e));
    }
  };
  const persistConfig = (config: TranslateConfig) => {
    const version = ++preferenceVersion.current;
    preferenceQueue.current = preferenceQueue.current.then(async () => {
      try {
        await saveTranslateConfig(config);
        if (alive.current && version === preferenceVersion.current) setActionError("");
      } catch (e) {
        if (alive.current && version === preferenceVersion.current) setActionError(String(e));
      }
    });
  };


  // 全屏滚动：整个面板（含头部/输入/结果/历史）作为一个滚动区，
  // 不再只有历史列表内部滚动；滚动超过阈值时显示「返回顶部」按钮。
  const scrollRef = useRef<HTMLDivElement>(null);
  const [showTop, setShowTop] = useState(false);
  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    setShowTop(el.scrollTop > 300);
  };
  const scrollToTop = () => {
    scrollRef.current?.scrollTo({ top: 0, behavior: "smooth" });
  };

  // 加载 AI 配置 + 划词翻译已保存的选择 + 翻译历史（含悬浮窗产生的记录）
  useEffect(() => {
    let disposed = false;
    alive.current = true;
    (async () => {
      try {
        const [aiCfg, tCfg] = await Promise.all([
          invoke<AiConfig>("get_ai_config"),
          getTranslateConfig(),
        ]);
        if (disposed) return;
        setProviders(aiCfg.providers);
        // 计算默认 provider：保存的 > 第一个可用
        let defProvider = "";
        if (tCfg.providerId && aiCfg.providers.some((p) => p.id === tCfg.providerId)) {
          defProvider = tCfg.providerId!;
        } else {
          const first = aiCfg.providers.find((p) => !p.openai_url) || aiCfg.providers[0];
          defProvider = first?.id || "";
        }
        setProviderId(defProvider);
        // 计算默认模型：保存的 > 该 provider 的 active_model_id > 第一个模型
        const selProvider = aiCfg.providers.find((p) => p.id === defProvider);
        let defModel = "";
        if (tCfg.modelId && selProvider?.models.some((m) => m.id === tCfg.modelId)) {
          defModel = tCfg.modelId!;
        } else if (selProvider?.active_model_id && selProvider.models.some((m) => m.id === selProvider.active_model_id)) {
          defModel = selProvider.active_model_id!;
        } else {
          defModel = selProvider?.models[0]?.id || "";
        }
        setModelId(defModel);
        // 目标语言（划词翻译默认值，持久化到划词翻译配置）
        if (tCfg.targetLang && tCfg.targetLang.trim()) {
          setTarget(tCfg.targetLang);
        }
        setModelInitialized(true);
      } catch (e) {
        if (disposed) return;
        setActionError(String(e));
        setModelInitialized(true);
      }
    })();
    // 翻译历史：载入持久化的历史（面板与划词悬浮窗共享），并监听后端变更实时刷新
    void refreshHistory();
    let unlisten: (() => void) | undefined;
    listen("translate-history-changed", () => { if (!disposed) void refreshHistory(); })
      .then((stop) => { if (disposed) stop(); else unlisten = stop; })
      .catch((e) => { if (!disposed) setActionError(String(e)); });
    return () => {
      disposed = true;
      alive.current = false;
      historyRequest.current++;
      unlisten?.();
      if (copyTimer.current) clearTimeout(copyTimer.current);
      if (entryTimer.current) clearTimeout(entryTimer.current);
    };
  }, []);

  // 目标语言：既用于本面板翻译，也作为划词翻译默认值持久化
  const changeTarget = (t: string) => {
    setTarget(t);
    persistConfig({ providerId, modelId, targetLang: t });
  };

  const selectedProvider = providers.find((p) => p.id === providerId);

  const translate = async () => {
    const text = source.trim();
    if (!text || sending.current || !modelInitialized || !selectedProvider) return;
    sending.current = true;
    setTranslating(true);
    setError("");
    setResult("");
    setCopied(false);
    try {
      const res = await doTranslate(text, providerId || null, modelId || null, target);
      if (!alive.current) return;
      setResult(res);
      // 历史已由后端 translate_text 统一记录（含悬浮窗），这里刷新列表即可
      void refreshHistory();
    } catch (e: any) {
      if (alive.current) setError(String(e));
    } finally {
      sending.current = false;
      if (alive.current) setTranslating(false);
    }
  };

  const copyResult = async () => {
    if (!result) return;
    try {
      await navigator.clipboard.writeText(result);
      if (!alive.current) return;
      setCopied(true);
      if (copyTimer.current) clearTimeout(copyTimer.current);
      copyTimer.current = setTimeout(() => setCopied(false), 1500);
    } catch (e) { if (alive.current) setActionError(String(e)); }
  };

  // 搜索按钮触发：把临时输入同步到 keyword，驱动下方 useMemo 过滤历史
  const handleSearch = () => setKeyword(searchInput);
  // 根据实际搜索词过滤历史（匹配原文/译文），仅点击搜索/回车后生效
  const filteredHistory = useMemo(() => {
    const kw = keyword.trim().toLowerCase();
    if (!kw) return history;
    return history.filter(
      (h) =>
        (h.source || "").toLowerCase().includes(kw) ||
        (h.result || "").toLowerCase().includes(kw)
    );
  }, [history, keyword]);

  const clearHistory = () => {
    // 置顶条目保留，清空后刷新列表显示剩余的置顶条目
    invoke("translate_history_clear")
      .then(refreshHistory)
      .catch((e) => { if (alive.current) setActionError(String(e)); });
  };

  // 复制历史条目的原文/译文
  const copyEntry = async (h: HistoryEntry, kind: "source" | "result") => {
    try {
      await navigator.clipboard.writeText(kind === "source" ? h.source : h.result);
      if (!alive.current) return;
      setCopiedEntry(h.id + ":" + kind);
      if (entryTimer.current) clearTimeout(entryTimer.current);
      entryTimer.current = setTimeout(() => setCopiedEntry(null), 1200);
    } catch (e) { if (alive.current) setActionError(String(e)); }
  };

  // 删除 / 置顶一条历史
  const deleteEntry = (id: string) => {
    invoke("translate_history_delete", { id })
      .then(refreshHistory)
      .catch((e) => { if (alive.current) setActionError(String(e)); });
  };
  const togglePin = (id: string) => {
    invoke("translate_history_pin", { id })
      .then(refreshHistory)
      .catch((e) => { if (alive.current) setActionError(String(e)); });
  };

  const isConfigured = providers.length > 0 && selectedProvider;

  return (
    <div ref={scrollRef} onScroll={onScroll} className="h-full w-full overflow-y-auto">
    <div className="w-full px-6 py-4 max-w-[1100px] mx-auto space-y-5 select-none text-slate-200">
      {/* 头部 */}
      <div className="flex flex-col md:flex-row md:items-center justify-between gap-3 border-b border-white/10 pb-4">
        <div className="flex items-center gap-3">
          <div className="w-9 h-9 rounded-card bg-[var(--module-accent)]/15 border border-[var(--module-accent)]/30 flex items-center justify-center">
            <Languages className="w-4 h-4 text-[var(--module-accent)]" />
          </div>
          <div>
            <h2 className="text-title font-bold text-white tracking-wide">{t("tranpanel.title")}</h2>
            <p className="text-tiny text-slate-500">
              {t("tranpanel.subtitle")}
            </p>
          </div>
        </div>
        {/* 模块专属设置入口（划词翻译热键） */}
        <ModuleSettingsButton title={t("tranpanel.settingsTitle")}>
          <TranslateModuleSettings />
        </ModuleSettingsButton>
      </div>

      {/* 模型选择（供应商 + 模型合并为一个选择器） */}
      <div>
        <label className={labelCls}>{t("tranpanel.model")}</label>
        <ModelSelector
          providers={providers}
          providerId={providerId}
          modelId={modelId}
          onSelect={(pid, mid) => {
            setProviderId(pid);
            setModelId(mid);
            persistConfig({ providerId: pid, modelId: mid, targetLang: target });
          }}
          disabled={!modelInitialized || providers.length === 0}
          emptyText={t("tranpanel.noProviderCfg")}
        />
      </div>

      {actionError && <div role="alert" className="ui-note-error flex items-start justify-between gap-2"><span className="break-words min-w-0">{actionError}</span><button type="button" className="ui-icon-btn w-7 h-7 shrink-0" aria-label={t("common.dialogClose")} onClick={() => setActionError("")}><X className="w-3.5 h-3.5" /></button></div>}
      {/* 翻译输入 / 结果 */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        {/* 原文 */}
        <div className="bg-white/[0.03] border border-white/10 rounded-card p-3 flex flex-col">
          <div className="flex items-center justify-between mb-2">
            <span className="text-tiny font-semibold text-slate-400">{t("tranpanel.source")}</span>
            <button
              onClick={() => setSource("")}
              disabled={!source}
              className="text-tiny text-slate-500 hover:text-slate-300 cursor-pointer disabled:opacity-40 flex items-center gap-1"
            >
              <X className="w-3 h-3" /> {t("tranpanel.clear")}
            </button>
          </div>
          <textarea
            aria-label={t("tranpanel.source")}
            ref={textRef}
            value={source}
            onChange={(e) => setSource(e.target.value)}
            placeholder={t("tranpanel.phInput")}
            className="w-full flex-1 min-h-[180px] bg-transparent text-body text-slate-100 placeholder:text-slate-600 focus:outline-none resize-none leading-relaxed"
            onKeyDown={(e) => {
              if ((e.ctrlKey || e.metaKey) && e.key === "Enter") { e.preventDefault(); void translate(); }
            }}
          />
        </div>

        {/* 结果 */}
        <div className="bg-white/[0.03] border border-white/10 rounded-card p-3 flex flex-col">
          <div className="flex items-center justify-between mb-2">
            <span className="text-tiny font-semibold text-slate-400">{t("tranpanel.translated")}</span>
            <div className="flex items-center gap-1.5">
              <select
                disabled={!modelInitialized}
                aria-label={t("tranpanel.translated")}
                value={target}
                onChange={(e) => changeTarget(e.target.value)}
                className="ui-input h-8 px-2 text-caption"
              >
                {["中文", "English", "日本語", "한국어", "Français", "Deutsch", "Русский", "Español"].map((t) => (
                  <option key={t} value={t}>
                    {t}
                  </option>
                ))}
              </select>
              {result && (
                <button
                  onClick={copyResult}
                  className="ui-icon-btn w-7 h-7"
                  title={t("tranpanel.copyResult")}
                >
                  {copied ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <Copy className="w-3.5 h-3.5" />}
                </button>
              )}
            </div>
          </div>
          {translating ? (
            <div role="status" className="flex-1 flex items-center justify-center gap-2 text-slate-400 text-body">
              <Loader2 className="w-4 h-4 animate-spin text-[var(--module-accent)]" />
              {t("tranpanel.translating")}
            </div>
          ) : error ? (
            <div role="alert" className="flex-1 text-body text-red-400 leading-relaxed break-words">{error}</div>
          ) : result ? (
            <div className="flex-1 text-body text-slate-100 leading-relaxed whitespace-pre-wrap break-words">{result}</div>
          ) : (
            <div className="flex-1 flex items-center justify-center text-slate-600 text-body">{t("tranpanel.resultPlaceholder")}</div>
          )}
        </div>
      </div>

      {/* 翻译按钮 */}
      <div className="flex items-center justify-end gap-2">
        <span className="text-tiny text-slate-500">{t("tranpanel.quickKey")}</span>
        <button
          onClick={translate}
          disabled={!modelInitialized || translating || !isConfigured || !source.trim()}
          aria-busy={translating}
          className={btnPrimary}
        >
          <ArrowRightLeft className="w-3.5 h-3.5" />
          {translating ? t("tranpanel.translating") : t("tranpanel.translate")}
        </button>
      </div>

      {/* 历史 */}
      {history.length > 0 && (
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <span className="text-tiny font-semibold text-slate-400">{t("tranpanel.history")}</span>
            <button
              onClick={clearHistory}
              title={t("tranpanel.clearHistory")}
              className="text-tiny text-slate-500 hover:text-red-400 cursor-pointer flex items-center gap-1"
            >
              <Trash2 className="w-3 h-3" /> {t("tranpanel.clearPinned")}
            </button>
          </div>

          {/* 历史搜索：输入不触发，点「搜索」或回车后才过滤 */}
          <div className="flex items-center gap-1 bg-white/5 border border-white/10 rounded-ctl pl-2 pr-1 h-9">
            <Search className="w-3 h-3 text-slate-500 flex-shrink-0" />
            <input
              aria-label={t("tranpanel.historyPh")}
              value={searchInput}
              onChange={(e) => setSearchInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleSearch();
                if (e.key === "Escape") {
                  setSearchInput("");
                  setKeyword("");
                }
              }}
              placeholder={t("tranpanel.historyPh")}
              className="flex-1 bg-transparent outline-none text-caption text-slate-200 placeholder:text-slate-500"
            />
            <button
              onClick={handleSearch}
              className="px-2 py-0.5 rounded-md bg-[var(--module-accent)] text-white text-tiny font-semibold hover:opacity-90 cursor-pointer"
              title={t("tranpanel.search")}
            >
              {t("tranpanel.search")}
            </button>
            {searchInput && (
              <button
                onClick={() => {
                  setSearchInput("");
                  setKeyword("");
                }}
                className="text-slate-500 hover:text-slate-300 cursor-pointer"
                title={t("tranpanel.clearTitle")}
              >
                <X className="w-3 h-3" />
              </button>
            )}
          </div>

          <div className="space-y-2 pr-1">
            {filteredHistory.map((h) => (
              <div
                key={h.id}
                className="bg-white/[0.02] border border-white/5 rounded-ctl p-3 space-y-1.5 hover:border-white/15 transition"
              >
                <div className="flex items-center justify-between text-micro text-slate-500">
                  <span>
                    {h.provider} · {h.model} → {h.target}
                  </span>
                  <div className="flex items-center gap-1">
                    <span className="mr-1">{new Date(h.ts).toLocaleTimeString()}</span>
                    <button
                      onClick={() => copyEntry(h, "source")}
                      title={t("tranpanel.copySource")}
                      className="px-1.5 py-0.5 rounded bg-white/5 border border-white/10 hover:bg-white/10 hover:text-white cursor-pointer flex items-center gap-1"
                    >
                      {copiedEntry === h.id + ":source" ? (
                        <Check className="w-3 h-3 text-emerald-400" />
                      ) : (
                        <Copy className="w-3 h-3" />
                      )}
                      <span>{t("tranpanel.source")}</span>
                    </button>
                    <button
                      onClick={() => copyEntry(h, "result")}
                      title={t("tranpanel.copyResult")}
                      className="px-1.5 py-0.5 rounded bg-white/5 border border-white/10 hover:bg-white/10 hover:text-white cursor-pointer flex items-center gap-1"
                    >
                      {copiedEntry === h.id + ":result" ? (
                        <Check className="w-3 h-3 text-emerald-400" />
                      ) : (
                        <Copy className="w-3 h-3" />
                      )}
                      <span>{t("tranpanel.translated")}</span>
                    </button>
                    <button
                      onClick={() => togglePin(h.id)}
                      title={h.pinned ? t("tranpanel.pinOff") : t("tranpanel.pinOn")}
                      className={`p-1 rounded hover:bg-white/10 cursor-pointer ${
                        h.pinned ? "text-emerald-400" : "hover:text-white"
                      }`}
                    >
                      <Pin className="w-3 h-3" />
                    </button>
                    <button
                      onClick={() => deleteEntry(h.id)}
                      title={t("tranpanel.delete")}
                      className="p-1 rounded hover:bg-red-500/20 hover:text-red-400 cursor-pointer"
                    >
                      <Trash2 className="w-3 h-3" />
                    </button>
                  </div>
                </div>
                <p className="text-tiny text-slate-400 line-clamp-2">{h.source}</p>
                <p className="text-body text-slate-100 leading-relaxed">{h.result}</p>
              </div>
            ))}
            {filteredHistory.length === 0 && (
              <p className="text-caption text-slate-500 text-center py-3">
                {t("tranpanel.noHistoryMatch", { keyword: keyword.trim() })}
              </p>
            )}
          </div>
        </div>
      )}

      {!isConfigured && (
        <Note tone="warn">{t("tranpanel.noProviderDesc")}</Note>
      )}
    </div>

      {/* 返回顶部：滚动超过阈值后浮现，平滑滚回顶部 */}
      {showTop && (
        <button
          onClick={scrollToTop}
          title={t("tranpanel.backToTop")}
          className="fixed bottom-8 right-8 z-40 flex h-9 w-9 items-center justify-center rounded-full border border-white/15 bg-surface-panel/90 text-slate-300 shadow-xl backdrop-blur transition hover:text-white hover:border-[var(--module-accent)]/60 cursor-pointer"
        >
          <ArrowUp className="w-4 h-4" />
        </button>
      )}
    </div>
  );
}
