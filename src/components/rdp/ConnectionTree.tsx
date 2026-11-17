import { useState, type ReactNode } from "react";
import { ChevronDown, ChevronRight, Folder, FolderOpen, FolderPlus, Pencil, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { buildTree, normalizePath, type FolderNode } from "./groupTree";

/** 树里只需要这些字段；RdpConnection 结构兼容（多余字段忽略）。 */
export interface TreeConn {
  id: string;
  name: string;
  group: string;
  kind: string;
  host: string;
  port: number;
  username: string;
  note?: string;
}

const DND_CONN = "conn:";
const DND_FOLDER = "folder:";

/**
 * 连接列表 = 文件夹树。
 *
 * 分组就是文件夹，连接自身不维护「分组字段」这一层概念：界面上只通过拖拽
 * 改路径（`prod/web` 这种存在 `group` 里）。空文件夹由后端单独存路径清单，
 * 因此这里要同时接收 `folders` 与 `connections`。
 */
export function ConnectionTree({ connections, folders, onMoveConnection, onMoveFolder, onRenameFolder, onDeleteFolder, onCreateFolder, renderActions, emptyText }: {
  connections: TreeConn[];
  folders: string[];
  onMoveConnection: (connectionId: string, folder: string) => void;
  /** 把 from 文件夹整体移到 to 下（to 为空 = 移到根） */
  onMoveFolder: (from: string, to: string) => void;
  onRenameFolder: (from: string, to: string) => void;
  onDeleteFolder: (path: string) => void;
  onCreateFolder: (path: string) => void;
  renderActions: (conn: TreeConn) => ReactNode;
  emptyText: string;
}) {
  const { t } = useTranslation();
  const tree = buildTree(connections, folders);
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [dropTarget, setDropTarget] = useState<string | null>(null);

  const toggle = (path: string) => setCollapsed((prev) => ({ ...prev, [path]: !prev[path] }));

  const handleDrop = (target: string, e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setDropTarget(null);
    const payload = e.dataTransfer.getData("text/plain");
    if (payload.startsWith(DND_CONN)) {
      onMoveConnection(payload.slice(DND_CONN.length), target);
    } else if (payload.startsWith(DND_FOLDER)) {
      const from = normalizePath(payload.slice(DND_FOLDER.length));
      // 不能把文件夹拖进自己或自己的后代里，否则路径会自我嵌套
      if (!target || target === from || target.startsWith(`${from}/`)) return;
      const leaf = from.split("/").pop() ?? from;
      onMoveFolder(from, target ? `${target}/${leaf}` : leaf);
    }
  };

  const dropProps = (target: string) => ({
    onDragOver: (e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setDropTarget(target);
    },
    onDragLeave: () => setDropTarget((prev) => (prev === target ? null : prev)),
    onDrop: (e: React.DragEvent) => handleDrop(target, e),
  });

  const commitCreate = () => {
    const name = newName.trim();
    setCreating(false);
    setNewName("");
    if (name) onCreateFolder(name);
  };

  const commitRename = (from: string) => {
    const value = draft.trim();
    setEditing(null);
    if (value && value !== from) {
      // 只改最后一段，保持它在原来的父级下
      const parent = from.split("/").slice(0, -1).join("/");
      onRenameFolder(from, parent ? `${parent}/${value}` : value);
    }
  };

  const renderConn = (conn: TreeConn, depth: number) => (
    <div
      key={conn.id}
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData("text/plain", `${DND_CONN}${conn.id}`);
        e.dataTransfer.effectAllowed = "move";
      }}
      className="flex items-center gap-2 border-t border-white/5 py-1.5 pr-2 hover:bg-white/5"
      style={{ paddingLeft: 26 + depth * 14 }}
    >
      <span className="w-14 shrink-0 text-micro text-slate-500">
        {conn.kind === "ssh" ? t("rdp.kindSshShort") : t("rdp.kindRdpShort")}
      </span>
      <span className="min-w-0 flex-1" title={conn.note || undefined}>
        <span className="block truncate font-medium text-slate-200">{conn.name}</span>
        <span className="block truncate text-micro text-slate-500">
          {conn.username ? `${conn.username}@` : ""}{conn.host}:{conn.port}
        </span>
      </span>
      <span className="flex shrink-0 gap-1">{renderActions(conn)}</span>
    </div>
  );

  const renderNode = (node: FolderNode<TreeConn>, depth: number) => {
    if (node.path === "") {
      // 根节点：不渲染自身，只渲染子级，同时作为「拖到根」的接收区
      return (
        <div {...dropProps("")} className={dropTarget === "" ? "ring-1 ring-inset ring-sky-400/40" : ""}>
          {creating && (
            <div className="flex items-center gap-1 border-t border-white/5 px-3 py-1.5" style={{ paddingLeft: 12 }}>
              <FolderPlus className="h-3.5 w-3.5 text-sky-400" />
              <input
                autoFocus
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                onBlur={commitCreate}
                onKeyDown={(e) => {
                  if (e.key === "Enter") commitCreate();
                  if (e.key === "Escape") { setCreating(false); setNewName(""); }
                }}
                placeholder={t("folders.namePlaceholder")}
                className="min-w-0 flex-1 rounded bg-black/40 px-1.5 py-0.5 text-tiny text-slate-100 outline-none ring-1 ring-sky-400"
              />
            </div>
          )}
          {node.folders.map((child) => renderNode(child, 0))}
          {node.items.map((conn) => renderConn(conn, 0))}
          {node.folders.length === 0 && node.items.length === 0 && !creating && (
            <div className="px-3 py-8 text-center text-slate-500">{emptyText}</div>
          )}
        </div>
      );
    }

    const isCollapsed = collapsed[node.path] ?? false;
    return (
      <div key={node.path}>
        <div
          {...dropProps(node.path)}
          className={`group flex items-center gap-1 border-t border-white/5 py-1.5 pr-2 hover:bg-white/5 ${dropTarget === node.path ? "bg-sky-500/10 ring-1 ring-inset ring-sky-400" : ""}`}
          style={{ paddingLeft: 8 + depth * 14 }}
          draggable
          onDragStart={(e) => {
            e.dataTransfer.setData("text/plain", `${DND_FOLDER}${node.path}`);
            e.dataTransfer.effectAllowed = "move";
          }}
        >
          <button className="p-0.5 text-slate-500 hover:text-slate-200" onClick={() => toggle(node.path)}>
            {isCollapsed ? <ChevronRight className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
          </button>
          {isCollapsed ? <Folder className="h-3.5 w-3.5 text-amber-300/80" /> : <FolderOpen className="h-3.5 w-3.5 text-amber-300/80" />}
          {editing === node.path ? (
            <input
              autoFocus
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={() => commitRename(node.path)}
              onKeyDown={(e) => {
                if (e.key === "Enter") commitRename(node.path);
                if (e.key === "Escape") setEditing(null);
              }}
              className="min-w-0 flex-1 rounded bg-black/40 px-1.5 py-0.5 text-tiny text-slate-100 outline-none ring-1 ring-sky-400"
            />
          ) : (
            <span className="min-w-0 flex-1 truncate text-tiny text-slate-200">{node.name}</span>
          )}
          <span className="text-micro text-slate-500">{node.items.length}</span>
          <span className="flex shrink-0 gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
            <button
              className="rounded p-0.5 text-slate-500 hover:text-slate-200"
              title={t("folders.rename")}
              onClick={() => { setEditing(node.path); setDraft(node.name); }}
            >
              <Pencil className="h-3 w-3" />
            </button>
            <button
              className="rounded p-0.5 text-slate-500 hover:text-rose-300"
              title={t("folders.remove")}
              onClick={() => onDeleteFolder(node.path)}
            >
              <Trash2 className="h-3 w-3" />
            </button>
          </span>
        </div>
        {!isCollapsed && (
          <>
            {node.folders.map((child) => renderNode(child, depth + 1))}
            {node.items.map((conn) => renderConn(conn, depth + 1))}
          </>
        )}
      </div>
    );
  };

  return (
    <div className="min-h-0 flex-1 overflow-auto rounded border border-white/10">
      <div className="sticky top-0 z-10 flex items-center gap-2 border-b border-white/10 bg-slate-900/95 px-2 py-1.5">
        <button
          className="flex items-center gap-1 rounded px-1.5 py-0.5 text-tiny text-slate-300 hover:bg-white/10"
          onClick={() => { setCreating(true); setNewName(""); }}
        >
          <FolderPlus className="h-3.5 w-3.5" />
          {t("folders.create")}
        </button>
        <span className="text-micro text-slate-500">{t("folders.hint")}</span>
      </div>
      {renderNode(tree, 0)}
    </div>
  );
}
