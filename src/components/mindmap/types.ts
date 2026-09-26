import { invoke } from "@tauri-apps/api/core";

// ─── 文件夹 ───

export interface MindmapFolder {
  id: string;
  name: string;
  sortOrder: number;
  documentCount: number;
  parentId: string | null;
  createdAt: string;
  updatedAt: string;
}

// ─── 文档 ───

export interface MindmapDocument {
  id: string;
  name: string;
  description: string;
  sourceType: string; // manual | ai_project | ai_text | task
  sourceDesc: string;
  folderId: string | null;
  nodeCount: number;
  stickerCount: number;
  createdAt: string;
  updatedAt: string;
  backgroundTexture: string;
  /** 布局方向：lr=左→右（默认） rl=右→左 tb=上→下 bt=下→上 */
  layoutDir: string;
  /** 累计 AI 导入次数（token 消耗留痕） */
  aiImports: number;
  /** 绑定的项目目录：一个导图文档至多绑定一个（AI 上下文与 @ 引用固定来自它） */
  projectDir: string | null;
  /** 累计输入 token */
  aiInputTokens: number;
  /** 累计输出 token */
  aiOutputTokens: number;
}

// ─── 节点 ───

export interface MindmapNode {
  id: string;
  documentId: string;
  parentId: string | null;
  name: string;
  detail: string;       // Markdown 详细内容
  kind: string;          // root/task/requirement/module/constraint/risk/other...
  color: string;         // hex
  /** 证据锚定：该节点对应的真实源码文件（项目相对路径，AI 标注 + 扫描校验） */
  sources?: string[];
  positionX: number;
  positionY: number;
  // 注：节点不再带 createdAt / updatedAt —— 时间账由库内部维护，节点只描述「是什么」
}

// ─── 贴纸 ───

export interface MindmapSticker {
  id: string;
  documentId: string;
  content: string;
  /** 图片贴纸的 data URL；文字贴纸为空 */
  imageData?: string;
  /** 用户调整后的旋转角度；旧数据为空时使用稳定默认角度 */
  rotation?: number;
  color: string;
  positionX: number;
  positionY: number;
  createdAt: string;
  updatedAt: string;
}

// ─── 自由关系线 ───

/** 两个节点之间的自由关系线：与父子树无关，可任意连接并带说明文字。 */
export interface MindmapLink {
  id: string;
  documentId: string;
  /** 起点节点 id */
  sourceId: string;
  /** 终点节点 id */
  targetId: string;
  /** 关系说明文字（可为空） */
  label: string;
  createdAt: string;
  updatedAt: string;
}

// ─── 完整负载 ───

export interface DocumentFull {
  document: MindmapDocument;
  nodes: MindmapNode[];
  stickers: MindmapSticker[];
  /** 自由关系线（旧数据可能为空） */
  links?: MindmapLink[];
}

/** 一次 AI 导入运行（或单个视图）的 token 消耗统计。 */
export interface UsageStats {
  requests: number;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
}

/** AI 项目探索的单轮记录：AI 为什么读这批文件、实际读了什么 */
export interface AiExploreRound {
  /** 轮次（从 1 开始） */
  round: number;
  /** AI 给出的本轮读取理由（一句话；空串 = 模型未提供） */
  reason: string;
  /** 本轮实际读取的文件（相对路径） */
  files: string[];
  /** 本轮 AI 请求确认存在的目录（不读取内容） */
  dirs: string[];
  /** 本轮读取是否触达单批字符预算（文件被截断时为 true） */
  truncated: boolean;
}

/** 某个视图生成失败的原因（不影响其它已成功的视图） */
export interface AiImportFailure {
  view: string;
  reason: string;
}

/** 单个视图的校验报告（导入完成弹窗展示用） */
export interface AiImportReport {
  documentId: string;
  /** 视图类型：architecture / workflow / dataflow / sequence / lifecycle */
  view: string;
  /** 导入的节点总数（含根） */
  nodeCount: number;
  /** 实际发生的 AI 调用轮数（1 = 首次即通过；>1 = 经过修复重试） */
  repairRounds: number;
  /** 修复循环耗尽后仍残留的校验错误（空 = 完全通过） */
  diagnostics: string[];
  /** 引用的证据文件总数（所有节点 sources 之和） */
  evidenceCount: number;
  /** 命中真实文件的证据数（evidenceVerified=false 时等于 evidenceCount） */
  evidenceHitCount: number;
  /** 证据是否经过文件集核验（项目模式 true，文本模式 false） */
  evidenceVerified: boolean;
  /** 有证据的节点数（无证据节点 = nodeCount - evidenceNodes，即纯 AI 推断） */
  evidenceNodes: number;
  /** 该视图生成消耗的 token */
  usage: UsageStats;
}

