import { describe, expect, it } from "vitest";
import { createProjectRequestScope } from "./projectRequestScope";
describe("SDK request ownership", () => {
  it("a full refresh supersedes a slow fast list and old details", () => {
    const scope = createProjectRequestScope();
    const fast = scope.beginList();
    const detail = scope.beginProject("node");
    const full = scope.beginList();
    expect(fast.current()).toBe(false);
    expect(detail.current()).toBe(false);
    expect(full.current()).toBe(true);
  });
  it("orders same-project reads without cancelling other projects", () => {
    const scope = createProjectRequestScope();
    const old = scope.beginProject("node");
    const other = scope.beginProject("redis");
    const next = scope.beginProject("node");
    expect(old.current()).toBe(false);
    expect(other.current()).toBe(true);
    expect(next.current()).toBe(true);
  });
  it("cleanup and StrictMode replay cannot revive old leases", () => {
    const scope = createProjectRequestScope();
    const list = scope.beginList();
    const detail = scope.beginProject("node");
    scope.reset();
    const next = scope.beginList();
    expect(list.current()).toBe(false);
    expect(detail.current()).toBe(false);
    expect(next.current()).toBe(true);
  });
});
