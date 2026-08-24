import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import { TaskFlowCanvas } from "../CanvasFlow";
import {
  Loader2,
  Network,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
  Search,
  Trash2,
  X,
} from "lucide-react";
import {
  TaskItem,
  TaskStatus,
  STATUS_META,
  UpdateTaskInput,
  tasksApi,
  deriveStatus,
} from "./types";
import { moduleAccent } from "../../utils/theme";

type Position = { x: number; y: number };
const inputClass = "w-full rounded-md border border-white/10 bg-slate-950/70 px-2 py-1.5 text-[10px] text-slate-200 outline-none focus:border-amber-400/60";
const button = "inline-flex items-center gap-1 rounded-md border border-white/10 bg-white/[0.05] px-2 py-1.5 text-[10px] text-slate-300 transition hover:bg-white/[0.1] hover:text-white disabled:opacity-40";
const iconButton = "inline-flex h-7 w-7 items-center justify-center rounded-md border border-white/10 bg-white/[0.04] text-slate-400 transition hover:bg-white/[0.1] hover:text-white disabled:opacity-40";

function statusOf(task: TaskItem): TaskStatus {
  return deriveStatus(task.progress);
}

function nodeColor(task: TaskItem): string {
  if (/^#[0-9a-f]{6}$/i.test(task.color)) return task.color;
  // 节点默认颜色 = 任务模块的主题色（跟随 --module-accent）
  return moduleAccent();
}

function formatTaskTime(value: string): string {
  const parsed = new Date(value.includes(" ") ? value.replace(" ", "T") : value);
  if (Number.isNaN(parsed.getTime())) return "未知";
  return new Intl.DateTimeFormat("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(parsed).replace(/\//g, "-");
}

function appendMarkdown(content: string, value: string): string {
  const base = content.trimEnd();
  return base ? `${base}\n\n${value}` : value;
}

function normalizedFilePath(path: string): string {
  return path.replace(/\\/g, "/");
}

function fileName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

function markdownFileLink(path: string): string {
  const normalized = normalizedFilePath(path);
  return `[${fileName(path)}](<${normalized}>)`;
}

function markdownImage(path: string): string {
  const normalized = normalizedFilePath(path);
  return `![${fileName(path)}](<${normalized}>)`;
}

export default function TaskPanel() {
  const [tasks, setTasks] = useState<TaskItem[]>([]);
  const [selectedSeries, setSelectedSeries] = useState<string | null>(null);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [statusFilter, setStatusFilter] = useState<TaskStatus | "all">("all");
  const [showTaskList, setShowTaskList] = useState(true);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [showCreate, setShowCreate] = useState(false);
  const [createParentId, setCreateParentId] = useState<string | null>(null);
  const [editing, setEditing] = useState<TaskItem | null>(null);
  const [draftTitle, setDraftTitle] = useState("");
  const [notice, setNotice] = useState("");
  const [deleteCandidate, setDeleteCandidate] = useState<TaskItem | null>(null);

  const flash = useCallback((message: string) => {
    setNotice(message);
    window.setTimeout(() => setNotice(""), 2200);
  }, []);

  const loadTasks = useCallback(async () => {
    setLoading(true);
    try {
      const result = await tasksApi.search("", false);
      setTasks(result);
      setSelectedSeries((current) => current && result.some((task) => task.id === current) ? current : null);
      setSelectedTaskId((current) => current && result.some((task) => task.id === current) ? current : null);
    } catch (error) {
      flash(`加载任务失败：${String(error)}`);
    } finally {
      setLoading(false);
    }
  }, [flash]);

  useEffect(() => { void loadTasks(); }, [loadTasks]);

  const updateTask = useCallback(async (id: string, patch: UpdateTaskInput): Promise<TaskItem | null> => {
    try {
      const updated = await tasksApi.update(id, patch);
      setTasks((current) => current.map((task) => task.id === id ? updated : task));
      return updated;
    } catch (error) {
      flash(`更新任务失败：${String(error)}`);
      return null;
    }
  }, [flash]);

  const savePosition = useCallback((id: string, position: Position) => {
    void updateTask(id, { positionX: Math.round(position.x), positionY: Math.round(position.y) });
  }, [updateTask]);

  const insertFile = useCallback(async (id: string, current: string) => {
    const selected = await openDialog({ multiple: false, directory: false, title: "插入文件路径" });
    if (typeof selected !== "string") return undefined;
    const next = appendMarkdown(current, markdownFileLink(selected));
    const updated = await updateTask(id, { description: next });
    return updated?.description ?? next;
  }, [updateTask]);

  const insertImage = useCallback(async (id: string, current: string) => {
    const selected = await openDialog({ multiple: false, directory: false, title: "插入图片", filters: [{ name: "图片", extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"] }] });
    if (typeof selected !== "string") return undefined;
    const next = appendMarkdown(current, markdownImage(selected));
    const updated = await updateTask(id, { description: next });
    return updated?.description ?? next;
  }, [updateTask]);

  const insertScreenshot = useCallback(async (id: string, current: string) => {
    try {
      const path = await invoke<string>("clipboard_save_latest_image_for_task");
      const next = appendMarkdown(current, markdownImage(path));
      const updated = await updateTask(id, { description: next });
      flash("截图已插入任务内容");
      return updated?.description ?? next;
    } catch (error) {
      flash(`插入截图失败：${String(error)}`);
      return undefined;
    }
  }, [flash, updateTask]);

  const openLocalFile = useCallback((path: string) => {
    void openPath(path);
  }, []);

  const connectTasks = useCallback(async (parentId: string, childId: string) => {
    if (parentId === childId) return;
    const byId = new globalThis.Map(tasks.map((task) => [task.id, task]));
    let current = byId.get(parentId);
    while (current?.parentId) {
      if (current.parentId === childId) {
        flash("不能连接为循环父子关系");
        return;
      }
      current = byId.get(current.parentId);
    }
    const child = byId.get(childId);
    if (!child || child.parentId === parentId) return;
    await updateTask(childId, { parentId });
  }, [flash, tasks, updateTask]);

  const setProgress = useCallback(async (task: TaskItem, progress: number) => {
    try {
      const updated = await tasksApi.setProgress(task.id, { progress });
      setTasks((current) => current.map((item) => item.id === updated.id ? updated : item));
    } catch (error) {
      flash(`更新状态失败：${String(error)}`);
    }
  }, [flash]);

  const deleteTask = useCallback(async (task: TaskItem) => {
    try {
      await tasksApi.remove(task.id);
      setTasks((current) => current.filter((item) => item.id !== task.id).map((item) => item.parentId === task.id ? { ...item, parentId: null } : item));
      if (selectedTaskId === task.id) setSelectedTaskId(null);
      if (selectedSeries === task.id) setSelectedSeries(null);
      setDeleteCandidate(null);
    } catch (error) {
      flash(`删除失败：${String(error)}`);
    }
  }, [flash, selectedSeries, selectedTaskId]);

  const saveTask = async () => {
    const title = draftTitle.trim();
    if (!title) return;
    setBusy(true);
    try {
      if (editing) {
        const updated = await tasksApi.update(editing.id, { title, parentId: editing.parentId });
        setTasks((current) => current.map((task) => task.id === updated.id ? updated : task));
      } else {
        const created = await tasksApi.create({ title, parentId: createParentId, color: moduleAccent(), description: "" });
        setTasks((current) => [...current, created]);
        setSelectedSeries(createParentId ? selectedSeries : created.id);
        setSelectedTaskId(created.id);
      }
      setShowCreate(false);
      setEditing(null);
      setDraftTitle("");
    } catch (error) {
      flash(`保存任务失败：${String(error)}`);
    } finally {
      setBusy(false);
    }
  };

  const filteredTasks = useMemo(() => {
    const keyword = search.trim().toLowerCase();
    return tasks.filter((task) => (!keyword || `${task.title} ${task.description}`.toLowerCase().includes(keyword)) && (statusFilter === "all" || statusOf(task) === statusFilter));
  }, [search, statusFilter, tasks]);
  const series = useMemo(() => filteredTasks.filter((task) => !task.parentId), [filteredTasks]);

  return (
    <div className="relative flex h-full min-h-0 flex-col bg-slate-950/25 text-slate-200">
      <header className="flex min-h-12 shrink-0 items-center gap-2 border-b border-white/10 px-3">
        <Network className="h-4 w-4 text-amber-300" /><span className="text-sm font-semibold text-white">任务画布</span>
        <button type="button" className={iconButton} onClick={() => setShowTaskList((visible) => !visible)} title={showTaskList ? "隐藏任务列表" : "显示任务列表"} aria-label={showTaskList ? "隐藏任务列表" : "显示任务列表"}>{showTaskList ? <PanelLeftClose className="h-3.5 w-3.5" /> : <PanelLeftOpen className="h-3.5 w-3.5" />}</button>
        <div className="ml-auto flex items-center gap-1.5"><div className="flex items-center gap-1 rounded-md border border-white/10 bg-white/[0.04] px-2"><Search className="h-3 w-3 text-slate-500" /><input value={search} onChange={(event) => setSearch(event.target.value)} placeholder="搜索任务" className="h-7 w-36 bg-transparent text-[11px] outline-none placeholder:text-slate-600" /></div><button type="button" className={button} onClick={() => { setEditing(null); setCreateParentId(null); setDraftTitle(""); setShowCreate(true); }}><Plus className="h-3 w-3" />新建系列</button></div>
      </header>
      <div className="flex min-h-0 flex-1">
        {showTaskList && <aside className="flex w-[220px] shrink-0 flex-col border-r border-white/10 bg-slate-950/30">
          <div className="flex items-center gap-1 border-b border-white/10 p-2">{(["all", "todo", "inProgress", "done"] as const).map((filter) => <button key={filter} type="button" onClick={() => setStatusFilter(filter)} className={`flex-1 rounded px-1 py-1.5 text-[9px] ${statusFilter === filter ? "bg-amber-400 text-slate-950" : "text-slate-500 hover:bg-white/[0.06]"}`}>{filter === "all" ? "全部" : STATUS_META[filter].label}</button>)}</div>
          <div className="min-h-0 flex-1 overflow-y-auto p-2">
            {loading ? <div className="flex justify-center py-10 text-slate-500"><Loader2 className="h-4 w-4 animate-spin" /></div> : series.map((task) => <div key={task.id} className={`mb-1.5 w-full rounded-md border px-2.5 py-2 text-left transition ${selectedSeries === task.id ? "border-[var(--module-accent-ring)] bg-[var(--module-accent-soft)]" : "border-white/10 bg-white/[0.02] hover:bg-white/[0.06]"}`}>
              <button type="button" onClick={() => { setSelectedSeries(task.id); setSelectedTaskId(task.id); }} className="w-full text-left">
                <span className="flex items-center gap-1.5"><span className="h-2 w-2 shrink-0 rounded-full" style={{ backgroundColor: nodeColor(task) }} /><span className="min-w-0 flex-1 truncate text-[11px] text-slate-200">{task.title}</span><span className="font-mono text-[9px] text-slate-500">{task.progress}%</span></span>
                <span className="mt-1.5 grid grid-cols-1 gap-0.5 text-[9px] leading-4 text-slate-500"><span>新增：{formatTaskTime(task.createdAt)}</span><span>修改：{formatTaskTime(task.updatedAt)}</span></span>
              </button>
              <div className="mt-2 flex items-center gap-1.5 border-t border-white/[0.06] pt-1.5">
                <select value={statusOf(task)} onChange={(event) => { const progress = event.target.value === "done" ? 100 : event.target.value === "inProgress" ? 50 : 0; void setProgress(task, progress); }} className="min-w-0 flex-1 rounded border border-white/10 bg-slate-950/70 px-1.5 py-1 text-[9px] text-slate-300 outline-none focus:border-[var(--module-accent-ring)]">
                  {(Object.keys(STATUS_META) as TaskStatus[]).map((status) => <option key={status} value={status}>{STATUS_META[status].label}</option>)}
                </select>
                <button type="button" className="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded border border-red-400/20 text-red-300/70 transition hover:border-red-400/50 hover:bg-red-400/10 hover:text-red-200" onClick={() => setDeleteCandidate(task)} title="删除任务" aria-label={`删除任务：${task.title}`}><Trash2 className="h-3 w-3" /></button>
              </div>
            </div>)}
            {!loading && series.length === 0 && <div className="py-10 text-center text-[10px] text-slate-600">没有匹配的系列</div>}
          </div>
        </aside>}
        <main className="relative min-w-0 flex-1">
          <TaskFlowCanvas tasks={tasks} selectedSeries={selectedSeries} selectedTaskId={selectedTaskId} onSelect={setSelectedTaskId} onAddChild={(id) => { setEditing(null); setCreateParentId(id); setDraftTitle(""); setShowCreate(true); }} onDelete={(task) => void deleteTask(task)} onProgress={(task, progress) => void setProgress(task, progress)} onUpdate={(id, patch) => void updateTask(id, patch)} onInsertFile={insertFile} onInsertImage={insertImage} onInsertScreenshot={insertScreenshot} onOpenFile={openLocalFile} onPositionChange={savePosition} onConnect={(parentId, childId) => void connectTasks(parentId, childId)} />
        </main>
      </div>
      {notice && <div className="absolute bottom-8 left-1/2 z-40 -translate-x-1/2 rounded-md border border-white/10 bg-slate-900 px-3 py-2 text-[11px] text-slate-200 shadow-xl">{notice}</div>}
      {showCreate && <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/60 p-4" onClick={() => setShowCreate(false)}><div className="w-[360px] rounded-lg border border-white/10 bg-[#101827] p-4 shadow-2xl" onClick={(event) => event.stopPropagation()}><div className="mb-3 flex items-center justify-between"><h3 className="text-sm font-semibold text-white">{editing ? "编辑任务" : createParentId ? "添加子任务" : "新建系列"}</h3><button type="button" className={iconButton} onClick={() => setShowCreate(false)}><X className="h-3.5 w-3.5" /></button></div><input autoFocus value={draftTitle} onChange={(event) => setDraftTitle(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") void saveTask(); }} placeholder="任务名称" className={inputClass} /><div className="mt-3 flex justify-end gap-2"><button type="button" className={button} onClick={() => setShowCreate(false)}>取消</button><button type="button" className="inline-flex items-center gap-1 rounded-md bg-[var(--module-accent)] px-3 py-1.5 text-[10px] font-semibold text-white" disabled={busy || !draftTitle.trim()} onClick={() => void saveTask()}>{busy && <Loader2 className="h-3 w-3 animate-spin" />}保存</button></div></div></div>}
      {deleteCandidate && <div className="fixed inset-0 z-[110] flex items-center justify-center bg-black/65 p-4 backdrop-blur-[2px]" onClick={() => setDeleteCandidate(null)}><div role="dialog" aria-modal="true" aria-labelledby="delete-task-title" className="w-full max-w-[380px] overflow-hidden rounded-xl border border-[var(--module-accent-ring)] bg-[#111827] shadow-2xl shadow-black/50" onClick={(event) => event.stopPropagation()}>
        <div className="border-b border-white/10 px-5 pb-4 pt-5"><div className="mb-3 flex h-10 w-10 items-center justify-center rounded-lg bg-red-400/10 text-red-300"><Trash2 className="h-5 w-5" /></div><h3 id="delete-task-title" className="text-sm font-semibold text-white">确认删除任务？</h3><p className="mt-1.5 text-[11px] leading-5 text-slate-400">将删除“<span className="font-medium text-slate-200">{deleteCandidate.title}</span>”。它的直接子任务会提升为顶层任务。</p></div>
        <div className="flex justify-end gap-2 bg-[var(--module-accent-soft)] px-5 py-3"><button type="button" className={button} onClick={() => setDeleteCandidate(null)}>取消</button><button type="button" className="inline-flex items-center gap-1 rounded-md bg-red-500 px-3 py-1.5 text-[10px] font-semibold text-white transition hover:bg-red-400" onClick={() => void deleteTask(deleteCandidate)}><Trash2 className="h-3 w-3" />确认删除</button></div>
      </div></div>}
    </div>
  );
}
