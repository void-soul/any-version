import { useTranslation } from "react-i18next";
import { Bell, Plus } from "lucide-react";
import { COL_W, LABEL_TONE, describeLabel, parseTimeInput, type BuddyAccount, type ExpiryColumn } from "./BuddyPanel.model";

type EditingCell = { accId: string; colId: string; value: string };
interface BuddyExpiryCellProps {
  acc: BuddyAccount;
  col: ExpiryColumn;
  cellEditingAcc: EditingCell | null;
  setEditingCell: (value: EditingCell | null) => void;
  now: number;
  startEditCell: (account: BuddyAccount, columnId: string) => void;
  normalizedColumnInput: (value: string) => string;
  commitCell: (account: BuddyAccount) => void;
  clearCell: (account: BuddyAccount, columnId: string) => void;
}

export function BuddyExpiryCell({ acc, col, cellEditingAcc, setEditingCell, now, startEditCell, normalizedColumnInput, commitCell, clearCell }: BuddyExpiryCellProps) {
  const { t } = useTranslation();

  const ts = acc.expiryTimes?.[col.id];
  if (cellEditingAcc && cellEditingAcc.colId === col.id) {
    return (
      <div key={col.id} style={{ width: COL_W }} className="px-1 py-1 flex-shrink-0 border-l border-white/5 relative">
        {/* 要输入完整日期时间，72px 的列放不下 → 浮出列外（编辑完即收） */}
        <div className="absolute left-0 top-0 z-20 w-44 bg-slate-900 border border-white/15 rounded p-1.5 shadow-lg">
          <input
            autoFocus
            value={cellEditingAcc.value}
            onChange={(e) => setEditingCell({ ...cellEditingAcc, value: e.target.value })}
            onBlur={(e) => setEditingCell({ ...cellEditingAcc, value: normalizedColumnInput(e.target.value) })}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                commitCell(acc);
              } else if (e.key === "Escape") {
                setEditingCell(null);
              }
            }}
            placeholder={t("buddy.timeInputPlaceholder")}
            className={`w-full bg-black/40 border rounded px-1 py-0.5 text-tiny text-white outline-none ${
              cellEditingAcc.value.trim() && parseTimeInput(cellEditingAcc.value) == null
                ? "border-rose-400/60"
                : "border-[var(--module-accent)]/50"
            }`}
          />
          <div className="flex items-center gap-1 mt-1">
            <button onClick={() => commitCell(acc)} className="px-1.5 py-0.5 rounded bg-[var(--module-accent)] text-white text-micro cursor-pointer">{t("buddy.saveLabels")}</button>
            {ts && ts > 0 && (
              <button onClick={() => clearCell(acc, col.id)} className="px-1.5 py-0.5 rounded bg-rose-500/10 text-rose-300 text-micro cursor-pointer">{t("buddy.clear")}</button>
            )}
            <button onClick={() => setEditingCell(null)} className="px-1.5 py-0.5 rounded text-slate-400 text-micro hover:bg-white/5 cursor-pointer">{t("buddy.cancel")}</button>
          </div>
        </div>
      </div>
    );
  }
  if (ts && ts > 0) {
    const st = describeLabel(ts, now);
    const tone = LABEL_TONE[st.tone];
    return (
      <div key={col.id} style={{ width: COL_W }} className="px-1 py-1 flex-shrink-0 border-l border-white/5">
        <button
          onClick={() => startEditCell(acc, col.id)}
          title={`${col.name} · ${st.absolute}`}
          className={`relative overflow-hidden rounded border px-1 py-0.5 w-full text-left ${tone.chip} ${
            st.recovered ? "animate-pulse ring-1 ring-emerald-400/70" : ""
          }`}
        >
          <div className={`absolute left-0 top-0 bottom-0 ${tone.bar}`} style={{ width: `${st.progress}%` }} />
          <span className={`relative tabular-nums flex items-center gap-1 truncate ${tone.text}`}>
            {st.recovered ? (<><Bell className="w-2.5 h-2.5 flex-shrink-0" />{t("buddy.recovered")}</>) : st.text}
          </span>
        </button>
      </div>
    );
  }
  return (
    <div key={col.id} style={{ width: COL_W }} className="px-1 py-1 flex-shrink-0 border-l border-white/5">
      <button
        onClick={() => startEditCell(acc, col.id)}
        title={t("buddy.addCellTime")}
        className="w-full rounded border border-dashed border-white/15 text-slate-600 hover:text-white hover:border-white/30 py-0.5 px-1 text-left flex items-center gap-0.5 cursor-pointer"
      >
        <Plus className="w-2.5 h-2.5" />
      </button>
    </div>
  );

}
