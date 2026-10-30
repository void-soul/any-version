import { X, Cloud } from "lucide-react";
export function ModalShell({ title, onClose, children }: { title: string; onClose: () => void; children: React.ReactNode }) {
  return (
    <div className="fixed inset-0 z-[110] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
      <div
        className="w-[460px] max-w-[95vw] max-h-[90vh] overflow-y-auto rounded-panel border border-white/10 bg-slate-900/95 shadow-2xl p-5"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-sm font-bold text-white flex items-center gap-2">
            <Cloud className="w-4 h-4 text-[var(--module-accent)]" /> {title}
          </h3>
          <button onClick={onClose} className="p-1 rounded hover:bg-white/10 text-slate-400 cursor-pointer">
            <X className="w-4 h-4" />
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}
