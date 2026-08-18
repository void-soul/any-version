import React, { useState, useEffect } from "react";
import { X, Folder, Sparkles, Sliders, Hash, Check } from "lucide-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Classification, ClassificationData } from "./types";

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
  const [name, setName] = useState("");
  const [parentId, setParentId] = useState<number | null>(null);
  const [classificationType, setClassificationType] = useState<number>(0);
  const [icon, setIcon] = useState("📁");
  const [customEmoji, setCustomEmoji] = useState("");
  const [associateFolderPath, setAssociateFolderPath] = useState("");
  const [associateFolderHiddenItems, setAssociateFolderHiddenItems] = useState("");
  const [itemShowOnly, setItemShowOnly] = useState<"default" | "file" | "folder">("default");
  const [aggregateItemCount, setAggregateItemCount] = useState(30);
  const [aggregateSort, setAggregateSort] = useState<"openNumber" | "lastOpen">("openNumber");
  const [excludeSearch, setExcludeSearch] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (editingCategory) {
      setName(editingCategory.name);
      setParentId(editingCategory.parentId);
      setClassificationType(editingCategory.classificationType);
      setIcon(editingCategory.data.icon || "📁");
      setAssociateFolderPath(editingCategory.data.associateFolderPath || "");
      setAssociateFolderHiddenItems(editingCategory.data.associateFolderHiddenItems || "");
      setItemShowOnly(editingCategory.data.itemShowOnly || "default");
      setAggregateItemCount(editingCategory.data.aggregateItemCount || 30);
      setAggregateSort(editingCategory.data.aggregateSort || "openNumber");
      setExcludeSearch(!!editingCategory.data.excludeSearch);
    } else {
      setName("");
      setParentId(currentParentId || null);
      setClassificationType(0);
      setIcon("📁");
      setAssociateFolderPath("");
      setAssociateFolderHiddenItems("");
      setItemShowOnly("default");
      setAggregateItemCount(30);
      setAggregateSort("openNumber");
      setExcludeSearch(false);
    }
  }, [editingCategory, currentParentId, isOpen]);

  if (!isOpen) return null;

  const handleSelectFolder = async () => {
    try {
      const selected = await openDialog({
        directory: true,
        multiple: false,
        title: "选择关联目录",
      });
      if (selected && typeof selected === "string") {
        setAssociateFolderPath(selected);
        if (!name) {
          const parts = selected.split(/[\\/]/);
          setName(parts[parts.length - 1] || "关联文件夹");
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
        icon: icon || "📁",
        associateFolderPath: classificationType === 1 ? associateFolderPath : undefined,
        associateFolderHiddenItems: classificationType === 1 ? associateFolderHiddenItems : undefined,
        itemShowOnly: classificationType === 1 ? itemShowOnly : "default",
        aggregateItemCount: classificationType === 2 ? aggregateItemCount : 30,
        aggregateSort: classificationType === 2 ? aggregateSort : "openNumber",
        excludeSearch,
      };

      const categoryToSave: Classification = {
        id: editingCategory ? editingCategory.id : 0,
        parentId: parentId || null,
        name: name.trim(),
        classificationType,
        data,
        shortcutKey: editingCategory?.shortcutKey || null,
        globalShortcutKey: !!editingCategory?.globalShortcutKey,
        order: editingCategory?.order || 0,
      };

      await onSave(categoryToSave);
      onClose();
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/60 backdrop-blur-sm p-4 select-none">
      <div className="bg-[#141927] border border-white/10 rounded-2xl w-full max-w-lg shadow-2xl overflow-hidden flex flex-col max-h-[90vh] animate-in fade-in zoom-in-95 duration-150">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-white/10 bg-white/[0.02]">
          <div className="flex items-center gap-2.5">
            <span className="text-xl">{icon}</span>
            <h3 className="text-sm font-semibold text-white">
              {editingCategory ? "编辑分类" : "新增分类"}
            </h3>
          </div>
          <button
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
            <label className="block text-xs font-medium text-slate-400 mb-1.5">分类类型</label>
            <div className="grid grid-cols-3 gap-2">
              {[
                { type: 0, label: "普通分类", desc: "手动管理项目" },
                { type: 1, label: "关联文件夹", desc: "实时同步本地目录" },
                { type: 2, label: "聚合分类", desc: "自动统计最常/最近使用" },
              ].map((t) => (
                <button
                  type="button"
                  key={t.type}
                  onClick={() => {
                    setClassificationType(t.type);
                    if (t.type === 1 && icon === "📁") setIcon("📂");
                    if (t.type === 2 && icon === "📁") setIcon("🔥");
                  }}
                  className={`p-2.5 rounded-xl border text-left transition cursor-pointer flex flex-col ${
                    classificationType === t.type
                      ? "bg-purple-600/20 border-purple-500/50 text-white"
                      : "bg-white/[0.02] border-white/5 text-slate-400 hover:bg-white/[0.05]"
                  }`}
                >
                  <span className="text-xs font-medium text-slate-200">{t.label}</span>
                  <span className="text-[10px] text-slate-500 mt-0.5">{t.desc}</span>
                </button>
              ))}
            </div>
          </div>

          {/* Name & Parent */}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            <div>
              <label className="block text-xs font-medium text-slate-400 mb-1.5">分类名称 *</label>
              <input
                type="text"
                required
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="例如：开发工具"
                className="w-full bg-white/5 border border-white/10 rounded-xl px-3 py-2 text-xs text-white focus:outline-none focus:border-purple-500 transition"
              />
            </div>
            <div>
              <label className="block text-xs font-medium text-slate-400 mb-1.5">父级分类 (可选)</label>
              <select
                value={parentId || ""}
                onChange={(e) => setParentId(e.target.value ? Number(e.target.value) : null)}
                className="w-full bg-[#1e2436] border border-white/10 rounded-xl px-3 py-2 text-xs text-white focus:outline-none focus:border-purple-500 transition cursor-pointer"
              >
                <option value="">顶级分类 (无父级)</option>
                {parentCategories
                  .filter((c) => !editingCategory || c.id !== editingCategory.id)
                  .map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.data.icon ? `${c.data.icon} ` : ""}{c.name}
                    </option>
                  ))}
              </select>
            </div>
          </div>

          {/* Icon / Emoji Selection */}
          <div>
            <label className="block text-xs font-medium text-slate-400 mb-1.5">分类图标 / Emoji</label>
            <div className="flex items-center gap-2 mb-2">
              <div className="w-9 h-9 rounded-xl bg-white/5 border border-white/10 flex items-center justify-center text-lg flex-shrink-0">
                {icon}
              </div>
              <input
                type="text"
                value={customEmoji}
                onChange={(e) => {
                  setCustomEmoji(e.target.value);
                  if (e.target.value) setIcon(e.target.value);
                }}
                placeholder="输入任意 Emoji 或字符"
                className="flex-1 bg-white/5 border border-white/10 rounded-xl px-3 py-1.5 text-xs text-white focus:outline-none focus:border-purple-500"
              />
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
                <label className="block text-xs font-medium text-slate-300 mb-1">关联文件夹路径 *</label>
                <div className="flex items-center gap-2">
                  <input
                    type="text"
                    required
                    value={associateFolderPath}
                    onChange={(e) => setAssociateFolderPath(e.target.value)}
                    placeholder="选择或输入本地目录路径"
                    className="flex-1 bg-black/30 border border-white/10 rounded-xl px-3 py-2 text-xs text-white focus:outline-none focus:border-purple-500"
                  />
                  <button
                    type="button"
                    onClick={handleSelectFolder}
                    className="px-3 py-2 bg-purple-600 hover:bg-purple-500 text-white text-xs rounded-xl transition cursor-pointer flex items-center gap-1.5"
                  >
                    <Folder className="w-3.5 h-3.5" />
                    浏览
                  </button>
                </div>
              </div>

              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label className="block text-xs font-medium text-slate-300 mb-1">仅显示</label>
                  <select
                    value={itemShowOnly}
                    onChange={(e: any) => setItemShowOnly(e.target.value)}
                    className="w-full bg-[#1e2436] border border-white/10 rounded-xl px-3 py-1.5 text-xs text-white focus:outline-none"
                  >
                    <option value="default">全部文件与子文件夹</option>
                    <option value="file">仅文件 (隐藏文件夹)</option>
                    <option value="folder">仅文件夹 (隐藏文件)</option>
                  </select>
                </div>
                <div>
                  <label className="block text-xs font-medium text-slate-300 mb-1">隐藏项过滤 (逗号隔开)</label>
                  <input
                    type="text"
                    value={associateFolderHiddenItems}
                    onChange={(e) => setAssociateFolderHiddenItems(e.target.value)}
                    placeholder="如：.git, node_modules, tmp"
                    className="w-full bg-black/30 border border-white/10 rounded-xl px-3 py-1.5 text-xs text-white focus:outline-none"
                  />
                </div>
              </div>
            </div>
          )}

          {/* Type 2: Aggregate Options */}
          {classificationType === 2 && (
            <div className="p-3.5 rounded-xl bg-purple-500/5 border border-purple-500/20 space-y-3">
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label className="block text-xs font-medium text-slate-300 mb-1">排序依据</label>
                  <select
                    value={aggregateSort}
                    onChange={(e: any) => setAggregateSort(e.target.value)}
                    className="w-full bg-[#1e2436] border border-white/10 rounded-xl px-3 py-1.5 text-xs text-white focus:outline-none"
                  >
                    <option value="openNumber">🔥 按打开次数最多 (高频使用)</option>
                    <option value="lastOpen">⏱️ 按最后打开时间 (最近使用)</option>
                  </select>
                </div>
                <div>
                  <label className="block text-xs font-medium text-slate-300 mb-1">汇聚显示数量上限</label>
                  <input
                    type="number"
                    min={5}
                    max={100}
                    value={aggregateItemCount}
                    onChange={(e) => setAggregateItemCount(Number(e.target.value))}
                    className="w-full bg-black/30 border border-white/10 rounded-xl px-3 py-1.5 text-xs text-white focus:outline-none"
                  />
                </div>
              </div>
            </div>
          )}

          {/* Additional switches */}
          <div className="pt-2">
            <label className="flex items-center gap-2 cursor-pointer text-xs text-slate-300">
              <input
                type="checkbox"
                checked={excludeSearch}
                onChange={(e) => setExcludeSearch(e.target.checked)}
                className="rounded border-white/10 bg-white/5 text-purple-600 focus:ring-0"
              />
              <span>在全局快速搜索中排除此分类下的项目</span>
            </label>
          </div>

          {/* Buttons */}
          <div className="flex items-center justify-end gap-2.5 pt-3 border-t border-white/10">
            <button
              type="button"
              onClick={onClose}
              className="px-4 py-2 rounded-xl text-xs font-medium text-slate-400 hover:text-white hover:bg-white/5 transition cursor-pointer"
            >
              取消
            </button>
            <button
              type="submit"
              disabled={saving}
              className="px-5 py-2 rounded-xl text-xs font-semibold bg-purple-600 hover:bg-purple-500 text-white shadow-lg shadow-purple-600/30 transition cursor-pointer disabled:opacity-50 flex items-center gap-1.5"
            >
              <Check className="w-3.5 h-3.5" />
              {saving ? "保存中..." : "保存分类"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
