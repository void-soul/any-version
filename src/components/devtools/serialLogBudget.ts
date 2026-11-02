import type { LogEntry } from "./serialUtils";
export function trimSerialLogs(entries: LogEntry[], maxEntries: number, maxBytes = 8 * 1024 * 1024): LogEntry[] {
  let bytes = 0;
  let start = entries.length;
  while (start > 0 && entries.length - start < maxEntries) {
    const entry = entries[start - 1];
    const cost = ((entry.text?.length ?? 0) + (entry.hex?.length ?? 0) + entry.time.length) * 2 + 64;
    if (bytes + cost > maxBytes) break;
    bytes += cost; start--;
  }
  return entries.slice(start);
}
