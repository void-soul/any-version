import { useEffect, useCallback } from "react";
import { theamedAlert, alertError } from "../shared/ThemedAlert";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ProjectDetail } from "./types";
import type { ConfirmRequest } from "../shared/ConfirmDialog";
import { type ProjectUIState } from "./ProjectDetailPanel.model";
import type { Dispatch, SetStateAction } from "react";

/** Owns the ProjectServiceActions feature, independently of page rendering. */
export function useProjectServiceActions({ pid, patch, refreshSingle, detail, setConfirmRequest, isAdmin }: {
  pid: string | null;
  patch: (id: string, partial: Partial<ProjectUIState>) => void;
  refreshSingle: (id: string) => Promise<void>;
  detail: ProjectDetail | null;
  setConfirmRequest: Dispatch<SetStateAction<ConfirmRequest | null>>;
  isAdmin: boolean;
}) {
  const { t } = useTranslation();

  /** 强制终止服务进程（安全停止失败并确认后调用） */
  const forceStopService = useCallback(async () => {
    if (!pid) return;
    patch(pid, { serviceCtrlLoading: true });
    try {
      await invoke("force_stop_service", { name: pid });
      await invoke("refresh_tray_menu");
      await refreshSingle(pid);
    } catch (e: unknown) {
      alertError(t("projdetail.serviceOpFail", { err: String(e) }));
    } finally {
      patch(pid, { serviceCtrlLoading: false });
    }
  }, [pid, patch, refreshSingle, t]);

  /** 真正执行服务启停（由确认弹窗或无需确认的路径调用） */
  const runServiceToggle = useCallback(async () => {
    if (!pid || !detail?.status?.service_status) return;
    const running = detail.status.service_status.running;
    const simpleService = detail.def.simple_mode || detail.status.is_simple_managed;

    patch(pid, { serviceCtrlLoading: true });
    try {
      if (running) {
        try {
          await invoke("stop_service", { name: pid });
        } catch (stopErr) {
          const msg = String(stopErr);
          // 安全停止失败：用统一弹窗询问是否强制终止。
          // 用户取消时不再抛出原错误（旧实现会紧跟一条"服务操作失败"的 alert，
          // 与「用户主动取消」的语义冲突）。
          patch(pid, { serviceCtrlLoading: false });
          setConfirmRequest({
            title: t("projdetail.stopFailTitle"),
            danger: true,
            confirmText: t("projdetail.forceStopBtn"),
            desc: t("projdetail.stopFailConfirm", { msg, pid }),
            onConfirm: () => {
              void forceStopService();
            },
          });
          return;
        }
      } else {
        if (!simpleService && !detail.status.active_version) {
          theamedAlert(t("projdetail.needEnableFirst"));
          patch(pid, { serviceCtrlLoading: false });
          return;
        }
        await invoke("start_service", {
          name: pid,
          version: simpleService ? null : detail.status.active_version,
        });
      }
      await invoke("refresh_tray_menu");
      await refreshSingle(pid);
    } catch (e: unknown) {
      alertError(t("projdetail.serviceOpFail", { err: String(e) }));
    } finally {
      patch(pid, { serviceCtrlLoading: false });
    }
  }, [pid, detail, patch, refreshSingle, forceStopService, t]);

  /** 服务启停入口：非管理员且有系统服务时先确认（统一弹窗），再执行 */
  const handleServiceToggle = useCallback(async () => {
    if (!pid || !detail?.status?.service_status) return;
    if (!isAdmin && Array.isArray(detail.def.service_names) && detail.def.service_names.length > 0) {
      setConfirmRequest({
        title: t("projdetail.adminWarnTitle"),
        confirmText: t("common.confirm"),
        desc: t("projdetail.adminWarn"),
        onConfirm: () => {
          void runServiceToggle();
        },
      });
      return;
    }
    await runServiceToggle();
  }, [pid, detail, isAdmin, runServiceToggle, t]);

  // 服务类项目：静默轮询服务状态，进程意外退出后前端能自动感知，
  // 避免界面仍显示陈旧 PID 导致「停止」误报「未检测到进程」。
  const _def = detail?.def;

  const isServiceProject = !!_def && (_def.is_service || _def.category === "service");

  useEffect(() => {
    if (!pid || !isServiceProject) return;
    const poll = () => {
      invoke<ProjectDetail>("project_detail", { id: pid })
        .then((d) => patch(pid, { detail: d }))
        .catch(() => {});
    };
    const t = setInterval(poll, 3000);
    return () => clearInterval(t);
  }, [pid, isServiceProject, patch]);

  // 后端在启动/停止/强制终止服务成功后主动 emit 该事件，前端即时刷新，
  // 不再等待 3s 轮询的缓存空窗，避免 redis/mysql 等启动较慢的服务状态更新不及时。
  useEffect(() => {
    if (!pid) return;
    let unlisten: (() => void) | undefined;
    listen<string>("service-status-changed", (e) => {
      if (e.payload === pid) {
        refreshSingle(pid);
      }
    }).then((u) => { unlisten = u; });
    return () => unlisten?.();
  }, [pid, refreshSingle]);
  return {
    handleServiceToggle,
  };
}
