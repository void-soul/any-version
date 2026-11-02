type FeedStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;
export function readRssCache(storage: FeedStorage, key: string, now: number, ttl: number): string | null {
  try {
    const value = JSON.parse(storage.getItem(key) || "null");
    return typeof value?.xml === "string" && value.xml.trim() && typeof value.fetchedAt === "number"
      && Number.isFinite(value.fetchedAt) && value.fetchedAt <= now && now - value.fetchedAt < ttl ? value.xml : null;
  } catch { return null; }
}
export function writeRssCache(storage: FeedStorage, key: string, xml: string, now: number): void {
  try {
    if (xml.trim()) storage.setItem(key, JSON.stringify({ xml, fetchedAt: now }));
    else storage.removeItem(key);
  } catch { /* A cache failure must not discard a successfully fetched feed. */ }
}
export function articleHttpUrl(value: string): string | null {
  try {
    const url = new URL(value);
    return url.protocol === "https:" || url.protocol === "http:" ? url.href : null;
  } catch { return null; }
}
