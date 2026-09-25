/** AI 检索结果的行解析（纯函数，便于单测）。
 *
 * 模型输出是**我们自己约定**的简化 Markdown：只会出现 `## ` / `### ` 标题、
 * `- [标题](链接) —— 说明` 清单项、以及普通段落。这里不做通用 Markdown 解析，
 * 只认这几种形态，认不出的当普通文本渲染（宁可朴素，也不要漏掉内容）。
 */
export type AiResultLine =
  | { kind: "blank" }
  | { kind: "heading"; level: 2 | 3; text: string }
  | { kind: "item"; title: string; url: string; note: string }
  | { kind: "bullet"; text: string }
  | { kind: "text"; text: string };

const ITEM_RE = /^[-*]\s+\[([^\]]+)\]\(([^)]+)\)(.*)$/;

export function parseAiResultLine(line: string): AiResultLine {
  const trimmed = line.trimEnd();
  if (!trimmed.trim()) return { kind: "blank" };
  if (trimmed.startsWith("### ")) {
    return { kind: "heading", level: 3, text: trimmed.slice(4) };
  }
  if (trimmed.startsWith("## ")) {
    return { kind: "heading", level: 2, text: trimmed.slice(3) };
  }
  const item = trimmed.match(ITEM_RE);
  if (item) {
    const note = (item[3] || "").replace(/^\s*[—–-]+\s*/, "").trim();
    return { kind: "item", title: item[1], url: item[2], note };
  }
  if (trimmed.startsWith("- ") || trimmed.startsWith("* ")) {
    return { kind: "bullet", text: trimmed.slice(2) };
  }
  return { kind: "text", text: trimmed };
}
