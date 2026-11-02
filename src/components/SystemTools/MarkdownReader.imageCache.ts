/** Bound pending entries and resolved data URL retention independently. */
export class MarkdownImageCache {
  private entries = new Map<string, { task: Promise<string | null>; bytes: number }>();
  private bytes = 0;
  constructor(private maxEntries = 32, private maxBytes = 8 * 1024 * 1024) {}
  get(key: string) {
    const entry = this.entries.get(key);
    if (!entry) return undefined;
    this.entries.delete(key); this.entries.set(key, entry);
    return entry.task;
  }
  set(key: string, task: Promise<string | null>) {
    this.delete(key);
    const entry = { task, bytes: key.length * 2 };
    this.entries.set(key, entry); this.bytes += entry.bytes; this.trim();
    void task.then((value) => {
      if (this.entries.get(key) !== entry) return;
      this.bytes -= entry.bytes;
      entry.bytes = (key.length + (value?.length ?? 0)) * 2;
      this.bytes += entry.bytes;
      if (entry.bytes > this.maxBytes) this.delete(key);
      this.trim();
    }, () => { if (this.entries.get(key) === entry) this.delete(key); });
  }
  delete(key: string) {
    const entry = this.entries.get(key);
    if (entry) { this.bytes -= entry.bytes; this.entries.delete(key); }
  }
  clear() { this.entries.clear(); this.bytes = 0; }
  private trim() {
    while (this.entries.size > this.maxEntries || this.bytes > this.maxBytes) {
      const key = this.entries.keys().next().value;
      if (key === undefined) break;
      this.delete(key);
    }
  }
}
export const imageCache = new MarkdownImageCache();
