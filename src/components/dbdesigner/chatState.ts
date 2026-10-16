// AI 对话的消息模型：**按轮次组织，而不是按状态字段**。
//
// 为什么不用一堆散落的状态（aiThinking / aiStream / aiProgress / aiNewDecisions…）：
// 那些状态描述的是「现在进行到哪一步」，而对话要的是「说过什么」—— 用户发过的每一句、
// AI 每轮的进度与结果，都得按顺序留在那儿。现在散着放，只能显示「当前这一轮」，
// 上一轮说过的话就丢了，这跟聊天窗口的预期完全不符。
//
// **临时态，关掉面板即丢**：完整对话要持久化的话得存进设计文件或单独的历史表，
// 那是另一件事（会话历史存哪、跨文档要不要带，都是产品决策）。这里先保证「一轮一轮看得见」。
import type { PatchApplied, RepairReport } from "./types";

export type ChatStatus = "thinking" | "writing" | "repair" | "done" | "clarify" | "error";

export interface ChatMsg {
  id: string;
  role: "user" | "assistant";
  /** 用户气泡的原文；助手侧用它放追问的问题 */
  text?: string;
  /** 助手这一轮的状态 */
  status?: ChatStatus;
  /** 思考全文（可折叠） */
  thinking?: string;
  /** 正文字数（正在写时增长） */
  chars?: number;
  /** 完成摘要，例如「已生成 4 个表并自动布局」 */
  summary?: string;
  /** 本轮新增的设计决策（会同时进设计文件） */
  decisions?: string[];
  /** 增量摘要：改了哪几张表 */
  patch?: { added: number; updated: number; removed: number };
  /** 自校验结果 */
  repair?: RepairReport;
  /** 追问卡：模型停下来问你 */
  clarify?: { question: string; options: string[] };
  /** 截断：模型中途写崩了 */
  truncated?: boolean;
  error?: string;
}

/** 一轮对话最多留这么多条气泡（超出丢最旧的）—— 面板高度有限，太长只能滚很久才看到开头 */
export const MAX_CHAT_MSGS = 40;

let seq = 0;
export function nextMsgId(): string {
  seq += 1;
  return `m${seq}`;
}

/**
 * 开一轮：用户气泡 + 助手占位气泡。
 *
 * 助手气泡先占位而不是等有结果再建：这样进度（思考中 / 正在写）才有地方显示，
 * 也不会出现「用户说完、AI 那半边空白闪一下」的跳动。
 */
export function startTurn(msgs: ChatMsg[], userText: string): ChatMsg[] {
  const next = [
    ...msgs,
    { id: nextMsgId(), role: "user" as const, text: userText },
    { id: nextMsgId(), role: "assistant" as const, status: "thinking" as const },
  ];
  return next.slice(-MAX_CHAT_MSGS);
}

/** 更新**最后一轮**助手气泡。前面几轮是历史，不该被后来的进度覆盖。 */
export function patchLast(
  msgs: ChatMsg[],
  patch: Partial<Omit<ChatMsg, "id" | "role">>,
): ChatMsg[] {
  for (let i = msgs.length - 1; i >= 0; i--) {
    if (msgs[i].role !== "assistant") continue;
    const out = msgs.slice();
    out[i] = { ...out[i], ...patch };
    return out;
  }
  return msgs;
}

/** 增量摘要 → 计数结构（渲染只需要「几张」，不必把整份 PatchApplied 塞进气泡） */
export function patchCounts(p: PatchApplied): ChatMsg["patch"] {
  return {
    added: p.addedTables.length,
    updated: p.updatedTables.length,
    removed: p.removedTables.length,
  };
}
