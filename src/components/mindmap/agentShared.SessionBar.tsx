import { useEffect, useState } from "react";
import { MessageCircle, Pencil, Plus, Trash2 } from "lucide-react";
import type { AgentSessionRow } from "./types";
import { shortTs, wbSelect } from "./agentShared.state";
/** 会话条：切换 / 新建 / 重命名 / 删除（仅 chat 模式：会话按文档持久化在后端） */
export function SessionBar({
  sessions, sessionId, loading, t, onSelect, onNew, onRename, onDelete,
}: {
  sessions: AgentSessionRow[];
  sessionId: string;
  loading: boolean;
  t: (k: string, o?: any) => string;
  onSelect: (id: string) => void;
  onNew: () => void;
  onRename: (id: string, title: string) => void;
  onDelete: (id: string) => void;
}) {
  const [renaming, setRenaming] = useState(false);
  const [draft, setDraft] = useState("");
  const [confirmDel, setConfirmDel] = useState(false);
  const cur = sessions.find((s) => s.id === sessionId) ?? null;

  // 切换会话后收起编辑态与删除确认，避免误伤刚切过去的会话
  useEffect(() => { setRenaming(false); setConfirmDel(false); }, [sessionId]);
  // 删除确认 3 秒后自动失效（两段式确认，不弹原生对话框）
  useEffect(() => {
    if (!confirmDel) return;
    const id = window.setTimeout(() => setConfirmDel(false), 3000);
    return () => window.clearTimeout(id);
  }, [confirmDel]);

  const labelOf = (s: AgentSessionRow) =>
    (s.title && s.title.trim()) ? s.title.trim() : `${t("agent.sessionUntitled")} ${shortTs(s.updatedAt)}`;

  return (
    <div className="flex shrink-0 items-center gap-1 rounded-ctl border border-white/10 bg-slate-950/70 px-2 py-1">
      <MessageCircle className="h-3 w-3 shrink-0 text-slate-500" />
      {renaming ? (
        <input
          autoFocus
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") { onRename(sessionId, draft.trim()); setRenaming(false); }
            else if (e.key === "Escape") { setRenaming(false); }
          }}
          onBlur={() => setRenaming(false)}
          placeholder={t("agent.sessionNamePh")}
          className="min-w-0 flex-1 rounded border border-cyan-400/40 bg-slate-950 px-1.5 py-0.5 text-tiny text-slate-200 outline-none"
        />
      ) : (
        <select
          value={sessionId}
          onChange={(e) => onSelect(e.target.value)}
          disabled={loading || sessions.length === 0}
          className={`${wbSelect} min-w-0 flex-1 border-0 bg-transparent px-1`}
          title={t("agent.sessionSwitchTip")}>
          {sessions.length === 0 && <option value="">{t("agent.sessionNone")}</option>}
          {sessions.map((s) => (
            <option key={s.id} value={s.id}>
              {labelOf(s)}{s.parentId ? t("agent.sessionForkMark") : ""}（{s.messageCount}）
            </option>
          ))}
        </select>
      )}
      <button type="button" onClick={onNew} disabled={loading}
        className="shrink-0 cursor-pointer rounded border border-white/10 bg-white/[0.04] p-0.5 text-slate-400 transition hover:text-white disabled:opacity-40"
        title={t("agent.newSessionTip")}>
        <Plus className="h-3 w-3" />
      </button>
      <button type="button" onClick={() => { setDraft(cur?.title ?? ""); setRenaming(true); }} disabled={loading || !sessionId}
        className="shrink-0 cursor-pointer rounded border border-white/10 bg-white/[0.04] p-0.5 text-slate-400 transition hover:text-white disabled:opacity-40"
        title={t("agent.renameSession")}>
        <Pencil className="h-3 w-3" />
      </button>
      <button type="button"
        onClick={() => { if (confirmDel) { onDelete(sessionId); setConfirmDel(false); } else setConfirmDel(true); }}
        disabled={loading || !sessionId || sessions.length <= 1}
        className={`shrink-0 cursor-pointer rounded border p-0.5 transition disabled:opacity-40 ${confirmDel ? "border-red-400/60 bg-red-500/15 text-red-300" : "border-white/10 bg-white/[0.04] text-slate-400 hover:text-white"}`}
        title={confirmDel ? t("agent.deleteSessionConfirm") : t("agent.deleteSession")}>
        <Trash2 className="h-3 w-3" />
      </button>
    </div>
  );
}
