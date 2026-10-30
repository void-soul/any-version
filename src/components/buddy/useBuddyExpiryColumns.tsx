import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { Check, X } from "lucide-react";
import { COL_ACTIONS, COL_CREDITS_MIN, COL_INFO, COL_W, formatTimeInput, parseTimeInput, type BuddyAccount, type ExpiryColumn } from "./BuddyPanel.model";
import type { Dispatch, SetStateAction } from "react";

/** Owns the BuddyExpiryColumns feature, independently of page rendering. */
export function useBuddyExpiryColumns({ platform, showMsg, setAccounts }: {
  platform: string;
  showMsg: (ok: boolean, text: string) => void;
  setAccounts: Dispatch<SetStateAction<BuddyAccount[]>>;
}) {
  const { t } = useTranslation();

  // 排序模式（额度 / 时间1/2/3 / 默认 last_used 倒序）
  // 过期时间列（CodeBuddy CN 专用）：列 schema + 单元格编辑 + 列增删改 + 用量弹窗 + 倒计时刷新
  const [expiryColumns, setExpiryColumns] = useState<ExpiryColumn[]>([]);

  const [editingCell, setEditingCell] = useState<{ accId: string; colId: string; value: string } | null>(null);

  const [addingColumn, setAddingColumn] = useState(false);

  const [newColumnName, setNewColumnName] = useState("");

  const [editingColumnId, setEditingColumnId] = useState<string | null>(null);

  const [columnNameDraft, setColumnNameDraft] = useState("");

  const [colBusy, setColBusy] = useState(false);

  const [now, setNow] = useState(() => Date.now());

  // ─── 过期时间列（两平台共享）：列 schema 全局一份，同邮箱账号的时间值互通 ───
  const loadExpiryColumns = useCallback(async () => {
    try {
      const cols = await invoke<ExpiryColumn[]>("buddy_get_expiry_columns");
      setExpiryColumns(cols ?? []);
    } catch (e) {
      showMsg(false, String(e));
    }
  }, []);

  useEffect(() => {
    void loadExpiryColumns();
  }, [loadExpiryColumns]);

  const genColumnId = () =>
    `col_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 7)}`;

  const persistColumns = async (next: ExpiryColumn[]) => {
    setColBusy(true);
    try {
      const saved = await invoke<ExpiryColumn[]>("buddy_set_expiry_columns", { columns: next });
      setExpiryColumns(saved ?? next);
      return saved;
    } catch (e) {
      showMsg(false, String(e));
      return null;
    } finally {
      setColBusy(false);
    }
  };

  const addColumn = async () => {
    const name = newColumnName.trim();
    if (!name) return;
    const saved = await persistColumns([...expiryColumns, { id: genColumnId(), name }]);
    if (saved) {
      setAddingColumn(false);
      setNewColumnName("");
    }
  };

  const startRenameColumn = (col: ExpiryColumn) => {
    setEditingColumnId(col.id);
    setColumnNameDraft(col.name);
  };

  const commitRenameColumn = async (id: string) => {
    const name = columnNameDraft.trim();
    setEditingColumnId(null);
    if (!name) return;
    const cur = expiryColumns.find((c) => c.id === id);
    if (cur && cur.name === name) return;
    await persistColumns(expiryColumns.map((c) => (c.id === id ? { ...c, name } : c)));
  };

  const deleteColumn = async (id: string) => {
    const saved = await persistColumns(expiryColumns.filter((c) => c.id !== id));
    if (saved) {
      setAccounts((prev) =>
        prev.map((a) => {
          if (!a.expiryTimes || !(id in a.expiryTimes)) return a;
          const copy = { ...a.expiryTimes };
          delete copy[id];
          return { ...a, expiryTimes: copy };
        })
      );
      if (editingCell?.colId === id) setEditingCell(null);
    }
  };

  const startEditCell = (acc: BuddyAccount, colId: string) => {
    const ts = acc.expiryTimes?.[colId];
    setEditingCell({ accId: acc.id, colId, value: ts && ts > 0 ? formatTimeInput(ts) : "" });
  };

  const applyCellTime = async (acc: BuddyAccount, colId: string, ts: number | null) => {
    const next = { ...(acc.expiryTimes ?? {}) };
    if (ts == null) delete next[colId];
    else next[colId] = ts;
    try {
      const updated = await invoke<BuddyAccount>("buddy_set_expiry_times", {
        platform,
        accountId: acc.id,
        times: next,
      });
      setAccounts((prev) => prev.map((a) => (a.id === updated.id ? updated : a)));
      setEditingCell(null);
    } catch (e) {
      showMsg(false, String(e));
    }
  };

  const commitCell = (acc: BuddyAccount) => {
    if (!editingCell) return;
    const raw = editingCell.value.trim();
    if (!raw) {
      void applyCellTime(acc, editingCell.colId, null);
      return;
    }
    const parsed = parseTimeInput(raw);
    if (parsed == null) {
      showMsg(false, t("buddy.invalidTimeFormat"));
      return;
    }
    void applyCellTime(acc, editingCell.colId, parsed);
  };

  const clearCell = (acc: BuddyAccount, colId: string) => {
    void applyCellTime(acc, colId, null);
  };

  const normalizedColumnInput = (raw: string) => {
    const p = parseTimeInput(raw);
    return p != null ? formatTimeInput(p) : raw;
  };

  const renderExpiryHeader = () => (<div className="flex items-stretch sticky top-0 z-10 bg-slate-950 border-b border-white/10 text-tiny text-slate-500">
                  <div style={{ width: COL_INFO }} className="px-3 py-1.5 flex-shrink-0">{t("buddy.colAccount")}</div>
                  {/* 填充列：吃掉剩余宽度（段多的账号最需要它） */}
                  <div
                    style={{ minWidth: COL_CREDITS_MIN }}
                    className="flex-1 min-w-0 px-2 py-1.5 border-l border-white/5"
                  >
                    {t("buddy.creditSegments")}
                  </div>
                  {/* 内置「到期」列：登录 token 过期时刻，只读，不属于自定义列 schema
                      列宽只够显示紧凑倒计时，格式说明放在表头的悬停里 */}
                  <div
                    style={{ width: COL_W }}
                    title={t("buddy.countdownFormat")}
                    className="px-1 py-1.5 flex-shrink-0 border-l border-white/5 truncate"
                  >
                    {t("buddy.expiresAt")}
                  </div>
                  {expiryColumns.map((col) => (
                    <div key={col.id} style={{ width: COL_W }} className="px-1 py-1 flex-shrink-0 border-l border-white/5 relative">
                      {editingColumnId === col.id ? (
                        // 改名框要打得下整列名，72px 的列放不下 → 浮出列外（改完即收）
                        <div className="absolute left-0 top-0 z-20 flex items-center gap-1 w-40 bg-slate-900 border border-white/15 rounded px-1 py-1 shadow-lg">
                          <input
                            autoFocus
                            value={columnNameDraft}
                            onChange={(e) => setColumnNameDraft(e.target.value)}
                            onKeyDown={(e) => {
                              if (e.key === "Enter") {
                                e.preventDefault();
                                void commitRenameColumn(col.id);
                              }
                            }}
                            className="min-w-0 flex-1 bg-black/40 border border-white/15 rounded px-1 py-0.5 text-tiny text-white outline-none"
                          />
                          <button onClick={() => void commitRenameColumn(col.id)} className="text-emerald-300 cursor-pointer flex-shrink-0"><Check className="w-3 h-3" /></button>
                          <button onClick={() => setEditingColumnId(null)} className="text-slate-500 cursor-pointer flex-shrink-0"><X className="w-3 h-3" /></button>
                        </div>
                      ) : (
                        <div className="flex items-center gap-1">
                          <button onClick={() => startRenameColumn(col)} title={`${col.name} · ${t("buddy.renameColumn")}`} className="truncate text-slate-300 hover:text-white cursor-pointer">{col.name}</button>
                          <button onClick={() => void deleteColumn(col.id)} title={t("buddy.deleteColumn")} className="text-slate-600 hover:text-rose-300 cursor-pointer flex-shrink-0"><X className="w-2.5 h-2.5" /></button>
                        </div>
                      )}
                    </div>
                  ))}
                  {/* 操作列表头不显示文字；左分隔线避免顶死时间列 */}
                  <div style={{ width: COL_ACTIONS }} className="px-2 py-1.5 flex-shrink-0 border-l border-white/5" />
                </div>);
  return {
    expiryColumns,
    editingCell,
    setEditingCell,
    addingColumn,
    setAddingColumn,
    newColumnName,
    setNewColumnName,
    colBusy,
    now,
    setNow,
    addColumn,
    startEditCell,
    commitCell,
    clearCell,
    normalizedColumnInput,
    renderExpiryHeader,
  };
}
