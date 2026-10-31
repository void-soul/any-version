import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, Ban, Brain, Coins, File, LayoutGrid, MessageCircle, RotateCcw, Search, Sparkles, Terminal, Wrench } from "lucide-react";
import type { AiConfig } from "../../contracts/ai";
import type { AgentSessionRow, AiImportResult } from "./types";
import { createEventBuffer, type EventBuffer } from "../../utils/eventBuffer";
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
  // 思考过程：step=agentTool 的工具名与参数；step=agentThought/agentReasoning 的文本
  tool?: string;
  args?: string;
  // 前端收到时打的时间戳（ms）
  at?: number;
}


/** AI 导入进度事件缓冲（模块级，App 生命周期内常驻）：思维导图面板切走/隐藏后，
 *  组件内的 mm-ai-progress 订阅随 Effects 销毁，后端即发即弃的事件会丢；
 *  由缓冲在模块作用域统一订阅并保存载荷，面板重新可见时完整重放（长任务不断流）。 */
export const mmAiProgressBuffer: EventBuffer<AiProgressEntry> = createEventBuffer<AiProgressEntry>(
  "mm-ai-progress",
  {
    owner: "mindmap",
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
  // 思考过程：读/写工具调用 + 模型调用前的想法 + 模型产出的推理文本
  agentTool: <Wrench className="h-3 w-3 text-cyan-300" />,
  agentThought: <Brain className="h-3 w-3 text-violet-300" />,
  agentReasoning: <Brain className="h-3 w-3 text-fuchsia-300" />,
  // 流式思考帧（step=reasoning）：推理模型边想边推的尾部预览，与 agentReasoning 同色系
  reasoning: <Brain className="h-3 w-3 text-fuchsia-300" />,
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
    // 流式思考帧：推理模型先想几十秒才动笔，这部分也要有进度，否则像卡死
    case "reasoning":
      return e.done ? t("mindmap.aiStepReasoningDone", { n: e.length ?? 0 }) : t("mindmap.aiStepReasoning", { n: e.length ?? 0 });
    case "ask":
      return t("agent.askStep", { n: e.round ?? 1, max: e.max ?? 3 });
    case "cancel":
      return t("mindmap.aiCancelled");
    case "agentOps":
      return t("mindmap.agentOpsStep", { n: e.ops?.length ?? 0 });
    // 思考过程：工具名翻译 + 参数（参数可能很长，交给 ActivityLine 折叠展示）
    case "agentTool":
      return t("mindmap.agentToolStep", { tool: toolLabel(t, e.tool ?? ""), n: e.round ?? 0 });
    case "agentThought":
      return t("mindmap.agentThoughtStep", { n: e.round ?? 0 });
    case "agentReasoning":
      return t("mindmap.agentReasoningStep", { n: e.round ?? 0 });
    default:
      return e.detail ?? e.step;
  }
}


/** 工具名的中文标签（与后端 AGENT_TOOLS_SPEC 的函数名对应） */
export const AGENT_TOOL_LABELS: Record<string, string> = {
  get_document_overview: "读取导图大纲",
  get_subtree: "读取子树",
  find_nodes: "查找节点",
  create_root_node: "新建主线",
  add_child_nodes: "新增平级节点",
  add_subtree: "新增子树",
  update_nodes: "更新节点",
  delete_nodes: "删除节点",
  move_nodes: "移动节点",
};


export const toolLabel = (t: (k: string, o?: any) => string, tool: string) =>
  t(AGENT_TOOL_LABELS[tool] ? `mindmap.agentTool_${tool}` : "", { defaultValue: tool });


// ════════════ 会话存储（模块级：最小化/关闭重开不丢对话） ════════════

/**
 * 会话消息：**按轮次组织，而不是按状态字段**。
 *
 * 与数据库设计器的 `chatState.ts` 同一套模型。早先这里只有 `{role, text}`，
 * 于是「这一轮改了什么」只能拼成一句话塞进正文：用户问「它到底动了几个节点」，
 * 答案是散在进度流里的，跑完就看不见了。现在一轮 = 一条用户气泡 + 一条助手气泡，
 * 助手那条的 status / summary / patch / truncated / error 就是这一轮的回执。
 */
