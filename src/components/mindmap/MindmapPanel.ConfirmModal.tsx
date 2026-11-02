import { btnSecondary, btnDanger } from "../shared/Button";
import { useTranslation } from "react-i18next";
import { createPortal } from "react-dom";
import "@xyflow/react/dist/style.css";
import { AlertTriangle } from "lucide-react";
// ════════════ 通用确认弹窗（强调/标题色由调用方按模块主题传入，不写死固定色）════════════

export function ConfirmModal({ title, message, accent, confirmText = "mindmap.confirmDeleteBtn", onConfirm, onClose }: { title: string; message: string; accent: string; confirmText?: string; onConfirm: () => void; onClose: () => void }) {
  const { t } = useTranslation();
  const confirmLabel = String(confirmText).includes(".") ? (t as any)(confirmText) : confirmText;
  return createPortal(
    <div className="fixed inset-0 z-[210] modal-mask flex items-center justify-center bg-black/70 p-6 backdrop-blur-[3px]">
      <div role="dialog" aria-modal="true" aria-label={title} className="ui-modal w-full max-w-[360px] overflow-hidden shadow-2xl" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center gap-2 border-b border-white/10 px-4 py-3">
          <AlertTriangle className="h-4 w-4 shrink-0" style={{ color: accent }} />
          <h3 className="text-title font-semibold text-white">{title}</h3>
        </div>
        <div className="px-4 py-3 text-caption leading-5 text-slate-300">{message}</div>
        <div className="flex justify-end gap-2 px-4 pb-4">
          <button type="button" className={btnSecondary} onClick={onClose}>{t("mindmap.cancel")}</button>
          <button type="button" className={btnDanger} onClick={onConfirm}>{confirmLabel}</button>
        </div>
      </div>
    </div>, document.body);
}
