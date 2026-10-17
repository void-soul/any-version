// AI 助手侧栏里的「会话条」：常驻一行显示当前会话，点开是会话管理弹框。
//
// 为什么放在输入框旁边而不是设置里：**选错模型的代价是整轮重跑**（一跑几十秒），
// 同理「这份对话属于哪份设计」必须在发请求之前就能看清、能改。
//
// 一个会话 = 一个 id + 一份对话。切换会话会改当前文档的 `doc.id`（因此文档变脏），
// 这符合直觉 —— 换了个身份就该落盘。
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, MessageSquarePlus, Plus, Trash2 } from "lucide-react";

import { SharedModal } from "../shared/Modal";
import { theamedConfirm } from "../shared/ThemedAlert";
import { sessionTitle, type SessionEntry } from "./sessionKey";

interface Props {
  /** 当前文档绑着的会话 id */
  currentId: string;
  /** 当前文档路径（未保存是空串），用于显示当前会话名 */
  currentPath: string;
  /** 文档名，路径为空时的兜底标题 */
  currentTitle: string;
  sessions: SessionEntry[];
  /** 生成中禁用切换：那一轮的请求已经用旧模型发出去了 */
  busy: boolean;
  /** 切到某个会话（会改 doc.id 并读回它的对话） */
  onSelect: (id: string) => void;
  /** 新建一个会话并切过去（对话为空） */
  onCreate: () => void;
  /** 删除一个会话的缓存 */
  onDelete: (id: string) => void;
}

/** 相对时间：比「2026/10/17 15:04」更好读，且不需要额外的日期库 */
function relTime(ts: number): string {
  if (!ts) return "";
  const diff = Date.now() - ts;
  const min = Math.floor(diff / 60000);
  if (min < 1) return "";
  if (min < 60) return `${min}m`;
  const h = Math.floor(min / 60);
  if (h < 24) return `${h}h`;
  return `${Math.floor(h / 24)}d`;
}

export default function SessionBar(p: Props) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const fallbackTitle = p.currentTitle || t("dbd.sessionUntitled");
  const currentLabel = p.sessions.find((s) => s.id === p.currentId)?.title
    ?? sessionTitle(p.currentPath, fallbackTitle);
  // 最近活跃的排前面；当前会话置顶（用户是冲着它来的）
  const list = [...p.sessions].sort((a, b) => {
    if (a.id === p.currentId) return -1;
    if (b.id === p.currentId) return 1;
    return (b.updatedAt || 0) - (a.updatedAt || 0);
  });

  const askDelete = (s: SessionEntry) => {
    void theamedConfirm(t("dbd.sessionDeleteMsg", { title: s.title || t("dbd.sessionUntitled") }), {
      title: t("dbd.sessionDeleteTitle"),
      confirmText: t("dbd.sessionDeleteOk"),
      danger: true,
    }).then((ok) => {
      if (ok) p.onDelete(s.id);
    });
  };

  return (
    <>
      <div className="flex items-center gap-1.5">
        <button
          type="button"
          onClick={() => setOpen(true)}
          title={t("dbd.sessionManage")}
          className="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5 rounded-ctl border border-white/10 bg-white/[0.04] px-2 py-1 text-left transition hover:bg-white/[0.08]"
        >
          <MessageSquarePlus className="h-3 w-3 flex-shrink-0 text-slate-500" />
          <span className="shrink-0 text-micro text-slate-500">{t("dbd.session")}</span>
          <span className="min-w-0 flex-1 truncate text-tiny text-slate-200">{currentLabel}</span>
          <ChevronDown className="h-3 w-3 flex-shrink-0 text-slate-500" />
        </button>
        <button
          type="button"
          onClick={p.onCreate}
          disabled={p.busy}
          title={t("dbd.sessionNew")}
          className="flex h-[26px] w-[26px] flex-shrink-0 cursor-pointer items-center justify-center rounded-ctl border border-white/10 bg-white/[0.04] text-slate-400 transition hover:bg-white/[0.08] hover:text-slate-100 disabled:opacity-40"
        >
          <Plus className="h-3.5 w-3.5" />
        </button>
      </div>

      <SharedModal open={open} onClose={() => setOpen(false)} title={t("dbd.sessionManage")} width={480}>
        <div className="max-h-[420px] space-y-1 overflow-y-auto">
          {list.length === 0 ? (
            <p className="px-2 py-6 text-center text-tiny text-slate-500">{t("dbd.sessionEmpty")}</p>
          ) : null}
          {list.map((s) => {
            const isCurrent = s.id === p.currentId;
            return (
              <div
                key={s.id}
                className={`group flex items-center gap-2 rounded-ctl border px-2 py-1.5 ${
                  isCurrent ? "border-[var(--module-accent)]/50 bg-[var(--module-accent)]/10" : "border-transparent hover:bg-white/[0.04]"
                }`}
              >
                <button
                  type="button"
                  disabled={p.busy}
                  onClick={() => {
                    setOpen(false);
                    p.onSelect(s.id);
                  }}
                  className="min-w-0 flex-1 cursor-pointer text-left disabled:cursor-not-allowed disabled:opacity-60"
                >
                  <div className="flex items-center gap-1.5">
                    <span className="min-w-0 truncate text-tiny text-slate-200">
                      {s.title || t("dbd.sessionUntitled")}
                    </span>
                    {isCurrent ? (
                      <span className="flex-shrink-0 text-[9px] text-[var(--module-accent)]">
                        {t("dbd.sessionCurrent")}
                      </span>
                    ) : null}
                    {relTime(s.updatedAt) ? (
                      <span className="flex-shrink-0 text-[9px] text-slate-600">{relTime(s.updatedAt)}</span>
                    ) : null}
                  </div>
                  {s.path ? (
                    <div className="truncate text-[10px] text-slate-600" title={s.path}>{s.path}</div>
                  ) : null}
                </button>
                <button
                  type="button"
                  onClick={() => askDelete(s)}
                  disabled={p.busy}
                  title={t("dbd.sessionDelete")}
                  className="flex h-6 w-6 flex-shrink-0 cursor-pointer items-center justify-center rounded-ctl text-slate-600 transition hover:bg-white/[0.08] hover:text-red-300 disabled:cursor-not-allowed disabled:opacity-30"
                >
                  <Trash2 className="h-3 w-3" />
                </button>
              </div>
            );
          })}
        </div>
      </SharedModal>
    </>
  );
}
