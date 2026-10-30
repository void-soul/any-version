import { useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import type { ApiProject, KeyValueItem } from "./types";
import type { Dispatch, SetStateAction } from "react";

/** Owns the ApiProjectActions feature, independently of page rendering. */
export function useApiProjectActions({ projects, activeProjectId, setShowTplPanel, tplItems, setProjects, setActiveProjectId, setConfirm }: {
  projects: ApiProject[];
  activeProjectId: string | null;
  setShowTplPanel: Dispatch<SetStateAction<boolean>>;
  tplItems: { total: number; outOfSync: number; groups: { label: string; items: (KeyValueItem & { synced: boolean; })[]; }[]; };
  setProjects: Dispatch<SetStateAction<ApiProject[]>>;
  setActiveProjectId: Dispatch<SetStateAction<string | null>>;
  setConfirm: Dispatch<SetStateAction<{ title: string; message: string; danger?: boolean | undefined; confirmText?: string | undefined; onConfirm: () => void; } | null>>;
}) {
  const { t } = useTranslation();

  // 从模板继承面板打开项目弹窗时，定位到哪个模板编辑区
  const [tplFocusSection, setTplFocusSection] = useState<"headers" | "params" | "body" | null>(null);

  const [projectModal, setProjectModal] = useState<{ open: boolean; project: ApiProject | null }>({ open: false, project: null });
  const openCreateProject = () => {
    setTplFocusSection(null);
    setProjectModal({ open: true, project: null });
  };

  const openEditProject = (p: ApiProject) => {
    setTplFocusSection(null);
    setProjectModal({ open: true, project: p });
  };

  // 模板继承面板的「打开项目模板」：定位到未同步项最多的分区
  const openProjectTpl = () => {
    const proj = projects.find((p) => p.id === activeProjectId);
    if (!proj) return;
    setShowTplPanel(false);
    const labelToSec = (label: string): "headers" | "params" | "body" | null =>
      label === "Headers" ? "headers" : label === "Query Params" ? "params" : label.startsWith("Body") ? "body" : null;
    const unsynced = tplItems.groups.filter((g) => g.items.some((it) => !it.synced));
    const primary = (unsynced.length > 0 ? unsynced : tplItems.groups)
      .map((g) => labelToSec(g.label))
      .find((s): s is "headers" | "params" | "body" => s !== null) ?? "headers";
    setTplFocusSection(primary);
    setProjectModal({ open: true, project: proj });
  };

  const saveProjectModal = async (name: string, description: string, commonHeaders: KeyValueItem[], commonParams: KeyValueItem[], commonBody: KeyValueItem[]) => {
    if (projectModal.project) {
      const updated = { ...projectModal.project, name, description, common_headers: commonHeaders, common_params: commonParams, common_body: commonBody };
      await invoke("api_update_project", { project: updated });
      setProjects(projects.map((p) => (p.id === updated.id ? updated : p)));
    } else {
      await invoke("api_create_project", { name, description });
      const list = await invoke<ApiProject[]>("api_list_projects");
      setProjects(list);
      setActiveProjectId(list[list.length - 1].id);
      // 新项目模板字段由后端默认空；若用户在创建时就填了模板，则再更新一次
      if (commonHeaders.length > 0 || commonParams.length > 0 || commonBody.length > 0) {
        const created = list[list.length - 1];
        await invoke("api_update_project", { project: { ...created, common_headers: commonHeaders, common_params: commonParams, common_body: commonBody } });
        setProjects(await invoke<ApiProject[]>("api_list_projects"));
      }
    }
    setTplFocusSection(null);
    setProjectModal({ open: false, project: null });
  };

  const deleteProject = async (id: string) => {
    const p = projects.find((x) => x.id === id);
    setConfirm({
      title: t("api.confirmDeleteProjectTitle"),
      message: t("api.confirmDeleteProjectMsg", { name: p?.name ?? "" }),
      danger: true,
      confirmText: t("common.delete"),
      onConfirm: async () => {
        await invoke("api_delete_project", { projectId: id });
        const list = projects.filter((x) => x.id !== id);
        setProjects(list);
        if (activeProjectId === id) setActiveProjectId(list[0]?.id ?? null);
      },
    });
  };
  return {
    tplFocusSection,
    setTplFocusSection,
    projectModal,
    setProjectModal,
    openCreateProject,
    openEditProject,
    openProjectTpl,
    saveProjectModal,
    deleteProject,
  };
}