export interface AgentMessage {
  id: number;
  role: "user" | "agent";
  text: string;
  ts: number;
  /** 落库消息 id：回放历史时带上，用于「从这条分叉」定位分叉点 */
  dbId?: string;
  /** 助手这一轮的状态（用户气泡没有） */
  status?: TurnStatus;
  /** 完成摘要，例如「已生成 3 个视图」 */
  summary?: string;
  /** 本轮改动的节点计数（由 Agent 提交的 ops 统计） */
  patch?: { added: number; updated: number; removed: number; moved: number };
  /** 模型输出被截断 / 写崩过（后端修复后才落下） */
  truncated?: boolean;
  /** 本轮失败原因 */
  error?: string;
}


export type TurnStatus = "thinking" | "writing" | "done" | "error";


export let agentMsgs: readonly AgentMessage[] = [];

export let agentMsgSeq = 0;

export const agentMsgSubs = new Set<() => void>();


export function emitAgentMsgs() {
  agentMsgSubs.forEach((fn) => fn());
}


export function pushAgentMessage(role: AgentMessage["role"], text: string, dbId?: string) {
  agentMsgs = [...agentMsgs, { id: ++agentMsgSeq, role, text, ts: Date.now(), dbId }];
  emitAgentMsgs();
}


/**
 * 开一轮：用户气泡 + 助手占位气泡。
 *
 * 助手气泡先占位而不是等有结果再建：这样进度（思考中 / 正在写）才有地方显示，
 * 也不会出现「用户说完、AI 那半边空白闪一下」的跳动。
 */
export function startAgentTurn(userText: string) {
  agentMsgs = [
    ...agentMsgs,
    { id: ++agentMsgSeq, role: "user", text: userText, ts: Date.now() },
    { id: ++agentMsgSeq, role: "agent", text: "", ts: Date.now(), status: "thinking" },
  ];
  emitAgentMsgs();
}


/**
 * 更新**最后一轮**助手气泡。前面几轮是历史，不该被后来的进度覆盖。
 *
 * 还没有助手气泡时（例如发请求就失败了）退化成新推一条 —— 调用方不必先判空。
 */
export function patchAgentTurn(patch: Partial<Pick<AgentMessage, "text" | "status" | "summary" | "patch" | "truncated" | "error">>) {
  for (let i = agentMsgs.length - 1; i >= 0; i--) {
    const msg = agentMsgs[i];
    if (msg.role !== "agent") continue;
    // 只补丁「本轮还没结束」的占位气泡（thinking / writing）。
    // 已结束的历史气泡一律不动 —— 追问等路径不会先开新轮，盲改最后一条会把
    // 上一轮的回执（摘要 / 改动计数 / 失败原因）覆盖成这一轮的状态。
    if (msg.status !== "thinking" && msg.status !== "writing") break;
    const out = agentMsgs.slice();
    out[i] = { ...msg, ...patch };
    agentMsgs = out;
    emitAgentMsgs();
    return;
  }
  // 没有在跑的气泡（例如请求发出前就失败）：新推一条，历史保持原样。
  agentMsgs = [
    ...agentMsgs,
    {
      id: ++agentMsgSeq,
      role: "agent",
      text: patch.text ?? patch.summary ?? patch.error ?? "",
      ts: Date.now(),
      status: patch.status ?? "done",
      summary: patch.summary,
      patch: patch.patch,
      truncated: patch.truncated,
      error: patch.error,
    },
  ];
  emitAgentMsgs();
}


/** 本轮 ops → 计数摘要（渲染只需要「几个节点」，不必把整份 ops 塞进气泡） */
export function opCounts(ops: readonly { action?: string }[]): NonNullable<AgentMessage["patch"]> {
  const c = { added: 0, updated: 0, removed: 0, moved: 0 };
  for (const op of ops) {
    if (op.action === "add") c.added += 1;
    else if (op.action === "update") c.updated += 1;
    else if (op.action === "delete") c.removed += 1;
    else if (op.action === "move") c.moved += 1;
  }
  return c;
}


