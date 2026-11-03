import { afterEach, describe, expect, it, vi } from "vitest";
afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); });
describe("JSON worker scalar documents", () => {
  it("bounds wide search matches and deep paths while reporting truncation", async () => {
    const scope = { postMessage: vi.fn(), onmessage: null as null | ((event: { data: unknown }) => void) };
    vi.stubGlobal("self", scope);
    await import("./jsonWorker");
    scope.onmessage!({ data: { id: 1, type: "parse", text: JSON.stringify(Array(30_000).fill("hit")) } });
    scope.onmessage!({ data: { id: 2, type: "search", query: "hit" } });
    const wide = scope.postMessage.mock.lastCall![0];
    expect(wide.truncated).toBe(true);
    expect(wide.paths.length).toBeLessThanOrEqual(20_000);
    expect(wide.directPaths.length).toBeLessThanOrEqual(20_000);
    const text = '{"x":'.repeat(500) + '1' + '}'.repeat(500);
    scope.onmessage!({ data: { id: 3, type: "parse", text } });
    scope.onmessage!({ data: { id: 4, type: "search", query: "x" } });
    const deep = scope.postMessage.mock.lastCall![0];
    expect(deep.truncated).toBe(true);
    expect(deep.paths.every((path: string) => path.split(".").length <= 257)).toBe(true);
  });
  it("searches valid null, false and zero instead of treating them as empty documents", async () => {
    const scope = { postMessage: vi.fn(), onmessage: null as null | ((event: { data: unknown }) => void) };
    vi.stubGlobal("self", scope);
    await import("./jsonWorker");
    for (const text of ["null", "false", "0"]) {
      scope.onmessage!({ data: { id: 1, type: "parse", text } });
      expect(scope.postMessage.mock.lastCall?.[0]).toMatchObject({ type: "parse", nodeCount: 1, error: null });
      scope.onmessage!({ data: { id: 2, type: "search", query: text } });
      expect(scope.postMessage.mock.lastCall?.[0]).toMatchObject({ type: "search", directPaths: ["root"] });
    }
  });
  it("clears a previous document after parse failure", async () => {
    const scope = { postMessage: vi.fn(), onmessage: null as null | ((event: { data: unknown }) => void) };
    vi.stubGlobal("self", scope);
    await import("./jsonWorker");
    scope.onmessage!({ data: { id: 1, type: "parse", text: "null" } });
    scope.onmessage!({ data: { id: 2, type: "parse", text: "broken" } });
    scope.onmessage!({ data: { id: 3, type: "search", query: "null" } });
    expect(scope.postMessage.mock.lastCall?.[0]).toMatchObject({ paths: [], directPaths: [] });
  });
});
