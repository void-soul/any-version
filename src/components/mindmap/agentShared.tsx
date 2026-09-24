// ════════════ 思维导图 AI Agent 共享设施 ════════════
// 从 MindmapPanel 抽出的 AI 导入公共层：
// - mm-ai-progress 事件缓冲（模块级单例，面板切走再切回不丢进度）
// - 进度事件的渲染辅助（图标/文案/用量格式化）
// - AgentWorkbench：AI 项目 / AI 文档共用的「智能体工作台」——
//   会话式多轮交互 + 阶段计划可视 + 工具调用透明（读取的文件可点击）
//   + 流式反馈 + 用量统计 + 停止，界面反馈对齐 AI 开发工具。

import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle, Ban, Brain, Check, ChevronDown, Coins, File, LayoutGrid, Loader2,
  MessageCircle, RotateCcw, Search, Send, Sparkles, Square, Terminal,
} from "lucide-react";
import type { AiConfig } from "../ai/types";
import type { AiImportResult } from "./types";
import { createEventBuffer, useEventBufferSnapshot, type EventBuffer } from "../../utils/eventBuffer";

// ─── 打开文件（编辑器优先） ───

/** 在全局配置的编辑器中打开项目内文件（证据文件 / AI 探索读取的文件共用）。
 *  未配置编辑器时由后端回退为资源管理器定位（launcher_reveal_file）。
 *  src 为绝对路径（盘符/斜杠开头）时直接使用，否则按项目相对路径拼接。 */
export function openSourceFile(projectRoot: string, src: string) {
  const isAbs = /^[a-zA-Z]:[\\/]/.test(src) || src.startsWith("/") || src.startsWith("\\\\");
  const p = isAbs ? src : `${projectRoot.replace(/[\\/]+$/, "").replace(/\\/g, "/")}/${src}`;
  void invoke("launcher_open_with_editor", { path: p }).catch((e) => console.error("打开文件失败:", p, e));
}

// ─── 事件缓冲（模块级单例） ───

/** 后端推送的进度事件（mm-ai-progress）：step 标记阶段，其余字段按步骤类型取用 */
export interface AiProgressEntry {
  step: "scan" | "explore" | "read" | "route" | "view" | "view_done" | "repair" | "fail" | "usage" | "stream" | "ask" | "cancel" | string;
  // AI 询问用户（step=ask）：表单载荷 { question, fields:[{key,label,type,options,default}] }
  ask?: { question?: string; fields?: { key: string; label?: string; type?: string; options?: string[]; default?: string }[] };
  // 视图落库完成（step=view_done）：文档 id，前端据此增量拉取渲染
  // （后端 emit 原样发 doc_id，事件缓冲不做 key 转换，故两种命名都要兼容）
  docId?: string;
  doc_id?: string;
  index?: number;
  round?: number;
  total?: number;
  reason?: string;
  done?: boolean;
  files?: string[];
  views?: string[];
  view?: string;
  count?: number;
  rounds?: number;
  detail?: string;
  // token 用量（step=usage）
  prompt_tokens?: number;
  completion_tokens?: number;
  total_tokens?: number;
  model?: string;
  // 流式输出（step=stream）：累计字符数与末尾预览
  length?: number;
  text?: string;
  // 断流重连（step=reconnect）：第几次续写/重发、上限、已收到的字符数、是否携带断点续写；
  // send=true 表示「连接阶段」重试（请求未成功发出/未拿到响应头），否则为断点续写
  attempt?: number;
  max?: number;
  resume?: boolean;
  send?: boolean;
  // Agent 写操作（step=agentOps）：待前端应用/确认的 ops 载荷（分级确认见 MindmapPanel）
  runId?: string;
  needConfirm?: boolean;
  ops?: import("./types").AgentOp[];
  // 无效点单回执（step=reject）：AI 本轮请求了目录结构里不存在的路径
  paths?: string[];
  // 前端收到时打的时间戳（ms）
  at?: number;
}

/** AI 导入进度事件缓冲（模块级，App 生命周期内常驻）：思维导图面板切走/隐藏后，
 *  组件内的 mm-ai-progress 订阅随 Effects 销毁，后端即发即弃的事件会丢；
 *  由缓冲在模块作用域统一订阅并保存载荷，面板重新可见时完整重放（长任务不断流）。 */
export const mmAiProgressBuffer: EventBuffer<AiProgressEntry> = createEventBuffer<AiProgressEntry>(
  "mm-ai-progress",
  {
    transform: (p) => ({ ...(p as Omit<AiProgressEntry, "at">), at: Date.now() }),
    limit: 500,
  }
);

// ─── 渲染辅助 ───

