import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { createPortal } from "react-dom";
import "@xyflow/react/dist/style.css";
import { X } from "lucide-react";
import { ACCENT } from "./MindmapPanel.model";
// ════════════ 创建文档弹窗 ════════════

export function CreateDocModal({ onClose, onCreate, folderId }: { onClose: () => void; onCreate: (name: string, desc: string, folderId: string | null) => void; folderId: string | null }) {
  const { t } = useTranslation();
  const [name, setName] = useState(""); const [desc, setDesc] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => { inputRef.current?.focus(); }, []);

  return createPortal(
    <div className="fixed inset-0 z-[200] modal-mask flex items-center justify-center bg-black/70 p-6 backdrop-blur-[3px]">
      <div className="w-[380px] rounded-card border border-white/10 bg-surface-panel p-5 shadow-2xl" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-sm font-semibold text-white">{t("mindmap.newMapTitle")}</h3>
          <button type="button" className="text-slate-500 hover:text-white" onClick={onClose}><X className="h-4 w-4" /></button>
        </div>
        <div className="space-y-3">
          <div><label className="text-tiny text-slate-400 block mb-1">{t("mindmap.nameLabel")}</label><input ref={inputRef} className="w-full h-9 rounded-ctl ui-input px-3 text-body text-white outline-none" value={name} onChange={(e) => setName(e.target.value)} placeholder={t("mindmap.mapNamePh")} onKeyDown={(e) => { if (e.key === "Enter" && name.trim()) onCreate(name.trim(), desc, folderId); }} /></div>
          <div><label className="text-tiny text-slate-400 block mb-1">{t("mindmap.descLabel")}</label><textarea className="w-full h-16 rounded-ctl ui-input px-3 py-2 text-body text-white outline-none resize-none" value={desc} onChange={(e) => setDesc(e.target.value)} placeholder={t("mindmap.descPh")} /></div>
          <button type="button" className="w-full rounded-ctl py-2 text-caption font-semibold text-white disabled:opacity-40" style={{ backgroundColor: ACCENT }}
            disabled={!name.trim()} onClick={() => { if (name.trim()) onCreate(name.trim(), desc, folderId); }}>{t("mindmap.create")}</button>
        </div>
      </div>
    </div>, document.body);
}
