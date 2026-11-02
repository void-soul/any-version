import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Pencil, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { SharedModal } from "../shared/Modal";
import { SharedButton, inputCls, labelCls } from "../shared/Button";
import { buddyAccountDisplayName } from "./accountDisplayName";
import type { BuddyAccount } from "./BuddyPanel.model";
import type { BuddyPlatformScope } from "./platformScope";

export function BuddyAccountName({ account, platform, scope, onSaved }: {
  account: BuddyAccount; platform: string; scope: BuddyPlatformScope;
  onSaved: (accountId: string, remark: string | null) => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const save = async () => {
    const lease = scope.capture();
    if (!lease.current() || saving) return;
    setSaving(true); setError(null);
    try {
      const remark = await invoke<string | null>("buddy_set_account_remark", {
        platform, accountId: account.id, remark: draft,
      });
      if (!lease.current()) return;
      onSaved(account.id, remark); setOpen(false);
    } catch (error) {
      if (lease.current()) setError(String(error));
    } finally {
      if (lease.current()) setSaving(false);
    }
  };
  return <>
    <button type="button" title={t("buddy.editAccountRemark")}
      onClick={() => { setDraft(account.remark ?? ""); setError(null); setOpen(true); }}
      className="group inline-flex items-center gap-1 min-w-0 cursor-pointer text-left">
      <span className="font-semibold text-white truncate max-w-[150px]">{buddyAccountDisplayName(account)}</span>
      <Pencil className="w-3 h-3 text-slate-500 group-hover:text-slate-200 flex-shrink-0" />
    </button>
    <SharedModal open={open} onClose={() => { if (!saving) setOpen(false); }}
      title={t("buddy.editAccountRemark")} width={420}>
      <form className="space-y-3" onSubmit={(event) => { event.preventDefault(); void save(); }}>
        <p className="text-tiny text-slate-500 break-all">{account.nickname?.trim() || account.email || account.uid}</p>
        <label className={labelCls}>
          {t("buddy.accountRemark")}
          <input autoFocus value={draft} onChange={(event) => setDraft(event.target.value)}
            disabled={saving} maxLength={200} placeholder={t("buddy.accountRemarkPlaceholder")}
            className={`${inputCls} mt-1`} />
        </label>
        <p className="text-tiny text-slate-500">{t("buddy.accountRemarkHint")}</p>
        {error && <p role="alert" className="text-tiny text-red-400 break-all">{error}</p>}
        <div className="flex justify-end gap-2">
          <SharedButton type="button" disabled={saving} onClick={() => setOpen(false)}>{t("common.cancel")}</SharedButton>
          <SharedButton variant="primary" type="submit" disabled={saving}>
            {saving && <Loader2 className="w-3 h-3 animate-spin" />}{t("common.save")}
          </SharedButton>
        </div>
      </form>
    </SharedModal>
  </>;
}