export const STEP_ICONS: Record<string, React.ReactNode> = {
  scan: <Search className="h-3 w-3 text-cyan-300" />,
  explore: <Brain className="h-3 w-3 text-violet-300" />,
  read: <File className="h-3 w-3 text-emerald-300" />,
  route: <LayoutGrid className="h-3 w-3 text-amber-300" />,
  reconnect: <RotateCcw className="h-3 w-3 text-orange-300" />,
  view: <Sparkles className="h-3 w-3 text-cyan-300" />,
  view_done: <Sparkles className="h-3 w-3 text-emerald-300" />,
  repair: <RotateCcw className="h-3 w-3 text-amber-300" />,
  reject: <AlertTriangle className="h-3 w-3 text-yellow-300" />,
  fail: <AlertTriangle className="h-3 w-3 text-red-300" />,
  usage: <Coins className="h-3 w-3 text-emerald-300" />,
  stream: <Terminal className="h-3 w-3 text-emerald-300" />,
  ask: <MessageCircle className="h-3 w-3 text-amber-300" />,
  cancel: <Ban className="h-3 w-3 text-red-300" />,
};

export const fmtNum = (n: number) => n.toLocaleString();
export const fmtDur = (ms: number) => {
  if (ms < 1000) return `${ms}ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)}s`;
  return `${Math.floor(s / 60)}m ${Math.round(s % 60)}s`;
};
export const fmtClock = (ts: number) => {
  const d = new Date(ts);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
};

export const VIEW_LABEL_KEYS: Record<string, string> = {
  architecture: "mindmap.viewArchitecture",
  workflow: "mindmap.viewWorkflow",
  dataflow: "mindmap.viewDataflow",
  sequence: "mindmap.viewSequence",
  lifecycle: "mindmap.viewLifecycle",
};

export const viewLabel = (t: (k: string, o?: any) => string, v: string) => t(VIEW_LABEL_KEYS[v] ?? v);

/** 把结构化进度事件转为一行可读文本（i18n） */
export function progressText(e: AiProgressEntry, t: (k: string, o?: any) => string): string {
  switch (e.step) {
    case "scan":
      return e.done ? t("mindmap.aiStepScanDone") : t("mindmap.aiStepScan");
    case "explore":
      return t("mindmap.aiStepExplore", {
        n: e.round,
        total: e.total,
        reason: e.reason || "",
        done: e.done ? ` — ${t("mindmap.aiExploreDone")}` : "",
      });
    case "read":
      return t("mindmap.aiStepReading", { files: (e.files ?? []).join(", ") });
    case "route":
      return e.views
        ? t("mindmap.aiStepViews", { views: e.views.map(v => viewLabel(t, v)).join(", ") })
        : t("mindmap.aiStepRouting");
    case "view":
      return t("mindmap.aiStepView", { view: viewLabel(t, e.view ?? "") });
    case "view_done":
      return t("mindmap.aiStepViewDone", { view: viewLabel(t, e.view ?? "") });
    case "reconnect":
      return e.send
        ? t("mindmap.aiStepReconnectSend", { n: e.attempt ?? 0, max: e.max ?? 0, msg: e.detail ?? "" })
        : t("mindmap.aiStepReconnectResume", { n: e.attempt ?? 0, max: e.max ?? 0, chars: e.length ?? 0 });
    case "repair":
      return t("mindmap.aiStepRepair", { count: e.count ?? 0, rounds: e.rounds ?? 0 });
    case "reject":
      return t("mindmap.aiStepReject", { paths: (e.paths ?? []).join(", ") });
    case "fail":
      return t("mindmap.aiStepFail", { msg: e.detail ?? "" });
    case "usage":
      return t("mindmap.aiStepUsage", { model: e.model ?? "" });
    case "stream":
      return e.done ? t("mindmap.aiStepStreamDone", { n: e.length ?? 0 }) : t("mindmap.aiStepStream");
    case "ask":
      return t("agent.askStep", { n: e.round ?? 1, max: e.max ?? 3 });
    case "cancel":
      return t("mindmap.aiCancelled");
    case "agentOps":
      return t("mindmap.agentOpsStep", { n: e.ops?.length ?? 0 });
    default:
      return e.detail ?? e.step;
  }
}

// ════════════ 会话存储（模块级：最小化/关闭重开不丢对话） ════════════

export interface AgentMessage {
  id: number;
  role: "user" | "agent";
  text: string;
  ts: number;
}

let agentMsgs: readonly AgentMessage[] = [];
let agentMsgSeq = 0;
const agentMsgSubs = new Set<() => void>();

export function pushAgentMessage(role: AgentMessage["role"], text: string) {
  agentMsgs = [...agentMsgs, { id: ++agentMsgSeq, role, text, ts: Date.now() }];
  agentMsgSubs.forEach((fn) => fn());
}

export function clearAgentMessages() {
  agentMsgs = [];
  agentMsgSubs.forEach((fn) => fn());
}

function subscribeAgentMsgs(fn: () => void) {
  agentMsgSubs.add(fn);
  return () => { agentMsgSubs.delete(fn); };
}

/** 组件内消费会话消息（引用稳定，未变化不重渲染） */
export function useAgentMessages(): readonly AgentMessage[] {
  return useSyncExternalStore(subscribeAgentMsgs, () => agentMsgs, () => agentMsgs);
}

