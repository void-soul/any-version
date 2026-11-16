import { useCallback, useEffect, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronDown, ChevronRight, Folder, FolderOpen } from "lucide-react";

import { reportInvokeError } from "../../utils/invokeOrReport";

interface SftpEntry {
  name: string;
  path: string;
  isDir: boolean;
  isSymlink: boolean;
  size: number;
  mtime: number | null;
}

/**
 * SFTP 左侧目录树（FinalShell 式：左边目录、右边目录内容）。
 *
 * 只在展开某个节点时按需列目录，不递归扫描——大目录/慢链路下递归会卡住整棵树。
 */
export function SftpTree({ sessionId, cwd, onSelect }: {
  sessionId: string;
  cwd: string;
  onSelect: (path: string) => void;
}) {
  const [children, setChildren] = useState<Record<string, SftpEntry[]>>({});
  const [expanded, setExpanded] = useState<Record<string, boolean>>({ "/": true });
  const [loading, setLoading] = useState<Record<string, boolean>>({});
  const [error, setError] = useState("");

  const load = useCallback(async (path: string) => {
    if (!sessionId) return;
    setLoading((prev) => ({ ...prev, [path]: true }));
    try {
      const [, list] = await invoke<[string, SftpEntry[]]>("sftp_list", {
        sessionId,
        current: path,
        path: ".",
      });
      setChildren((prev) => ({ ...prev, [path]: list.filter((entry) => entry.isDir) }));
    } catch (e) {
      setError(reportInvokeError(e, "sftp_tree"));
    } finally {
      setLoading((prev) => ({ ...prev, [path]: false }));
    }
  }, [sessionId]);

  useEffect(() => {
    if (sessionId) void load("/");
  }, [sessionId, load]);

  const toggle = async (path: string) => {
    const next = !expanded[path];
    setExpanded((prev) => ({ ...prev, [path]: next }));
    if (next) await load(path);
  };

  const renderNode = (path: string, name: string, depth: number): ReactNode => {
    const isOpen = expanded[path] ?? false;
    const active = cwd === path;
    return (
      <div key={path}>
        <div
          className={`flex items-center gap-1 rounded px-1 py-0.5 text-tiny cursor-pointer ${
            active ? "bg-sky-500/20 text-sky-200" : "text-slate-300 hover:bg-white/5"
          }`}
          style={{ paddingLeft: 4 + depth * 12 }}
          onClick={() => onSelect(path)}
        >
          <button
            className="shrink-0 text-slate-500"
            aria-label={isOpen ? "collapse" : "expand"}
            onClick={(e) => {
              e.stopPropagation();
              void toggle(path);
            }}
          >
            {isOpen ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
          </button>
          {isOpen
            ? <FolderOpen className="h-3 w-3 shrink-0 text-sky-400" />
            : <Folder className="h-3 w-3 shrink-0 text-slate-500" />}
          <span className="truncate">{name}</span>
          {loading[path] && <span className="text-micro text-slate-500">…</span>}
        </div>
        {isOpen && (children[path] ?? []).map((child) => renderNode(child.path, child.name, depth + 1))}
      </div>
    );
  };

  return (
    <div className="flex w-[180px] shrink-0 flex-col overflow-auto rounded border border-white/10 bg-black/20 p-1">
      {error && <div className="p-1 text-micro text-rose-300">{error}</div>}
      {renderNode("/", "/", 0)}
    </div>
  );
}