export function clearAgentMessages() {
  agentMsgs = [];
  agentMsgSubs.forEach((fn) => fn());
}


export function subscribeAgentMsgs(fn: () => void) {
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
export let answeredAskIds: ReadonlySet<number> = new Set();

export const answeredAskSubs = new Set<() => void>();


export function markAskAnswered(at: number) {
  if (answeredAskIds.has(at)) return;
  answeredAskIds = new Set(answeredAskIds).add(at);
  answeredAskSubs.forEach((fn) => fn());
}


export function clearAnsweredAsks() {
  answeredAskIds = new Set();
  answeredAskSubs.forEach((fn) => fn());
}


export function subscribeAnsweredAsks(fn: () => void) {
  answeredAskSubs.add(fn);
  return () => { answeredAskSubs.delete(fn); };
}


export function useAnsweredAsks(): ReadonlySet<number> {
  return useSyncExternalStore(subscribeAnsweredAsks, () => answeredAskIds, () => answeredAskIds);
}


// ════════════ 工作台输入草稿（模块级：隐藏/关闭卸载组件后不丢） ════════════
// AgentWorkbench 的输入框原先用组件内 state，而「最小化」与「关闭」都会把它卸载，
// 于是隐藏一下再恢复、或关掉重新打开，辛苦敲的提示词就没了。草稿放模块级存储，
// 唯一清空点是成功提交（submit 里写空）。


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
  /** 合并选择回调：一次确定供应商 + 模型（ModelSelector 用，避免两步回调的闭包旧值） */
  onSelectModel: (pid: string, mid: string) => void;
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
  /** 新建会话（后端建一条新历史；前端清屏由调用方负责） */
  onNewSession: () => void;
  /** chat 模式会话列表（一个导图多个会话） */
  sessions?: AgentSessionRow[];
  sessionId?: string;
  onSelectSession?: (id: string) => void;
  onRenameSession?: (id: string, title: string) => void;
  onDeleteSession?: (id: string) => void;
  /** 从某条消息分叉出新会话（messageId 为空 = fork 整个会话） */
  onForkSession?: (messageId: string) => void;
  projectRoot?: string | null;
}


/** ISO 时间 → MM-DD HH:mm（会话列表的回退标题用） */
export function shortTs(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}


/** 事件 step → 阶段序号（项目模式 6 阶段） */
export const PHASE_STEPS_PROJECT: Record<string, number> = { scan: 0, explore: 1, read: 1, route: 2, view: 3, usage: 3, reconnect: 3, repair: 4, reject: 4, view_done: 5 };

/** 事件 step → 阶段序号（文本模式 3 阶段） */
export const PHASE_STEPS_TEXT: Record<string, number> = { view: 0, usage: 0, reconnect: 0, repair: 1, reject: 1, view_done: 2 };

export const PHASE_KEYS_PROJECT = ["agent.phaseScan", "agent.phaseExplore", "agent.phaseRoute", "agent.phaseGenerate", "agent.phaseValidate", "agent.phaseDraw"] as const;

export const PHASE_KEYS_TEXT = ["agent.phaseGenerate", "agent.phaseValidate", "agent.phaseDraw"] as const;


/** AI 输入框自动增高的上限（px）：到顶后内部滚动，否则长指令会把对话区挤没 */
export const INPUT_MAX_HEIGHT = 160;


export const wbSelect = "ui-input h-8 min-w-0 px-2 disabled:opacity-50";

export const wbBtn = "ui-btn px-2 py-1.5 disabled:opacity-40";


// ════════════ 询问表单：AI 遇到歧义时向用户提问，用户填写后回填继续 ════════════

export interface AskField {
  key: string;
  label: string;
  type?: string; // text | textarea | select
  options?: string[];
  default?: string;
}