// ════════════ 已回答询问标记（模块级：最小化/恢复重挂载不丢） ════════════
// 记录已提交回答的询问事件时间戳。AgentWorkbench 卸载/重挂载（弹窗最小化再恢复）
// 后，已回答的询问不再重复弹表单——此时后端询问通道已消费，再次提交只会报错。
let answeredAskIds: ReadonlySet<number> = new Set();
const answeredAskSubs = new Set<() => void>();

export function markAskAnswered(at: number) {
  if (answeredAskIds.has(at)) return;
  answeredAskIds = new Set(answeredAskIds).add(at);
  answeredAskSubs.forEach((fn) => fn());
}

export function clearAnsweredAsks() {
  answeredAskIds = new Set();
  answeredAskSubs.forEach((fn) => fn());
}

function subscribeAnsweredAsks(fn: () => void) {
  answeredAskSubs.add(fn);
  return () => { answeredAskSubs.delete(fn); };
}

export function useAnsweredAsks(): ReadonlySet<number> {
  return useSyncExternalStore(subscribeAnsweredAsks, () => answeredAskIds, () => answeredAskIds);
}

// ════════════ AgentWorkbench：AI 项目 / AI 文档共用智能体工作台 ════════════
//
// 对齐 AI 开发工具的 Agent 体验：
// - 阶段计划可视（扫描→探索→规划→生成→校验→绘制，实时状态：待办/进行/完成/失败）
// - 工具调用透明（每轮探索的理由、读取的文件清单，文件可点击在编辑器打开）
// - 流式反馈（AI 实时输出预览）+ 用量统计（请求/输入/输出/总 token/用时）
// - 多轮会话（首轮下达任务，完成后可继续追问，结果增量追加到当前画布）
// - 停止 / 校验报告 / 最小化后台运行

export type AgentWorkbenchMode = "project" | "text" | "chat";

export interface AgentWorkbenchProps {
  mode: AgentWorkbenchMode;
  onModeChange: (mode: AgentWorkbenchMode) => void;
  /** chat 模式：文档绑定的项目目录（AI 上下文与 @ 引用固定来自它） */
  projectDir?: string | null;
  /** chat 模式：绑定目录下的文件清单（@ 候选） */
  projectFiles?: string[];
  /** chat 模式：打开目录选择器并绑定到当前文档 */
  onBindProjectDir?: () => void;
  documents: { id: string; name: string; sourceType: string }[];
  targetDocumentId: string;
  onTargetDocumentChange: (id: string) => void;
  providers: AiConfig["providers"];
  providerId: string;
  modelId: string;
  onProviderChange: (pid: string) => void;
  onModelChange: (mid: string) => void;
  // 项目模式
  projectPath: string;
  onPickProject: () => void;
  aiDepth: number;
  onDepthChange: (d: number) => void;
  aiViews: string[];
  onViewsChange: (views: string[]) => void;
  // 文本模式
  textTitle: string;
  onTextTitleChange: (v: string) => void;
  // 运行
  loading: boolean;
  onRun: (instruction: string) => void;
  onStop: () => void;
  /** AI 询问用户时提交表单答案（后端据此回填提示词继续生成） */
  onAnswer: (answer: Record<string, unknown> | string) => void;
  /** 最近一次运行结果（完成时展示结果卡；新一轮开始后隐藏） */
  result: AiImportResult | null;
  /** 最近一次运行的错误文本（用户主动停止时为空） */
  runError: string;
  onShowReport: () => void;
  onNewSession: () => void;
  projectRoot?: string | null;
}

/** 事件 step → 阶段序号（项目模式 6 阶段） */
const PHASE_STEPS_PROJECT: Record<string, number> = { scan: 0, explore: 1, read: 1, route: 2, view: 3, usage: 3, reconnect: 3, repair: 4, reject: 4, view_done: 5 };
/** 事件 step → 阶段序号（文本模式 3 阶段） */
const PHASE_STEPS_TEXT: Record<string, number> = { view: 0, usage: 0, reconnect: 0, repair: 1, reject: 1, view_done: 2 };
const PHASE_KEYS_PROJECT = ["agent.phaseScan", "agent.phaseExplore", "agent.phaseRoute", "agent.phaseGenerate", "agent.phaseValidate", "agent.phaseDraw"] as const;
const PHASE_KEYS_TEXT = ["agent.phaseGenerate", "agent.phaseValidate", "agent.phaseDraw"] as const;

const wbSelect = "h-8 min-w-0 rounded-md border border-white/10 bg-slate-950/70 px-2 text-xs text-slate-200 outline-none focus:border-cyan-400/60 disabled:opacity-50";
const wbBtn = "inline-flex cursor-pointer items-center gap-1 rounded-md border border-white/10 bg-white/[0.05] px-2 py-1.5 text-[10px] text-slate-300 transition hover:bg-white/[0.1] hover:text-white disabled:cursor-default disabled:opacity-40";

