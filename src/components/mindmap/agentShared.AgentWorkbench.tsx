import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Brain, ChevronDown, GitFork, Loader2, RotateCcw, Search, Send, Sparkles, Square, Terminal } from "lucide-react";
import { ModelSelector } from "../shared/ai/ModelSelector";
import { ThinkingBlock } from "../shared/ThinkingBlock";
import { useEventBufferSnapshot } from "../../utils/eventBuffer";
import { matchProjectFiles } from "./atFiles";
import { INPUT_MAX_HEIGHT, clearAgentMessages, fmtDur, fmtNum, markAskAnswered, mmAiProgressBuffer, opCounts, patchAgentTurn, pushAgentMessage, startAgentTurn, type AgentWorkbenchMode, type AgentWorkbenchProps, type AiProgressEntry, useAgentMessages, useAnsweredAsks, viewLabel, wbBtn, wbSelect } from "./agentShared.state";
import { SessionBar } from "./agentShared.SessionBar";
import { PhaseStepper } from "./agentShared.PhaseStepper";
import { ActivityLine } from "./agentShared.ActivityLine";
import { AskForm } from "./agentShared.AskForm";
import { ResultCard } from "./agentShared.ResultCard";
let agentDraft = "";

export function AgentWorkbench(props: AgentWorkbenchProps) {
  const {
    mode, onModeChange, documents, targetDocumentId, onTargetDocumentChange,
    providers, providerId, modelId, onSelectModel,
    projectPath, onPickProject, aiDepth, onDepthChange, aiViews, onViewsChange,
    textTitle, onTextTitleChange, loading, onRun, onStop, onAnswer, result, runError,
    onShowReport, onNewSession, projectRoot, projectDir, projectFiles, onBindProjectDir,
    sessions, sessionId, onSelectSession, onRenameSession, onDeleteSession, onForkSession,
  } = props;
  const { t } = useTranslation();

  const messages = useAgentMessages();
  const entries = useEventBufferSnapshot(mmAiProgressBuffer);
  // 草稿从模块级存储恢复（见文件上方 agentDraft 的说明）
  const [input, setInput] = useState(() => agentDraft);
  /** 写草稿：组件 state 与模块级存储一起更新，避免两处不一致 */
  const setDraft = (value: string) => { agentDraft = value; setInput(value); };
  // @ 引用文件：输入 @ 后弹出绑定目录文件候选（↑↓ 选择、Enter/Tab 选中、Esc 取消）
  const [atQuery, setAtQuery] = useState<string | null>(null);
  const [atIndex, setAtIndex] = useState(0);
  const [cfgOpen, setCfgOpen] = useState(true);
  const [now, setNow] = useState(() => Date.now());
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const isFirstRun = messages.length === 0;
  // 已回答的询问（模块级，按事件时间戳去重）：回答后表单收起、不再重复弹出，
  // 且弹窗最小化再恢复（组件重挂载）后仍保持，避免已回答的询问重复弹表单。
  const answeredAsks = useAnsweredAsks();

  // 秒表：驱动用时跳动
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, []);

  // 首轮开始运行后自动收起配置区（用户可随时展开修改，供追问轮使用）
  useEffect(() => { if (loading) setCfgOpen(false); }, [loading]);

  // 正文开始流出 → 本轮状态从「思考中」切到「正在写」（每轮只切一次，避免每帧写气泡）
  const wroteRef = useRef(false);
  useEffect(() => {
    if (!loading) { wroteRef.current = false; return; }
    if (wroteRef.current) return;
    if (!entries.some((e) => e.step === "stream" && !e.done)) return;
    wroteRef.current = true;
    patchAgentTurn({ status: "writing" });
  }, [loading, entries]);

  // 运行结束（loading true→false）：把回执写进**本轮**那条助手气泡
  // （摘要 / 改动计数 / 截断 / 失败原因），而不是再推一条新气泡 ——
  // 一轮一轮看得见，才对得上聊天窗口的预期。
  const prevLoading = useRef(loading);
  useEffect(() => {
    const was = prevLoading.current;
    prevLoading.current = loading;
    if (was === loading || !was) return;
    // 本轮改了什么：ops 是后端提交给前端落图的写操作，进度缓冲每轮开始会清空，
    // 所以这里看到的就是这一轮的全部改动。
    const ops = entries.filter((e) => e.step === "agentOps").flatMap((e) => e.ops ?? []);
    const patch = ops.length ? opCounts(ops) : undefined;
    // 截断：模型输出写崩过、后端修复后才落下 —— 用户得知道这一轮的结果打过补丁
    const truncated = entries.some((e) => e.step === "repair");
    const cancelled = entries.some((e) => e.step === "cancel");
    // 取消优先：result 可能仍是上一轮的旧结果，不能落入成功分支
    if (cancelled) { patchAgentTurn({ status: "error", text: t("mindmap.aiCancelled"), patch, truncated }); return; }
    if (runError) { patchAgentTurn({ status: "error", error: `${t("agent.runFailed")}：${runError}`, patch, truncated }); return; }
    if (result) {
      const fails = result.failures.length
        ? t("mindmap.viewFailures", { count: result.failures.length, names: result.failures.map((f) => f.view).join("、") })
        : "";
      patchAgentTurn({
        status: "done",
        summary: t("agent.runDone", { count: result.documents.length, names: result.documents.map((d) => d.document.name).join("、"), failures: fails }),
        patch,
        truncated,
      });
    }
  }, [loading, result, runError, entries]);

  // 会话/进度变化 → 滚动到底部
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages.length, entries.length, loading]);

  // 输入框随内容自动增高（到上限后内部滚动）：
  // 写长指令（例如「分析这个文件并画一张图」）时不该挤在一行里靠滚动看。
  // 先置 auto 再取 scrollHeight，否则内容变少时高度不会回缩。
  useEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, INPUT_MAX_HEIGHT)}px`;
  }, [input]);

  // 用量统计（usage 事件累计）
  const totals = useMemo(() => {
    let req = 0, inT = 0, outT = 0, total = 0; let model = "";
    for (const p of entries) {
      if (p.step !== "usage") continue;
      req++; inT += p.prompt_tokens ?? 0; outT += p.completion_tokens ?? 0; total += p.total_tokens ?? inT + outT;
      if (p.model) model = p.model;
    }
    return { req, inT, outT, total, model };
  }, [entries]);
  const startedAt = entries.length ? (entries[0].at ?? now) : 0;
  const elapsed = entries.length ? now - startedAt : 0;
  const lastStream = useMemo(() => {
    for (let i = entries.length - 1; i >= 0; i--) {
      const e = entries[i];
      if (e.step === "stream") return e.done ? null : { length: e.length ?? 0, text: e.text ?? "" };
      if (e.step === "cancel" || e.step === "fail") return null;
    }
    return null;
  }, [entries]);
  // 流式思考帧（reasoning）：思考块收尾（done）或正文开始输出（后面出现 stream）后为 null
  const lastReasoning = useMemo(() => {
    for (let i = entries.length - 1; i >= 0; i--) {
      const e = entries[i];
      if (e.step === "reasoning") return e.done ? null : { length: e.length ?? 0, text: e.text ?? "" };
      if (e.step === "stream" || e.step === "cancel" || e.step === "fail") return null;
    }
    return null;
  }, [entries]);
  /** 思考全文（不因 done 而清空）：供 <ThinkingBlock> 展示。
   *  后端从这一版起每帧都带累积全文，所以展开后是完整推理链条，不再是尾部 200 字。 */
  const thinkingText = useMemo(() => {
    for (let i = entries.length - 1; i >= 0; i--) {
      const e = entries[i];
      if (e.step === "reasoning") return { text: e.text ?? "", done: !!e.done };
    }
    return null;
  }, [entries]);

  // 时间线：折叠重复的 stream / reasoning 进行中帧（与旧 AiProgressLog 一致）
  const timeline = useMemo(() => {
    const out: AiProgressEntry[] = [];
    for (const p of entries) {
      if ((p.step === "stream" || p.step === "reasoning") && !p.done) {
        const l = out[out.length - 1];
        if (l && l.step === p.step && !l.done) continue;
      }
      out.push(p);
    }
    return out;
  }, [entries]);

  // @ 候选：按光标前的 @ 关键词匹配绑定目录文件清单（排序与上限见 atFiles.ts）
  const atMatches = useMemo(
    () => (atQuery === null ? [] : matchProjectFiles(projectFiles ?? [], atQuery)),
    [atQuery, projectFiles],
  );

  const onInputChange = (value: string) => {
    setDraft(value);
    const caret = inputRef.current?.selectionStart ?? value.length;
    const m = value.slice(0, caret).match(/(?:^|\s)@([^\s@]*)$/);
    if (m && (projectFiles?.length ?? 0) > 0) { setAtQuery(m[1]); setAtIndex(0); } else { setAtQuery(null); }
  };

  const pickAt = (path: string) => {
    const caret = inputRef.current?.selectionStart ?? input.length;
    const before = input.slice(0, caret).replace(/@([^\s@]*)$/, `@${path} `);
    setDraft(before + input.slice(caret));
    setAtQuery(null);
    inputRef.current?.focus();
  };

  // 待回答的询问：时间线中最近一条 step=ask 且尚未回答、且运行仍在进行的事件。
  // AI 遇到歧义时后端推送 ask 事件并阻塞等待；用户填写表单提交后（onAnswer）
  // 该询问标记为已回答，表单收起，AI 基于回答继续生成（可能再次提问）。
  const pendingAsk = useMemo(() => {
    if (!loading) return null;
    for (let i = entries.length - 1; i >= 0; i--) {
      const e = entries[i];
      if (e.step !== "ask") continue;
      const at = e.at ?? 0;
      if (answeredAsks.has(at)) continue;
      const ask = (e as any).ask;
      if (!ask || typeof ask !== "object") continue;
      return { at, question: ask.question ?? "", fields: Array.isArray(ask.fields) ? ask.fields : [], round: e.round ?? 1, max: e.max ?? 3 };
    }
    return null;
  }, [entries, loading, answeredAsks]);

  const submitAsk = (answer: Record<string, unknown> | string) => {
    if (!pendingAsk) return;
    onAnswer(answer);
    markAskAnswered(pendingAsk.at);
    pushAgentMessage("user", t("agent.askAnswered"));
  };

  const sendDisabled = loading
    || (mode === "text" && isFirstRun && !input.trim())
    || (mode === "project" && isFirstRun && !projectPath)
    || (mode === "chat" && isFirstRun && !input.trim());

  const submit = () => {
    if (sendDisabled) return;
    const txt = input.trim();
    if (!isFirstRun && !txt) return;
    // 用户气泡：首轮无指令时按配置生成摘要文本
    const bubble = txt
      || (mode === "chat"
        ? t("agent.userContinue")
        : mode === "project"
          ? t("agent.userRunProject", { path: projectPath.split(/[\\/]/).pop() ?? projectPath, depth: aiDepth, views: aiViews.length ? aiViews.map((v) => viewLabel(t, v)).join("、") : t("agent.viewsAuto") })
          : textTitle || t("agent.userRunText"));
    startAgentTurn(bubble);
    setDraft("");
    onRun(txt);
  };

  const statusColor = loading ? "text-cyan-300" : runError ? "text-red-300" : "text-slate-400";

  return (
    <div className="flex h-full min-h-0 flex-col gap-2" style={{ height: "calc(88vh - 120px)", minHeight: 440 }}>
      {/* 会话条（chat 模式）：一个导图多个会话，可切换/新建/重命名/删除 */}
      {mode === "chat" && (
        <SessionBar
          sessions={sessions ?? []}
          sessionId={sessionId ?? ""}
          loading={loading}
          t={t}
          onSelect={(id) => onSelectSession?.(id)}
          onNew={() => onNewSession()}
          onRename={(id, title) => onRenameSession?.(id, title)}
          onDelete={(id) => onDeleteSession?.(id)}
        />
      )}
      {/* 状态条：阶段计划 + 实时统计 */}
      <div className="flex shrink-0 flex-wrap items-center gap-2 rounded-ctl border border-white/10 bg-slate-950/70 px-2 py-1.5">
        <span className="inline-flex shrink-0 items-center gap-1 text-micro font-semibold uppercase tracking-wide text-slate-400">
          <Brain className={`h-3 w-3 ${loading ? "animate-pulse text-cyan-300" : "text-slate-500"}`} />
          {t("agent.consoleTitle")}
        </span>
        <span className={`text-micro font-medium ${statusColor}`}>{loading ? t("agent.working") : runError ? t("agent.failed") : t("agent.ready")}</span>
        <div className="min-w-0 flex-1 overflow-x-auto">
          <PhaseStepper mode={mode} entries={entries} loading={loading} t={t} />
        </div>
        {entries.length > 0 && (
          <span className="flex shrink-0 items-center gap-1.5 font-mono text-[8px] tabular-nums text-slate-400">
            {totals.model && <span className="max-w-[110px] truncate rounded bg-white/[0.06] px-1.5 py-px text-slate-400" title={totals.model}>{totals.model}</span>}
            <span>{t("mindmap.aiStatRequests", { count: totals.req })}</span>
            <span className="text-emerald-300/90">↑{fmtNum(totals.inT)}</span>
            <span className="text-cyan-300/90">↓{fmtNum(totals.outT)}</span>
            <span className="text-slate-200">{fmtNum(totals.total)}</span>
            <span className="text-slate-500">{t("mindmap.aiStatElapsed", { s: fmtDur(elapsed) })}</span>
          </span>
        )}
        {loading && (
          <button type="button" onClick={onStop}
            className="inline-flex shrink-0 cursor-pointer items-center gap-1 rounded border border-red-400/40 bg-red-500/10 px-2 py-0.5 text-[8px] font-semibold text-red-300 transition hover:border-red-400/80 hover:bg-red-500/25 hover:text-red-200"
            title={t("mindmap.aiStopTitle")}>
            <Square className="h-2.5 w-2.5 fill-current" />{t("mindmap.aiStop")}
          </button>
        )}
      </div>

      {/* 会话区：用户指令 + agent 回执 + 实时工具调用 */}
      <div ref={scrollRef} className="min-h-0 flex-1 space-y-1.5 overflow-y-auto rounded-ctl border border-white/10 bg-slate-950/50 p-2">
        {isFirstRun && !loading && (
          <div className="flex items-center gap-2 rounded-ctl border border-dashed border-white/10 bg-white/[0.02] px-3 py-2.5">
            <Sparkles className="h-4 w-4 shrink-0 text-cyan-300/70" />
            <p className="text-tiny leading-4 text-slate-400">
              {mode === "project" ? t("agent.hintProject") : t("agent.hintText")}
            </p>
          </div>
        )}
        {messages.map((m, i) => (
          <div key={m.id} className={`group flex items-end gap-1 ${m.role === "user" ? "justify-end" : "justify-start"}`}>
            <div className={`max-w-[85%] rounded-ctl px-2.5 py-1.5 text-tiny leading-4 ${m.role === "user" ? "border border-cyan-400/25 bg-cyan-400/10 text-cyan-100" : "border border-white/10 bg-white/[0.04] text-slate-300"}`}>
              {m.text}
              {/* 本轮回执：**收在这条气泡里**（与数据库设计器同一处理）——
                  早先成功/失败都再推一条新消息，几轮下来「哪条是哪轮的」全靠猜。
                  进行中的那条（最后一条且仍在跑）显示思考中，其余按终态显示。 */}
              {m.role === "agent" && (m.status || m.summary || m.patch || m.error) && (
                <div className="mt-1 space-y-0.5 border-t border-white/10 pt-1">
                  {m.status && m.status !== "done" && (
                    <div className={`flex items-center gap-1 text-micro ${m.status === "error" ? "text-red-300" : "text-slate-400"}`}>
                      {m.status === "thinking" || (m.status === "writing" && loading && i === messages.length - 1)
                        ? <><Loader2 className="h-2.5 w-2.5 animate-spin" />{t(m.status === "writing" ? "agent.writing" : "agent.thinking")}</>
                        : null}
                    </div>
                  )}
                  {m.summary && <div className="text-slate-300">{m.summary}</div>}
                  {m.patch && (
                    <div className="text-micro text-slate-400">
                      {([
                        m.patch.added > 0 ? t("mindmap.aiTurnAdded", { count: m.patch.added }) : null,
                        m.patch.updated > 0 ? t("mindmap.aiTurnUpdated", { count: m.patch.updated }) : null,
                        m.patch.removed > 0 ? t("mindmap.aiTurnRemoved", { count: m.patch.removed }) : null,
                        m.patch.moved > 0 ? t("mindmap.aiTurnMoved", { count: m.patch.moved }) : null,
                      ].filter(Boolean).join("，") || t("mindmap.aiTurnNoop"))}
                    </div>
                  )}
                  {m.truncated && <div className="text-micro text-amber-400">{t("mindmap.aiTurnTruncated")}</div>}
                  {m.error && <div className="text-micro text-red-300">{m.error}</div>}
                </div>
              )}
              </div>
            {/* 从这条分叉：复制该条及之前的历史到新会话（原会话不动） */}
            {onForkSession && m.dbId && !loading && (
              <button type="button" onClick={() => onForkSession(m.dbId as string)}
                className="hidden shrink-0 cursor-pointer rounded border border-white/10 bg-white/[0.04] p-0.5 text-slate-500 transition hover:border-cyan-400/50 hover:text-cyan-300 group-hover:inline-flex"
                title={t("agent.forkHereTip")}>
                <GitFork className="h-2.5 w-2.5" />
              </button>
            )}
          </div>
        ))}
        {/* 思考过程 / 工具调用轨迹：运行中实时滚动；跑完仍保留，便于回看「它到底干了什么」 */}
        {(loading || timeline.length > 0) && (
          <div className="space-y-1 rounded-ctl border border-white/5 bg-black/20 p-2">
            {timeline.map((e, i) => <ActivityLine key={`${e.at}-${i}`} e={e} projectRoot={projectRoot} t={t} />)}
            {loading && (lastStream ? (
              <div className="flex items-start gap-1.5">
                <span className="mt-0.5 shrink-0"><Terminal className="h-3 w-3 animate-pulse text-emerald-300" /></span>
                <div className="min-w-0 flex-1">
                  <div className="text-micro text-emerald-200/90">{t("mindmap.aiStepStream")}</div>
                  <div className="mt-0.5 max-h-[72px] overflow-y-auto rounded border border-emerald-400/15 bg-emerald-400/[0.03] px-1.5 py-1 font-mono text-[8px] leading-3.5 whitespace-pre-wrap text-emerald-100/80">
                    {lastStream.text || "…"}
                    <span className="ml-0.5 inline-block h-2 w-1 animate-pulse bg-emerald-300 align-middle" />
                  </div>
                  <div className="mt-0.5 font-mono text-[7px] text-slate-500">{fmtNum(lastStream.length)} chars</div>
                </div>
              </div>
            ) : lastReasoning ? (
              // 推理模型「只思考不写字」阶段：共享的思考块（可折叠、全文可复制）。
              // 原来是这里自己写的一份预览框，只显示尾部 200 字。
              <div className="flex items-start gap-1.5">
                <span className="mt-0.5 shrink-0"><Brain className="h-3 w-3 animate-pulse text-fuchsia-300" /></span>
                <div className="min-w-0 flex-1">
                  <ThinkingBlock
                    text={thinkingText?.text ?? ""}
                    pending={!thinkingText?.done}
                    tail={lastReasoning.text}
                  />
                </div>
              </div>
            ) : (
              <div className="flex items-center gap-1.5 pl-1">
                <Loader2 className="h-3 w-3 animate-spin text-slate-400" />
                <span className="text-[8px] text-slate-500">{timeline.length ? t("agent.thinking") : t("mindmap.aiStepScan")}</span>
              </div>
            ))}
          </div>
        )}
        {/* AI 询问用户：后端推送 ask 事件后阻塞等待，渲染表单让用户填写；
            提交后 onAnswer 回填后端、AI 基于回答继续（可能再次提问）。 */}
        {pendingAsk && <AskForm ask={pendingAsk} onSubmit={submitAsk} t={t} />}
        {/* 完成后的结果卡（新一轮开始后自动隐藏：loading 时不渲染） */}
        {!loading && result && <ResultCard result={result} t={t} onShowReport={onShowReport} />}
      </div>

      {/* 任务配置（可折叠；首轮默认展开，运行开始后收起，追问前可展开改参数） */}
      <div className="shrink-0 rounded-ctl border border-white/10 bg-slate-950/70">
        <button type="button" onClick={() => setCfgOpen(!cfgOpen)}
          className="flex w-full cursor-pointer items-center gap-1.5 px-2.5 py-1.5 text-left">
          <ChevronDown className={`h-3 w-3 shrink-0 text-slate-500 transition-transform ${cfgOpen ? "" : "-rotate-90"}`} />
          <span className="text-micro font-semibold uppercase tracking-wide text-slate-400">{t("agent.configTitle")}</span>
          <span className="ml-auto text-[8px] text-slate-600">
            {mode === "chat"
              ? (projectDir || t("mindmap.agentNoDir"))
              : mode === "project"
                ? (projectPath || t("mindmap.pickDir"))
                : (textTitle || t("agent.textUntitled"))}
          </span>
        </button>
        {cfgOpen && (
          <div className="space-y-2 border-t border-white/5 p-2.5">
            <div className="grid grid-cols-2 gap-2">
              <select className={wbSelect} value={mode} onChange={(e) => onModeChange(e.target.value as AgentWorkbenchMode)} disabled={loading}>
                <option value="chat">{t("mindmap.aiTaskChat")}</option>
                <option value="text">{t("mindmap.aiTaskText")}</option>
                <option value="project">{t("mindmap.aiTaskProject")}</option>
              </select>
              <select className={wbSelect} value={targetDocumentId} onChange={(e) => onTargetDocumentChange(e.target.value)} disabled={loading}>
                <option value="">{t("mindmap.aiTargetDocument")}</option>
                {documents.map((d) => <option key={d.id} value={d.id}>{d.name}</option>)}
              </select>
            </div>
            <ModelSelector
              providers={providers}
              providerId={providerId}
              modelId={modelId}
              onSelect={(pid, mid) => onSelectModel(pid, mid)}
              disabled={loading}
              compact
              placeholder={t("mindmap.pickProvider")}
              emptyText={t("mindmap.pickProvider")}
              title={t("mindmap.pickModel")}
            />
            {mode === "chat" ? (
              // 绑定项目目录：@ 引用与对话上下文固定来自它（一个导图至多绑定一个）
              <div className="flex items-center gap-2">
                <button type="button" className={`${wbBtn} max-w-[55%]`} onClick={onBindProjectDir} disabled={!onBindProjectDir}>
                  <Search className="h-3 w-3" />
                  <span className="truncate">{projectDir ? projectDir.split(/[\\/]/).pop() : t("mindmap.agentBindDir")}</span>
                </button>
                <span className="min-w-0 flex-1 truncate font-mono text-[8px] text-slate-600" title={projectDir ?? ""}>{projectDir ?? t("mindmap.agentNoDir")}</span>
              </div>
            ) : mode === "project" ? (
              <>
                <div className="flex items-center gap-2">
                  <button type="button" className={`${wbBtn} max-w-[55%]`} onClick={onPickProject}>
                    <Search className="h-3 w-3" />
                    <span className="truncate">{projectPath ? projectPath.split(/[\\/]/).pop() : t("mindmap.pickDir")}</span>
                  </button>
                  <span className="min-w-0 flex-1 truncate font-mono text-[8px] text-slate-600" title={projectPath}>{projectPath}</span>
                </div>
                <div>
                  <div className="mb-0.5 flex items-center justify-between">
                    <span className="text-micro font-semibold uppercase tracking-wide text-slate-500">{t("mindmap.aiDepthTitle")}</span>
                    <span className="text-micro font-semibold text-cyan-300">{t(`mindmap.aiDepth${aiDepth}`)}</span>
                  </div>
                  <input type="range" min={1} max={5} step={1} value={aiDepth} onChange={(e) => onDepthChange(Number(e.target.value))} className="w-full accent-cyan-400" />
                  <p className="mt-0.5 text-[8px] leading-3.5 text-slate-600">{t(`mindmap.aiDepthDesc${aiDepth}`)}</p>
                </div>
                <div>
                  <div className="mb-0.5 text-micro font-semibold uppercase tracking-wide text-slate-500">{t("mindmap.aiViewsTitle")}</div>
                  <div className="flex flex-wrap gap-1">
                    {["architecture", "workflow", "dataflow"].map((v) => {
                      const on = aiViews.includes(v);
                      return (
                        <button key={v} type="button"
                          className={`cursor-pointer rounded-md border px-1.5 py-0.5 text-micro transition ${on ? "border-cyan-400/60 bg-cyan-400/15 text-white" : "border-white/10 bg-slate-950/60 text-slate-400 hover:text-slate-200"}`}
                          onClick={() => onViewsChange(on ? aiViews.filter((x) => x !== v) : [...aiViews, v])}>
                          {viewLabel(t, v)}
                        </button>
                      );
                    })}
                  </div>
                  <p className="mt-0.5 text-[8px] leading-3.5 text-slate-600">{aiViews.length ? t("mindmap.aiViewsPicked", { count: aiViews.length }) : t("mindmap.aiViewsAuto")}</p>
                </div>
              </>
            ) : (
              <div>
                <label className="mb-0.5 block text-micro font-semibold uppercase tracking-wide text-slate-500">{t("mindmap.reqTitlePh")}</label>
                <input className={`w-full rounded-md border border-white/10 bg-slate-950/70 px-2 py-1.5 text-tiny text-slate-200 outline-none focus:border-cyan-400/60`}
                  value={textTitle} onChange={(e) => onTextTitleChange(e.target.value)} placeholder={t("mindmap.reqTitlePh")} />
              </div>
            )}
          </div>
        )}
      </div>

      {/* 指令输入区：首轮下达任务；完成后继续追问（结果增量追加到画布） */}
      <div className="relative flex shrink-0 items-end gap-1.5">
        {/* @ 引用文件候选（IDE 式）：输入 @ 触发，↑↓/Enter/Tab/Esc 操作 */}
        {atQuery !== null && atMatches.length > 0 && (
          <div className="absolute bottom-full left-0 right-0 z-20 mb-1 max-h-40 overflow-y-auto rounded-ctl border border-white/10 bg-slate-900/95 py-1 shadow-xl">
            {atMatches.map((f, i) => (
              <button key={f} type="button" onMouseDown={(e) => { e.preventDefault(); pickAt(f); }}
                className={`block w-full cursor-pointer truncate px-2.5 py-1 text-left font-mono text-tiny transition ${i === atIndex ? "bg-[var(--module-accent)]/25 text-white" : "text-slate-300 hover:bg-white/5"}`}>
                {f}
              </button>
            ))}
          </div>
        )}
        <textarea
          ref={inputRef}
          value={input}
          onChange={(e) => onInputChange(e.target.value)}
          onKeyDown={(e) => {
            if (atQuery !== null && atMatches.length > 0) {
              if (e.key === "ArrowDown") { e.preventDefault(); setAtIndex((i) => (i + 1) % atMatches.length); return; }
              if (e.key === "ArrowUp") { e.preventDefault(); setAtIndex((i) => (i - 1 + atMatches.length) % atMatches.length); return; }
              if (e.key === "Enter" || e.key === "Tab") { e.preventDefault(); pickAt(atMatches[atIndex]); return; }
              if (e.key === "Escape") { e.preventDefault(); setAtQuery(null); return; }
            }
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); submit(); }
          }}
          rows={isFirstRun && mode === "text" ? 3 : 1}
          placeholder={
            isFirstRun
              ? (mode === "chat" ? t("agent.inputPhChat") : mode === "project" ? t("agent.inputPhProject") : t("agent.inputPhText"))
              : t("agent.inputPhFollowUp")
          }
          disabled={loading}
          className="min-h-[34px] flex-1 resize-none overflow-y-auto rounded-ctl border border-white/10 bg-slate-950/70 px-2.5 py-2 text-caption leading-relaxed text-slate-200 outline-none focus:border-cyan-400/60 disabled:opacity-50"
        />
        {!loading && (
          <button type="button" onClick={submit} disabled={sendDisabled}
            className="inline-flex h-[34px] shrink-0 cursor-pointer items-center gap-1 rounded-ctl px-3 text-tiny font-semibold text-white transition disabled:cursor-default disabled:opacity-40"
            style={{ backgroundColor: "var(--module-accent, #22d3ee)" }}>
            <Send className="h-3 w-3" />
            {isFirstRun ? t("agent.runFirst") : t("agent.followUp")}
          </button>
        )}
        {/* chat 模式的「新会话」走会话条上的 +（后端真建新会话）；非 chat 模式没有会话概念，仍就地清屏 */}
        {mode !== "chat" && messages.length > 0 && !loading && (
          <button type="button"
            onClick={() => { clearAgentMessages(); onNewSession(); }}
            className="inline-flex h-[34px] shrink-0 cursor-pointer items-center gap-1 rounded-ctl border border-white/10 bg-white/[0.04] px-2 text-micro text-slate-400 transition hover:text-slate-200"
            title={t("agent.newSessionTip")}>
            <RotateCcw className="h-3 w-3" />
            {t("agent.newSession")}
          </button>
        )}
      </div>
    </div>
  );
}
