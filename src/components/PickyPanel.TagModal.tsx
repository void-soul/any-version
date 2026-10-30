import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Trash2, X } from "lucide-react";
import { type PickyBookmark, type PickyTag } from "./PickyPanel.model";
// ─── 标签管理 ───

export function TagModal({
  bookmark,
  allTags,
  selectedIds,
  onToggle,
  onCreate,
  onDelete,
  onClose,
}: {
  bookmark: PickyBookmark;
  allTags: PickyTag[];
  selectedIds: string[];
  onToggle: (tagId: string) => void;
  onCreate: (name: string) => void;
  onDelete: (t: PickyTag) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [newName, setNewName] = useState("");

  const submit = () => {
    if (!newName.trim()) return;
    onCreate(newName.trim());
    setNewName("");
  };

  return (
    <div className="fixed inset-0 z-[110] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
      <div
        className="w-[420px] max-w-[95vw] rounded-panel border border-white/10 bg-slate-900/95 shadow-2xl p-5"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between mb-1">
          <h3 className="text-sm font-bold text-white">{t("picky.tagsTitle")}</h3>
          <button onClick={onClose} className="p-1 rounded hover:bg-white/10 text-slate-400 cursor-pointer">
            <X className="w-4 h-4" />
          </button>
        </div>
        <p className="text-tiny text-slate-500 mb-3 truncate">{t("picky.tagsHint", { title: bookmark.title })}</p>

        <div className="flex flex-wrap gap-1.5 min-h-[40px] max-h-[200px] overflow-y-auto mb-3">
          {allTags.length === 0 && <p className="text-tiny text-slate-600">{t("picky.noTags")}</p>}
          {allTags.map((tg) => {
            const on = selectedIds.includes(tg.id);
            return (
              <span key={tg.id} className="inline-flex items-center gap-1">
                <button
                  onClick={() => onToggle(tg.id)}
                  className={`vex-chip px-2.5 py-1 rounded-full text-tiny border transition-colors cursor-pointer ${
                    on ? "bg-white/10 border-white/30 text-white" : "bg-white/[0.03] border-white/10 text-slate-400 hover:border-white/25"
                  }`}
                  style={on ? { background: `${tg.color}33`, borderColor: `${tg.color}66`, color: tg.color } : undefined}
                >
                  {on ? "✓ " : ""}#{tg.name}
                </button>
                <button
                  onClick={() => onDelete(tg)}
                  className="p-0.5 text-slate-600 hover:text-red-400 cursor-pointer"
                  title={t("picky.delTag", { name: tg.name })}
                >
                  <Trash2 className="w-2.5 h-2.5" />
                </button>
              </span>
            );
          })}
        </div>

        <div className="flex gap-2">
          <input
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && submit()}
            placeholder={t("picky.newTagPh")}
            className="flex-1 glass-input px-3 py-1.5 text-body bg-black/30 border border-white/10 rounded-ctl focus:outline-none focus:border-[var(--module-accent)]/50"
          />
          <button
            onClick={submit}
            disabled={!newName.trim()}
            className="px-3 py-1.5 rounded-ctl text-caption bg-[var(--module-accent)] text-white font-semibold cursor-pointer hover:opacity-85 disabled:opacity-50 flex items-center gap-1"
          >
            <Plus className="w-3 h-3" /> {t("picky.new")}
          </button>
        </div>
      </div>
    </div>
  );
}