/** 阶段步进器：按事件流推导各阶段状态（待办/进行/完成/失败） */
function PhaseStepper({ mode, entries, loading, t }: { mode: AgentWorkbenchMode; entries: readonly AiProgressEntry[]; loading: boolean; t: (k: string, o?: any) => string }) {
  // 对话模式没有「阶段计划」：工具循环的事件走时间线即可
  if (mode === "chat") return null;
  const phaseMap = mode === "project" ? PHASE_STEPS_PROJECT : PHASE_STEPS_TEXT;
  const keys = mode === "project" ? PHASE_KEYS_PROJECT : PHASE_KEYS_TEXT;
  const { maxPhase, failPhase, drawCount, cancelled } = useMemo(() => {
    let m = -1, fp = -1, dc = 0;
    let cancelled = false;
    for (const e of entries) {
      if (e.step === "cancel") cancelled = true;
      if (e.step === "view_done") dc++;
      if (e.step === "fail") { const p = phaseMap["view"] ?? 0; fp = Math.max(fp, p); }
      const p = phaseMap[e.step];
      if (p !== undefined && p > m) m = p;
    }
    return { maxPhase: m, failPhase: fp, drawCount: dc, cancelled };
  }, [entries, phaseMap]);
  // 运行结束（未失败）：全部阶段视为完成
  const finished = !loading && maxPhase >= 0;
  return (
    <div className="flex items-center gap-0.5 px-1 py-0.5">
      {keys.map((k, i) => {
        let state: "pending" | "active" | "done" | "fail" = "pending";
        if (i < maxPhase) state = "done";
        else if (i === maxPhase) state = finished ? "done" : "active";
        if (i === failPhase) state = "fail";
        return (
          <div key={k} className="flex items-center gap-0.5">
            {i > 0 && <span className={`h-px w-2.5 ${i <= maxPhase ? "bg-cyan-400/50" : "bg-white/10"}`} />}
            <span
              className={`inline-flex items-center gap-1 rounded-full border px-1.5 py-0.5 text-[8px] font-medium ${
                state === "done" ? "border-emerald-400/30 bg-emerald-400/10 text-emerald-300"
                : state === "active" ? "border-cyan-400/50 bg-cyan-400/10 text-cyan-300"
                : state === "fail" ? "border-red-400/40 bg-red-400/10 text-red-300"
                : "border-white/10 bg-white/[0.03] text-slate-500"
              }`}>
              {state === "active" ? <Loader2 className="h-2.5 w-2.5 animate-spin" />
                : state === "done" ? <Check className="h-2.5 w-2.5" />
                : state === "fail" ? <AlertTriangle className="h-2.5 w-2.5" />
                : <span className="h-1 w-1 rounded-full bg-current" />}
              {t(k)}{i === keys.length - 1 && drawCount > 0 && <span className="text-[7px] opacity-80">×{drawCount}</span>}
            </span>
          </div>
        );
      })}
      {cancelled && <span className="ml-1 inline-flex items-center gap-1 rounded-full border border-red-400/30 bg-red-400/10 px-1.5 py-0.5 text-[8px] text-red-300"><Ban className="h-2.5 w-2.5" />{t("agent.stopped")}</span>}
    </div>
  );
}

