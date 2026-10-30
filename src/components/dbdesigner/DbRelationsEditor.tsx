import { useTranslation } from "react-i18next";
import "@xyflow/react/dist/style.css";
import { ArrowRight, Plus, Trash2 } from "lucide-react";
import { type DbDesignDocument, type DbDesignRelation } from "./types";
import { FK_ACTIONS, REL_KINDS } from "./DbDesignerPanel.model";
import type { Dispatch, SetStateAction } from "react";

interface DbRelationsEditorProps {
  REL_FILTERS: readonly [readonly ["all", string], readonly ["parent", string], readonly ["child", string]];
  setRelFilter: Dispatch<SetStateAction<"all" | "child" | "parent">>;
  relFilter: "all" | "child" | "parent";
  addRelation: () => void;
  shownRelations: DbDesignRelation[];
  doc: DbDesignDocument;
  removeRelation: (id: string) => void;
  updateRelation: (id: string, patch: Partial<DbDesignRelation>) => void;
  toggleRelationMirror: (id: string, mirror: boolean) => void;
}

/** Pure DbRelationsEditor view; its owning panel retains side effects and state. */
export function DbRelationsEditor({ REL_FILTERS, setRelFilter, relFilter, addRelation, shownRelations, doc, removeRelation, updateRelation, toggleRelationMirror }: DbRelationsEditorProps) {
  const { t } = useTranslation();
  return (
    <div className="flex min-h-0 flex-1 flex-col">
          <div className="flex shrink-0 items-center gap-2 border-b border-white/5 px-3 py-1.5">
            {/* 只看自己这一侧的关系：审一张表时另外那些是噪音 */}
            <div className="flex shrink-0 items-center gap-0.5 rounded-ctl bg-white/5 p-0.5">
              {REL_FILTERS.map(([id, label]) => (
                <button
                  key={id}
                  onClick={() => setRelFilter(id)}
                  className={`cursor-pointer rounded px-1.5 py-0.5 text-micro transition ${
                    relFilter === id
                      ? "bg-[var(--module-accent)] text-white"
                      : "text-slate-400 hover:text-slate-200"
                  }`}
                >
                  {label}
                </button>
              ))}
            </div>
            <p className="min-w-0 flex-1 truncate text-micro text-slate-600">{t("dbd.connectHint")}</p>
            <button onClick={addRelation} className="ui-btn shrink-0 px-1.5 py-0.5 text-micro">
              <Plus className="h-3 w-3" /> {t("dbd.addRelation")}
            </button>
          </div>
          <div className="min-h-0 flex-1 overflow-auto p-2">
    {shownRelations.length === 0 ? (
      <div className="text-micro text-slate-600">{t("dbd.noRelations")}</div>
    ) : null}
    {/* 一条关联一个小卡片，网格排布（宽屏一行 4 个）。
        方向用「主表在上 / 子表在下 + 箭头图标」表达，原来那个 ↓ 字符太轻，
         分不清谁是主表；现在主表天蓝、子表琥珀，两端下拉也各自染色。 */}
    <div className="grid grid-cols-1 gap-2 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
    {shownRelations.map((r) => {
      const parentName = doc.nodes.find((n) => n.id === r.to.node)?.name ?? "?";
      const childName = doc.nodes.find((n) => n.id === r.from.node)?.name ?? "?";
      return (
      <div
        key={r.id}
        className="flex flex-col gap-1 rounded-ctl border border-white/5 bg-black/20 p-1.5 text-micro"
      >
        <div className="flex items-center gap-1">
          <span className="min-w-0 flex-1 truncate text-tiny font-semibold text-sky-300" title={t("dbd.relParent")}>
            {parentName}.{r.to.field}
          </span>
          <button
            onClick={() => removeRelation(r.id)}
            className="ui-btn shrink-0 p-0.5"
            title={t("common.delete")}
          >
            <Trash2 className="h-3 w-3" />
          </button>
        </div>
        <div className="flex items-center gap-1 text-slate-600">
          <span className="h-px flex-1 bg-white/10" />
          <ArrowRight className="h-3 w-3 flex-shrink-0" />
          <span className="h-px flex-1 bg-white/10" />
        </div>
        <div className="min-w-0 truncate text-tiny font-semibold text-amber-300" title={t("dbd.relChild")}>
          {childName}.{r.from.field}
        </div>

        <div className="mt-0.5 flex items-center gap-1">
          <select
            value={r.to.node}
            onChange={(e) => updateRelation(r.id, { to: { ...r.to, node: e.target.value } })}
            title={t("dbd.relParent")}
            className="min-w-0 flex-1 rounded bg-white/5 px-1 py-0.5 text-micro text-sky-300 outline-none"
          >
            {doc.nodes.filter((n) => n.kind === "table").map((n) => (
              <option key={n.id} value={n.id}>{n.name}</option>
            ))}
          </select>
          <select
            value={r.to.field}
            onChange={(e) => updateRelation(r.id, { to: { ...r.to, field: e.target.value } })}
            className="min-w-0 w-[5.5rem] rounded bg-white/5 px-1 py-0.5 font-mono text-micro text-sky-300 outline-none"
          >
            {(doc.nodes.find((n) => n.id === r.to.node)?.table?.fields ?? []).map((f) => (
              <option key={f.name} value={f.name}>{f.name}</option>
            ))}
          </select>
        </div>
        <div className="flex items-center gap-1">
          <select
            value={r.from.node}
            onChange={(e) => updateRelation(r.id, { from: { ...r.from, node: e.target.value } })}
            title={t("dbd.relChild")}
            className="min-w-0 flex-1 rounded bg-white/5 px-1 py-0.5 text-micro text-amber-300 outline-none"
          >
            {doc.nodes.filter((n) => n.kind === "table").map((n) => (
              <option key={n.id} value={n.id}>{n.name}</option>
            ))}
          </select>
          <select
            value={r.from.field}
            onChange={(e) => updateRelation(r.id, { from: { ...r.from, field: e.target.value } })}
            className="min-w-0 w-[5.5rem] rounded bg-white/5 px-1 py-0.5 font-mono text-micro text-amber-300 outline-none"
          >
            {(doc.nodes.find((n) => n.id === r.from.node)?.table?.fields ?? []).map((f) => (
              <option key={f.name} value={f.name}>{f.name}</option>
            ))}
          </select>
        </div>

        <div className="mt-0.5 flex items-center gap-1">
          <select
            value={r.kind}
            onChange={(e) => updateRelation(r.id, { kind: e.target.value as DbDesignRelation["kind"] })}
            className="rounded bg-white/5 px-1 py-0.5 text-micro text-slate-200 outline-none"
          >
            {REL_KINDS.map((k) => (
              <option key={k} value={k}>{k}</option>
            ))}
          </select>
          <select
            value={r.onDelete ?? "RESTRICT"}
            onChange={(e) => updateRelation(r.id, { onDelete: e.target.value })}
            className="min-w-0 flex-1 rounded bg-white/5 px-1 py-0.5 text-micro text-slate-200 outline-none"
            title={t("dbd.onDelete")}
          >
            {FK_ACTIONS.map((a) => (
              <option key={a} value={a}>{a}</option>
            ))}
          </select>
          {/* 跟随主键 = 子表的外键列跟着父表主键走。默认由「拖金色锚点」自动判定，
              这里可以手动改 —— 已有设计忘了开也能就地补救。 */}
          <label
            className="flex shrink-0 cursor-pointer items-center gap-1 text-micro text-slate-400"
            title={t("dbd.hintMirror")}
          >
            <input
              type="checkbox"
              className="h-3 w-3 accent-[var(--module-accent)]"
              checked={!!r.mirror}
              onChange={(e) => toggleRelationMirror(r.id, e.target.checked)}
            />
            {t("dbd.chipMirror")}
          </label>
        </div>
      </div>
      );
    })}
    </div>
          </div>
        </div>
  );
}
