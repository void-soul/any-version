import { useState } from "react";
import { ChevronDown, ChevronRight, FolderTree } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { ConnectionGroup, GroupableConnection } from "./connectionGroups";

/** 分组树侧栏：点击分组筛选连接列表，未分组固定在最下方。 */
export function GroupSidebar<T extends GroupableConnection>({ groups, total, selected, collapsed, onSelect, onToggle, onMove }: {
  groups: ConnectionGroup<T>[];
  total: number;
  selected: string | null;
  collapsed: Record<string, boolean>;
  onSelect: (group: string | null) => void;
  onToggle: (key: string) => void;
  /** 把连接拖到某个分组（未分组传空字符串）。 */
  onMove?: (connectionId: string, group: string) => void;
}) {
  const { t } = useTranslation();
  const labelOf = (g: ConnectionGroup<T>) => (g.isUngrouped ? t("connGroups.ungrouped") : g.label);
  const [dragOver, setDragOver] = useState<string | null>(null);

  return (
    <aside className="w-48 shrink-0 overflow-auto rounded border border-white/10 p-2">
      <div className="mb-2 flex items-center gap-1.5 text-tiny text-slate-400">
        <FolderTree className="h-3.5 w-3.5" />
        {t("connGroups.title")}
      </div>
      <button
        className={`mb-1 flex w-full items-center justify-between rounded px-2 py-1 text-tiny hover:bg-white/10 ${selected === null ? "bg-sky-500/15 text-sky-200" : "text-slate-300"}`}
        onClick={() => onSelect(null)}
      >
        <span>{t("connGroups.all")}</span>
        <span className="text-micro text-slate-500">{total}</span>
      </button>
      {groups.map((g) => {
        const isCollapsed = collapsed[g.key] ?? false;
        const active = selected === g.key;
        return (
          <div key={g.key || "__ungrouped"}>
            <div
              className={`flex items-center rounded px-1 text-tiny ${active ? "bg-sky-500/15 text-sky-200" : "text-slate-300"} ${dragOver === g.key ? "ring-1 ring-sky-400" : ""}`}
              onDragOver={(e) => {
                if (!onMove) return;
                e.preventDefault();
                setDragOver(g.key);
              }}
              onDragLeave={() => setDragOver((prev) => (prev === g.key ? null : prev))}
              onDrop={(e) => {
                if (!onMove) return;
                e.preventDefault();
                setDragOver(null);
                const id = e.dataTransfer.getData("text/plain");
                if (id) onMove(id, g.key);
              }}
            >
              <button className="p-0.5 text-slate-500 hover:text-slate-200" onClick={() => onToggle(g.key)} title={isCollapsed ? t("connGroups.expand") : t("connGroups.collapse")}>
                {isCollapsed ? <ChevronRight className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
              </button>
              <button className="flex min-w-0 flex-1 items-center justify-between py-1 pr-1" onClick={() => onSelect(g.key)}>
                <span className="truncate">{labelOf(g)}</span>
                <span className="ml-1 text-micro text-slate-500">{g.items.length}</span>
              </button>
            </div>
            {!isCollapsed && (
              <ul className="mb-1 ml-4 border-l border-white/5 pl-2">
                {g.items.map((item) => (
                  <li
                    key={item.id}
                    draggable={Boolean(onMove)}
                    onDragStart={(e) => {
                      e.dataTransfer.setData("text/plain", item.id);
                      e.dataTransfer.effectAllowed = "move";
                    }}
                    className="truncate py-0.5 text-micro text-slate-400 hover:text-slate-200"
                    title={item.name}
                  >
                    {item.name}
                  </li>
                ))}
              </ul>
            )}
          </div>
        );
      })}
    </aside>
  );
}
