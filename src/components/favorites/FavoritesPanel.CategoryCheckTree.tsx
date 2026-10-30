import { type FavoriteCategoryNode } from "./types";
/** 选择器里的勾选树（缩进 + 复选框）。 */
export function CategoryCheckTree({
  nodes, depth, selected, onToggle,
}: {
  nodes: FavoriteCategoryNode[];
  depth: number;
  selected: number[];
  onToggle: (id: number) => void;
}) {
  return (
    <>
      {nodes.map((c) => (
        <div key={c.id}>
          <label
            className="flex items-center gap-1.5 py-1 rounded hover:bg-white/5 cursor-pointer"
            style={{ paddingLeft: depth * 12 }}
          >
            <input
              type="checkbox"
              checked={selected.includes(c.id)}
              onChange={() => onToggle(c.id)}
              className="accent-[var(--module-accent)] cursor-pointer"
            />
            <span className="text-caption text-slate-200 truncate">{c.name}</span>
            <span className="ml-auto text-tiny text-slate-500">{c.count}</span>
          </label>
          {c.children.length > 0 && (
            <CategoryCheckTree
              nodes={c.children}
              depth={depth + 1}
              selected={selected}
              onToggle={onToggle}
            />
          )}
        </div>
      ))}
    </>
  );
}
