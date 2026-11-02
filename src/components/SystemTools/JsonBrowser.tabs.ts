export type JsonTab = { id: string; path: string | null; name: string; text: string; savedText: string | null };
export function jsonTabDirty(tab: JsonTab): boolean {
  return tab.savedText === null ? tab.text.trim().length > 0 : tab.text !== tab.savedText;
}
/** Saving a captured snapshot may update only that tab; newer edits stay dirty. */
export function recordJsonSave(tabs: JsonTab[], id: string, path: string, savedText: string): JsonTab[] {
  const name = path.split(/[\\/]/).pop() || "document.json";
  return tabs.map((tab) => tab.id === id ? { ...tab, path, name, savedText } : tab);
}
