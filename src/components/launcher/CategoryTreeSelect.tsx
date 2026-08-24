import { useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, ChevronRight, FolderTree } from "lucide-react";
import { Classification } from "./types";

interface CategoryTreeSelectProps {
  classifications: Classification[];
  value: number;
  onChange: (id: number) => void;
  placeholder?: string;
  /** 需要从树中排除的分类 id（例如编辑分类时排除自身，避免成为自己的父级） */
  excludeId?: number | null;
  /** 是否允许“无/顶级”选项（用于父级分类选择） */
  allowNone?: boolean;
  noneLabel?: string;
  /** 排除某分类后是否将它的子分类一并隐藏（父级选择场景） */
  hideDescendantsOfExclude?: boolean;
}

interface TreeNode {
  category: Classification;
  children: TreeNode[];
}

/** 从扁平分类数组构建层级树（基于 parentId） */
function buildTree(
  classifications: Classification[],
  excludeId?: number | null,
  hideDescendantsOfExclude?: boolean
): TreeNode[] {
  const byParent = new Map<number | null, Classification[]>();
  for (const c of classifications) {
    if (excludeId != null && c.id === excludeId) continue;
    const key = c.parentId ?? null;
    const list = byParent.get(key) || [];
    list.push(c);
    byParent.set(key, list);
  }
  const descendents = new Set<number>();
  if (hideDescendantsOfExclude && excludeId != null) {
    const queue = [...(byParent.get(excludeId) || [])];
    while (queue.length) {
      const c = queue.shift()!;
      descendents.add(c.id);
      queue.push(...(byParent.get(c.id) || []));
    }
  }

  const toNode = (parentId: number | null): TreeNode[] => {
    const kids = (byParent.get(parentId) || [])
      .filter((c) => !descendents.has(c.id))
      .sort((a, b) => a.order - b.order);
    return kids.map((c) => ({
      category: c,
      children: toNode(c.id),
    }));
  };

  return toNode(null);
}

/** 解析某个分类在树中的完整路径名称，例如 “一级 / 二级 / 三级” */
function resolvePath(
  classifications: Classification[],
  id: number
): string {
  const map = new Map<number, Classification>();
  for (const c of classifications) map.set(c.id, c);
  const parts: string[] = [];
  let cur = map.get(id);
  while (cur) {
    parts.unshift(cur.data?.icon ? `${cur.data.icon} ${cur.name}` : cur.name);
    cur = cur.parentId != null ? map.get(cur.parentId) : undefined;
  }
  return parts.join(" / ");
}

