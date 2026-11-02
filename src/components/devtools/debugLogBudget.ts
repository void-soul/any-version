export function trimDebugLogs<T extends { text: string; hex?: string; time: string; meta?: { data?: string; id?: string; event?: string } }>(entries: T[], maxEntries: number, maxBytes = 8 * 1024 * 1024): T[] {
  let bytes = 0;
  let start = entries.length;
  while (start > 0 && entries.length - start < maxEntries) {
    const entry = entries[start - 1];
    const cost = (entry.text.length + (entry.hex?.length ?? 0) + entry.time.length + (entry.meta?.data?.length ?? 0) + (entry.meta?.id?.length ?? 0) + (entry.meta?.event?.length ?? 0)) * 2 + 64;
    if (bytes + cost > maxBytes) break;
    bytes += cost; start--;
  }
  return entries.slice(start);
}
