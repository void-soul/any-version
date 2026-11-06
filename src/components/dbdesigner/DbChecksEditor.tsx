import type { DbCheck, DbDesignNode } from "./types";
import { useTranslation } from "react-i18next";
import { Plus, Trash2 } from "lucide-react";

interface DbChecksEditorProps {
  addCheck: (nodeId: string) => void;
  selected: DbDesignNode;
  updateCheck: (nodeId: string, index: number, patch: Partial<DbCheck>) => void;
  removeCheck: (nodeId: string, index: number) => void;
}

/** 表级 CHECK 约束编辑器：名字（可留空）+ 条件表达式。纯视图，状态由 owning panel 持有。 */
export function DbChecksEditor({ addCheck, selected, updateCheck, removeCheck }: DbChecksEditorProps) {
  const { t } = useTranslation();
  const checks = selected.table?.checks ?? [];
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center gap-2 border-b border-white/5 px-3 py-1.5">
        <p className="min-w-0 flex-1 truncate text-micro text-slate-600">{t("dbd.checkHint")}</p>
        <button onClick={() => addCheck(selected.id)} className="ui-btn shrink-0 px-1.5 py-0.5 text-micro">
          <Plus className="h-3 w-3" /> {t("dbd.addCheck")}
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-2">
        {checks.length === 0 ? (
          <div className="text-micro text-slate-600">{t("dbd.noChecks")}</div>
        ) : null}
        <div className="grid grid-cols-1 gap-2 xl:grid-cols-2">
          {checks.map((check, i) => (
            <div
              key={i}
              className="flex items-center gap-1 rounded-ctl border border-white/5 bg-black/20 p-1.5"
            >
              <input
                value={check.name}
                onChange={(e) => updateCheck(selected.id, i, { name: e.target.value })}
                placeholder={t("dbd.checkName")}
                title={t("dbd.checkName")}
                className="min-w-0 w-1/3 rounded bg-black/30 px-1.5 py-0.5 font-mono text-micro text-slate-100 outline-none placeholder:font-sans placeholder:text-slate-600"
              />
              <input
                value={check.expression}
                onChange={(e) => updateCheck(selected.id, i, { expression: e.target.value })}
                placeholder={t("dbd.checkExpression")}
                title={t("dbd.checkExpression")}
                className="min-w-0 flex-1 rounded bg-black/30 px-1.5 py-0.5 font-mono text-micro text-slate-100 outline-none placeholder:font-sans placeholder:text-slate-600"
              />
              <button
                onClick={() => removeCheck(selected.id, i)}
                className="ui-btn shrink-0 p-0.5"
                title={t("common.delete")}
              >
                <Trash2 className="h-3 w-3" />
              </button>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
