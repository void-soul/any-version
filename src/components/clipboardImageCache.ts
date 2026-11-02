// A conservative UTF-16 budget bounds cached data URLs even when the panel stays mounted.
export class ClipboardImageCache {
  private entries = new Map<number, string>();
  private bytes = 0;
  constructor(private maxEntries = 128, private maxBytes = 8 * 1024 * 1024) {
    if (!Number.isSafeInteger(maxEntries) || maxEntries < 1 || !Number.isSafeInteger(maxBytes) || maxBytes < 0) {
      throw new RangeError("Invalid thumbnail cache capacity");
    }
  }
  get(id: number): string | undefined {
    const value = this.entries.get(id);
    if (value !== undefined) { this.entries.delete(id); this.entries.set(id, value); }
    return value;
  }
  set(id: number, value: string): void {
    this.delete(id);
    const size = value.length * 2;
    if (size > this.maxBytes || this.maxEntries < 1) return;
    this.entries.set(id, value); this.bytes += size;
    while (this.entries.size > this.maxEntries || this.bytes > this.maxBytes) {
      this.delete(this.entries.keys().next().value!);
    }
  }
  delete(id: number): void {
    const value = this.entries.get(id);
    if (value !== undefined) { this.bytes -= value.length * 2; this.entries.delete(id); }
  }
  clear(): void { this.entries.clear(); this.bytes = 0; }
  get size(): number { return this.entries.size; }
  get byteSize(): number { return this.bytes; }
}
