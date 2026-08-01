import React, { useState, useEffect } from "react";
import { X, Save, Clock, CalendarClock, History, ArrowRight } from "lucide-react";
import {
  TaskItem,
  TaskLog,
  TaskMoveRecord,
  TaskPriority,
  PRIORITY_META,
  tasksApi,
  todayStr,
  addDays,
  humanDate,
  formatMinutes,
} from "./types";

// ─── 通用弹窗外壳 ───

function Modal({
  title,
  icon,
  onClose,
  children,
  wide,
}: {
  title: string;
  icon: React.ReactNode;
  onClose: () => void;
  children: React.ReactNode;
  wide?: boolean;
}) {
  return (
    <div
      className="fixed inset-0 z-[100] bg-black/60 backdrop-blur-sm flex items-center justify-center p-6"
      onClick={onClose}
    >
      <div
        className={`w-full ${wide ? "max-w-2xl" : "max-w-md"} max-h-[85vh] overflow-y-auto rounded-xl bg-[#0e1220] border border-amber-500/20 shadow-2xl`}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="sticky top-0 bg-[#0e1220] flex items-center justify-between px-4 py-3 border-b border-white/5">
          <h4 className="text-xs font-bold text-amber-300 flex items-center gap-1.5">
            {icon} {title}
          </h4>
          <button onClick={onClose} className="text-slate-500 hover:text-slate-300 cursor-pointer">
            <X className="w-4 h-4" />
          </button>
        </div>
        <div className="p-4 space-y-3">{children}</div>
      </div>
    </div>
  );
}

const inputCls =
  "w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 focus:outline-none focus:border-amber-500";
const labelCls = "text-[9px] text-slate-500 block mb-1";
const primaryBtn =
  "px-3 py-1.5 rounded-lg bg-amber-500 hover:bg-amber-400 text-slate-900 text-[10px] font-bold cursor-pointer transition-all flex items-center gap-1 disabled:opacity-50 disabled:cursor-not-allowed";
const ghostBtn =
  "px-3 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-400 hover:text-white cursor-pointer transition-all";

// ─── 新建 / 编辑任务 ───

