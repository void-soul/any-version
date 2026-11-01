import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const project = (id: string) => ({ id, name: id, path: `E:/projects/${id}`, addedAt: "" });

beforeEach(() => {
  vi.resetModules();
  invoke.mockReset();
});

describe("shared AI projects", () => {
  it("opens a project by registered ID through the backend", async () => {
    const store = await import("./useProjects");
    invoke.mockResolvedValue(undefined);
    await store.openProjectDir("saved-project");
    expect(invoke).toHaveBeenCalledWith("open_skill_project_directory", { projectId: "saved-project" });
  });

  it("refreshes the persisted list, counts and unchanged selection details", async () => {
    const store = await import("./useProjects");
    invoke.mockImplementation(async (command) => command === "list_projects" ? [project("a")] : []);
    await store.reloadProjects();
    store.setPicked(new Set(["a", "removed"]));
    invoke.mockImplementation(async (command) => command === "list_projects"
      ? [project("a"), project("b")]
      : [{ projectId: "a", total: 3, linked: 2, local: 1, broken: 0 }]);
    const details = vi.fn().mockResolvedValue(undefined);
    await store.refreshProjectsAndSelection(details);
    expect(store.getProjectsSnapshot().projects.map((p) => p.id)).toEqual(["a", "b"]);
    expect(store.getProjectsSnapshot().counts.a.total).toBe(3);
    expect([...store.getProjectsSnapshot().picked]).toEqual(["a"]);
    expect(details).toHaveBeenCalledWith("a");
  });

  it("loads replacement selection and clears the last deleted selection", async () => {
    const store = await import("./useProjects");
    invoke.mockImplementation(async (command) => command === "list_projects" ? [project("a")] : []);
    await store.reloadProjects();
    const details = vi.fn().mockResolvedValue(undefined);
    invoke.mockImplementation(async (command) => command === "list_projects" ? [project("b")] : []);
    await store.refreshProjectsAndSelection(details);
    expect(details).toHaveBeenLastCalledWith("b");
    invoke.mockResolvedValue([]);
    await store.refreshProjectsAndSelection(details);
    expect(details).toHaveBeenLastCalledWith("");
    expect(store.getProjectsSnapshot().selProject).toBe("");
  });

  it("forces a new read after an older in-flight request", async () => {
    const store = await import("./useProjects");
    let finish!: (value: unknown) => void;
    invoke.mockImplementation(async (command) => command === "list_projects"
      ? new Promise((resolve) => { finish = resolve; }) : []);
    const initial = store.reloadProjects();
    const details = vi.fn().mockResolvedValue(undefined);
    const refresh = store.refreshProjectsAndSelection(details);
    expect(store.getProjectsSnapshot().loading).toBe(true);
    invoke.mockImplementation(async (command) => command === "list_projects" ? [project("new")] : []);
    finish([project("old")]);
    await Promise.all([initial, refresh]);
    expect(store.getProjectsSnapshot().projects[0].id).toBe("new");
    expect(details).toHaveBeenCalledWith("new");
    expect(invoke.mock.calls.filter(([command]) => command === "list_projects")).toHaveLength(2);
  });

  it("preserves the last good snapshot and exposes list or count failures", async () => {
    const store = await import("./useProjects");
    invoke.mockImplementation(async (command) => command === "list_projects" ? [project("a")] : []);
    await store.reloadProjects();
    const details = vi.fn();
    invoke.mockRejectedValue(new Error("cannot read projects"));
    await store.refreshProjectsAndSelection(details);
    expect(store.getProjectsSnapshot().error).toContain("cannot read projects");
    expect(store.getProjectsSnapshot().projects[0].id).toBe("a");
    expect(store.getProjectsSnapshot().loading).toBe(false);
    expect(details).not.toHaveBeenCalled();
    invoke.mockImplementation(async (command) => {
      if (command === "list_projects") return [project("b")];
      throw new Error("cannot scan skills");
    });
    await store.refreshProjectsAndSelection(details);
    expect(store.getProjectsSnapshot().error).toContain("cannot scan skills");
    expect(details).not.toHaveBeenCalled();
    invoke.mockResolvedValue([]);
    await store.refreshProjectsAndSelection(details);
    expect(store.getProjectsSnapshot().error).toBeNull();
  });
});
