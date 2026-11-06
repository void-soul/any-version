import { Fragment, useMemo, useRef, useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronRight, RotateCcw, Trash2, X } from "lucide-react";
import { summarizeSelection, toggleSelectAll, type LogRow } from "./twoapiLogModel";

export type LogFilter = "all" | "errors" | "usage";
export type LogSortKey = "ts" | "credit" | "totalTokens" | "durationMs";

interface TwoApiLogTableProps {
  rows: LogRow[];
  hiddenCount: number;
  onDeleteRow: (key: string) => void;
  onRestoreHidden: () => void;
}

const tdCls = "px-1.5 py-1 border-b border-white/5 tabular-nums";
const thCls = "px-1.5 py-1 text-left font-semibold text-slate-400 border-b border-white/10 cursor-pointer select-none hover:text-slate-100";

const fmt = (n: number | null, digits = 0) =>
  n === null ? "-" : n.toLocaleString(undefined, { maximumFractionDigits: digits });

export function TwoApiLogTable({ rows, hiddenCount, onDeleteRow, onRestoreHidden }: TwoApiLogTableProps) {
  const { t } = useTranslation();
  const [filter, setFilter] = useState<LogFilter>("all");
  const [sortKey, setSortKey] = useState<LogSortKey>("ts");
  const [desc, setDesc] = useState(true);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const selectAllRef = useRef<HTMLInputElement>(null);

  const shown = useMemo(() => {
    const filtered = rows.filter((r) => {
      if (filter === "errors") return r.error !== null || (r.status !== null && r.status >= 400);
      if (filter === "usage") return r.credit !== null || r.totalTokens !== null;
      return true;
    });
    const value = (r: LogRow) => {
      if (sortKey === "ts") return r.ts;
      return r[sortKey] ?? -1;
    };
    return [...filtered].sort((a, b) => {
      const av = value(a);
      const bv = value(b);
      const cmp = typeof av === "string" ? av.localeCompare(bv as string) : (av as number) - (bv as number);
      return desc ? -cmp : cmp;
    });
  }, [rows, filter, sortKey, desc]);

  const allSelected = shown.length > 0 && shown.every((r) => selected.has(r.key));
  const someSelected = shown.some((r) => selected.has(r.key));
  useEffect(() => {
    if (selectAllRef.current) selectAllRef.current.indeterminate = someSelected && !allSelected;
  }, [someSelected, allSelected]);

  const selection = useMemo(() => summarizeSelection(rows, selected), [rows, selected]);

  const sortBy = (key: LogSortKey) => {
    if (key === sortKey) setDesc((d) => !d);
    else {
      setSortKey(key);
      setDesc(true);
    }
  };
  const arrow = (key: LogSortKey) => (sortKey === key ? (desc ? " ↓" : " ↑") : "");

  const filters: Array<[LogFilter, string]> = [
    ["all", t("buddy.twoapi.filterAll")],
    ["errors", t("buddy.twoapi.filterErrors")],
    ["usage", t("buddy.twoapi.filterUsage")],
  ];

  return (
    <div className="flex min-h-0 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-1 px-2 py-1.5">
        {filters.map(([id, label]) => (
          <button
            key={id}
            onClick={() => setFilter(id)}
            className={`rounded px-1.5 py-0.5 text-micro transition ${
              filter === id
                ? "bg-[var(--module-accent)] text-white"
                : "bg-white/5 text-slate-400 hover:text-slate-100"
            }`}
          >
            {label}
          </button>
        ))}
        {hiddenCount > 0 && (
          <button
            onClick={onRestoreHidden}
            title={t("buddy.twoapi.restoreHidden")}
            className="inline-flex items-center gap-0.5 rounded bg-white/5 px-1.5 py-0.5 text-micro text-amber-300 hover:text-amber-100 transition"
          >
            <RotateCcw className="h-3 w-3" />
            {t("buddy.twoapi.hiddenCount", { count: hiddenCount })}
          </button>
        )}
        <span className="ml-auto text-micro text-slate-600 tabular-nums">
          {shown.length} / {rows.length}
        </span>
      </div>

      {selection.count > 0 && (
        <div className="flex shrink-0 flex-wrap items-center gap-x-2 gap-y-0.5 border-b border-white/8 bg-[var(--module-accent-soft)] px-2 py-1 text-micro text-slate-200">
          <span className="font-semibold">{t("buddy.twoapi.selectedCount", { count: selection.count })}</span>
          <span className="tabular-nums">
            {t("buddy.twoapi.selTokens", {
              total: selection.totalTokens, input: selection.inTokens, output: selection.outTokens,
            })}
          </span>
          <span className="tabular-nums">
            {t("buddy.twoapi.selDuration", {
              total: selection.totalDurationMs, avg: selection.avgDurationMs,
            })}
          </span>
          <span className="tabular-nums">{t("buddy.twoapi.selCredit", { credit: selection.totalCredit })}</span>
          <button
            onClick={() => setSelected(new Set())}
            title={t("buddy.twoapi.deselectAll")}
            className="ml-auto inline-flex items-center gap-0.5 rounded bg-white/10 px-1 py-0.5 hover:bg-white/20"
          >
            <X className="h-3 w-3" /> {t("buddy.twoapi.deselectAll")}
          </button>
        </div>
      )}

      <div className="max-h-64 min-h-0 overflow-auto">
        {shown.length === 0 ? (
          <div className="px-2 py-3 text-micro text-slate-600">{t("buddy.twoapi.noRequestLog")}</div>
        ) : (
          <table className="w-full table-fixed text-micro">
            <colgroup>
              <col className="w-[28px]" />
              <col className="w-[76px]" />
              <col className="w-[130px]" />
              <col className="w-[54px]" />
              <col className="w-[58px]" />
              <col className="w-[54px]" />
              <col className="w-[90px]" />
              <col className="w-[48px]" />
              <col className="w-[30px]" />
            </colgroup>
            <thead className="sticky top-0 bg-[#12151c]">
              <tr>
                <th className="px-1 py-1 border-b border-white/10 text-left">
                  <input
                    ref={selectAllRef}
                    type="checkbox"
                    checked={allSelected}
                    onChange={() => setSelected(toggleSelectAll(shown, selected))}
                    title={allSelected ? t("buddy.twoapi.deselectAll") : t("buddy.twoapi.selectAll")}
                    aria-label={t("buddy.twoapi.colSelected")}
                    className="h-3 w-3 cursor-pointer accent-[var(--module-accent)]"
                  />
                </th>
                <th className={thCls} onClick={() => sortBy("ts")}>
                  {t("buddy.twoapi.colTime")}{arrow("ts")}
                </th>
                <th className={thCls}>{t("buddy.twoapi.colModel")}</th>
                <th className={thCls}>{t("buddy.twoapi.colStatus")}</th>
                <th className={thCls} onClick={() => sortBy("durationMs")}>
                  {t("buddy.twoapi.colDuration")}{arrow("durationMs")}
                </th>
                <th className={thCls} onClick={() => sortBy("credit")}>
                  {t("buddy.twoapi.colCredit")}{arrow("credit")}
                </th>
                <th className={thCls} onClick={() => sortBy("totalTokens")}>
                  {t("buddy.twoapi.colTokens")}{arrow("totalTokens")}
                </th>
                <th className={thCls}>{t("buddy.twoapi.colQuota")}</th>
                <th className="px-1 py-1 border-b border-white/10 text-slate-500" title={t("buddy.twoapi.colRowActions")} />
              </tr>
            </thead>
            <tbody>
              {shown.map((r) => {
                const open = expanded === r.key;
                const statusColor = r.status === null
                  ? "text-slate-500"
                  : r.status < 300
                    ? "text-emerald-300"
                    : "text-rose-300";
                return (
                  <Fragment key={r.key}>
                    <tr
                      onClick={() => setExpanded(open ? null : r.key)}
                      className={`cursor-pointer text-slate-300 hover:bg-white/5 ${selected.has(r.key) ? "bg-white/5" : ""}`}
                      title={r.error ?? t("buddy.twoapi.expandRaw")}
                    >
                      <td className={tdCls} onClick={(e) => e.stopPropagation()}>
                        <input
                          type="checkbox"
                          checked={selected.has(r.key)}
                          onChange={() => {
                            const next = new Set(selected);
                            if (next.has(r.key)) next.delete(r.key);
                            else next.add(r.key);
                            setSelected(next);
                          }}
                          aria-label={t("buddy.twoapi.colSelected")}
                          className="h-3 w-3 cursor-pointer accent-[var(--module-accent)]"
                        />
                      </td>
                      <td className={tdCls + " font-mono text-slate-500"}>
                        <span className="inline-flex items-center gap-0.5">
                          {open ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
                          {r.ts}
                        </span>
                      </td>
                      <td className={tdCls + " truncate"} title={r.model ?? r.path ?? ""}>
                        {r.model ?? r.path ?? "-"}
                      </td>
                      <td className={tdCls + " " + statusColor}>{r.status ?? "-"}</td>
                      <td className={tdCls}>{r.durationMs === null ? "-" : `${r.durationMs}ms`}</td>
                      <td className={tdCls}>{fmt(r.credit, 2)}</td>
                      <td className={tdCls} title={`${r.inTokens ?? "-"} → ${r.outTokens ?? "-"}`}>
                        {r.inTokens ?? "-"}/{r.outTokens ?? "-"} ({fmt(r.totalTokens)})
                      </td>
                      <td className={tdCls}>{r.quotaPercent === null ? "-" : `${r.quotaPercent}%`}</td>
                      <td className={tdCls} onClick={(e) => e.stopPropagation()}>
                        <button
                          onClick={() => onDeleteRow(r.key)}
                          title={t("buddy.twoapi.deleteLog")}
                          aria-label={t("buddy.twoapi.deleteLog")}
                          className="inline-flex rounded p-0.5 text-slate-500 hover:bg-rose-500/20 hover:text-rose-300 transition"
                        >
                          <Trash2 className="h-3 w-3" />
                        </button>
                      </td>
                    </tr>
                    {open ? (
                      <tr key={r.key + "-raw"} className="bg-black/30">
                        <td colSpan={9} className="px-2 py-1 font-mono text-[10px] leading-relaxed text-slate-400">
                          {r.rawLines.map((line, i) => (
                            <div key={i} className="whitespace-pre-wrap break-all">{r.ts} {line}</div>
                          ))}
                        </td>
                      </tr>
                    ) : null}
                  </Fragment>
                );
              })}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
