import { Fragment, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronRight } from "lucide-react";
import type { LogRow } from "./twoapiLogModel";

export type LogFilter = "all" | "errors" | "usage";
export type LogSortKey = "ts" | "credit" | "totalTokens" | "durationMs";

interface TwoApiLogTableProps {
  rows: LogRow[];
}

const tdCls = "px-1.5 py-1 border-b border-white/5 tabular-nums";
const thCls = "px-1.5 py-1 text-left font-semibold text-slate-400 border-b border-white/10 cursor-pointer select-none hover:text-slate-100";

const fmt = (n: number | null, digits = 0) =>
  n === null ? "-" : n.toLocaleString(undefined, { maximumFractionDigits: digits });

export function TwoApiLogTable({ rows }: TwoApiLogTableProps) {
  const { t } = useTranslation();
  const [filter, setFilter] = useState<LogFilter>("all");
  const [sortKey, setSortKey] = useState<LogSortKey>("ts");
  const [desc, setDesc] = useState(true);
  const [expanded, setExpanded] = useState<string | null>(null);

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
      <div className="flex shrink-0 items-center gap-1 px-2 py-1.5">
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
        <span className="ml-auto text-micro text-slate-600 tabular-nums">
          {shown.length} / {rows.length}
        </span>
      </div>

      <div className="max-h-64 min-h-0 overflow-auto">
        {shown.length === 0 ? (
          <div className="px-2 py-3 text-micro text-slate-600">{t("buddy.twoapi.noRequestLog")}</div>
        ) : (
          <table className="w-full table-fixed text-micro">
            <colgroup>
              <col className="w-[76px]" />
              <col className="w-[130px]" />
              <col className="w-[54px]" />
              <col className="w-[58px]" />
              <col className="w-[54px]" />
              <col className="w-[90px]" />
              <col className="w-[48px]" />
            </colgroup>
            <thead className="sticky top-0 bg-[#12151c]">
              <tr>
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
                      className="cursor-pointer text-slate-300 hover:bg-white/5"
                      title={r.error ?? t("buddy.twoapi.expandRaw")}
                    >
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
                    </tr>
                    {open ? (
                      <tr key={r.key + "-raw"} className="bg-black/30">
                        <td colSpan={7} className="px-2 py-1 font-mono text-[10px] leading-relaxed text-slate-400">
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
