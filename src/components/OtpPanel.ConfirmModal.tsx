import { Trash2, X } from "lucide-react";
import { useTranslation } from "react-i18next";
// 通用删除确认弹窗（替代原生 confirm，主题色随模块动态变化）
export function ConfirmModal({
  title,
  message,
  danger,
  onConfirm,
  onClose,
}: {
  title: string;
  message: string;
  danger?: boolean;
  onConfirm: () => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div
      className="fixed inset-0 z-[130] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4"
    >
      <div
        className="w-[360px] max-w-[95vw] rounded-panel border border-white/10 bg-slate-900/95 shadow-2xl p-5"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2.5 mb-4">
          <div
            className={`w-9 h-9 rounded-card flex items-center justify-center border ${
              danger
                ? "bg-rose-500/15 border-rose-500/30 text-rose-400"
                : "bg-[var(--module-accent)]/15 border-[var(--module-accent)]/30 text-[var(--module-accent)]"
            }`}
          >
            <Trash2 className="w-4 h-4" />
          </div>
          <div className="flex-1">
            <h3 className="text-sm font-bold text-white">{title}</h3>
            <p className="text-tiny text-slate-500">
              {danger ? t("otp.confirmDanger") : t("otp.confirmNormal")}
            </p>
          </div>
          <button
            onClick={onClose}
            className="p-1.5 rounded-ctl hover:bg-white/10 text-slate-400 cursor-pointer"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <p className="text-body text-slate-300 leading-relaxed mb-5">{message}</p>

        <div className="flex justify-end gap-2">
          <button
            onClick={onClose}
            className="px-3 py-1.5 rounded-ctl text-caption text-slate-400 hover:bg-white/5 cursor-pointer"
          >
            {t("otp.cancel")}
          </button>
          <button
            onClick={onConfirm}
            className={`px-4 py-1.5 rounded-ctl text-caption text-white font-semibold cursor-pointer hover:opacity-85 flex items-center gap-1 ${
              danger
                ? "bg-rose-600 hover:bg-rose-500"
                : "ui-btn-primary"
            }`}
          >
            <Trash2 className="w-3 h-3" /> {t("otp.delete")}
          </button>
        </div>
      </div>
    </div>
  );
}