/** AI 类型路由导入结果：一次生成多个视图，各自落在独立文档 */
export interface AiImportResult {
  documents: DocumentFull[];
  /** 应切换到的主文档 id（第一个成功的视图） */
  primaryId: string;
  failures: AiImportFailure[];
  reports: AiImportReport[];
  /** 项目探索过程：每轮读取的文件清单与理由（文本模式 / 探索失败时为空） */
  exploration: AiExploreRound[];
  /** 本次运行的总体消耗（含类型路由与探索阶段） */
  usage: UsageStats;
}

// ─── 输入类型 ───

export interface CreateDocInput {
  name: string;
  description?: string;
  sourceType?: string;
  folderId?: string | null;
}

export interface UpdateDocInput {
  id: string;
  name?: string;
  description?: string;
  folderId?: string | null;
}

export interface UpsertNodeInput {
  documentId: string;
  node: MindmapNode;
}

export interface DeleteNodeInput {
  documentId: string;
  nodeId: string;
}

export interface PositionInput {
  nodeId: string;
  x: number;
  y: number;
}

export interface UpsertStickerInput {
  documentId: string;
  sticker: MindmapSticker;
}

export interface DeleteStickerInput {
  documentId: string;
  stickerId: string;
}

export interface UpsertLinkInput {
  documentId: string;
  link: MindmapLink;
}

export interface DeleteLinkInput {
  documentId: string;
  linkId: string;
}

export interface AiProjectInput {
  documentId: string;
  projectPath: string;
  providerId?: string | null;
  modelId?: string | null;
  userHint?: string | null;
  /** 产物深度 1（最浅清单）→ 5（最深业务流+判定方式）；缺省 3 */
  depth?: number;
  /** 要生成的视图（architecture/workflow/dataflow）；空 = AI 自动判断 */
  views?: string[];
  /** 追问修改已有思维导图时，按完整结果替换目标文档节点；首轮为 false */
  replaceExisting?: boolean;
  /** 本次运行的取消标识：前端生成 UUID 传入，点「停止」时按此中断导入 */
  runId: string;
}

export interface AiTextInput {
  documentId: string;
  text: string;
  title: string;
  providerId?: string | null;
  modelId?: string | null;
  /** 追问修改已有思维导图时，按完整结果替换目标文档节点；首轮为 false */
  replaceExisting?: boolean;
  /** 本次运行的取消标识：同 AiProjectInput.runId */
  runId: string;
}

export interface RegenerateInput {
  documentId: string;
  nodeId: string;
  providerId?: string | null;
  modelId?: string | null;
}

export interface MoveDocumentInput {
  documentId: string;
  folderId: string | null;
}

export interface MoveFolderInput {
  folderId: string;
  parentId: string | null;
}

export interface CreateFolderInput {
  name: string;
  parentId?: string | null;
}

export interface UpdateFolderInput {
  id: string;
  name?: string;
}

// ─── API ───

export const mmApi = {
  init: () => invoke<void>("mm_init"),
  list: (folderId?: string | null) => invoke<MindmapDocument[]>("mm_list_documents", { folderId: folderId ?? null }),
  create: (i: CreateDocInput) => invoke<MindmapDocument>("mm_create_document", { input: i }),
  update: (i: UpdateDocInput) => invoke<void>("mm_update_document", { input: i }),
  remove: (id: string) => invoke<void>("mm_delete_document", { id }),
  load: (id: string) => invoke<DocumentFull | null>("mm_load_document", { id }),

  listFolders: () => invoke<MindmapFolder[]>("mm_list_folders"),
  createFolder: (i: CreateFolderInput) => invoke<MindmapFolder>("mm_create_folder", { input: i }),
  updateFolder: (i: UpdateFolderInput) => invoke<void>("mm_update_folder", { input: i }),
  deleteFolder: (id: string) => invoke<void>("mm_delete_folder", { id }),
  moveFolder: (i: MoveFolderInput) => invoke<void>("mm_move_folder", { input: i }),
  moveDocument: (i: MoveDocumentInput) => invoke<void>("mm_move_document", { input: i }),
  updateBackgroundTexture: (documentId: string, texture: string) => invoke<void>("mm_update_background_texture", { documentId, texture }),
  updateLayoutDir: (documentId: string, dir: string) => invoke<void>("mm_update_layout_dir", { documentId, dir }),

  upsertNode: (i: UpsertNodeInput) => invoke<void>("mm_upsert_node", { input: i }),
  deleteNode: (i: DeleteNodeInput) => invoke<void>("mm_delete_node", { input: i }),
  updatePositions: (id: string, pos: PositionInput[]) => invoke<void>("mm_update_positions", { documentId: id, positions: pos }),

  upsertSticker: (i: UpsertStickerInput) => invoke<void>("mm_upsert_sticker", { input: i }),
  deleteSticker: (i: DeleteStickerInput) => invoke<void>("mm_delete_sticker", { input: i }),

  upsertLink: (i: UpsertLinkInput) => invoke<void>("mm_upsert_link", { input: i }),
  deleteLink: (i: DeleteLinkInput) => invoke<void>("mm_delete_link", { input: i }),

  exportMd: (id: string) => invoke<string>("mm_export_markdown", { documentId: id }),
  aiFromProject: (i: AiProjectInput) => invoke<AiImportResult>("mm_ai_from_project", { input: i }),
  aiFromText: (i: AiTextInput) => invoke<AiImportResult>("mm_ai_from_text", { input: i }),
  aiCancel: (runId: string) => invoke<void>("mm_ai_cancel", { runId }),
  aiAnswer: (runId: string, answer: Record<string, unknown> | string) => invoke<void>("mm_ai_answer", { runId, answer }),
  regenerateNode: (i: RegenerateInput) => invoke<DocumentFull>("mm_regenerate_node", { input: i }),

  agentChat: (i: AgentChatInput) => invoke<AgentChatResult>("mm_agent_chat", { input: i }),
  agentGetSession: (documentId: string) => invoke<string>("mm_agent_get_session", { documentId }),
  agentListMessages: (sessionId: string) => invoke<AgentMessageRow[]>("mm_agent_list_messages", { sessionId }),
  agentListSessions: (documentId: string) => invoke<AgentSessionRow[]>("mm_agent_list_sessions", { documentId }),
  agentNewSession: (documentId: string) => invoke<string>("mm_agent_new_session", { documentId }),
  agentDeleteSession: (sessionId: string) => invoke<void>("mm_agent_delete_session", { sessionId }),
  agentRenameSession: (sessionId: string, title: string) => invoke<void>("mm_agent_rename_session", { sessionId, title }),
  /** 分叉：复制截至 messageId（空串 = 全部）的历史到新会话，返回新会话 id */
  agentForkSession: (sessionId: string, messageId: string) => invoke<string>("mm_agent_fork_session", { sessionId, messageId }),
  bindDocumentDir: (documentId: string, dir: string | null) => invoke<void>("mm_bind_document_dir", { documentId, dir }),
  listProjectFiles: (documentId: string) => invoke<string[]>("mm_list_project_files", { documentId }),
};

