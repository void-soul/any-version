import type { Item } from "./types";

/** Return launcher items ordered by usage count, then by name for stable ties. */
export function sortLauncherItemsByUsage<T extends Pick<Item, "name" | "data">>(items: readonly T[]): T[] {
  return [...items]
    .filter((item) => (item.data.openNumber ?? 0) > 0)
    .sort((a, b) =>
      (b.data.openNumber ?? 0) - (a.data.openNumber ?? 0)
      || a.name.localeCompare(b.name),
    );
}
