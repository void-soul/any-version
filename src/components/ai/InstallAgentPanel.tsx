import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Bot, Send, Sparkles, Wrench, CheckCircle, AlertTriangle } from "lucide-react";
import type { AiConfig } from "./types";

/** 后端 install_agent::InstallAgentStep */
interface AgentStep {
  step: string;
  text: string;
  tool?: string;
  ok?: boolean;
}

interface AgentReply {
  text: string;
  steps: AgentStep[];
}

interface ChatLine {
  role: "user" | "agent" | "step";
  text: string;
  tool?: string;
  ok?: boolean;
}

const TOOL_LABEL: Record<string, string> = {
  list_tools: "列出工具",
  get_tool_info: "查看工具",
  install_tool: "执行安装",
};

export default function InstallAgentPanel() {
  const { t } = useTranslation();
  const [config, setConfig] = useState<AiConfig | null>(null);
  const [providerId, setProviderId] = useState<string>("");
  const [modelId, setModelId] = useState<string>("");
  const [input, setInput] = useState("");
  const [lines, setLines] = useState<ChatLine[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const logRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    invoke<AiConfig>("get_ai_config")
      .then((cfg) => {
        setConfig(cfg);
        const usable = cfg.providers.filter((p) => p.api_key.trim() !== "" && p.models.length > 0);
        if (usable.length > 0) {
          setProviderId(usable[0].id);
          setModelId(usable[0].active_model_id || usable[0].models[0]?.id || "");
        }
      })
      .catch(() => {});
  }, []);

  // 后端每推一步就追加一行：装工具动辄几十秒，不能让用户盯着转圈猜
  useEffect(() => {
    const unlisten = listen<AgentStep>("install-agent-progress", (e) => {
      const s = e.payload;
      if (s.step === "thinking") return;
      setLines((prev) => [...prev, { role: "step", text: s.text, tool: s.tool, ok: s.ok }]);
    });
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  useEffect(() => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [lines]);

  const usableProviders = (config?.providers ?? []).filter(
    (p) => p.api_key.trim() !== "" && p.models.length > 0,
  );
  const models = config?.providers.find((p) => p.id === providerId)?.models ?? [];

  const send = async () => {
    const text = input.trim();
    if (!text || busy) return;
    setInput("");
    setError(null);
    setLines((prev) => [...prev, { role: "user", text }]);
    setBusy(true);
    try {
      const reply = await invoke<AgentReply>("install_agent_chat", {
        providerId: providerId || null,
        modelId: modelId || null,
        prompt: text,
      });
      setLines((prev) => [...prev, { role: "agent", text: reply.text }]);
    } catch (e: any) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="h-full flex flex-col min-h-0 p-4 gap-3">
      <div className="flex items-center gap-2 flex-shrink-0">
        <div className="w-9 h-9 rounded-xl bg-[var(--module-accent)]/15 border border-[var(--module-accent)]/30 flex items-center justify-center">
          <Bot className="w-4 h-4 text-[var(--module-accent)]" />
        </div>
        <div className="flex-1 min-w-0">
          <h3 className="text-sm font-bold text-white">{t("installagent.title")}</h3>
          <p className="text-[10px] text-slate-500">{t("installagent.hint")}</p>
        </div>
        {/* 模型可自由选择：默认取第一个可用供应商，随时可换 */}
        <select
          value={providerId}
          onChange={(e) => {
            setProviderId(e.target.value);
            const p = config?.providers.find((x) => x.id === e.target.value);
            setModelId(p?.active_model_id || p?.models[0]?.id || "");
          }}
          className="glass-input px-2 h-7 text-[10px] cursor-pointer max-w-[140px]"
        >
          {usableProviders.length === 0 && <option value="">{t("installagent.noProvider")}</option>}
          {usableProviders.map((p) => (
            <option key={p.id} value={p.id}>{p.name}</option>
          ))}
        </select>
        <select
          value={modelId}
          onChange={(e) => setModelId(e.target.value)}
          className="glass-input px-2 h-7 text-[10px] cursor-pointer max-w-[160px]"
        >
          {models.map((m) => (
            <option key={m.id} value={m.id}>{m.id}</option>
          ))}
        </select>
      </div>

      <div
        ref={logRef}
        className="flex-1 min-h-0 overflow-y-auto rounded-xl border border-white/5 bg-slate-900/30 p-3 space-y-2"
      >
        {lines.length === 0 && (
          <div className="text-[11px] text-slate-500 py-6 text-center">
            <Sparkles className="w-4 h-4 mx-auto mb-2 text-slate-600" />
            {t("installagent.placeholder")}
          </div>
        )}
        {lines.map((line, i) => {
          if (line.role === "user") {
            return (
              <div key={i} className="flex justify-end">
                <div className="max-w-[80%] rounded-lg px-2.5 py-1.5 text-[11px] bg-[var(--module-accent)]/20 text-white break-all">
                  {line.text}
                </div>
              </div>
            );
          }
          if (line.role === "agent") {
            return (
              <div key={i} className="flex gap-2">
                <Bot className="w-3.5 h-3.5 flex-shrink-0 mt-0.5 text-[var(--module-accent)]" />
                <div className="text-[11px] text-slate-200 whitespace-pre-wrap break-all">{line.text}</div>
              </div>
            );
          }
          // 工具过程行
          const ok = line.ok;
          return (
            <div key={i} className="flex items-start gap-2 pl-1">
              {ok === undefined ? (
                <Wrench className="w-3 h-3 flex-shrink-0 mt-0.5 text-slate-500" />
              ) : ok ? (
                <CheckCircle className="w-3 h-3 flex-shrink-0 mt-0.5 text-emerald-400" />
              ) : (
                <AlertTriangle className="w-3 h-3 flex-shrink-0 mt-0.5 text-amber-400" />
              )}
              <div className="min-w-0 flex-1">
                {line.tool && (
                  <span className="text-[9px] text-slate-500 mr-1.5">
                    {TOOL_LABEL[line.tool] ?? line.tool}
                  </span>
                )}
                <span className="text-[10px] text-slate-400 whitespace-pre-wrap break-all">{line.text}</span>
              </div>
            </div>
          );
        })}
      </div>

      {error && (
        <div className="flex-shrink-0 text-[10px] text-rose-400 break-all">{error}</div>
      )}

      <div className="flex items-center gap-2 flex-shrink-0">
        <input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); void send(); } }}
          placeholder={t("installagent.inputPh")}
          className="flex-1 bg-slate-900 border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 focus:outline-none focus:border-[var(--module-accent)]"
        />
        <button
          onClick={() => void send()}
          disabled={busy || !input.trim()}
          className="px-3 py-2 rounded-lg text-[11px] bg-[var(--module-accent)] hover:opacity-90 text-white font-semibold cursor-pointer disabled:opacity-40 flex items-center gap-1"
        >
          <Send className="w-3 h-3" />
          {busy ? t("installagent.running") : t("installagent.send")}
        </button>
      </div>
    </div>
  );
}
