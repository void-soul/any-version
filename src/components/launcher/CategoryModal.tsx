import { useState } from "react";
import { useTranslation } from "react-i18next";
import { X, Folder, Check, Eraser } from "lucide-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Classification, ClassificationData } from "./types";
import CategoryTreeSelect from "./CategoryTreeSelect";

interface CategoryModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSave: (category: Classification) => Promise<void>;
  editingCategory: Classification | null;
  parentCategories: Classification[];
  currentParentId?: number | null;
}

const EMOJI_PRESETS = [
  "🚀", "⭐", "🔥", "💻", "🛠️", "⚙️", "📁", "🌐", "🎮", "🎵",
  "📦", "⚡", "🎨", "📝", "📊", "🔒", "💡", "☕", "🤖", "🧭",
  "📚", "🎯", "🕹️", "🎬", "💎", "🧩", "📡", "🛡️", "🏷️", "✨"
];

export default function CategoryModal({
  isOpen,
  onClose,
  onSave,
  editingCategory,
  parentCategories,
  currentParentId,
}: CategoryModalProps) {
  const { t } = useTranslation();
  const [name, setName] = useState(editingCategory ? editingCategory.name : "");
  const [parentId, setParentId] = useState<number | null>(
    editingCategory ? editingCategory.parentId : (currentParentId || null)
  );
  const [classificationType, setClassificationType] = useState<number>(
    editingCategory ? (editingCategory.classificationType || 0) : 0
  );
  const [icon, setIcon] = useState(editingCategory?.data?.icon ?? "📁");
  const [customEmoji, setCustomEmoji] = useState("");
  const [associateFolderPath, setAssociateFolderPath] = useState(
    editingCategory?.data?.associateFolderPath || ""
  );
  const [associateFolderHiddenItems, setAssociateFolderHiddenItems] = useState(
    editingCategory?.data?.associateFolderHiddenItems || ""
  );
  const [itemShowOnly, setItemShowOnly] = useState<"default" | "file" | "folder">(
    editingCategory?.data?.itemShowOnly || "default"
  );
  const [excludeSearch, setExcludeSearch] = useState(
    !!editingCategory?.data?.excludeSearch
  );
  const [defaultCollapsed, setDefaultCollapsed] = useState(
    !!editingCategory?.data?.defaultCollapsed
  );
  const [saving, setSaving] = useState(false);

  // State is already cleanly initialized from props in useState() upon modal mount (via key)

  // 按 Escape 关闭弹窗（暂时注释以测试）
  /*
  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [isOpen, onClose]);
  */

  if (!isOpen) return null;

  const handleSelectFolder = async () => {
    try {
      const selected = await openDialog({
        directory: true,
        multiple: false,
        title: t("category.selectDirTitle"),
      });
      if (selected && typeof selected === "string") {
        setAssociateFolderPath(selected);
        if (!name) {
          const parts = selected.split(/[\\/]/);
          setName(parts[parts.length - 1] || t("category.defaultFolderName"));
        }
      }
    } catch (e) {
      console.error(e);
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;

    setSaving(true);
    try {
      const data: ClassificationData = {
        icon: icon.trim() || null,
        associateFolderPath: classificationType === 1 ? associateFolderPath : undefined,
        associateFolderHiddenItems: classificationType === 1 ? associateFolderHiddenItems : undefined,
        itemShowOnly: classificationType === 1 ? itemShowOnly : "default",
        excludeSearch,
        defaultCollapsed,
      };

      const category: Classification = {
        id: editingCategory ? editingCategory.id : 0,
        parentId: parentId || null,
        name: name.trim(),
        classificationType,
        data,
        shortcutKey: editingCategory ? editingCategory.shortcutKey : null,
        globalShortcutKey: editingCategory ? editingCategory.globalShortcutKey : false,
        order: editingCategory ? editingCategory.order : 0,
      };

      await onSave(category);
      onClose();
    } catch (e) {
      console.error(e);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-[100] modal-mask flex items-center justify-center bg-black/60 backdrop-blur-sm p-4"
    >
      <div
        className="bg-[#141927] border border-white/10 rounded-2xl w-full max-w-lg shadow-2xl overflow-hidden flex flex-col max-h-[90vh] animate-in fade-in zoom-in-95 duration-150 text-slate-100"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-white/10 bg-white/[0.02]">
          <div className="flex items-center gap-2.5">
            <span className="text-xl">{icon}</span>
            <h3 className="text-sm font-semibold text-white">
              {editingCategory ? t("category.editTitle") : t("category.addTitle")}
            </h3>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="p-1 rounded-lg text-slate-400 hover:text-white hover:bg-white/5 transition cursor-pointer"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Content */}
        <form onSubmit={handleSubmit} className="p-5 overflow-y-auto space-y-4 flex-1">
          {/* Classification Type Selector */}
          <div>
            <label className="block text-xs font-medium text-slate-400 mb-1.5">{t("category.typeLabel")}</label>
            <div className="grid grid-cols-2 gap-2">
              {[
                { type: 0, label: t("category.typeNormal"), desc: t("category.typeNormalDesc") },
                { type: 1, label: t("category.typeFolder"), desc: t("category.typeFolderDesc") },
              ].map((opt) => (
                <button
                  type="button"
                  key={opt.type}
                  onClick={() => {
                    setClassificationType(opt.type);
                    if (opt.type === 1 && icon === "📁") setIcon("📂");
                  }}
                  className={`p-2.5 rounded-xl border text-left transition cursor-pointer flex flex-col ${
                    classificationType === opt.type
                      ? "bg-purple-600/20 border-purple-500/50 text-white"
                      : "bg-white/[0.02] border-white/5 text-slate-400 hover:bg-white/[0.05]"
                  }`}
                >
                  <span className="text-xs font-medium text-slate-200">{opt.label}</span>
                  <span className="text-[10px] text-slate-500 mt-0.5">{opt.desc}</span>
                </button>
              ))}
            </div>
          </div>

          {/* Name & Parent */}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            <div>
              <label className="block text-xs font-medium text-slate-400 mb-1.5">{t("category.nameLabel")}</label>
              <input
                autoFocus
                type="text"
                required
                autoComplete="off"
                spellCheck={false}
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder={t("category.namePh")}
                className="w-full bg-white/5 border border-white/10 rounded-xl px-3 py-2 text-xs text-white focus:outline-none focus:border-purple-500 transition select-text"
              />
            </div>
            <div>
              <label className="block text-xs font-medium text-slate-400 mb-1.5">{t("category.parentLabel")}</label>
              <CategoryTreeSelect
                classifications={parentCategories}
                value={parentId || 0}
                onChange={(id) => setParentId(id === 0 ? null : id)}
                placeholder={t("category.parentPh")}
                allowNone
                noneLabel={t("category.topLevel")}
                excludeId={editingCategory ? editingCategory.id : null}
                hideDescendantsOfExclude
              />
            </div>
          </div>

          {/* Icon / Emoji Selection */}
          <div>
            <label className="block text-xs font-medium text-slate-400 mb-1.5">{t("category.iconLabel")}</label>
            <div className="flex items-center gap-2 mb-2">
              <div className="w-9 h-9 rounded-xl bg-white/5 border border-white/10 flex items-center justify-center text-lg flex-shrink-0">
                {icon || <span className="text-slate-600">∅</span>}
              </div>
              <input
                type="text"
                autoComplete="off"
                spellCheck={false}
                value={customEmoji}
                onChange={(e) => {
                  setCustomEmoji(e.target.value);
                  if (e.target.value) setIcon(e.target.value);
                }}
                placeholder={t("category.iconPh")}
                className="flex-1 bg-white/5 border border-white/10 rounded-xl px-3 py-1.5 text-xs text-white focus:outline-none focus:border-purple-500 select-text"
              />
              <button
                type="button"
                onClick={() => {
                  setIcon("");
                  setCustomEmoji("");
                }}
                disabled={!icon}
                className="w-9 h-9 rounded-xl border border-white/10 bg-white/5 text-slate-500 hover:text-white hover:bg-white/10 transition cursor-pointer disabled:cursor-not-allowed disabled:opacity-30 flex items-center justify-center"
                title={t("category.clearIcon")}
                aria-label={t("category.clearIcon")}
              >
                <Eraser className="w-3.5 h-3.5" />
              </button>
            </div>
            <div className="flex flex-wrap gap-1.5 p-2 rounded-xl bg-white/[0.02] border border-white/5 max-h-24 overflow-y-auto">
              {EMOJI_PRESETS.map((em) => (
                <button
                  type="button"
                  key={em}
                  onClick={() => setIcon(em)}
                  className={`w-7 h-7 rounded-lg flex items-center justify-center text-sm transition cursor-pointer ${
                    icon === em ? "bg-purple-600 text-white scale-110" : "hover:bg-white/10"
                  }`}
                >
                  {em}
                </button>
              ))}
            </div>
          </div>

          {/* Type 1: Associated Folder Options */}
          {classificationType === 1 && (
            <div className="p-3.5 rounded-xl bg-purple-500/5 border border-purple-500/20 space-y-3">
              <div>
                <label className="block text-xs font-medium text-slate-300 mb-1">{t("category.folderPathLabel")}</label>
                <div className="flex items-center gap-2">
                  <input
                    type="text"
                    required
                    autoComplete="off"
                    spellCheck={false}
                    value={associateFolderPath}
                    onChange={(e) => setAssociateFolderPath(e.target.value)}
                    placeholder={t("category.folderPathPh")}
                    className="flex-1 bg-black/30 border border-white/10 rounded-xl px-3 py-2 text-xs text-white focus:outline-none focus:border-purple-500 select-text"
                  />
                  <button
                    type="button"
                    onClick={handleSelectFolder}
                    className="px-3 py-2 bg-purple-600 hover:bg-purple-500 text-white text-xs rounded-xl transition cursor-pointer flex items-center gap-1.5"
                  >
                    <Folder className="w-3.5 h-3.5" />
                    {t("category.browse")}
                  </button>
                </div>
              </div>

              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label className="block text-xs font-medium text-slate-300 mb-1">{t("category.showOnly")}</label>
                  <select
                    value={itemShowOnly}
                    onChange={(e: any) => setItemShowOnly(e.target.value)}
                    className="w-full bg-[#1e2436] border border-white/10 rounded-xl px-3 py-1.5 text-xs text-white focus:outline-none"
                  >
                    <option value="default">{t("category.optAll")}</option>
                    <option value="file">{t("category.optFile")}</option>
                    <option value="folder">{t("category.optFolder")}</option>
                  </select>
                </div>
                <div>
                  <label className="block text-xs font-medium text-slate-300 mb-1">{t("category.hideFilter")}</label>
                  <input
                    type="text"
                    autoComplete="off"
                    spellCheck={false}
                    value={associateFolderHiddenItems}
                    onChange={(e) => setAssociateFolderHiddenItems(e.target.value)}
                    placeholder={t("category.hideFilterPh")}
                    className="w-full bg-black/30 border border-white/10 rounded-xl px-3 py-1.5 text-xs text-white focus:outline-none select-text"
                  />
                </div>
              </div>
            </div>
          )}

          {/* Additional switches */}
          <div className="pt-2 space-y-2.5">
            <label className="flex items-center gap-2 cursor-pointer text-xs text-slate-300">
              <input
                type="checkbox"
                checked={excludeSearch}
                onChange={(e) => setExcludeSearch(e.target.checked)}
                className="rounded border-white/10 bg-white/5 text-purple-600 focus:ring-0"
              />
              <span>{t("category.excludeSearch")}</span>
            </label>
            <label
              className="flex items-start gap-2 cursor-pointer text-xs text-slate-300"
              title={t("category.collapseTip")}
            >
              <input
                type="checkbox"
                checked={defaultCollapsed}
                onChange={(e) => setDefaultCollapsed(e.target.checked)}
                className="mt-0.5 rounded border-white/10 bg-white/5 text-purple-600 focus:ring-0"
              />
              <span>
                {t("category.collapseLabel")}
                <span className="block text-[10px] text-slate-500 mt-0.5">
                  {t("category.collapseDesc")}
                </span>
              </span>
            </label>
          </div>

          {/* Buttons */}
          <div className="flex items-center justify-end gap-2.5 pt-3 border-t border-white/10">
            <button
              type="button"
              onClick={onClose}
              className="px-4 py-2 rounded-xl text-xs font-medium text-slate-400 hover:text-white hover:bg-white/5 transition cursor-pointer"
            >
              {t("common.cancel")}
            </button>
            <button
              type="submit"
              disabled={saving}
              className="px-5 py-2 rounded-xl text-xs font-semibold bg-purple-600 hover:bg-purple-500 text-white shadow-lg shadow-purple-600/30 transition cursor-pointer disabled:opacity-50 flex items-center gap-1.5"
            >
              <Check className="w-3.5 h-3.5" />
              {saving ? t("category.saving") : t("category.saveCategory")}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
