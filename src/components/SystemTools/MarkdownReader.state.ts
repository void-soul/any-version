import type { Tab } from "./MarkdownReader.model";
export function recordMarkdownSave(tabs: Tab[], path: string, savedContent: string): Tab[] {
  return tabs.map((tab) => tab.path === path ? { ...tab, savedContent, dirty: tab.content !== savedContent } : tab);
}
export function trimMarkdownHistory(history: string[], maxBytes = 8 * 1024 * 1024, maxEntries = 100): string[] {
  let bytes = 0;
  let start = history.length;
  while (start > 0 && history.length - start < maxEntries) {
    const cost = history[start - 1].length * 2;
    if (bytes + cost > maxBytes) break;
    bytes += cost; start--;
  }
  return history.slice(start);
}
