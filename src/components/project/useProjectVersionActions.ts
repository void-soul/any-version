import { useCallback } from "react";
import { theamedAlert, alertError } from "../shared/ThemedAlert";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import type { ProjectDetail } from "./types";
import type { ConfirmRequest } from "../shared/ConfirmDialog";
import { type ProjectUIState } from "./ProjectDetailPanel.model";
import type { Dispatch, SetStateAction, RefObject } from "react";

/** Owns the ProjectVersionActions feature, independently of page rendering. */
export function useProjectVersionActions({ pid, eventProjectRef, patch, refreshSingle, detail, setConfirmRequest, onProjectUpdate }: {
  pid: string | null;
  eventProjectRef: RefObject<string | null>;
  patch: (id: string, partial: Partial<ProjectUIState>) => void;
  refreshSingle: (id: string) => Promise<void>;
  detail: ProjectDetail | null;
  setConfirmRequest: Dispatch<SetStateAction<ConfirmRequest | null>>;
  onProjectUpdate: ((id: string) => Promise<void>) | undefined;
}) {
  const { t } = useTranslation();

  const handleInstall = useCallback(async (version: string) => {
    if (!pid) return;
    eventProjectRef.current = pid;
    patch(pid, { installingVersion: version, installStep: t("projdetail.stepDownloading"), downloadProgress: null });
    try {
      const parts = version.includes(" · ") ? version.split(" · ")[1] : version;
      const cleanVer = parts.trim().split(" ")[0];
      await invoke("project_install_version", { id: pid, version: cleanVer });
      await refreshSingle(pid);
    } catch (e: unknown) {
      alertError(t("projdetail.installFail", { err: String(e) }));
    } finally {
      setTimeout(() => {
        if (eventProjectRef.current === pid) {
          patch(pid, { installingVersion: null, downloadProgress: null, installStep: "" });
          eventProjectRef.current = null;
        }
      }, 3000);
    }
  }, [pid, eventProjectRef, patch, refreshSingle, t]);

  const handleCancelInstall = useCallback(async () => {
    if (!pid) return;
    try {
      await invoke("project_cancel_install", { id: pid });
    } catch {
      // 忽略错误（任务可能已完成）
    } finally {
      patch(pid, { installingVersion: null, downloadProgress: null, installStep: "" });
      eventProjectRef.current = null;
    }
  }, [pid, patch]);

  const handleRefreshRemoteVersions = useCallback(async () => {
    if (!pid) return;
    patch(pid, { loadingRemote: true });
    try {
      const result = await invoke<{ versions: string[]; updated_at: number; from_cache: boolean }>(
        "project_list_remote_versions", { id: pid, force: true }
      );
      patch(pid, { remoteVersions: result.versions, versionsUpdatedAt: result.updated_at, loadingRemote: false });
    } catch (e: unknown) {
      alertError(t("projdetail.refreshVersionsFail", { err: String(e) }));
      patch(pid, { loadingRemote: false });
    }
  }, [pid, patch]);

  /** 真正卸载版本（由确认弹窗回调触发） */
  const doUninstall = useCallback(async (version: string) => {
    if (!pid) return;
    try {
      await invoke("project_uninstall_version", { id: pid, version });
      await refreshSingle(pid);
    } catch (e: unknown) {
      alertError(t("projdetail.uninstallFail", { err: String(e) }));
    }
  }, [pid, refreshSingle, t]);

  const handleUninstall = useCallback(async (version: string) => {
    if (!pid || !detail) return;
    setConfirmRequest({
      title: t("projsub.uninstallThis"),
      danger: true,
      confirmText: t("projdetail.confirmUninstallBtn"),
      desc: t("projdetail.uninstallConfirm", { name: detail.status.display_name, version }),
      onConfirm: () => {
        void doUninstall(version);
      },
    });
  }, [pid, detail, doUninstall, setConfirmRequest, t]);

  const handleUse = useCallback(async (version: string) => {
    if (!pid) return;
    patch(pid, { switchingVersion: version, detectStep: t("projdetail.switching", { version }) });
    try {
      await invoke("project_use_version", { id: pid, version });
      const detail = await invoke<ProjectDetail>("project_detail", { id: pid });
      patch(pid, { detail, switchingVersion: null, detectStep: "" });
      if (onProjectUpdate) await onProjectUpdate(pid);
    } catch (e: unknown) {
      alertError(t("projdetail.switchFail", { err: String(e) }));
      patch(pid, { switchingVersion: null, detectStep: "" });
    }
  }, [pid, patch, onProjectUpdate, t]);

  const handleRepairEnv = useCallback(async () => {
    if (!pid) return;
    patch(pid, { repairingEnv: true });
    try {
      await invoke("project_repair_env_vars", { id: pid });
      await refreshSingle(pid);
      theamedAlert(t("projdetail.envFixed"));
    } catch (e: unknown) {
      alertError(t("projdetail.envFixFail", { err: String(e) }));
    } finally {
      patch(pid, { repairingEnv: false });
    }
  }, [pid, patch, refreshSingle]);
  return {
    handleInstall,
    handleCancelInstall,
    handleRefreshRemoteVersions,
    handleUninstall,
    handleUse,
    handleRepairEnv,
  };
}
