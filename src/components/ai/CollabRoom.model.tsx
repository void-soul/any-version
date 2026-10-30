import React, { useEffect, useState } from "react";
import { Terminal, FileText, Globe, ListTodo, FilePen, Brain, Wrench } from "lucide-react";
import type { DetectedAiTool } from "../../contracts/ai";
export const DYNAMIC_COLOR_PALETTE = [
  "bg-orange-500", "bg-emerald-500", "bg-violet-500",
  "bg-cyan-500", "bg-amber-500", "bg-pink-500",
  "bg-rose-500", "bg-indigo-500", "bg-teal-500"
];

export function senderColor(sender: string, customColor?: string): string {
  if (sender === "user") return "bg-blue-500";
  if (customColor) return customColor;
  let hash = 0;
  for (let i = 0; i < sender.length; i++) {
    hash = (hash << 5) - hash + sender.charCodeAt(i);
    hash |= 0;
  }
  const idx = Math.abs(hash) % DYNAMIC_COLOR_PALETTE.length;
  return DYNAMIC_COLOR_PALETTE[idx];
}

// ─── 人类化状态文案 ───
// 运行中（思考/工作）的随机短语池（i18n 键）
export const THINKING_KEYS = [
  "thinking", "analyzing", "letMeThink", "organizing", "wording",
  "searching", "coding", "checking", "reasoning", "planning",
];

// 错误时的随机短语池（i18n 键）
export const ERROR_KEYS = [
  "errOops", "errHard", "errWrong", "errHelp", "errBlocked", "errUnexpected",
];

/// 随机选择一个短语
export function pickRandom(arr: string[]): string {
  return arr[Math.floor(Math.random() * arr.length)];
}

/// 轮换短语的自定义 Hook：每隔 intervalMs 毫秒随机切换一个短语
export function useRotatingPhrase(phrases: string[], active: boolean, intervalMs = 4000): string {
  const [idx, setIdx] = useState(() => Math.floor(Math.random() * phrases.length));
  useEffect(() => {
    if (!active) return;
    const timer = setInterval(() => {
      setIdx(Math.floor(Math.random() * phrases.length));
    }, intervalMs);
    return () => clearInterval(timer);
  }, [active, phrases.length, intervalMs]);
  return phrases[idx] || phrases[0] || "";
}

// ─── 头像/昵称来源 ───
// 唯一权威来源是 ai-tools/<id>/config.json（通过工具 Tab 的「资料」编辑写入后端）。
// 消息体本身也携带 avatar 与 nickname 覆盖（见 CollabMessage），保证历史消息稳定显示。
/// 获取有效头像：消息携带 > 工具配置 > null
export function getToolAvatar(tools: DetectedAiTool[], toolId: string, msgAvatar?: string | null): string | null {
  if (msgAvatar) return msgAvatar;
  const t = tools.find((x) => x.id === toolId);
  return t?.avatar ?? null;
}

/// 获取有效昵称：消息携带 > 工具配置 > null
export function getToolNickname(tools: DetectedAiTool[], toolId: string, msgNickname?: string | null): string | null {
  if (msgNickname) return msgNickname;
  const t = tools.find((x) => x.id === toolId);
  return t?.nickname ?? null;
}

/// 耗时格式化：毫秒 → "1.2s" / "1m03s"
export function formatDuration(ms: number): string {
  const totalSec = Math.round(ms / 1000);
  if (totalSec < 60) return `${totalSec}s`;
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  return `${m}m${s.toString().padStart(2, "0")}s`;
}

/** agent 在线状态点颜色（working=绿脉冲, thinking=琥珀脉冲, online=绿, offline=灰） */
export function agentDotClass(status?: string): string {
  if (status === "working") return "bg-emerald-400 animate-pulse";
  if (status === "thinking") return "bg-amber-400 animate-pulse";
  if (status === "online") return "bg-emerald-500";
  return "bg-slate-600";
}

export function agentStatusTitle(t: (k: string) => string, status?: string): string {
  if (status === "working") return t("collab.statusWorking");
  if (status === "thinking") return t("collab.statusThinking");
  if (status === "online") return t("collab.statusOnline");
  return t("collab.statusOffline");
}

// 从 open-tag 移植：按活动文本关键词映射工具图标（优先精确匹配，回退兜底）。
// open-tag 用结构化 toolName 字段，Kira 仅传合并后的 activity 字符串，
// 故对字符串做关键词匹配——只要文本含 bash/read/web/todo 等词即给对应图标。
export type IconType = React.ComponentType<{ className?: string }>;

export const TOOL_ICON_RULES: Array<[RegExp, IconType]> = [
  [/(bash|exec|shell|cmd|command|run|terminal)/i, Terminal],
  [/(read|open|cat|view|load)/i, FileText],
  [/(write|edit|create|save|update|modify|patch)/i, FilePen],
  [/(web|http|fetch|curl|request|url|search|browser)/i, Globe],
  [/(todo|task|list|plan)/i, ListTodo],
  [/(think|reason|analyze)/i, Brain],
];

export const TOOL_ICON_FALLBACK: IconType = Wrench;
