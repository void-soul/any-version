import type { DbDesignNode } from "./types";
import { useTranslation } from "react-i18next";
import "@xyflow/react/dist/style.css";
import { Plus, Trash2 } from "lucide-react";
import { typeLabel, type DbIndex } from "./types";

interface DbIndexesEditorProps {
  addIndex: (nodeId: string) => void;
  selected: DbDesignNode;
  updateIndex: (nodeId: string, index: number, patch: Partial<DbIndex>) => void;
  removeIndex: (nodeId: string, index: number) => void;
  toggleIndexField: (nodeId: string, index: number, field: string) => void;
}

/** Pure DbIndexesEditor view; its owning panel retains side effects and state. */
export function DbIndexesEditor({ addIndex, selected, updateIndex, removeIndex, toggleIndexField }: DbIndexesEditorProps) {
  const { t } = useTranslation();
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center gap-2 border-b border-white/5 px-3 py-1.5">
        <p className="min-w-0 flex-1 truncate text-micro text-slate-600">{t("dbd.indexHint")}</p>
        <button onClick={() => addIndex(selected.id)} className="ui-btn shrink-0 px-1.5 py-0.5 text-micro">
          <Plus className="h-3 w-3" /> {t("dbd.addIndex")}
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-2">
        {(selected.table?.indexes ?? []).length === 0 ? (
          <div className="text-micro text-slate-600">{t("dbd.noIndexes")}</div>
        ) : null}
        <div className="grid grid-cols-1 gap-2 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
          {(selected.table?.indexes ?? []).map((idx, i) => (
            /* 一个索引 = 名称 + 类型 + 「哪些列的组合」。列用勾选而不是手打逗号：
               既能多列组合，也不会打出不存在的字段名。 */
            <div
              key={`${idx.name}-${i}`}
              className="flex flex-col gap-1.5 rounded-ctl border border-white/5 bg-black/20 p-1.5"
            >
              <div className="flex items-center gap-1">
                <input
                  value={idx.name}
                  onChange={(e) => updateIndex(selected.id, i, { name: e.target.value })}
                  placeholder={t("dbd.indexName")}
                  className="min-w-0 flex-1 rounded bg-black/30 px-1.5 py-0.5 font-mono text-micro text-slate-100 outline-none placeholder:font-sans placeholder:text-slate-600"
                />
                <button
                  onClick={() => removeIndex(selected.id, i)}
                  className="ui-btn shrink-0 p-0.5"
                  title={t("common.delete")}
                >
                  <Trash2 className="h-3 w-3" />
                </button>
              </div>
              <select
                value={idx.kind}
                onChange={(e) => updateIndex(selected.id, i, { kind: e.target.value as DbIndex["kind"] })}
                className="rounded bg-white/5 px-1 py-0.5 text-micro text-slate-200 outline-none"
              >
                <option value="index">{t("dbd.indexKindIndex")}</option>
                <option value="unique">{t("dbd.indexKindUnique")}</option>
                <option value="fulltext">{t("dbd.indexKindFulltext")}</option>
              </select>
              <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
                {(selected.table?.fields ?? []).map((f) => (
                  /* 勾选框而不是「点一下就切换」的 chip：多选组合时勾选框的
                     「当前勾了哪些」一目了然，不用点完再回头看配色。 */
                  <label
                    key={f.name}
                    className="flex cursor-pointer items-center gap-1 text-micro text-slate-300"
                    title={typeLabel(f.type)}
                  >
                    <input
                      type="checkbox"
                      className="h-3 w-3 accent-[var(--module-accent)]"
                      checked={idx.fields.includes(f.name)}
                      onChange={() => toggleIndexField(selected.id, i, f.name)}
                    />
                    <span className="font-mono">{f.name}</span>
                    {f.pk ? <span className="text-amber-400">*</span> : null}
                  </label>
                ))}
                {(selected.table?.fields ?? []).length === 0 ? (
                  <span className="text-micro text-slate-600">{t("dbd.noFields")}</span>
                ) : null}
              </div>
              {idx.fields.length > 0 ? (
                <div className="font-mono text-micro text-slate-500" title={t("dbd.indexChosenHint")}>
                  {idx.fields.join(" , ")}
                </div>
              ) : null}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