/** 单条工具调用/进度行：图标 + 文案 + 可选文件 chips（可点击打开） */
function ActivityLine({ e, projectRoot, t }: { e: AiProgressEntry; projectRoot?: string | null; t: (k: string, o?: any) => string }) {
  const files = e.step === "read" || e.step === "explore" ? e.files ?? [] : [];
  return (
    <div className="flex items-start gap-1.5">
      <span className="mt-0.5 shrink-0 font-mono text-[7px] text-slate-600">{e.at ? fmtClock(e.at) : ""}</span>
      <span className="mt-0.5 shrink-0">{STEP_ICONS[e.step] ?? <Terminal className="h-3 w-3 text-slate-400" />}</span>
      <div className="min-w-0 flex-1">
        <div className="text-[9px] leading-4 text-slate-300">
          {e.step === "read" && (e.files?.length ?? 0) > 0 ? t("mindmap.aiStepReadFiles", { count: (e.files ?? []).length }) : progressText(e, t)}
        </div>
        {files.length > 0 && (
          <div className="mt-0.5 flex flex-wrap gap-0.5">
            {files.map((f) => (
              <button key={f} type="button" disabled={!projectRoot}
                onClick={(ev) => { ev.stopPropagation(); if (projectRoot) openSourceFile(projectRoot, f); }}
                className={`nodrag nopan inline-flex max-w-[160px] items-center gap-0.5 truncate rounded border border-white/10 bg-white/[0.04] px-1 py-px font-mono text-[8px] ${projectRoot ? "cursor-pointer text-slate-300 transition hover:border-cyan-400/50 hover:text-cyan-200" : "text-slate-500"}`}
                title={projectRoot ? f : ""}>
                <File className="h-2 w-2 shrink-0" /><span className="truncate">{f.split(/[\\/]/).pop()}</span>
              </button>
            ))}
          </div>
        )}
        {e.step === "reject" && (e.paths?.length ?? 0) > 0 && (
          <div className="mt-0.5 flex flex-wrap gap-0.5">
            {e.paths!.map((p) => (
              <span key={p} className="max-w-[180px] truncate rounded border border-yellow-400/20 bg-yellow-400/5 px-1 py-px font-mono text-[8px] text-yellow-300/80" title={p}>{p}</span>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

/** 运行完成的结果卡：视图清单 + 节点/证据/用量统计 + 校验报告入口 */
function ResultCard({ result, t, onShowReport }: { result: AiImportResult; t: (k: string, o?: any) => string; onShowReport: () => void }) {
  const usage = result.usage;
  const totalNodes = result.documents.reduce((s, d) => s + d.nodes.length, 0);
  return (
    <div className="overflow-hidden rounded-lg border border-emerald-400/20 bg-emerald-400/[0.04]">
      <div className="flex items-center gap-2 border-b border-emerald-400/15 px-2.5 py-1.5">
        <Check className="h-3 w-3 text-emerald-300" />
        <span className="text-[10px] font-semibold text-emerald-200">{t("agent.resultTitle", { count: result.documents.length })}</span>
        <span className="ml-auto font-mono text-[8px] tabular-nums text-slate-400">
          {t("mindmap.aiRunUsageLine", { req: usage.requests, in: fmtNum(usage.inputTokens), out: fmtNum(usage.outputTokens), total: fmtNum(usage.totalTokens) })}
        </span>
      </div>
      <div className="space-y-1 px-2.5 py-2">
        {result.documents.map((d) => {
          const r = result.reports.find((x) => x.documentId === d.document.id);
          const ok = (r?.diagnostics.length ?? 0) === 0;
          return (
            <div key={d.document.id} className="flex items-center gap-1.5 text-[9px]">
              <span className="shrink-0 rounded border border-cyan-400/40 bg-cyan-400/10 px-1.5 py-px text-[8px] text-cyan-300">{viewLabel(t, r?.view ?? "architecture")}</span>
              <span className="min-w-0 flex-1 truncate text-slate-300" title={d.document.name}>{d.document.name}</span>
              <span className="shrink-0 font-mono text-[8px] text-slate-500">{r ? t("mindmap.nodeCount", { count: r.nodeCount }) : `${d.nodes.length}`}</span>
              <span className={`shrink-0 rounded px-1 py-px text-[8px] ${ok ? "bg-emerald-500/15 text-emerald-300" : "bg-amber-500/15 text-amber-300"}`}>
                {ok ? t("mindmap.validationOk") : t("mindmap.validationError")}
              </span>
            </div>
          );
        })}
        <div className="flex items-center gap-2 pt-0.5 font-mono text-[8px] text-slate-500">
          <span>{t("mindmap.nodeCount", { count: totalNodes })}</span>
          {result.failures.length > 0 && (
            <span className="text-amber-300/80">{t("mindmap.viewFailures", { count: result.failures.length, names: result.failures.map((f) => f.view).join("、") })}</span>
          )}
        </div>
      </div>
      <div className="border-t border-emerald-400/15 px-2.5 py-1.5">
        <button type="button" onClick={onShowReport} className="cursor-pointer text-[9px] font-medium text-cyan-300 transition hover:text-cyan-200">
          {t("agent.viewReport")} →
        </button>
      </div>
    </div>
  );
}

// ════════════ 询问表单：AI 遇到歧义时向用户提问，用户填写后回填继续 ════════════

interface AskField {
  key: string;
  label: string;
  type?: string; // text | textarea | select
  options?: string[];
  default?: string;
}

function AskForm({ ask, onSubmit, t }: {
  ask: { at: number; question: string; fields: AskField[]; round: number; max: number };
  onSubmit: (answer: Record<string, unknown> | string) => void;
  t: (k: string, o?: any) => string;
}) {
  const [values, setValues] = useState<Record<string, string>>(() => {
    const init: Record<string, string> = {};
    for (const f of ask.fields) init[f.key] = f.default ?? "";
    return init;
  });
  const [freeText, setFreeText] = useState("");
  const setField = (key: string, v: string) => setValues((p) => ({ ...p, [key]: v }));
  const submit = () => {
    const hasFields = ask.fields.length > 0;
    const answer: Record<string, unknown> = { ...values };
    if (!hasFields) answer.text = freeText;
    onSubmit(answer);
  };
  const canSubmit = ask.fields.length === 0 ? true : true; // 允许留空（后端按未填写兜底）
  void canSubmit;
  return (
    <div className="rounded-lg border border-amber-400/30 bg-amber-400/[0.05] p-2.5">
      <div className="mb-1.5 flex items-center gap-1.5">
        <span className="inline-flex h-4 w-4 items-center justify-center rounded-full bg-amber-400/20 text-amber-300">
          <Sparkles className="h-3 w-3" />
        </span>
        <span className="text-[10px] font-semibold text-amber-200">{t("agent.askTitle")}</span>
        <span className="ml-auto text-[8px] text-amber-300/70">{t("agent.askRound", { n: ask.round, max: ask.max })}</span>
      </div>
      {ask.question && <p className="mb-2 text-[10px] leading-4 text-slate-200">{ask.question}</p>}
      <div className="space-y-2">
        {ask.fields.map((f) => (
          <div key={f.key}>
            <label className="mb-0.5 block text-[9px] font-medium text-slate-400">{f.label}</label>
            {f.type === "select" ? (
              <select value={values[f.key] ?? ""} onChange={(e) => setField(f.key, e.target.value)}
                className="h-7 w-full rounded-md border border-white/10 bg-slate-950/70 px-2 text-[10px] text-slate-200 outline-none focus:border-amber-400/60">
                <option value="">{t("agent.askPick")}</option>
                {(f.options ?? []).map((o) => <option key={o} value={o}>{o}</option>)}
              </select>
            ) : f.type === "textarea" ? (
              <textarea value={values[f.key] ?? ""} onChange={(e) => setField(f.key, e.target.value)} rows={2}
                className="w-full resize-none rounded-md border border-white/10 bg-slate-950/70 px-2 py-1.5 text-[10px] text-slate-200 outline-none focus:border-amber-400/60" />
            ) : (
              <input value={values[f.key] ?? ""} onChange={(e) => setField(f.key, e.target.value)}
                className="h-7 w-full rounded-md border border-white/10 bg-slate-950/70 px-2 text-[10px] text-slate-200 outline-none focus:border-amber-400/60" />
            )}
          </div>
        ))}
        {ask.fields.length === 0 && (
          <textarea value={freeText} onChange={(e) => setFreeText(e.target.value)} rows={2}
            placeholder={t("agent.askFreePh")}
            className="w-full resize-none rounded-md border border-white/10 bg-slate-950/70 px-2 py-1.5 text-[10px] text-slate-200 outline-none focus:border-amber-400/60" />
        )}
      </div>
      <div className="mt-2 flex justify-end">
        <button type="button" onClick={submit}
          className="inline-flex cursor-pointer items-center gap-1 rounded-md border border-amber-400/40 bg-amber-400/15 px-3 py-1 text-[10px] font-semibold text-amber-100 transition hover:bg-amber-400/25">
          <Send className="h-3 w-3" />{t("agent.askSubmit")}
        </button>
      </div>
    </div>
  );
}

export function AgentWorkbench(props: AgentWorkbenchProps) {
  const {
    mode, onModeChange, documents, targetDocumentId, onTargetDocumentChange,
    providers, providerId, modelId, onProviderChange, onModelChange,
    projectPath, onPickProject, aiDepth, onDepthChange, aiViews, onViewsChange,
    textTitle, onTextTitleChange, loading, onRun, onStop, onAnswer, result, runError,
    onShowReport, onNewSession, projectRoot, projectDir, projectFiles, onBindProjectDir,
  } = props;
  const { t } = useTranslation();

  const messages = useAgentMessages();
  const entries = useEventBufferSnapshot(mmAiProgressBuffer);
  const [input, setInput] = useState("");
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

  // 运行结束（loading true→false）：向会话写入 agent 回执（成功摘要 / 失败原因 / 已取消）
  const prevLoading = useRef(loading);
  useEffect(() => {
    const was = prevLoading.current;
    prevLoading.current = loading;
    if (was === loading || !was) return;
    const cancelled = mmAiProgressBuffer.snapshot().some((e) => e.step === "cancel");
    // 取消优先：result 可能仍是上一轮的旧结果，不能落入成功分支
    if (cancelled) { pushAgentMessage("agent", t("mindmap.aiCancelled")); return; }
    if (runError) { pushAgentMessage("agent", `${t("agent.runFailed")}：${runError}`); return; }
    if (result) {
      const fails = result.failures.length
        ? t("mindmap.viewFailures", { count: result.failures.length, names: result.failures.map((f) => f.view).join("、") })
        : "";
      pushAgentMessage("agent", t("agent.runDone", { count: result.documents.length, names: result.documents.map((d) => d.document.name).join("、"), failures: fails }));
    }
  }, [loading, result, runError]);

  // 会话/进度变化 → 滚动到底部
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages.length, entries.length, loading]);

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

  // 时间线：折叠重复的 stream 进行中帧（与旧 AiProgressLog 一致）
  const timeline = useMemo(() => {
    const out: AiProgressEntry[] = [];
    for (const p of entries) {
      if (p.step === "stream" && !p.done) {
        const l = out[out.length - 1];
        if (l && l.step === "stream" && !l.done) continue;
      }
      out.push(p);
    }
    return out;
  }, [entries]);

  const providerModels = useMemo(() => providers.find((p) => p.id === providerId)?.models ?? [], [providers, providerId]);

  // @ 候选：按光标前的 @ 关键词过滤绑定目录文件清单
  const atMatches = useMemo(
    () => (atQuery === null ? [] : (projectFiles ?? []).filter((f) => f.toLowerCase().includes(atQuery.toLowerCase())).slice(0, 8)),
    [atQuery, projectFiles],
  );

  const onInputChange = (value: string) => {
    setInput(value);
    const caret = inputRef.current?.selectionStart ?? value.length;
    const m = value.slice(0, caret).match(/(?:^|\s)@([^\s@]*)$/);
    if (m && (projectFiles?.length ?? 0) > 0) { setAtQuery(m[1]); setAtIndex(0); } else { setAtQuery(null); }
  };

  const pickAt = (path: string) => {
    const caret = inputRef.current?.selectionStart ?? input.length;
    const before = input.slice(0, caret).replace(/@([^\s@]*)$/, `@${path} `);
    setInput(before + input.slice(caret));
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
    pushAgentMessage("user", bubble);
    setInput("");
    onRun(txt);
  };

  const statusColor = loading ? "text-cyan-300" : runError ? "text-red-300" : "text-slate-400";

  return (
    <div className="flex h-full min-h-0 flex-col gap-2" style={{ height: "calc(88vh - 120px)", minHeight: 440 }}>
      {/* 状态条：阶段计划 + 实时统计 */}
      <div className="flex shrink-0 flex-wrap items-center gap-2 rounded-lg border border-white/10 bg-slate-950/70 px-2 py-1.5">
        <span className="inline-flex shrink-0 items-center gap-1 text-[9px] font-semibold uppercase tracking-wide text-slate-400">
          <Brain className={`h-3 w-3 ${loading ? "animate-pulse text-cyan-300" : "text-slate-500"}`} />
          {t("agent.consoleTitle")}
        </span>
        <span className={`text-[9px] font-medium ${statusColor}`}>{loading ? t("agent.working") : runError ? t("agent.failed") : t("agent.ready")}</span>
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
      <div ref={scrollRef} className="min-h-0 flex-1 space-y-1.5 overflow-y-auto rounded-lg border border-white/10 bg-slate-950/50 p-2">
        {isFirstRun && !loading && (
          <div className="flex items-center gap-2 rounded-lg border border-dashed border-white/10 bg-white/[0.02] px-3 py-2.5">
            <Sparkles className="h-4 w-4 shrink-0 text-cyan-300/70" />
            <p className="text-[10px] leading-4 text-slate-400">
              {mode === "project" ? t("agent.hintProject") : t("agent.hintText")}
            </p>
          </div>
        )}
        {messages.map((m) => (
          <div key={m.id} className={`flex ${m.role === "user" ? "justify-end" : "justify-start"}`}>
            <div className={`max-w-[85%] rounded-lg px-2.5 py-1.5 text-[10px] leading-4 ${m.role === "user" ? "border border-cyan-400/25 bg-cyan-400/10 text-cyan-100" : "border border-white/10 bg-white/[0.04] text-slate-300"}`}>
              {m.text}
            </div>
          </div>
        ))}
        {/* 实时工具调用（运行中） */}
        {loading && (
          <div className="space-y-1 rounded-lg border border-white/5 bg-black/20 p-2">
            {timeline.map((e, i) => <ActivityLine key={`${e.at}-${i}`} e={e} projectRoot={projectRoot} t={t} />)}
            {lastStream ? (
              <div className="flex items-start gap-1.5">
                <span className="mt-0.5 shrink-0"><Terminal className="h-3 w-3 animate-pulse text-emerald-300" /></span>
                <div className="min-w-0 flex-1">
                  <div className="text-[9px] text-emerald-200/90">{t("mindmap.aiStepStream")}</div>
                  <div className="mt-0.5 max-h-[72px] overflow-y-auto rounded border border-emerald-400/15 bg-emerald-400/[0.03] px-1.5 py-1 font-mono text-[8px] leading-3.5 whitespace-pre-wrap text-emerald-100/80">
                    {lastStream.text || "…"}
                    <span className="ml-0.5 inline-block h-2 w-1 animate-pulse bg-emerald-300 align-middle" />
                  </div>
                  <div className="mt-0.5 font-mono text-[7px] text-slate-500">{fmtNum(lastStream.length)} chars</div>
                </div>
              </div>
            ) : (
              <div className="flex items-center gap-1.5 pl-1">
                <Loader2 className="h-3 w-3 animate-spin text-slate-400" />
                <span className="text-[8px] text-slate-500">{timeline.length ? t("agent.thinking") : t("mindmap.aiStepScan")}</span>
              </div>
            )}
          </div>
        )}
        {/* AI 询问用户：后端推送 ask 事件后阻塞等待，渲染表单让用户填写；
            提交后 onAnswer 回填后端、AI 基于回答继续（可能再次提问）。 */}
        {pendingAsk && <AskForm ask={pendingAsk} onSubmit={submitAsk} t={t} />}
        {/* 完成后的结果卡（新一轮开始后自动隐藏：loading 时不渲染） */}
        {!loading && result && <ResultCard result={result} t={t} onShowReport={onShowReport} />}
      </div>

      {/* 任务配置（可折叠；首轮默认展开，运行开始后收起，追问前可展开改参数） */}
      <div className="shrink-0 rounded-lg border border-white/10 bg-slate-950/70">
        <button type="button" onClick={() => setCfgOpen(!cfgOpen)}
          className="flex w-full cursor-pointer items-center gap-1.5 px-2.5 py-1.5 text-left">
          <ChevronDown className={`h-3 w-3 shrink-0 text-slate-500 transition-transform ${cfgOpen ? "" : "-rotate-90"}`} />
          <span className="text-[9px] font-semibold uppercase tracking-wide text-slate-400">{t("agent.configTitle")}</span>
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
            <div className="grid grid-cols-3 gap-2">
              <select className={wbSelect} value={mode} onChange={(e) => onModeChange(e.target.value as AgentWorkbenchMode)} disabled={loading}>
                <option value="chat">{t("mindmap.aiTaskChat")}</option>
                <option value="text">{t("mindmap.aiTaskText")}</option>
                <option value="project">{t("mindmap.aiTaskProject")}</option>
              </select>
              <select className={wbSelect} value={targetDocumentId} onChange={(e) => onTargetDocumentChange(e.target.value)} disabled={loading}>
                <option value="">{t("mindmap.aiTargetDocument")}</option>
                {documents.map((d) => <option key={d.id} value={d.id}>{d.name}</option>)}
              </select>
              <select className={wbSelect} value={providerId}
                onChange={(e) => { const pid = e.target.value; onProviderChange(pid); const p = providers.find((x) => x.id === pid); onModelChange(p?.active_model_id ?? p?.models[0]?.id ?? ""); }}>
                <option value="">{t("mindmap.pickProvider")}</option>
                {providers.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
              </select>
            </div>
            <div className="grid grid-cols-2 gap-2">
              <select className={wbSelect} value={modelId} onChange={(e) => onModelChange(e.target.value)} disabled={!providerId}>
                <option value="">{t("mindmap.pickModel")}</option>
                {providerModels.map((m) => <option key={m.id} value={m.id}>{m.name || m.id}</option>)}
              </select>
            </div>
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
                    <span className="text-[9px] font-semibold uppercase tracking-wide text-slate-500">{t("mindmap.aiDepthTitle")}</span>
                    <span className="text-[9px] font-semibold text-cyan-300">{t(`mindmap.aiDepth${aiDepth}`)}</span>
                  </div>
                  <input type="range" min={1} max={5} step={1} value={aiDepth} onChange={(e) => onDepthChange(Number(e.target.value))} className="w-full accent-cyan-400" />
                  <p className="mt-0.5 text-[8px] leading-3.5 text-slate-600">{t(`mindmap.aiDepthDesc${aiDepth}`)}</p>
                </div>
                <div>
                  <div className="mb-0.5 text-[9px] font-semibold uppercase tracking-wide text-slate-500">{t("mindmap.aiViewsTitle")}</div>
                  <div className="flex flex-wrap gap-1">
                    {["architecture", "workflow", "dataflow"].map((v) => {
                      const on = aiViews.includes(v);
                      return (
                        <button key={v} type="button"
                          className={`cursor-pointer rounded-md border px-1.5 py-0.5 text-[9px] transition ${on ? "border-cyan-400/60 bg-cyan-400/15 text-white" : "border-white/10 bg-slate-950/60 text-slate-400 hover:text-slate-200"}`}
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
                <label className="mb-0.5 block text-[9px] font-semibold uppercase tracking-wide text-slate-500">{t("mindmap.reqTitlePh")}</label>
                <input className={`w-full rounded-md border border-white/10 bg-slate-950/70 px-2 py-1.5 text-[10px] text-slate-200 outline-none focus:border-cyan-400/60`}
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
          <div className="absolute bottom-full left-0 right-0 z-20 mb-1 max-h-40 overflow-y-auto rounded-lg border border-white/10 bg-slate-900/95 py-1 shadow-xl">
            {atMatches.map((f, i) => (
              <button key={f} type="button" onMouseDown={(e) => { e.preventDefault(); pickAt(f); }}
                className={`block w-full cursor-pointer truncate px-2.5 py-1 text-left font-mono text-[10px] transition ${i === atIndex ? "bg-[var(--module-accent)]/25 text-white" : "text-slate-300 hover:bg-white/5"}`}>
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
          className="max-h-[120px] min-h-[34px] flex-1 resize-none rounded-lg border border-white/10 bg-slate-950/70 px-2.5 py-2 text-[11px] text-slate-200 outline-none focus:border-cyan-400/60 disabled:opacity-50"
        />
        {!loading && (
          <button type="button" onClick={submit} disabled={sendDisabled}
            className="inline-flex h-[34px] shrink-0 cursor-pointer items-center gap-1 rounded-lg px-3 text-[10px] font-semibold text-white transition disabled:cursor-default disabled:opacity-40"
            style={{ backgroundColor: "var(--module-accent, #22d3ee)" }}>
            <Send className="h-3 w-3" />
            {isFirstRun ? t("agent.runFirst") : t("agent.followUp")}
          </button>
        )}
        {messages.length > 0 && !loading && (
          <button type="button"
            onClick={() => { clearAgentMessages(); onNewSession(); }}
            className="inline-flex h-[34px] shrink-0 cursor-pointer items-center gap-1 rounded-lg border border-white/10 bg-white/[0.04] px-2 text-[9px] text-slate-400 transition hover:text-slate-200"
            title={t("agent.newSessionTip")}>
            <RotateCcw className="h-3 w-3" />
            {t("agent.newSession")}
          </button>
        )}
      </div>
    </div>
  );
}
