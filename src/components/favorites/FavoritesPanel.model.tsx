
/** 分类栏宽度：默认 180px（比原先固定的 160px 宽 20px），范围与后端 clamp 一致。 */
export const DEFAULT_LEFT_WIDTH = 180;

/**
 * 长任务名（与后端 `commands/favorites/commands.rs` 的 TASK_* 一致）。
 *
 * 三个平台的导入可以同时跑（各拉各的接口），但导入与「加工」（归类 / 失效检测）
 * 必须互斥：加工要扫全库挑条目，和正在写入的导入抢同一份数据。
 */
export type FavTask = "github" | "bilibili" | "zhihu" | "bookmark" | "classify" | "check";

export const IMPORT_TASKS: FavTask[] = ["github", "bilibili", "zhihu", "bookmark"];

export const isImportTask = (task: FavTask) => IMPORT_TASKS.includes(task);

export const MIN_LEFT_WIDTH = 140;

export const MAX_LEFT_WIDTH = 420;

/** AI 检索右栏宽度（与后端 settings.rs 的 MIN/MAX_AI_WIDTH 一致） */
export const DEFAULT_AI_WIDTH = 360;

export const MIN_AI_WIDTH = 260;

export const MAX_AI_WIDTH = 640;

/** 检索 Agent 轮数：与后端 `clamp_agent_rounds` 同一范围，默认 6。 */
export const MIN_AGENT_ROUNDS = 1;

export const MAX_AGENT_ROUNDS = 20;

export const DEFAULT_AGENT_ROUNDS = 6;

export const AI_TOOL_LABEL: Record<string, string> = {
  search_favorites: "检索",
  get_favorite_content: "读正文",
  list_favorite_tags: "看分类",
};