export default function CategoryTreeSelect({
  classifications,
  value,
  onChange,
  placeholder = "选择分类",
  excludeId,
  allowNone = false,
  noneLabel = "顶级分类 (无父级)",
  hideDescendantsOfExclude = false,
}: CategoryTreeSelectProps) {
  const [open, setOpen] = useState(false);
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const [search, setSearch] = useState("");
  const containerRef = useRef<HTMLDivElement>(null);

  const tree = useMemo(
    () => buildTree(classifications, excludeId, hideDescendantsOfExclude),
    [classifications, excludeId, hideDescendantsOfExclude]
  );

  // 打开时默认展开第一层，便于快速选择
  useEffect(() => {
    if (open) {
      setExpanded(new Set(tree.filter((n) => n.children.length > 0).map((n) => n.category.id)));
    }
  }, [open, tree]);

  // 点击外部关闭
  useEffect(() => {
    if (!open) return;
    const onClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onClickOutside);
    return () => document.removeEventListener("mousedown", onClickOutside);
  }, [open]);

  const selectedLabel = allowNone && value === 0
    ? noneLabel
    : resolvePath(classifications, value) || placeholder;

  // 扁平化带深度信息，用于搜索与渲染
  const flatten = (nodes: TreeNode[], depth: number): Array<{ node: TreeNode; depth: number }> => {
    const result: Array<{ node: TreeNode; depth: number }> = [];
    for (const n of nodes) {
      result.push({ node: n, depth });
      if (expanded.has(n.category.id)) {
        result.push(...flatten(n.children, depth + 1));
      }
    }
    return result;
  };
  const flatList = flatten(tree, 0);

  const keyword = search.trim().toLowerCase();
  const visibleList = keyword
    ? (() => {
        const collect = (nodes: TreeNode[]): TreeNode[] =>
          nodes.flatMap((n) => [
            n,
            ...collect(n.children),
          ]);
        return collect(tree)
          .map((n) => n.category)
          .filter((c) => c.name.toLowerCase().includes(keyword) || (c.data?.icon || "").includes(keyword))
          .map((c) => ({ node: { category: c, children: [] }, depth: 0 }));
      })()
    : flatList;

  const toggle = (id: number, e: React.MouseEvent) => {
    e.stopPropagation();
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  return (
    <div className="relative" ref={containerRef}>
      {/* Trigger */}
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="w-full bg-[#1e2436] border border-white/10 rounded-xl px-3 py-2 text-xs text-white focus:outline-none focus:border-purple-500 transition cursor-pointer flex items-center justify-between gap-2 text-left"
      >
        <span className="flex items-center gap-1.5 min-w-0 truncate">
          <FolderTree className="w-3.5 h-3.5 text-purple-400 flex-shrink-0" />
          <span className="truncate">{selectedLabel}</span>
        </span>
        <ChevronDown className={`w-3.5 h-3.5 text-slate-500 flex-shrink-0 transition-transform ${open ? "rotate-180" : ""}`} />
      </button>

      {/* Dropdown */}
      {open && (
        <div className="absolute z-50 mt-1 w-full bg-[#1a2030] border border-white/10 rounded-xl shadow-2xl shadow-black/50 overflow-hidden">
          {/* Search box */}
          <div className="p-2 border-b border-white/5">
            <input
              autoFocus
              type="text"
              autoComplete="off"
              spellCheck={false}
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="搜索分类..."
              className="w-full bg-black/30 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-white focus:outline-none focus:border-purple-500 select-text"
            />
          </div>

          {/* Tree */}
          <div className="max-h-56 overflow-y-auto p-1.5">
            {allowNone && (
              <button
                type="button"
                onClick={() => {
                  onChange(0);
                  setOpen(false);
                  setSearch("");
                }}
                className={`w-full text-left px-2.5 py-1.5 rounded-lg text-xs transition cursor-pointer flex items-center gap-1.5 ${
                  value === 0
                    ? "bg-purple-600/20 text-white"
                    : "text-slate-400 hover:bg-white/5 hover:text-slate-200"
                }`}
              >
                <span className="text-slate-500">—</span>
                {noneLabel}
              </button>
            )}

            {visibleList.length === 0 && !allowNone && (
              <div className="px-2.5 py-4 text-center text-xs text-slate-500">未找到分类</div>
            )}

            {visibleList.map(({ node, depth }) => {
              const c = node.category;
              const hasChildren = node.children.length > 0;
              const isExpanded = expanded.has(c.id);
              const isSelected = value === c.id;
              return (
                <div
                  key={c.id}
                  className="flex items-center gap-1 rounded-lg cursor-pointer hover:bg-white/5 transition"
                  style={{ paddingLeft: `${depth * 16 + 8}px` }}
                >
                  {hasChildren ? (
                    <button
                      type="button"
                      onClick={(e) => toggle(c.id, e)}
                      className="w-4 h-4 flex items-center justify-center text-slate-500 hover:text-white transition cursor-pointer flex-shrink-0"
                    >
                      {isExpanded ? (
                        <ChevronDown className="w-3 h-3" />
                      ) : (
                        <ChevronRight className="w-3 h-3" />
                      )}
                    </button>
                  ) : (
                    <span className="w-4 h-4 flex-shrink-0" />
                  )}
                  <button
                    type="button"
                    onClick={() => {
                      onChange(c.id);
                      setOpen(false);
                      setSearch("");
                    }}
                    className={`flex-1 flex items-center gap-1.5 px-1.5 py-1.5 rounded-lg text-xs transition cursor-pointer min-w-0 ${
                      isSelected
                        ? "bg-purple-600/20 text-white"
                        : "text-slate-300 hover:text-white"
                    }`}
                  >
                    <span className="truncate">
                      {c.data?.icon ? `${c.data.icon} ` : ""}{c.name}
                    </span>
                  </button>
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
