import { describe, expect, it } from "vitest";
import { MODULE_CATALOG, MODULE_CATALOG_MAP, moduleLabel } from "../moduleCatalog";
import { resolveModuleLayout } from "../moduleRegistry";

describe("module catalog boundaries", () => {
  it("provides navigation metadata without attaching page components", () => {
    expect(MODULE_CATALOG.length).toBeGreaterThan(0);
    expect(new Set(MODULE_CATALOG.map((m) => m.id)).size).toBe(MODULE_CATALOG.length);
    expect(MODULE_CATALOG.every((m) => !("Component" in m))).toBe(true);
    expect(MODULE_CATALOG_MAP.tasks.id).toBe("tasks");
    expect(moduleLabel("unknown-module")).toBe("unknown-module");
  });

  it("keeps required settings available even when persisted disabled IDs include it", () => {
    const layout = resolveModuleLayout([], [], ["settings", "music"]);
    expect(layout.toolbarModules.some((m) => m.id === "settings")).toBe(true);
    expect(layout.allEnabled.some((m) => m.id === "music")).toBe(false);
  });
});
