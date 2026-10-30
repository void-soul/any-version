import type { DbDesignNode } from "./types";
import { useTranslation } from "react-i18next";
import "@xyflow/react/dist/style.css";
import { NODE_COLORS, isPresetColor } from "./nodeColors";
import type { Dispatch, SetStateAction } from "react";

interface DbNodeAppearanceEditorProps {
  allTags: string[];
  toggleNodeTag: (nodeId: string, tag: string) => void;
  selected: DbDesignNode;
  newTag: string;
  setNewTag: Dispatch<SetStateAction<string>>;
  updateNodeMeta: (nodeId: string, patch: Partial<DbDesignNode>) => void;
  commitNodeColor: (nodeId: string, hex: string) => void;
}

/** Pure DbNodeAppearanceEditor view; its owning panel retains side effects and state. */
export function DbNodeAppearanceEditor({ allTags, toggleNodeTag, selected, newTag, setNewTag, updateNodeMeta, commitNodeColor }: DbNodeAppearanceEditorProps) {
  const { t } = useTranslation();
  return (
    <div className="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-white/5 px-3 py-1.5">
      <span className="text-micro text-slate-500">{t("dbd.tags")}</span>
      {allTags.map((tag) => (
        <button
          key={tag}
          onClick={() => toggleNodeTag(selected.id, tag)}
          className={`cursor-pointer rounded-full border px-1.5 py-0.5 text-micro transition ${
            (selected.tags ?? []).includes(tag)
              ? "border-[var(--module-accent)] bg-[var(--module-accent)]/20 text-[var(--module-accent)]"
              : "border-white/10 text-slate-500 hover:border-white/20 hover:text-slate-300"
          }`}
        >
          {tag}
        </button>
      ))}
      <input
        value={newTag}
        onChange={(e) => setNewTag(e.target.value)}
        onKeyDown={(e) => {
          if (e.key !== "Enter") return;
          const tag = newTag.trim();
          if (!tag) return;
          if (!(selected.tags ?? []).includes(tag)) toggleNodeTag(selected.id, tag);
          setNewTag("");
        }}
        placeholder={t("dbd.tagAddPh")}
        className="w-20 rounded bg-white/5 px-1.5 py-0.5 text-micro text-slate-200 outline-none placeholder:text-slate-600"
      />
      <span className="ml-1 text-micro text-slate-500">{t("dbd.color")}</span>
      {NODE_COLORS.map((c) => (
        <button
          key={c}
          onClick={() => updateNodeMeta(selected.id, { color: selected.color === c ? "" : c })}
          title={c}
          className={`h-3.5 w-3.5 cursor-pointer rounded-full border-2 transition ${
            (selected.color ?? "") === c ? "scale-110 border-white" : "border-transparent"
          }`}
          style={{ background: c }}
        />
      ))}
      {/* 原生取色器：预设之外还要对齐某个特定色（品牌色 / 与别处一致）时用。
          左边一道竖线是为了和上面 12 个预设点区分开 —— 它不是「又一个预设」。 */}
      <span className="mx-0.5 h-3.5 w-px flex-shrink-0 bg-white/15" />
      <input
        type="color"
        value={isPresetColor(selected.color) ? selected.color! : NODE_COLORS[0]}
        onChange={(e) => commitNodeColor(selected.id, e.target.value)}
        title={t("dbd.colorCustom")}
        className="h-3.5 w-5 flex-shrink-0 cursor-pointer rounded border border-white/20 bg-transparent p-0"
      />
      {/* 预设色靠「再点一次同一个圆点」取消，自定义色没有这个等价操作，
          所以给它补一个清除按钮（否则用户设了自定义色就回不去默认强调色） */}
      {selected.color && !isPresetColor(selected.color) && (
        <button
          onClick={() => updateNodeMeta(selected.id, { color: "" })}
          title={t("dbd.colorClear")}
          className="flex h-3.5 w-3.5 flex-shrink-0 cursor-pointer items-center justify-center rounded-full border border-white/15 text-micro leading-none text-slate-400 transition hover:border-red-400/40 hover:text-red-300"
        >
          ×
        </button>
      )}
    </div>
  );
}
