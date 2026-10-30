import { useState } from "react";
import { Pencil, Check, X, Loader2, FolderPlus } from "lucide-react";
import { useTranslation } from "react-i18next";
// 新建 / 重命名分类弹窗（替代原生 prompt，美化样式）
export function CategoryModal({
  mode,
  initial,
  busy,
  onSubmit,
  onClose,
}: {
  mode: "add" | "rename";
  initial: string;
  busy: boolean;
  onSubmit: (title: string) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(initial);

  const submit = () => {
    const trimmed = name.trim();
    if (!trimmed) return;
    onSubmit(trimmed);
  };

  const onKey = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") submit();
  };

  return (
    <div
      className="fixed inset-0 z-[120] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4"
    >
      <div
        className="w-[360px] max-w-[95vw] rounded-panel border border-white/10 bg-slate-900/95 shadow-2xl p-5"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2.5 mb-4">
          <div className="w-9 h-9 rounded-card bg-[var(--module-accent)]/15 border border-[var(--module-accent)]/30 flex items-center justify-center">
            {mode === "add" ? (
              <FolderPlus className="w-4 h-4 text-[var(--module-accent)]" />
            ) : (
              <Pencil className="w-4 h-4 text-[var(--module-accent)]" />
            )}
          </div>
          <div className="flex-1">
            <h3 className="text-sm font-bold text-white">
              {mode === "add" ? t("otp.newCatTitle") : t("otp.renameCatTitle")}
            </h3>
            <p className="text-tiny text-slate-500">
              {mode === "add" ? t("otp.catHintAdd") : t("otp.catHintRename")}
            </p>
          </div>
          <button
            onClick={onClose}
            className="p-1.5 rounded-ctl hover:bg-white/10 text-slate-400 cursor-pointer"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <input
          autoFocus
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={onKey}
          placeholder={t("otp.catNamePh")}
          className="w-full glass-input px-3 py-2.5 text-body bg-black/30 border border-white/10 rounded-ctl focus:outline-none focus:border-[var(--module-accent)]/60 placeholder:text-slate-600"
        />

        <div className="flex justify-end gap-2 mt-4">
          <button
            onClick={onClose}
            className="px-3 py-1.5 rounded-ctl text-caption text-slate-400 hover:bg-white/5 cursor-pointer"
          >
            {t("otp.cancel")}
          </button>
          <button
            onClick={submit}
            disabled={busy || !name.trim()}
            className="px-4 py-1.5 rounded-ctl text-caption bg-[var(--module-accent)] text-white font-semibold cursor-pointer hover:opacity-85 disabled:opacity-50 flex items-center gap-1"
          >
            {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />} {t("otp.save")}
          </button>
        </div>
      </div>
    </div>
  );
}
