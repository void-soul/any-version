import { ChevronDown, ChevronRight } from "lucide-react";
import { type FavoriteCategoryNode } from "./types";
/** 分类树（递归）：缩进表示层级，箭头展开/折叠，右键出操作菜单。
 *  操作逻辑对齐启动模块的分类：点选筛选（含子分类）、右键改名/新建子分类/上下移/删除。 */
export function CategoryTree({
  nodes, depth, expanded, selectedId, onToggle, onSelect, onContextMenu,
}: {
  nodes: FavoriteCategoryNode[];
  depth: number;
  expanded: Set<number>;
  selectedId: number | null;
  onToggle: (id: number) => void;
  onSelect: (id: number, name: string) => void;
  onContextMenu: (id: number, name: string, x: number, y: number) => void;
}) {
  return (
    <>
      {nodes.map((c) => {
        const hasKids = c.children.length > 0;
        const open = expanded.has(c.id);
        return (
          <div key={c.id}>
            <div
              className={`flex items-center gap-0.5 rounded text-caption cursor-pointer transition-colors ${
                selectedId === c.id
                  ? "bg-[var(--module-accent-soft)] text-white"
                  : "text-slate-400 hover:bg-white/5"
              }`}
              style={{ paddingLeft: 4 + depth * 10 }}
              onClick={() => onSelect(c.id, c.name)}
              onContextMenu={(e) => {
                e.preventDefault();
                onContextMenu(c.id, c.name, e.clientX, e.clientY);
              }}
              title={c.name}
            >
              <button
                onClick={(e) => { e.stopPropagation(); if (hasKids) onToggle(c.id); }}
                className={`w-3 shrink-0 text-slate-600 hover:text-slate-300 ${hasKids ? "cursor-pointer" : "invisible"}`}
              >
                {open ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
              </button>
              {/* 分组一视同仁：kind="follow" 的区分样式已随「关注分组」功能一起删除，
                历史遗留的 follow 分组也按普通分组显示（不替用户删数据）。 */}
              <span className="truncate flex-1 py-1" title={c.name}>
                {c.name}
              </span>
              <span className="pr-1 text-slate-500">{c.total}</span>
            </div>
            {open && hasKids && (
              <CategoryTree
                nodes={c.children}
                depth={depth + 1}
                expanded={expanded}
                selectedId={selectedId}
                onToggle={onToggle}
                onSelect={onSelect}
                onContextMenu={onContextMenu}
              />
            )}
          </div>
        );
      })}
    </>
  );
}