// ─── AI Agent（右栏对话）───

/** Agent 提交给前端应用的写操作（后端只裁决不执行，落图走既有写路径） */
export interface AgentOp {
  action: "add" | "update" | "delete" | "move";
  /** add = 后端生成的节点 id；update/delete/move = 目标节点 id */
  id?: string;
  /** add/move 携带；`null` = 建根节点（无父节点） */
  parentId?: string | null;
  name?: string;
  detail?: string;
  kind?: string;
  /** add/update 携带；后端已校验为 #RRGGBB，不合法时为 null */
  color?: string | null;
  /** add 携带：该节点的证据锚定文件（项目相对路径或绝对路径），分析文件后回填 */
  sources?: string[];
}

/** 会话消息（落库形态，回放只展示不执行 opsJson） */
export interface AgentMessageRow {
  id: string;
  sessionId: string;
  role: string;
  content: string;
  opsJson: string;
  createdAt: string;
}

export interface AgentChatInput {
  documentId: string;
  sessionId?: string;
  message?: string;
  selectedNodeIds?: string[];
  /** @ 引用的文件（相对绑定目录路径），内容截断后进上下文 */
  attachedFiles?: string[];
  providerId?: string | null;
  modelId?: string | null;
  runId?: string;
}

export interface AgentChatResult {
  sessionId: string;
  reply: string;
  rounds: number;
}

/** 会话列表项：一个导图可有多个会话（各自独立的历史，可互相分叉） */
export interface AgentSessionRow {
  id: string;
  documentId: string;
  /** 用户自定义标题；为空时前端回退展示首条用户消息 */
  title: string | null;
  /** 分叉来源会话 id */
  parentId: string | null;
  /** 分叉点消息 id（该条及之前的消息被复制过来） */
  forkedFromMessageId: string | null;
  createdAt: string;
  updatedAt: string;
  messageCount: number;
}

/**
 * Agent 的写操作**全部直接应用**，不再弹确认清单。
 *
 * 早先删除/移动要等用户在右栏裁决（后端阻塞最长 10 分钟，模型只能干等），
 * 现在统一直接落图 —— 界面有撤销快照，用户随时 Ctrl+Z 回退。
 * 保留这个函数是为了集中表达这条策略（也让调用方保持单一入口）。
 */
export function partitionAgentOps(ops: AgentOp[]): { auto: AgentOp[]; confirm: AgentOp[] } {
  return { auto: ops, confirm: [] };
}

// ─── 节点颜色映射 ───

export const KIND_COLORS: Record<string, string> = {
  root: "#f8fafc",
  module: "#22d3ee",
  component: "#34d399",
  service: "#fb7185",
  route: "#f97316",
  config: "#94a3b8",
  file: "#94a3b8",
  requirement: "#fbbf24",
  task: "#60a5fa",
  constraint: "#a78bfa",
  risk: "#fb7185",
  other: "#64748b",
};

export function kindColor(k: string): string { return KIND_COLORS[k] ?? KIND_COLORS.other; }