export function TaskEditModal({
  task,
  defaultDate,
  onClose,
  onSaved,
}: {
  task: TaskItem | null;
  defaultDate: string | null;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [title, setTitle] = useState(task?.title ?? "");
  const [description, setDescription] = useState(task?.description ?? "");
  const [scheduledDate, setScheduledDate] = useState<string>(
    task ? task.scheduledDate ?? "" : defaultDate ?? ""
  );
  const [priority, setPriority] = useState<TaskPriority>(task?.priority ?? "medium");
  const [estimate, setEstimate] = useState(String(task?.estimateMinutes ?? 0));
  const [tags, setTags] = useState(task?.tags ?? "");
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState("");

  const save = async () => {
    if (!title.trim()) {
      setErr("请填写任务标题");
      return;
    }
    setSaving(true);
    setErr("");
    try {
      const payload = {
        title: title.trim(),
        description,
        scheduledDate: scheduledDate || null,
        priority,
        estimateMinutes: Number(estimate) || 0,
        tags,
      };
      if (task) {
        await tasksApi.update(task.id, payload);
      } else {
        await tasksApi.create(payload);
      }
      onSaved();
      onClose();
    } catch (e) {
      setErr(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      title={task ? "编辑任务" : "新建任务"}
      icon={<Save className="w-3.5 h-3.5" />}
      onClose={onClose}
    >
      <div>
        <label className={labelCls}>标题</label>
        <input
          autoFocus
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && !e.shiftKey && save()}
          placeholder="今天要完成什么？"
          className={inputCls}
        />
      </div>
      <div>
        <label className={labelCls}>描述（可选）</label>
        <textarea
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          rows={3}
          placeholder="补充背景、验收标准…"
          className={`${inputCls} resize-none`}
        />
      </div>
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className={labelCls}>计划日期（留空=收集箱）</label>
          <input
            type="date"
            value={scheduledDate}
            onChange={(e) => setScheduledDate(e.target.value)}
            className={inputCls}
          />
        </div>
        <div>
          <label className={labelCls}>优先级</label>
          <select
            value={priority}
            onChange={(e) => setPriority(e.target.value as TaskPriority)}
            className={inputCls}
          >
            {(["urgent", "high", "medium", "low"] as TaskPriority[]).map((p) => (
              <option key={p} value={p}>
                {PRIORITY_META[p].label}
              </option>
            ))}
          </select>
        </div>
        <div>
          <label className={labelCls}>预计耗时（分钟）</label>
          <input
            type="number"
            min={0}
            value={estimate}
            onChange={(e) => setEstimate(e.target.value)}
            className={inputCls}
          />
        </div>
        <div>
          <label className={labelCls}>标签（逗号分隔）</label>
          <input
            value={tags}
            onChange={(e) => setTags(e.target.value)}
            placeholder="工作,重要"
            className={inputCls}
          />
        </div>
      </div>
      {err && <p className="text-[10px] text-red-400">{err}</p>}
      <div className="flex justify-end gap-2 pt-1">
        <button onClick={onClose} className={ghostBtn}>
          取消
        </button>
        <button onClick={save} disabled={saving} className={primaryBtn}>
          <Save className="w-3 h-3" /> {saving ? "保存中…" : "保存"}
        </button>
      </div>
    </Modal>
  );
}

// ─── 更新进度 + 写复盘日志 ───

export function TaskProgressModal({
  task,
  onClose,
  onSaved,
}: {
  task: TaskItem;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [progress, setProgress] = useState(task.progress);
  const [content, setContent] = useState("");
  const [minutes, setMinutes] = useState("0");
  const [carry, setCarry] = useState(false);
  const [carryDate, setCarryDate] = useState(addDays(todayStr(), 1));
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState("");

  const save = async () => {
    setSaving(true);
    setErr("");
    try {
      await tasksApi.setProgress(task.id, {
        progress,
        logContent: content,
        minutesSpent: Number(minutes) || 0,
        // 仅未完成时才允许结转
        carryToDate: carry && progress < 100 ? carryDate : null,
        moveReason: carry ? "今日未完成，顺延" : "",
      });
      onSaved();
      onClose();
    } catch (e) {
      setErr(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal title="更新进度" icon={<Clock className="w-3.5 h-3.5" />} onClose={onClose}>
      <p className="text-[11px] text-slate-300 font-semibold">{task.title}</p>

      <div>
        <label className={labelCls}>
          完成度：<span className="text-amber-300 font-mono font-bold">{progress}%</span>
          {progress >= 100 && <span className="text-emerald-400 ml-1">（将标记为已完成）</span>}
        </label>
        <input
          type="range"
          min={0}
          max={100}
          step={5}
          value={progress}
          onChange={(e) => setProgress(Number(e.target.value))}
          className="w-full accent-amber-500 cursor-pointer"
        />
        <div className="flex gap-1 mt-1.5">
          {[0, 25, 50, 75, 100].map((p) => (
            <button
              key={p}
              onClick={() => setProgress(p)}
              className={`flex-1 py-1 rounded text-[9px] font-semibold cursor-pointer transition-all ${
                progress === p
                  ? "bg-amber-500 text-slate-900"
                  : "bg-white/5 text-slate-400 hover:bg-white/10"
              }`}
            >
              {p}%
            </button>
          ))}
        </div>
      </div>

      <div>
        <label className={labelCls}>今日进展 / 复盘（可选）</label>
        <textarea
          value={content}
          onChange={(e) => setContent(e.target.value)}
          rows={3}
          placeholder="做了什么？卡在哪里？下一步…"
          className={`${inputCls} resize-none`}
        />
      </div>

      <div>
        <label className={labelCls}>实际投入（分钟）</label>
        <input
          type="number"
          min={0}
          value={minutes}
          onChange={(e) => setMinutes(e.target.value)}
          className={inputCls}
        />
      </div>

      {progress < 100 && (
        <div className="p-2.5 rounded-lg bg-white/5 border border-white/10 space-y-2">
          <label className="flex items-center gap-2 text-[10px] text-slate-300 cursor-pointer">
            <input
              type="checkbox"
              checked={carry}
              onChange={(e) => setCarry(e.target.checked)}
              className="accent-amber-500"
            />
            未完成，顺延到
          </label>
          {carry && (
            <input
              type="date"
              value={carryDate}
              onChange={(e) => setCarryDate(e.target.value)}
              className={inputCls}
            />
          )}
        </div>
      )}

      {err && <p className="text-[10px] text-red-400">{err}</p>}
      <div className="flex justify-end gap-2 pt-1">
        <button onClick={onClose} className={ghostBtn}>
          取消
        </button>
        <button onClick={save} disabled={saving} className={primaryBtn}>
          <Save className="w-3 h-3" /> {saving ? "保存中…" : "保存"}
        </button>
      </div>
    </Modal>
  );
}

// ─── 改期 / 结转 ───

export function TaskMoveModal({
  task,
  onClose,
  onSaved,
}: {
  task: TaskItem;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [toDate, setToDate] = useState(task.scheduledDate ?? todayStr());
  const [reason, setReason] = useState("");
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState("");

  const quick = [
    { label: "今天", date: todayStr() },
    { label: "明天", date: addDays(todayStr(), 1) },
    { label: "后天", date: addDays(todayStr(), 2) },
    { label: "下周", date: addDays(todayStr(), 7) },
  ];

  const save = async (target: string | null) => {
    setSaving(true);
    setErr("");
    try {
      await tasksApi.move(task.id, { toDate: target, reason });
      onSaved();
      onClose();
    } catch (e) {
      setErr(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal title="改期 / 结转" icon={<CalendarClock className="w-3.5 h-3.5" />} onClose={onClose}>
      <p className="text-[11px] text-slate-300 font-semibold">{task.title}</p>
      <p className="text-[10px] text-slate-500">
        当前计划：{task.scheduledDate ? humanDate(task.scheduledDate) : "收集箱（未排期）"}
      </p>

      <div className="flex gap-1">
        {quick.map((q) => (
          <button
            key={q.label}
            onClick={() => setToDate(q.date)}
            className={`flex-1 py-1.5 rounded text-[9px] font-semibold cursor-pointer transition-all ${
              toDate === q.date
                ? "bg-amber-500 text-slate-900"
                : "bg-white/5 text-slate-400 hover:bg-white/10"
            }`}
          >
            {q.label}
          </button>
        ))}
      </div>

      <div>
        <label className={labelCls}>目标日期</label>
        <input
          type="date"
          value={toDate}
          onChange={(e) => setToDate(e.target.value)}
          className={inputCls}
        />
      </div>
      <div>
        <label className={labelCls}>原因（便于复盘为何一再顺延）</label>
        <input
          value={reason}
          onChange={(e) => setReason(e.target.value)}
          placeholder="如：被临时插入的需求打断"
          className={inputCls}
        />
      </div>

      {err && <p className="text-[10px] text-red-400">{err}</p>}
      <div className="flex justify-between gap-2 pt-1">
        <button onClick={() => save(null)} disabled={saving} className={ghostBtn}>
          移入收集箱
        </button>
        <div className="flex gap-2">
          <button onClick={onClose} className={ghostBtn}>
            取消
          </button>
          <button onClick={() => save(toDate)} disabled={saving} className={primaryBtn}>
            <ArrowRight className="w-3 h-3" /> 确认改期
          </button>
        </div>
      </div>
    </Modal>
  );
}

// ─── 历史记录（日志 + 转移轨迹） ───

export function TaskHistoryModal({ task, onClose }: { task: TaskItem; onClose: () => void }) {
  const [logs, setLogs] = useState<TaskLog[]>([]);
  const [moves, setMoves] = useState<TaskMoveRecord[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    (async () => {
      try {
        const [l, m] = await Promise.all([
          tasksApi.listLogs(task.id),
          tasksApi.listMoves(task.id),
        ]);
        setLogs(l);
        setMoves(m);
      } catch (e) {
        console.error(e);
      } finally {
        setLoading(false);
      }
    })();
  }, [task.id]);

  return (
    <Modal
      title="历史记录"
      icon={<History className="w-3.5 h-3.5" />}
      onClose={onClose}
      wide
    >
      <p className="text-[11px] text-slate-300 font-semibold">{task.title}</p>

      {loading ? (
        <p className="text-[10px] text-slate-500">加载中…</p>
      ) : (
        <div className="grid grid-cols-2 gap-4">
          {/* 复盘日志 */}
          <div className="space-y-2">
            <h5 className="text-[10px] font-bold text-amber-300">复盘日志（{logs.length}）</h5>
            {logs.length === 0 && <p className="text-[10px] text-slate-600">暂无日志</p>}
            {logs.map((l) => (
              <div key={l.id} className="p-2 rounded-lg bg-slate-900/50 border border-white/5">
                <div className="flex items-center justify-between">
                  <span className="text-[9px] text-slate-500">{l.logDate}</span>
                  <span className="text-[9px] font-mono text-amber-300">
                    {l.progressBefore}% → {l.progressAfter}%
                  </span>
                </div>
                {l.content && (
                  <p className="text-[10px] text-slate-300 mt-1 whitespace-pre-wrap">{l.content}</p>
                )}
                {l.minutesSpent > 0 && (
                  <p className="text-[9px] text-slate-500 mt-1">
                    投入 {formatMinutes(l.minutesSpent)}
                  </p>
                )}
              </div>
            ))}
          </div>

          {/* 转移轨迹 */}
          <div className="space-y-2">
            <h5 className="text-[10px] font-bold text-amber-300">变更轨迹（{moves.length}）</h5>
            {moves.length === 0 && <p className="text-[10px] text-slate-600">暂无变更</p>}
            {moves.map((m) => (
              <div key={m.id} className="p-2 rounded-lg bg-slate-900/50 border border-white/5">
                <div className="flex items-center gap-1.5 text-[9px] text-slate-400">
                  <span>{m.fromDate ?? "收集箱"}</span>
                  <ArrowRight className="w-2.5 h-2.5 text-amber-400" />
                  <span className="text-slate-200">{m.toDate ?? "收集箱"}</span>
                  <span className="ml-auto font-mono text-slate-500">
                    {m.fromProgress}%→{m.toProgress}%
                  </span>
                </div>
                {m.reason && <p className="text-[10px] text-slate-400 mt-1">{m.reason}</p>}
                <p className="text-[9px] text-slate-600 mt-0.5">{m.movedAt.replace("T", " ")}</p>
              </div>
            ))}
          </div>
        </div>
      )}
    </Modal>
  );
}
