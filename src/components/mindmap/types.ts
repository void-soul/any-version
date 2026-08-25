import { invoke } from "@tauri-apps/api/core";

// ─── 文件夹 ───

export interface MindmapFolder {
  id: string;
  name: string;
  sortOrder: number;
  documentCount: number;
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
  color: string;
  positionX: number;
  positionY: number;
  createdAt: string;
  updatedAt: string;
}

// ─── 完整负载 ───

export interface DocumentFull {
  document: MindmapDocument;
  nodes: MindmapNode[];
  stickers: MindmapSticker[];
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

export interface AiProjectInput {
  documentId: string;
  projectPath: string;
  providerId?: string | null;
  modelId?: string | null;
}

export interface AiTextInput {
  documentId: string;
  text: string;
  title: string;
  providerId?: string | null;
  modelId?: string | null;
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

export interface CreateFolderInput {
  name: string;
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
  moveDocument: (i: MoveDocumentInput) => invoke<void>("mm_move_document", { input: i }),

  upsertNode: (i: UpsertNodeInput) => invoke<void>("mm_upsert_node", { input: i }),
  deleteNode: (i: DeleteNodeInput) => invoke<void>("mm_delete_node", { input: i }),
  updatePositions: (id: string, pos: PositionInput[]) => invoke<void>("mm_update_positions", { documentId: id, positions: pos }),

  upsertSticker: (i: UpsertStickerInput) => invoke<void>("mm_upsert_sticker", { input: i }),
  deleteSticker: (i: DeleteStickerInput) => invoke<void>("mm_delete_sticker", { input: i }),

  exportMd: (id: string) => invoke<string>("mm_export_markdown", { documentId: id }),
  aiFromProject: (i: AiProjectInput) => invoke<DocumentFull>("mm_ai_from_project", { input: i }),
  aiFromText: (i: AiTextInput) => invoke<DocumentFull>("mm_ai_from_text", { input: i }),
  regenerateNode: (i: RegenerateInput) => invoke<DocumentFull>("mm_regenerate_node", { input: i }),
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