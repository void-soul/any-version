import { useState } from "react";
import { useTranslation } from "react-i18next";
import { FolderInput } from "lucide-react";
import type { ApiModule } from "./types";
import { ACCENT } from "./panelParts";
import { SharedModal } from "../shared/Modal";
import { SharedButton } from "../shared/Button";
// ─── 模块转移弹窗（选择目标模块） ───
export function MoveModuleModal({ module, modules, onClose, onMoved }: {
  module: ApiModule;
  modules: ApiModule[];
  onClose: () => void;
  onMoved: (targetId: string) => void;
}) {
  const [targetId, setTargetId] = useState("");
  const { t } = useTranslation();
  const targets = modules.filter((m) => m.id !== module.id);
  return (
    <SharedModal
      open
      onClose={onClose}
      width={420}
      title={
        <span className="inline-flex items-center gap-2">
          <FolderInput className="w-4 h-4" style={{ color: ACCENT }} /> {t("api.moveTitle")}
        </span>
      }
      footer={
        <>
          <SharedButton onClick={onClose}>{t("common.cancel")}</SharedButton>
          <SharedButton onClick={() => onMoved(targetId)} variant="primary" disabled={!targetId}>
            {t("api.move")}
          </SharedButton>
        </>
      }
    >
      <p className="text-body leading-relaxed text-slate-400 mb-3">
        {t("api.moveDesc", { name: module.name })}
      </p>
      <select
        value={targetId}
        onChange={(e) => setTargetId(e.target.value)}
        className="w-full rounded-ctl border border-white/10 bg-black/30 px-3 py-2 text-body text-slate-200 focus:outline-none focus:border-[var(--module-accent)]/60 cursor-pointer"
      >
        <option value="">{t("api.selectTarget")}</option>
        {targets.map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
      </select>
    </SharedModal>
  );
}
