// AI 会话记忆的纯逻辑：从 2600 行的面板里抽出来才能测。
//
// 这里是「决策记录」与「会话历史」两条记忆的合并规则。它们都只存在于前端
// 内存 / 文档字段里，不进提示词拼装（那部分在后端）。

import type { ChatTurn } from "./types";

/**
 * 合并决策记录：**保留文档里已有的，只把本轮新增的追加上去**。
 *
 * 为什么不由模型自己维护：它每轮都要回吐整份设计文档，很可能把你之前写的
 * 决策丢了或改写。记忆是文档的一等公民，不是模型可以随手改写的一个字段 ——
 * 真丢了的代价是「三个月前为什么这么设计」再也找不回来。
 *
 * 去重按「去空白 + 忽略大小写」：模型下一轮很可能把同一条决策换个说法再写一遍，
 * 逐条累加会让记录越来越长而内容其实没变。
 */
export function mergeDecisions(
  existing: string[] | undefined,
  incoming: string[],
): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  const key = (s: string) => s.replace(/\s+/g, "").toLowerCase();
  for (const d of [...(existing ?? []), ...incoming]) {
    const t = (d ?? "").trim();
    if (!t) continue; // 空行 / 纯空白不算一条决策
    const k = key(t);
    if (seen.has(k)) continue;
    seen.add(k);
    out.push(t);
  }
  return out;
}

/** 会话历史保留的轮数（滚动窗口）。3 轮足够消解「再把它改一下」这类指代。 */
export const HISTORY_TURNS = 3;

/**
 * 历史滚动窗口：只留最近 N 轮。
 *
 * 截断按**轮次**而不是条数 —— 一轮是「用户一句 + 助手一句」两条，
 * 按条数截会把某轮的问答劈成两半，模型看到「有问无答」会再去问一遍。
 */
export function trimHistory(history: ChatTurn[], turns = HISTORY_TURNS): ChatTurn[] {
  const max = turns * 2;
  return history.length <= max ? history : history.slice(history.length - max);
}