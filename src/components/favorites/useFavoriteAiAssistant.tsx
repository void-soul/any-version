import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { Bot, Send, Sparkles, Loader2 } from "lucide-react";
import { ThinkingBlock } from "../shared/ThinkingBlock";
import { AI_TOOL_LABEL } from "./FavoritesPanel.model";
import { AiResultMarkdown } from "./FavoritesPanel.AiResultMarkdown";

/** Owns the FavoriteAiAssistant feature, independently of page rendering. */
export function useFavoriteAiAssistant({ providerId, modelId, aiWidth, toggleAiPanel }: {
  providerId: string;
  modelId: string;
  aiWidth: number;
  toggleAiPanel: () => void;
}) {
  const { t } = useTranslation();

  const [aiInput, setAiInput] = useState("");

  const [aiResult, setAiResult] = useState("");

  const [aiSteps, setAiSteps] = useState<{ step: string; text: string; tool?: string }[]>([]);

  const [aiBusy, setAiBusy] = useState(false);

  const [aiError, setAiError] = useState<string | null>(null);

  const aiLogRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const unlisten = listen<{ step: string; text: string; tool?: string }>(
      "favorites-agent-progress",
      (event) => {
        const s = event.payload;
        // 以前这里连 step === "thinking" 一起过滤掉，于是「思考」在收藏检索里
        // 什么都不显示；后端现在发的是 step === "reasoning"（模型的真实思考内容），
        // 要收下来渲染成可展开的思考块。
        if (s.step === "done") return;
        setAiSteps((prev) => [...prev, s]);
      },
    );
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  useEffect(() => {
    const el = aiLogRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [aiSteps]);

  const runAiSearch = async () => {
    const text = aiInput.trim();
    if (!text || aiBusy) return;
    setAiBusy(true);
    setAiError(null);
    setAiResult("");
    setAiSteps([]);
    try {
      const reply = await invoke<{ text: string }>("fav_agent_search", {
        providerId: providerId || null,
        modelId: modelId || null,
        prompt: text,
      });
      setAiResult(reply.text);
    } catch (e: any) {
      setAiError(String(e));
    } finally {
      setAiBusy(false);
    }
  };

const renderAiAssistant = () => (<aside
                className="shrink-0 flex flex-col min-h-0 glass-panel p-3 gap-2"
                style={{ width: aiWidth }}
              >
                <div className="flex items-center gap-2">
                  <div className="w-7 h-7 rounded-ctl bg-[var(--module-accent)]/15 border border-[var(--module-accent)]/30 flex items-center justify-center">
                    <Bot className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="text-caption font-bold text-white truncate">{t("favorites.aiSearchTitle")}</div>
                    <div className="text-micro text-slate-500 truncate">{t("favorites.aiSearchHint")}</div>
                  </div>
                  <button
                    onClick={toggleAiPanel}
                    className="p-1 rounded hover:bg-white/10 text-slate-400 hover:text-white cursor-pointer"
                    title={t("favorites.aiPanelClose")}
                  >
                    ✕
                  </button>
                </div>

                <div className="flex items-center gap-1.5">
                  <input
                    value={aiInput}
                    onChange={(e) => setAiInput(e.target.value)}
                    onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); void runAiSearch(); } }}
                    placeholder={t("favorites.aiSearchPh")}
                    className="flex-1 min-w-0 ui-input rounded-ctl px-2 py-1.5 text-caption text-slate-200 focus:outline-none focus:border-[var(--module-accent)]"
                  />
                  <button
                    onClick={() => void runAiSearch()}
                    disabled={aiBusy || !aiInput.trim()}
                    className="px-2 py-1.5 rounded-ctl text-tiny ui-btn-primary text-white font-semibold cursor-pointer disabled:opacity-40 flex items-center gap-1"
                  >
                    {aiBusy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Send className="w-3 h-3" />}
                    {aiBusy ? t("favorites.aiSearchRunning") : t("favorites.aiSearchGo")}
                  </button>
                </div>

                {/* 检索过程：让用户看得见 agent 到底查了什么，而不是黑箱等结果。
                    reasoning 步骤（模型的真实思考）走共享的 ThinkingBlock——
                    以前它被上面的过滤器整个丢掉，前端只显示工具调用。 */}
                {aiSteps.length > 0 && (
                  <div ref={aiLogRef} className="max-h-40 shrink-0 overflow-y-auto rounded-ctl border border-white/5 bg-black/30 p-2 space-y-1">
                    {aiSteps.map((s, i) =>
                      s.step === "reasoning" ? (
                        <ThinkingBlock key={i} text={s.text} pending={aiBusy && i === aiSteps.length - 1} />
                      ) : (
                        <div key={i} className="text-micro text-slate-500 flex gap-1.5">
                          <span className="flex-shrink-0 text-slate-600">{AI_TOOL_LABEL[s.tool ?? ""] ?? "过程"}</span>
                          <span className="min-w-0 break-all">{s.text.slice(0, 160)}</span>
                        </div>
                      ),
                    )}
                  </div>
                )}

                <div className="min-h-0 flex-1 overflow-y-auto rounded-card border border-white/5 bg-slate-900/30 p-2.5">
                  {aiError ? (
                    <div className="text-caption text-rose-400 break-all">{aiError}</div>
                  ) : aiResult ? (
                    <AiResultMarkdown text={aiResult} />
                  ) : (
                    <div className="text-caption text-slate-500 py-6 text-center">
                      <Sparkles className="w-4 h-4 mx-auto mb-2 text-slate-600" />
                      {t("favorites.aiSearchPlaceholder")}
                    </div>
                  )}
                </div>
              </aside>);
  return {
    setAiResult,
    setAiSteps,
    aiBusy,
    setAiError,
    renderAiAssistant,
  };
}
