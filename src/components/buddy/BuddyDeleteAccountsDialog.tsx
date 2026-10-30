import { useTranslation } from "react-i18next";
import { Trash2 } from "lucide-react";
import { type BuddyAccount } from "./BuddyPanel.model";
import type { Dispatch, SetStateAction } from "react";

interface BuddyDeleteAccountsDialogProps {
  setDeleteIds: Dispatch<SetStateAction<string[] | null>>;
  deleteIds: string[];
  accounts: BuddyAccount[];
  displayName: (acc: BuddyAccount) => string;
  deleteAccounts: (ids: string[]) => Promise<void>;
  busy: boolean;
}

/** Pure BuddyDeleteAccountsDialog view; its owning panel retains side effects and state. */
export function BuddyDeleteAccountsDialog({ setDeleteIds, deleteIds, accounts, displayName, deleteAccounts, busy }: BuddyDeleteAccountsDialogProps) {
  const { t } = useTranslation();
  return (
    <div className="fixed inset-0 z-[130] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
      <div className="w-[360px] max-w-[95vw] rounded-panel border border-white/10 bg-slate-900/95 shadow-2xl p-5">
        <div className="flex items-center gap-2.5 mb-4">
          <div className="w-9 h-9 rounded-card bg-rose-500/15 border border-rose-500/30 flex items-center justify-center">
            <Trash2 className="w-4 h-4 text-rose-400" />
          </div>
          <div className="flex-1">
            <h3 className="text-sm font-bold text-white">{t("buddy.delTitle")}</h3>
            <p className="text-tiny text-slate-500">{t("buddy.delHint")}</p>
          </div>
          <button onClick={() => setDeleteIds(null)} className="p-1.5 rounded-ctl hover:bg-white/10 text-slate-400 cursor-pointer">
            ✕
          </button>
        </div>
        <p className="text-body text-slate-300 leading-relaxed mb-5">
          {deleteIds.length === 1
            ? (() => {
                const target = accounts.find((a) => a.id === deleteIds[0]);
                return t("buddy.delMsgSingle", {
                  name: target ? displayName(target) : deleteIds[0],
                });
              })()
            : (() => {
                // 按用户要求列出账号名让用户确认删的是哪些；超过 8 个截断（全列会撑爆弹窗）
                const names = deleteIds
                  .map((id) => accounts.find((a) => a.id === id))
                  .filter((a): a is BuddyAccount => !!a)
                  .map((a) => displayName(a));
                const shown = names.slice(0, 8).join("、");
                return t("buddy.delMsg", {
                  names: names.length > 8 ? `${shown} 等 ${names.length} 个` : shown,
                });
              })()}
        </p>
        <div className="flex justify-end gap-2">
          <button onClick={() => setDeleteIds(null)} className="px-3 py-1.5 rounded-ctl text-caption text-slate-400 hover:bg-white/5 cursor-pointer">
            {t("buddy.cancel")}
          </button>
          <button
            onClick={() => void deleteAccounts(deleteIds)}
            disabled={busy}
            className="px-4 py-1.5 rounded-ctl text-caption bg-rose-600 hover:bg-rose-500 text-white font-semibold cursor-pointer disabled:opacity-50"
          >
            {t("buddy.deleteConfirm")}
          </button>
        </div>
      </div>
    </div>
  );
}
