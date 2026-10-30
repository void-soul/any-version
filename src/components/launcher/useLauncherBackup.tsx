import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { theamedConfirm } from "../shared/ThemedAlert";

/** Owns the LauncherBackup feature, independently of page rendering. */
export function useLauncherBackup({ showToast, loadData }: {
  showToast: (msg: string) => void;
  loadData: () => Promise<void>;
}) {
  const { t } = useTranslation();


  // 备份导入/导出进行中
  const [backupBusy, setBackupBusy] = useState(false);


  // 导出 / 导入整套启动器数据
  const handleExportBackup = async () => {
    if (backupBusy) return;
    try {
      const path = await saveDialog({
        title: t("launcher.exportBackupTitle"),
        defaultPath: `kira-launcher-${new Date().toISOString().slice(0, 10)}.json`,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!path || typeof path !== "string") return;
      setBackupBusy(true);
      await invoke("launcher_export_backup", { path });
      showToast(t("launcher.exportBackupOk"));
    } catch (e) {
      showToast(t("launcher.exportBackupFail", { err: String(e) }));
    } finally {
      setBackupBusy(false);
    }
  };


  const handleImportBackup = async () => {
    if (backupBusy) return;
    try {
      const selected = await openDialog({
        title: t("launcher.importBackupTitle"),
        multiple: false,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      const path = Array.isArray(selected) ? selected[0] : selected;
      if (!path || typeof path !== "string") return;
      const ok = await theamedConfirm(t("launcher.importBackupConfirm"), { danger: true });
      if (!ok) return;
      setBackupBusy(true);
      await invoke("launcher_import_backup", { path });
      await loadData();
      showToast(t("launcher.importBackupOk"));
    } catch (e) {
      showToast(t("launcher.importBackupFail", { err: String(e) }));
    } finally {
      setBackupBusy(false);
    }
  };
  return {
    backupBusy,
    handleExportBackup,
    handleImportBackup,
  };
}
