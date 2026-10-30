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
      <div className="w-[360px] overflow-hidden rounded-card border border-white/10 bg-surface-panel shadow-2xl" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center gap-2 border-b border-white/10 px-4 py-3">
          <AlertTriangle className="h-4 w-4 shrink-0" style={{ color: accent }} />
          <h3 className="text-sm font-semibold text-white">{title}</h3>
        </div>
        <div className="px-4 py-3 text-caption leading-5 text-slate-300">{message}</div>
        <div className="flex justify-end gap-2 px-4 pb-4">
          <button type="button" className="rounded-md border border-white/10 bg-white/[0.05] px-4 py-1.5 text-caption text-slate-300 hover:bg-white/10 hover:text-white" onClick={onClose}>{t("mindmap.cancel")}</button>
          <button type="button" className="rounded-md px-4 py-1.5 text-caption font-semibold text-white" style={{ backgroundColor: "#ef4444", boxShadow: `0 0 14px ${accent}66` }} onClick={onConfirm}>{confirmLabel}</button>
        </div>
      </div>
    </div>, document.body);
}
