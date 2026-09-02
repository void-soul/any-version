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
  description: string;
  detail: string;       // Markdown 详细内容
  kind: string;          // root/task/requirement/module/constraint/risk/other...
  color: string;         // hex
  progress: number;      // 0-100
  /** 计划时间（ISO 8601，可空） */
  planAt?: string | null;
  /** 计划重复：none=不重复 / daily=每天 / weekly=每周 */
  repeat?: string;
  /** 证据锚定：该节点对应的真实源码文件（项目相对路径，AI 标注 + 扫描校验） */
  sources?: string[];
  positionX: number;
  positionY: number;
  createdAt: string;
  updatedAt: string;
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

// ─── 额外连线（多输入 DAG：节点除树形父节点外，还可从任意节点接多条输入） ───

export interface MindmapLink {
  id: string;
  documentId: string;
  /** 来源（输出）节点 */
  sourceId: string;
  /** 目标（输入）节点 */
  targetId: string;
  /** 输入端名称（如「模型」「正面条件」）；空时用来源节点名 */
  label: string;
  createdAt: string;
  updatedAt: string;
}

// ─── 完整负载 ───

export interface DocumentFull {
  document: MindmapDocument;
  nodes: MindmapNode[];
  stickers: MindmapSticker[];
  /** 额外连线（多输入）；节点主父级仍由 parentId 承担 */
  links: MindmapLink[];
}

/** 一次 AI 导入运行（或单个视图）的 token 消耗统计。 */
export interface UsageStats {
  requests: number;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
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
  /** 本次运行的取消标识：前端生成 UUID 传入，点「停止」时按此中断导入 */
  runId: string;
}

export interface AiTextInput {
  documentId: string;
  text: string;
  title: string;
  providerId?: string | null;
  modelId?: string | null;
  /** 本次运行的取消标识：同 AiProjectInput.runId */
  runId: string;
}

export interface RegenerateInput {
  documentId: string;
  nodeId: string;
  providerId?: string | null;
  modelId?: string | null;
}

/** 指定日期范围内的具体计划发生记录（计划日历聚合展示用）。
 *  重复计划（daily/weekly）已由后端在查询时展开为逐次发生。 */
export interface PlannedOccurrence {
  id: string;
  documentId: string;
  documentName: string;
  name: string;
  kind: string;
  color: string;
  /** 原始计划时间（ISO 8601，用于打开详情时回显） */
  planAt: string;
  /** 计划重复：none / daily / weekly */
  repeat?: string;
  /** 本次发生的日期 YYYY-MM-DD（本地时间） */
  occurDay: string;
  /** 本次发生的具体时间（本地时间字符串，如 2026-08-30T09:00:00） */
  occurAt: string;
}

export interface MovePlanOccurrenceInput {
  nodeId: string;
  /** 拖拽来源日期 YYYY-MM-DD（该次发生的 occurDay） */
  fromDay: string;
  /** 拖拽目标日期 YYYY-MM-DD */
  toDay: string;
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
  regenerateNode: (i: RegenerateInput) => invoke<DocumentFull>("mm_regenerate_node", { input: i }),

  plannedOccurrences: (start: string, end: string) => invoke<PlannedOccurrence[]>("mm_planned_occurrences", { start, end }),
  movePlanOccurrence: (i: MovePlanOccurrenceInput) => invoke<void>("mm_move_plan_occurrence", { input: i }),
};

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