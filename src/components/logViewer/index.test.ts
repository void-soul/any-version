import { describe, expect, it } from "vitest";
import { buildLogIndex, DEFAULT_LOG_LIMITS } from "./index";
const bytes = (text: string) => new TextEncoder().encode(text);
const scan = (text: string, limits = DEFAULT_LOG_LIMITS) => buildLogIndex(bytes(text), async () => {}, () => true, () => {}, limits);
describe("bounded log indexing", () => {
  it("handles empty files, trailing newlines and classifies CRLF lines", async () => {
    expect((await scan(""))?.totalLines).toBe(0);
    const index = (await scan("ok\r\nERROR x\nwarn\n"))!;
    expect([...index.offsets]).toEqual([0, 4, 12]);
    expect([...index.levels]).toEqual([0, 2, 1]);
    expect(index).toMatchObject({ totalLines: 3, errorCount: 1, warnCount: 1 });
  });
  it("stops after a yield when a different file or unmount takes ownership", async () => {
    let current = true;
    expect(await buildLogIndex(bytes("log"), async () => { current = false; }, () => current, () => {})).toBeNull();
  });
  it("rejects byte, row and giant-line budgets before committing an index", async () => {
    await expect(scan("1234", { ...DEFAULT_LOG_LIMITS, maxBytes: 3 })).rejects.toThrow("logTooLarge");
    await expect(scan("a\nb\nc", { ...DEFAULT_LOG_LIMITS, maxLines: 2 })).rejects.toThrow("logTooManyLines");
    await expect(scan("1234", { ...DEFAULT_LOG_LIMITS, maxLineBytes: 3 })).rejects.toThrow("logLineTooLong");
    expect((await scan("a\nb\n", { ...DEFAULT_LOG_LIMITS, maxLines: 2 }))?.totalLines).toBe(2);
  });
});
