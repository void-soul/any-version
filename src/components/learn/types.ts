import { invoke } from "@tauri-apps/api/core";

// ─── 类型定义（与 src-tauri/src/commands/learn/models.rs 一一对应） ───

export interface LearnNode {
  id: string;
  name: string;
  parentId: string | null;
  /** 一句话概述（画布节点副标题） */
  description: string;
  /** 详细说明（markdown） */
  detail: string;
  /** module / component / class / function / service / route / file / config / other */
  kind: string;
  positionX: number;
  positionY: number;
}

export interface LearnGraph {
  projectPath: string;
  projectName: string;
  /** 项目整体概述（markdown） */
  summary: string;
  generatedAt: string;
  nodes: LearnNode[];
}

export interface LearnMeta {
  projectPath: string;
  projectName: string;
  generatedAt: string;
  nodeCount: number;
}

export interface GenerateLearnInput {
  projectPath: string;
  providerId?: string | null;
  modelId?: string | null;
}

export interface GenerateFromTextInput {
  text: string;
  title: string;
  providerId?: string | null;
  modelId?: string | null;
}

export interface RegenerateNodeInput {
  projectPath: string;
  nodeId: string;
  providerId?: string | null;
  modelId?: string | null;
}

export interface NodePosition {
  nodeId: string;
  x: number;
  y: number;
}

// ─── 后端调用封装 ───

export const learnApi = {
  generate: (input: GenerateLearnInput) => invoke<LearnGraph>("learn_generate", { input }),
  generateFromText: (input: GenerateFromTextInput) => invoke<LearnGraph>("learn_generate_from_text", { input }),
  regenerateNode: (input: RegenerateNodeInput) => invoke<LearnGraph>("learn_regenerate_node", { input }),
  list: () => invoke<LearnMeta[]>("learn_list"),
  load: (projectPath: string) => invoke<LearnGraph | null>("learn_load", { projectPath }),
  remove: (projectPath: string) => invoke<void>("learn_delete", { projectPath }),
  /** 将分析结果导出为结构化 Markdown 文档字符串。 */
  exportMarkdown: (projectPath: string) => invoke<string>("learn_export_markdown", { projectPath }),
  /** 持久化节点拖放位置。 */
  updatePositions: (projectPath: string, positions: NodePosition[]) => invoke<void>("learn_update_positions", { projectPath, positions }),
};

// ─── 节点类型元信息（图标 + 配色） ───

export const LEARN_KIND_META: Record<string, { label: string; color: string }> = {
  root: { label: "根", color: "#f8fafc" },
  module: { label: "模块", color: "#22d3ee" },
  lib: { label: "库", color: "#a78bfa" },
  component: { label: "组件", color: "#34d399" },
  class: { label: "类", color: "#fbbf24" },
  function: { label: "函数", color: "#60a5fa" },
  service: { label: "服务", color: "#fb7185" },
  route: { label: "路由", color: "#f97316" },
  config: { label: "配置", color: "#94a3b8" },
  file: { label: "文件", color: "#94a3b8" },
  entry: { label: "入口", color: "#f59e0b" },
  other: { label: "其他", color: "#64748b" },
};

export function kindColor(kind: string): string {
  return LEARN_KIND_META[kind]?.color ?? LEARN_KIND_META.other.color;
}
