import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import type { ProjectStatus, ProjectDetail } from "./types";
import { type ProjectUIState } from "./ProjectDetailPanel.model";

/** Owns the ProjectDetection feature, independently of page rendering. */
export function useProjectDetection({ patch }: {
  patch: (id: string, partial: Partial<ProjectUIState>) => void;
}) {
  const { t } = useTranslation();

  // 首次打开项目时的逐步检测队列
  const runInitialDetection = useCallback(async (id: string, proj: ProjectStatus) => {
    const steps: Array<{ label: string; run: () => Promise<void> }> = [];

    // 步骤1：加载项目详情（版本、环境变量等）
    steps.push({
      label: t("projdetail.detecting", { name: proj.display_name }),
      run: async () => {
        try {
          const d = await invoke<ProjectDetail>("project_detail", { id });
          patch(id, { detail: d, detailLoaded: true });
        } catch (e) {
          console.error("检测项目详情失败:", e);
        }
      },
    });

    // 步骤2：获取远程版本列表（优先读缓存）
    steps.push({
      label: t("projdetail.fetchingVersions", { name: proj.display_name }),
      run: async () => {
        try {
          const result = await invoke<{ versions: string[]; updated_at: number; from_cache: boolean }>(
            "project_list_remote_versions", { id, force: false }
          );
          patch(id, { remoteVersions: result.versions, versionsUpdatedAt: result.updated_at });
        } catch (e) {
          console.error("获取远程版本列表失败:", e);
        }
      },
    });

    // 步骤3：标记全局包待检测（不在初始检测中执行，太慢）
    // 全局包检测改为懒加载：用户切换到"全局包"标签页时才触发

    patch(id, { detailLoading: true, loadingRemote: true, loadingPackages: true, detectTotal: steps.length });

    for (let i = 0; i < steps.length; i++) {
      patch(id, { detectIndex: i + 1, detectStep: steps[i].label });
      await steps[i].run();
    }

    patch(id, {
      detailLoading: false,
      loadingRemote: false,
      loadingPackages: false,
      detectStep: "",
      detectIndex: 0,
      detectTotal: 0,
    });
  }, [patch]);
  return {
    runInitialDetection,
  };
}
