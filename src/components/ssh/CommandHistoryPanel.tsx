import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { History, Trash2 } from "lucide-react";

const MAX_ITEMS = 100;
const MAX_LEN = 500;

function storageKey(connectionId: string): string {
  return `av.ssh.cmdhistory.${connectionId}`;
}

/** 读取某连接的命令缓存；脏数据整体丢弃，不让坏值卡住面板。 */
export function loadHistory(connectionId: string): string[] {
  try {
    const raw = JSON.parse(localStorage.getItem(storageKey(connectionId)) ?? "[]");
    if (!Array.isArray(raw)) return [];
    return raw.filter((v): v is string => typeof v === "string" && v.trim() !== "").slice(0, MAX_ITEMS);
  } catch {
    return [];
  }
}

/** 追加一条命令：去重后置顶，限制条数与单条长度。 */
export function pushHistory(connectionId: string, cmd: string): string[] {
  const text = cmd.trim();
  if (!text) return loadHistory(connectionId);
  const next = [text.slice(0, MAX_LEN), ...loadHistory(connectionId).filter((v) => v !== text)].slice(0, MAX_ITEMS);
  try { localStorage.setItem(storageKey(connectionId), JSON.stringify(next)); }
  catch { /* 隐私模式/配额满：仅本次会话可用 */ }
  return next;
}

export function clearHistory(connectionId: string): string[] {
  try { localStorage.removeItem(storageKey(connectionId)); }
  catch { /* 忽略：清理失败不影响内存中的列表 */ }
  return [];
}

/** 命令缓存：FinalShell 式「输入过的命令可一点重发」。 */
export function CommandHistoryPanel({ connectionId, items, onRun, onChanged }: {
  connectionId: string;
  items: string[];
  onRun: (cmd: string) => void;
  onChanged: (next: string[]) => void;
}) {
  const { t } = useTranslation();
  const [list, setList] = useState<string[]>(items);

  useEffect(() => { setList(items); }, [items]);

  const refresh = useCallback(() => setList(loadHistory(connectionId)), [connectionId]);
  useEffect(() => { refresh(); }, [refresh]);

  const clear = () => {
    const next = clearHistory(connectionId);
    setList(next);
    onChanged(next);
  };

  return (
    <section className="flex min-h-0 flex-1 flex-col rounded border border-white/10 p-2 text-tiny">
      <div className="mb-2 flex items-center gap-1.5 text-slate-400">
        <History className="h-3.5 w-3.5" />
        {t("cmdHistory.title")}
        <button className="ml-auto text-slate-500 hover:text-rose-300" title={t("cmdHistory.clear")} onClick={clear}>
          <Trash2 className="h-3 w-3" />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        {list.length === 0 ? (
          <div className="py-4 text-center text-micro text-slate-500">{t("cmdHistory.empty")}</div>
        ) : (
          <ul className="space-y-0.5">
            {list.map((cmd, i) => (
              <li key={`${i}-${cmd}`}>
                <button
                  className="w-full truncate rounded px-1 py-0.5 text-left font-mono text-micro text-slate-300 hover:bg-white/10 hover:text-slate-100"
                  title={t("cmdHistory.run", { cmd })}
                  onClick={() => onRun(cmd)}
                >
                  {cmd}
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
