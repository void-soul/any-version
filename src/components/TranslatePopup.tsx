import React, { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Pin, PinOff, Copy, Check, X, Languages, Loader2, ArrowRightLeft } from "lucide-react";

interface TranslateResult {
  source?: string;
  result?: string;
  target?: string;
  loading?: boolean;
  error?: boolean;
}

interface AiProvider {
  id: string;
  name: string;
  openai_url: string;
  models: { id: string; name: string }[];
}
interface AiConfig {
  providers: AiProvider[];
}

const TARGETS = ["中文", "English", "日本語", "한국어", "Français", "Deutsch", "Русский", "Español"];

export default function TranslatePopup() {
  const [result, setResult] = useState<TranslateResult | null>(null);
  const [copyOk, setCopyOk] = useState(false);
  const [translating, setTranslating] = useState(false);
  // 模型选择（默认取全局设置/翻译模块的 translate_config）
  const [providers, setProviders] = useState<AiProvider[]>([]);
  const [provId, setProvId] = useState("");
  const [modelId, setModelId] = useState("");
  // 可编辑的原文（从 result.source 初始化，可修改后重新翻译）
  const [sourceText, setSourceText] = useState("");
  const pinnedRef = useRef(false);
  const [, setPinnedTick] = useState(0);

  const appWindow = getCurrentWindow();

  // 关闭/隐藏悬浮窗
  const hidePopup = () => {
    invoke("hide_translate_popup");
  };

  const togglePin = () => {
    pinnedRef.current = !pinnedRef.current;
    setPinnedTick((t) => t + 1);
  };

  const [copySourceOk, setCopySourceOk] = useState(false);

  const copyText = async (text: string | undefined, kind: "source" | "result") => {
    if (!text) return;
    await navigator.clipboard.writeText(text);
    if (kind === "source") {
      setCopySourceOk(true);
      setTimeout(() => setCopySourceOk(false), 1500);
    } else {
      setCopyOk(true);
      setTimeout(() => setCopyOk(false), 1500);
    }
  };

  const copySource = () => copyText(sourceText || result?.source, "source");
  const copyResult = () => copyText(result?.result, "result");

  // 重新翻译当前原文（用当前选择的模型/目标语言）；默认翻译 sourceText
  const retranslate = async (opts?: { provId?: string; modelId?: string; lang?: string; text?: string }) => {
    const text = opts?.text ?? sourceText ?? result?.source;
    if (!text || translating) return;
    const pid = opts?.provId ?? provId;
    const mid = opts?.modelId ?? modelId;
    const lang = opts?.lang ?? result?.target ?? "中文";
    setTranslating(true);
    try {
      const translated = await invoke<string>("translate_text", {
        text,
        providerId: pid || null,
        modelId: mid || null,
        targetLang: lang,
      });
      setResult({ source: text, result: translated, target: lang });
    } catch (e: any) {
      setResult({ source: text, result: String(e), target: lang, error: true });
    } finally {
      setTranslating(false);
    }
  };

  // 翻译按钮：翻译当前编辑后的原文
  const doTranslate = () => retranslate({ text: sourceText });
  const doTranslateRef = useRef(doTranslate);
  doTranslateRef.current = doTranslate;
  const copyResultRef = useRef(copyResult);
  copyResultRef.current = copyResult;
  const hidePopupRef = useRef(hidePopup);
  hidePopupRef.current = hidePopup;

  // 切换目标语言
  const changeTarget = (lang: string) => retranslate({ lang });

  // 切换供应商（联动模型）并保存，随后重新翻译
  const changeProvider = (pid: string) => {
    if (!providers.length) return;
    setProvId(pid);
    const p = providers.find((x) => x.id === pid);
    const m = p?.models[0]?.id || "";
    setModelId(m);
    saveTranslateModel(pid, m);
    if (sourceText) retranslate({ provId: pid, modelId: m });
  };

  // 切换模型并保存，随后重新翻译
  const changeModel = (mid: string) => {
    setModelId(mid);
    saveTranslateModel(provId, mid);
    if (sourceText) retranslate({ modelId: mid });
  };

  // 保存模型选择（作为全局默认，逻辑同翻译模块）
  const saveTranslateModel = (pid: string, mid: string) => {
    invoke("save_translate_config", {
      config: { providerId: pid, modelId: mid, targetLang: null },
    }).catch((e) => console.error("保存翻译默认模型失败:", e));
  };

  useEffect(() => {
    // 加载 AI 配置 + 翻译默认模型（默认取全局设置/翻译模块的 translate_config）
    (async () => {
      try {
        const [aiCfg, tCfg] = await Promise.all([
          invoke<AiConfig>("get_ai_config"),
          invoke<{ providerId: string | null; modelId: string | null }>("get_translate_config"),
        ]);
        setProviders(aiCfg.providers);
        const defProv =
          tCfg.providerId && aiCfg.providers.some((p) => p.id === tCfg.providerId)
            ? tCfg.providerId!
            : (aiCfg.providers[0]?.id || "");
        setProvId(defProv);
        const p = aiCfg.providers.find((x) => x.id === defProv);
        const defModel =
          tCfg.modelId && p?.models.some((m) => m.id === tCfg.modelId)
            ? tCfg.modelId!
            : (p?.models[0]?.id || "");
        setModelId(defModel);
      } catch (e) {
        console.error("加载翻译配置失败:", e);
      }
    })();
  }, []);

  useEffect(() => {
    // 窗口首次创建时，后端在页面 JS 挂载前就会 show/set_focus/emit，
    // 事件会丢失，且 set_focus 早于监听器注册导致 onFocusChanged 也不触发——
    // 因此挂载时主动拉取最近一次划词翻译结果，立即显示最新原文。
    // 同时显式聚焦 WebView2 内容：仅聚焦顶层 HWND 时前端收不到
    // tauri://focus / tauri://blur，导致「失焦自动关闭（钉住判断）」失效。
    getCurrentWebview()
      .setFocus()
      .catch(() => {});
    invoke<TranslateResult | null>("get_last_translate_result")
      .then((last) => {
        if (last && last.source) {
          setResult(last);
          setTranslating(!!last.loading);
          setSourceText(last.source);
        }
      })
      .catch(() => {});
    // 监听后端推送的翻译结果
    const unlisten = listen<TranslateResult>("translate-result", (e) => {
      setResult(e.payload);
      setTranslating(!!e.payload.loading);
      if (e.payload.source) setSourceText(e.payload.source);
    });
    // 失焦自动隐藏（除非已钉住）
    const unFocus = appWindow.onFocusChanged(({ payload: focused }) => {
      if (focused) {
        // 窗口获得焦点：主动拉取最近一次结果，兜底同步（修复窗口复用/事件丢失导致不更新）
        invoke<TranslateResult | null>("get_last_translate_result")
          .then((last) => {
            if (last && last.source) {
              setResult(last);
              setTranslating(!!last.loading);
              setSourceText(last.source);
            }
          })
          .catch(() => {});
      } else if (!pinnedRef.current) {
        appWindow.hide();
      }
    });
    // 快捷键：ESC 关闭、Ctrl+C 复制译文、Ctrl+Enter 翻译
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        hidePopupRef.current();
      } else if ((e.ctrlKey || e.metaKey) && (e.key === "c" || e.key === "C")) {
        e.preventDefault();
        copyResultRef.current();
      } else if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
        e.preventDefault();
        doTranslateRef.current();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      unlisten.then((f) => f());
      unFocus.then((f) => f());
      window.removeEventListener("keydown", onKey, true);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 轮询兜底：translate-result 事件可能在窗口新建（页面未挂载）/页面重载等时序下
  // 丢失，导致 UI 一直停留在「翻译中」。只要处于翻译中，就定时拉取最近一次结果，
  // 保证悬浮窗最终收敛到最新状态（翻译完成/失败后 loading=false，自动停止轮询）。
  useEffect(() => {
    if (!translating) return;
    const timer = setInterval(() => {
      invoke<TranslateResult | null>("get_last_translate_result")
        .then((last) => {
          if (last && last.source) {
            setResult(last);
            setTranslating(!!last.loading);
            setSourceText(last.source);
          }
        })
        .catch(() => {});
    }, 400);
    return () => clearInterval(timer);
  }, [translating]);

  const pinned = pinnedRef.current;

  return (
    <>
      {/* 悬浮窗是透明无边框窗口：覆盖全局 body 背景为透明，避免显示深色方块 */}
      <style>{`
        html, body {
          background: transparent !important;
          background-color: transparent !important;
        }
      `}</style>
      <div className="w-screen h-screen bg-transparent select-none">
        <div
          className="w-full h-full rounded-none border border-white/15 bg-[#1b1d23]/95 shadow-2xl shadow-black/60 overflow-hidden"
          data-tauri-drag-region
        >
        {/* 标题栏：可拖拽（原生 app-region drag，兼容透明无边框窗口） */}
        <div
          className="flex items-center justify-between px-3 py-2 border-b border-white/10 cursor-move"
          data-tauri-drag-region
          style={{ WebkitAppRegion: "drag" } as any}
        >
          <div className="flex items-center gap-1.5 text-slate-300">
            <Languages className="w-3.5 h-3.5 text-emerald-400" />
            <span className="text-[11px] font-semibold tracking-wide">翻译</span>
            {result?.target && (
              <span className="text-[9px] px-1.5 py-0.5 rounded bg-white/10 text-slate-400">
                目标：{result.target}
              </span>
            )}
          </div>
          <div className="flex items-center gap-1" style={{ WebkitAppRegion: "no-drag" } as any}>
            {/* 钉住 */}
            <button
              onClick={togglePin}
              className={`p-1.5 rounded-md cursor-pointer transition ${
                pinned
                  ? "bg-emerald-500/20 text-emerald-400"
                  : "text-slate-400 hover:bg-white/10 hover:text-white"
              }`}
              title={pinned ? "已钉住（不自动消失）" : "未钉住（失焦自动隐藏）"}
            >
              {pinned ? <Pin className="w-3.5 h-3.5" /> : <PinOff className="w-3.5 h-3.5" />}
            </button>
            {/* 复制译文 */}
            {result?.result && (
              <button
                onClick={copyResult}
                className="p-1.5 rounded-md text-slate-400 hover:bg-white/10 hover:text-white cursor-pointer"
                title="复制译文"
              >
                {copyOk ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <Copy className="w-3.5 h-3.5" />}
              </button>
            )}
            {/* 关闭 */}
            <button
              onClick={hidePopup}
              className="p-1.5 rounded-md text-slate-400 hover:bg-red-500/20 hover:text-red-400 cursor-pointer"
              title="关闭"
            >
              <X className="w-3.5 h-3.5" />
            </button>
          </div>
        </div>

        {/* 模型选择工具栏（默认继承全局设置，逻辑同翻译模块） */}
        <div className="px-3 py-1.5 border-b border-white/5 flex items-center gap-1.5">
          <Languages className="w-3 h-3 text-emerald-400 shrink-0" />
          <select
            value={provId}
            onChange={(e) => changeProvider(e.target.value)}
            disabled={translating || providers.length === 0}
            className="text-[9px] px-1.5 py-0.5 rounded bg-white/10 text-slate-300 border border-white/10 focus:outline-none disabled:opacity-50 cursor-pointer min-w-0 flex-1"
            title="翻译供应商"
          >
            {providers.length === 0 && <option value="">（无供应商）</option>}
            {providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
          <select
            value={modelId}
            onChange={(e) => changeModel(e.target.value)}
            disabled={translating || !provId}
            className="text-[9px] px-1.5 py-0.5 rounded bg-white/10 text-slate-300 border border-white/10 focus:outline-none disabled:opacity-50 cursor-pointer min-w-0 flex-1"
            title="翻译模型"
          >
            {(providers.find((p) => p.id === provId)?.models || []).map((m) => (
              <option key={m.id} value={m.id}>
                {m.name || m.id}
              </option>
            ))}
          </select>
          <select
            value={result?.target || "中文"}
            onChange={(e) => changeTarget(e.target.value)}
            disabled={translating}
            className="text-[9px] px-1.5 py-0.5 rounded bg-white/10 text-slate-300 border border-white/10 focus:outline-none disabled:opacity-50 cursor-pointer shrink-0"
            title="目标语言"
          >
            {TARGETS.map((t) => (
              <option key={t} value={t}>
                → {t}
              </option>
            ))}
          </select>
        </div>

        {/* 内容区：原文(可编辑) + 译文 + 翻译按钮 */}
        <div className="px-3 py-2.5 space-y-2.5 max-h-[360px] overflow-y-auto">
          {/* 原文：可编辑 textarea */}
          <div>
            <div className="flex items-center justify-between mb-1">
              <span className="text-[9px] text-slate-500">原文</span>
              {(sourceText || result?.source) && (
                <button
                  onClick={copySource}
                  className="text-slate-500 hover:text-white hover:bg-white/10 rounded p-1 cursor-pointer"
                  title="复制原文"
                >
                  {copySourceOk ? <Check className="w-3 h-3 text-emerald-400" /> : <Copy className="w-3 h-3" />}
                </button>
              )}
            </div>
            <textarea
              value={sourceText}
              onChange={(e) => setSourceText(e.target.value)}
              placeholder="输入或选中文本后按划词热键"
              className="w-full min-h-[60px] bg-white/5 border border-white/10 rounded-lg px-2 py-1.5 text-[11px] text-slate-200 placeholder:text-slate-600 focus:outline-none focus:border-emerald-500/50 resize-none leading-relaxed"
            />
          </div>

          {/* 翻译按钮 */}
          <div className="flex items-center justify-end gap-1.5">
            <span className="text-[9px] text-slate-600">
              目标：{result?.target || "中文"}
            </span>
            <button
              onClick={doTranslate}
              disabled={translating || !sourceText.trim()}
              className="px-3 py-1 rounded-lg text-[10px] font-semibold bg-emerald-500/90 text-white hover:bg-emerald-500 transition cursor-pointer disabled:opacity-50 flex items-center gap-1"
            >
              <ArrowRightLeft className="w-3 h-3" />
              {translating ? "翻译中…" : "翻译"}
            </button>
          </div>

          {/* 译文 */}
          <div>
            <div className="flex items-center justify-between mb-1">
              <span className="text-[9px] text-slate-500">译文</span>
              {result?.result && (
                <button
                  onClick={copyResult}
                  className="text-slate-500 hover:text-white hover:bg-white/10 rounded p-1 cursor-pointer"
                  title="复制译文"
                >
                  {copyOk ? <Check className="w-3 h-3 text-emerald-400" /> : <Copy className="w-3 h-3" />}
                </button>
              )}
            </div>
            {translating ? (
              <div className="flex items-center gap-2 text-slate-400 text-[11px] py-1">
                <Loader2 className="w-3.5 h-3.5 animate-spin text-emerald-400" />
                翻译中…
              </div>
            ) : result?.error ? (
              <div className="text-[11px] text-red-400 leading-relaxed whitespace-pre-wrap break-words">
                {result.result}
              </div>
            ) : (
              <div className="text-[12px] text-slate-100 leading-relaxed whitespace-pre-wrap break-words">
                {result?.result || (sourceText ? "等待翻译…" : "请选中文本后按划词热键")}
              </div>
            )}
          </div>
        </div>
      </div>
      </div>
    </>
  );
}
