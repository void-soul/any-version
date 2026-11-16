import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { FolderPlus, Pencil, Play, Plus, Trash2, X } from "lucide-react";

import { reportInvokeError } from "../../utils/invokeOrReport";

interface SnippetItem {
  id: string;
  name: string;
  /** 可多行；发送时逐行执行 */
  command: string;
}

interface SnippetFolder {
  id: string;
  name: string;
  items: SnippetItem[];
}

interface SnippetFile {
  version: number;
  folders: SnippetFolder[];
}

interface Editing {
  folderId: string;
  item: SnippetItem;
}

const btn = "inline-flex items-center gap-1.5 rounded px-2 py-1 text-micro border border-white/10 bg-black/30 text-slate-200 hover:bg-white/10 disabled:opacity-40";
const field = "w-full rounded border border-white/10 bg-black/30 px-2 py-1.5 text-tiny text-slate-100 outline-none focus:border-sky-500/60";

/**
 * FinalShell 式快捷命令：左侧目录、右侧命令卡片。
 * 点击卡片把命令发到当前终端（多行逐行执行）。
 */
export function SshSnippetPanel({ onRun }: { onRun: (command: string) => void }) {
  const { t } = useTranslation();
  const [data, setData] = useState<SnippetFile>({ version: 1, folders: [] });
  const [activeId, setActiveId] = useState("");
  const [editing, setEditing] = useState<Editing | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    let disposed = false;
    void invoke<SnippetFile>("ssh_snippets_load")
      .then((next) => {
        if (disposed) return;
        setData(next);
        setActiveId(next.folders[0]?.id ?? "");
      })
      .catch((e) => { if (!disposed) setError(reportInvokeError(e, "ssh_snippets_load")); });
    return () => { disposed = true; };
  }, []);

  const persist = useCallback(async (next: SnippetFile) => {
    setBusy(true);
    try {
      await invoke("ssh_snippets_save", { data: next });
      setData(next);
      setError("");
    } catch (e) {
      setError(reportInvokeError(e, "ssh_snippets_save"));
    } finally {
      setBusy(false);
    }
  }, []);

  const active = data.folders.find((folder) => folder.id === activeId) ?? data.folders[0];

  const addFolder = async () => {
    const name = window.prompt(t("snippets.folderName"));
    if (!name?.trim()) return;
    const folder: SnippetFolder = { id: crypto.randomUUID(), name: name.trim(), items: [] };
    const next = { ...data, folders: [...data.folders, folder] };
    await persist(next);
    setActiveId(folder.id);
  };

  const renameFolder = async (folder: SnippetFolder) => {
    const name = window.prompt(t("snippets.folderName"), folder.name);
    if (!name?.trim()) return;
    await persist({
      ...data,
      folders: data.folders.map((item) => (item.id === folder.id ? { ...item, name: name.trim() } : item)),
    });
  };

  const removeFolder = async (folder: SnippetFolder) => {
    if (!window.confirm(t("snippets.confirmDeleteFolder", { name: folder.name }))) return;
    await persist({ ...data, folders: data.folders.filter((item) => item.id !== folder.id) });
    if (activeId === folder.id) setActiveId(data.folders.find((item) => item.id !== folder.id)?.id ?? "");
  };

  const saveItem = async () => {
    if (!editing) return;
    const name = editing.item.name.trim();
    const command = editing.item.command.trim();
    if (!name || !command) return;
    const item: SnippetItem = {
      ...editing.item,
      id: editing.item.id || crypto.randomUUID(),
      name,
      command,
    };
    const next: SnippetFile = {
      ...data,
      folders: data.folders.map((folder) => {
        if (folder.id !== editing.folderId) return folder;
        const exists = folder.items.some((entry) => entry.id === item.id);
        return {
          ...folder,
          items: exists
            ? folder.items.map((entry) => (entry.id === item.id ? item : entry))
            : [...folder.items, item],
        };
      }),
    };
    await persist(next);
    setEditing(null);
  };

  const removeItem = async (folder: SnippetFolder, item: SnippetItem) => {
    if (!window.confirm(t("snippets.confirmDeleteItem", { name: item.name }))) return;
    await persist({
      ...data,
      folders: data.folders.map((entry) => (
        entry.id === folder.id
          ? { ...entry, items: entry.items.filter((candidate) => candidate.id !== item.id) }
          : entry
      )),
    });
  };

  return (
    <div className="flex h-full min-h-0 gap-2 text-slate-200">
      <aside className="flex w-40 shrink-0 flex-col rounded border border-white/10 bg-black/20 p-1">
        <div className="mb-1 flex items-center gap-1 px-1 text-micro text-slate-400">
          {t("snippets.title")}
          <button className={`${btn} ml-auto px-1`} title={t("snippets.newFolder")} onClick={() => void addFolder()}>
            <FolderPlus className="h-3 w-3" />
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-auto">
          {data.folders.map((folder) => (
            <div
              key={folder.id}
              className={`group flex items-center gap-1 rounded px-1 py-0.5 text-tiny cursor-pointer ${
                active?.id === folder.id ? "bg-sky-500/20 text-sky-200" : "text-slate-300 hover:bg-white/5"
              }`}
              onClick={() => setActiveId(folder.id)}
            >
              <span className="truncate">{folder.name}</span>
              <span className="ml-auto text-micro text-slate-500">{folder.items.length}</span>
              <button className="hidden text-slate-500 hover:text-slate-100 group-hover:block" onClick={(e) => { e.stopPropagation(); void renameFolder(folder); }}>
                <Pencil className="h-3 w-3" />
              </button>
              <button className="hidden text-slate-500 hover:text-rose-300 group-hover:block" onClick={(e) => { e.stopPropagation(); void removeFolder(folder); }}>
                <Trash2 className="h-3 w-3" />
              </button>
            </div>
          ))}
          {data.folders.length === 0 && (
            <div className="px-1 py-2 text-micro text-slate-500">{t("snippets.empty")}</div>
          )}
        </div>
      </aside>

      <section className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex items-center gap-2">
          <span className="text-tiny font-medium">{active?.name ?? t("snippets.title")}</span>
          <button
            className={btn}
            disabled={!active || busy}
            onClick={() => active && setEditing({ folderId: active.id, item: { id: "", name: "", command: "" } })}
          >
            <Plus className="h-3 w-3" />{t("snippets.newItem")}
          </button>
        </div>

        {error && <div className="rounded border border-rose-400/30 bg-rose-400/10 px-2 py-1 text-micro text-rose-200">{error}</div>}

        <div className="min-h-0 flex-1 overflow-auto">
          <div className="grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-1.5">
            {(active?.items ?? []).map((item) => (
              <div
                key={item.id}
                className="group flex flex-col rounded border border-white/10 bg-black/20 p-2 text-left hover:border-sky-500/40 hover:bg-sky-500/5"
              >
                <div className="flex items-center gap-1">
                  <button className="min-w-0 flex-1 truncate text-left text-tiny text-slate-200" title={item.command} onClick={() => onRun(item.command)}>
                    {item.name}
                  </button>
                  <button className="text-slate-500 hover:text-sky-300" title={t("snippets.run")} onClick={() => onRun(item.command)}>
                    <Play className="h-3 w-3" />
                  </button>
                  <button className="text-slate-500 hover:text-slate-100" title={t("snippets.edit")} onClick={() => setEditing({ folderId: active!.id, item: { ...item } })}>
                    <Pencil className="h-3 w-3" />
                  </button>
                  <button className="text-slate-500 hover:text-rose-300" title={t("snippets.delete")} onClick={() => active && void removeItem(active, item)}>
                    <Trash2 className="h-3 w-3" />
                  </button>
                </div>
                <div className="mt-1 truncate font-mono text-micro text-slate-500">{item.command.split(/\r?\n/)[0]}</div>
              </div>
            ))}
          </div>
          {(active?.items ?? []).length === 0 && (
            <div className="py-4 text-center text-micro text-slate-500">{t("snippets.empty")}</div>
          )}
        </div>
      </section>

      {editing && (
        <div className="fixed inset-0 z-[260] flex items-center justify-center bg-black/60 p-4" onClick={() => setEditing(null)}>
          <div className="w-[520px] max-w-full space-y-2 rounded border border-white/10 bg-slate-900 p-3" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center justify-between">
              <span className="text-caption font-semibold">{editing.item.id ? t("snippets.edit") : t("snippets.newItem")}</span>
              <button className={`${btn} px-1`} onClick={() => setEditing(null)}><X className="h-3.5 w-3.5" /></button>
            </div>
            <label className="block text-micro text-slate-400">
              {t("snippets.itemName")}
              <input
                className={`${field} mt-1`}
                value={editing.item.name}
                onChange={(e) => setEditing({ ...editing, item: { ...editing.item, name: e.target.value } })}
              />
            </label>
            <label className="block text-micro text-slate-400">
              {t("snippets.command")}
              <textarea
                className={`${field} mt-1 h-40 resize-y font-mono`}
                value={editing.item.command}
                onChange={(e) => setEditing({ ...editing, item: { ...editing.item, command: e.target.value } })}
              />
            </label>
            <div className="flex justify-end gap-2">
              <button className={btn} onClick={() => setEditing(null)}>{t("snippets.cancel")}</button>
              <button className={btn} disabled={busy || !editing.item.name.trim() || !editing.item.command.trim()} onClick={() => void saveItem()}>
                {t("snippets.save")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
