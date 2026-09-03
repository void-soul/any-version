import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  CheckCircle2,
  Globe2,
  Loader2,
  MousePointer2,
  Play,
  Square,
  Terminal,
  UserRound,
  AlertTriangle,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { createEventBuffer, useEventBufferSnapshot } from "../utils/eventBuffer";

type AiProvider = {
  id: string;
  name: string;
  api_key: string;
  openai_url: string;
  models: { id: string; name: string }[];
  active_model_id: string | null;
};

type AiConfig = { providers: AiProvider[] };

type PageEvent = {
  type: "log" | "activity" | "step" | "status" | "result" | "error" | "ask_user";
  payload: Record<string, unknown>;
};

const pageEvents = createEventBuffer<PageEvent>("page-agent-event", { limit: 1200 });

function textOf(value: unknown): string {
  if (typeof value === "string") return value;
  if (value == null) return "";
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

export default function PagePanel() {
  const { t } = useTranslation();
  const events = useEventBufferSnapshot(pageEvents);
  const [providers, setProviders] = useState<AiProvider[]>([]);
  const [providerId, setProviderId] = useState("");
  const [modelId, setModelId] = useState("");
  const [url, setUrl] = useState("https://www.zhihu.com");
  const [task, setTask] = useState("");
  const [maxSteps, setMaxSteps] = useState(25);
  const [allowScript, setAllowScript] = useState(false);
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState("");
  const [error, setError] = useState("");
  const [question, setQuestion] = useState("");
  const [answer, setAnswer] = useState("");

  useEffect(() => {
    invoke<AiConfig>("get_ai_config")
      .then((config) => {
        const available = (config.providers ?? []).filter(
          (provider) => provider.api_key && provider.openai_url && provider.models?.length,
        );
        setProviders(available);
        const provider = available[0];
        if (!provider) return;
        setProviderId(provider.id);
        setModelId(provider.active_model_id && provider.models.some((m) => m.id === provider.active_model_id)
          ? provider.active_model_id
          : provider.models[0]?.id ?? "");
      })
      .catch((e) => setError(String(e)));
  }, []);

  const selectedProvider = providers.find((provider) => provider.id === providerId);
  const models = selectedProvider?.models ?? [];
  const status = useMemo(() => {
    for (let i = events.length - 1; i >= 0; i -= 1) {
      if (events[i].type === "status") return textOf(events[i].payload.status);
    }
    return running ? "running" : "idle";
  }, [events, running]);

  const visibleLogs = events.filter((event) => event.type === "log" || event.type === "activity" || event.type === "step");

  useEffect(() => {
    // 只有最后一条控制事件仍是 ask_user 时才显示问题；用户回答后，
    // 后端会发送 status=running，后续日志不应再次唤醒旧问题。
    const latestControl = [...events].reverse().find(
      (event) => event.type === "ask_user" || event.type === "status",
    );
    if (latestControl?.type === "ask_user") {
      setQuestion(textOf(latestControl.payload.question));
    } else if (latestControl?.type === "status" && textOf(latestControl.payload.status) !== "waiting_user") {
      setQuestion("");
    }
  }, [events]);

  const changeProvider = (id: string) => {
    setProviderId(id);
    const provider = providers.find((item) => item.id === id);
    setModelId(provider?.active_model_id && provider.models.some((m) => m.id === provider.active_model_id)
      ? provider.active_model_id
      : provider?.models[0]?.id ?? "");
  };

  const run = async () => {
    if (running || !url.trim() || !task.trim()) return;
    setRunning(true);
    setResult("");
    setError("");
    setQuestion("");
    setAnswer("");
    pageEvents.clear();
    try {
      const response = await invoke<{ success: boolean; data: string }>("page_agent_run", {
        request: {
          url: url.trim(),
          task: task.trim(),
          providerId: providerId || null,
          modelId: modelId || null,
          allowScript,
          maxSteps,
        },
      });
      setResult(response.data);
      if (!response.success) setError(t("page.taskFailed"));
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  };

  const stop = async () => {
    try {
      await invoke("page_agent_stop");
    } catch (e) {
      setError(String(e));
    }
  };

  const answerQuestion = async () => {
    try {
      await invoke("page_agent_answer", { answer });
      setQuestion("");
      setAnswer("");
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="h-full min-h-0 overflow-y-auto px-6 py-5 text-slate-200">
      <div className="mx-auto flex max-w-[1180px] flex-col gap-4">
        <div className="flex items-center gap-3 border-b border-white/10 pb-4">
          <div className="flex h-10 w-10 items-center justify-center rounded-xl border border-[var(--module-accent)]/30 bg-[var(--module-accent)]/10">
            <Globe2 className="h-5 w-5 text-[var(--module-accent)]" />
          </div>
          <div>
            <h2 className="text-sm font-bold text-white">{t("page.title")}</h2>
            <p className="text-[11px] text-slate-500">{t("page.subtitle")}</p>
          </div>
          <div className="ml-auto flex items-center gap-2 text-[10px] text-slate-400">
            <span className={`h-2 w-2 rounded-full ${running ? "animate-pulse bg-emerald-400" : "bg-slate-600"}`} />
            {running ? t("page.running") : t("page.idle")}
          </div>
        </div>

        <div className="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]">
          <section className="flex flex-col gap-3 rounded-xl border border-white/10 bg-white/[0.03] p-4">
            <div className="flex items-center gap-2 text-xs font-semibold text-white">
              <MousePointer2 className="h-4 w-4 text-[var(--module-accent)]" />
              {t("page.taskSettings")}
            </div>
            <label className="text-[10px] text-slate-400">
              {t("page.url")}
              <input value={url} onChange={(e) => setUrl(e.target.value)} disabled={running} className="mt-1 w-full rounded-lg border border-white/10 bg-black/20 px-3 py-2 text-xs text-slate-100 outline-none focus:border-[var(--module-accent)]/60 disabled:opacity-50" placeholder="https://example.com" />
            </label>
            <label className="text-[10px] text-slate-400">
              {t("page.task")}
              <textarea value={task} onChange={(e) => setTask(e.target.value)} disabled={running} className="mt-1 min-h-[130px] w-full resize-y rounded-lg border border-white/10 bg-black/20 px-3 py-2 text-xs leading-relaxed text-slate-100 outline-none focus:border-[var(--module-accent)]/60 disabled:opacity-50" placeholder={t("page.taskPlaceholder")} />
            </label>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <label className="text-[10px] text-slate-400">
                {t("page.provider")}
                <select value={providerId} onChange={(e) => changeProvider(e.target.value)} disabled={running || providers.length === 0} className="mt-1 w-full rounded-lg border border-white/10 bg-black/20 px-2 py-2 text-xs text-slate-100 outline-none disabled:opacity-50">
                  {providers.length === 0 && <option value="">{t("page.noProvider")}</option>}
                  {providers.map((provider) => <option key={provider.id} value={provider.id}>{provider.name}</option>)}
                </select>
              </label>
              <label className="text-[10px] text-slate-400">
                {t("page.model")}
                <select value={modelId} onChange={(e) => setModelId(e.target.value)} disabled={running || !selectedProvider} className="mt-1 w-full rounded-lg border border-white/10 bg-black/20 px-2 py-2 text-xs text-slate-100 outline-none disabled:opacity-50">
                  {models.map((model) => <option key={model.id} value={model.id}>{model.name || model.id}</option>)}
                </select>
              </label>
            </div>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <label className="text-[10px] text-slate-400">
                {t("page.maxSteps")}
                <input type="number" min={1} max={100} value={maxSteps} onChange={(e) => setMaxSteps(Math.max(1, Math.min(100, Number(e.target.value) || 1)))} disabled={running} className="mt-1 w-full rounded-lg border border-white/10 bg-black/20 px-3 py-2 text-xs text-slate-100 outline-none disabled:opacity-50" />
              </label>
              <label className="flex items-end gap-2 pb-2 text-[10px] text-slate-400">
                <input type="checkbox" checked={allowScript} onChange={(e) => setAllowScript(e.target.checked)} disabled={running} className="accent-[var(--module-accent)]" />
                {t("page.allowScript")}
              </label>
            </div>
            <div className="rounded-lg border border-sky-500/20 bg-sky-500/5 p-2.5 text-[10px] leading-relaxed text-sky-200/80">
              <UserRound className="mr-1 inline h-3.5 w-3.5" />{t("page.loginHint")}
            </div>
            <div className="flex items-center gap-2 pt-1">
              {!running ? (
                <button onClick={run} disabled={!url.trim() || !task.trim() || !selectedProvider} className="flex items-center gap-1.5 rounded-lg bg-[var(--module-accent)] px-4 py-2 text-xs font-semibold text-white transition hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40">
                  <Play className="h-3.5 w-3.5" />{t("page.run")}
                </button>
              ) : (
                <button onClick={stop} className="flex items-center gap-1.5 rounded-lg bg-red-600 px-4 py-2 text-xs font-semibold text-white transition hover:bg-red-500">
                  <Square className="h-3.5 w-3.5" />{t("page.stop")}
                </button>
              )}
              <span className="text-[10px] text-slate-500">{status}</span>
            </div>
            {question && running && (
              <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 p-3 text-[11px] text-amber-100">
                <div className="mb-2 flex items-center gap-1.5 font-semibold"><UserRound className="h-3.5 w-3.5" />{t("page.userActionRequired")}</div>
                <div className="mb-2 whitespace-pre-wrap leading-relaxed">{question}</div>
                <div className="flex gap-2">
                  <input value={answer} onChange={(e) => setAnswer(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") void answerQuestion(); }} className="min-w-0 flex-1 rounded-md border border-white/10 bg-black/20 px-2 py-1.5 text-[11px] text-white outline-none focus:border-amber-400/60" placeholder={t("page.answerPlaceholder")} />
                  <button onClick={() => void answerQuestion()} className="rounded-md bg-amber-500 px-3 py-1.5 text-[10px] font-semibold text-slate-950 hover:bg-amber-400">{t("page.answer")}</button>
                </div>
              </div>
            )}
            {error && <div className="flex items-start gap-1.5 break-words text-[11px] text-red-400"><AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />{error}</div>}
          </section>

          <section className="flex min-h-[430px] flex-col rounded-xl border border-white/10 bg-black/20 p-4">
            <div className="mb-2 flex items-center justify-between">
              <div className="flex items-center gap-2 text-xs font-semibold text-white"><Terminal className="h-4 w-4 text-[var(--module-accent)]" />{t("page.activity")}</div>
              <span className="font-mono text-[10px] text-slate-600">{visibleLogs.length}</span>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto rounded-lg border border-white/5 bg-black/30 p-3 font-mono text-[10px] leading-relaxed">
              {visibleLogs.length === 0 ? <div className="flex h-full min-h-[330px] items-center justify-center text-center text-slate-600">{t("page.activityPlaceholder")}</div> : visibleLogs.map((event, index) => {
                const line = event.type === "activity" ? textOf(event.payload.activity) : event.type === "step" ? textOf(event.payload.event) : textOf(event.payload.line);
                return <div key={`${index}-${line}`} className="mb-1 break-words text-slate-300"><span className="mr-2 text-slate-600">{String(index + 1).padStart(3, "0")}</span>{line}</div>;
              })}
            </div>
            {result && <div className="mt-3 rounded-lg border border-emerald-500/20 bg-emerald-500/5 p-3 text-xs leading-relaxed text-emerald-100"><div className="mb-1 flex items-center gap-1.5 font-semibold"><CheckCircle2 className="h-3.5 w-3.5" />{t("page.result")}</div><div className="whitespace-pre-wrap break-words">{result}</div></div>}
          </section>
        </div>
        <div className="flex items-center gap-2 text-[10px] text-slate-600"><Loader2 className="h-3 w-3" />{t("page.browserNote")}</div>
      </div>
    </div>
  );
}
