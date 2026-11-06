/**
 * ApiPanel 的全部弹窗集合（AUDIT-049 职责拆分）。
 * 从主面板抽离，主面板只负责布局与请求编排；这里只做「弹窗是否显示 + 变更后刷新」。
 */
import { invoke } from "@tauri-apps/api/core";
import type {
    ApiProject, ApiEnvironment, ApiModule, PresetHeaderSet, KeyValueItem,
} from "./types";
import type { ProjectTemplateSection } from "./panelModals";
import { PresetHeadersModal, EnvModal, ProjectModal, ModuleModal } from "./panelModals";
import { ImportModal } from "./panelSubs";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import { MoveModuleModal } from "./ApiPanel.MoveModuleModal";

export interface ConfirmState {
    title: string;
    message: string;
    danger?: boolean;
    confirmText?: string;
    onConfirm: () => void;
}

export interface ProjectModalState { open: boolean; project: ApiProject | null }
export interface ModuleModalState { open: boolean; module: ApiModule | null }

export interface ApiPanelModalsProps {
    activeProjectId: string | null;
    envs: ApiEnvironment[];
    activeEnvId: string | null;
    setEnvs: (v: ApiEnvironment[]) => void;
    setActiveEnvId: (v: string | null) => void;
    modules: ApiModule[];
    setModules: (v: ApiModule[]) => void;
    presetSets: PresetHeaderSet[];
    setPresetSets: (v: PresetHeaderSet[]) => void;
    setSelectedId: (v: string | null) => void;
    refreshEndpoints: () => Promise<void>;

    envModal: boolean;
    setEnvModal: (v: boolean) => void;
    presetModal: boolean;
    setPresetModal: (v: boolean) => void;
    importModal: boolean;
    setImportModal: (v: boolean) => void;

    projectModal: ProjectModalState;
    setProjectModal: (v: ProjectModalState) => void;
    tplFocusSection: ProjectTemplateSection | null;
    setTplFocusSection: (v: ProjectTemplateSection | null) => void;
    saveProjectModal: (
        name: string, description: string,
        commonHeaders: KeyValueItem[], commonParams: KeyValueItem[], commonBody: KeyValueItem[],
    ) => Promise<void> | void;

    moduleModal: ModuleModalState;
    setModuleModal: (v: ModuleModalState) => void;
    saveModuleModal: (name: string, description: string) => Promise<void> | void;

    confirm: ConfirmState | null;
    setConfirm: React.Dispatch<React.SetStateAction<ConfirmState | null>>;
    moveModule: ApiModule | null;
    setMoveModule: (v: ApiModule | null) => void;
    doMoveModule: (targetId: string) => Promise<void> | void;
}

export function ApiPanelModals(p: ApiPanelModalsProps) {
    const projectId = p.activeProjectId;

    const reloadProjectScope = async () => {
        if (!projectId) return;
        const [modList, envList, presetList] = await Promise.all([
            invoke<ApiModule[]>("api_list_modules", { projectId }),
            invoke<ApiEnvironment[]>("api_list_environments", { projectId }),
            invoke<PresetHeaderSet[]>("api_list_preset_headers", { projectId }),
        ]);
        p.setModules(modList);
        p.setEnvs(envList);
        p.setPresetSets(presetList);
    };

    return (
        <>
            {p.envModal && projectId && (
                <EnvModal
                    projectId={projectId}
                    envs={p.envs}
                    activeEnvId={p.activeEnvId}
                    onClose={() => p.setEnvModal(false)}
                    onChanged={async () => {
                        const envList = await invoke<ApiEnvironment[]>("api_list_environments", { projectId });
                        p.setEnvs(envList);
                        const project = (await invoke<ApiProject[]>("api_list_projects"))
                            .find((x) => x.id === projectId);
                        p.setActiveEnvId(project?.active_env_id ?? envList[0]?.id ?? null);
                    }}
                />
            )}

            {p.presetModal && projectId && (
                <PresetHeadersModal
                    projectId={projectId}
                    sets={p.presetSets}
                    onClose={() => p.setPresetModal(false)}
                    onChanged={async () => {
                        p.setPresetSets(await invoke<PresetHeaderSet[]>("api_list_preset_headers", { projectId }));
                    }}
                />
            )}

            {p.importModal && projectId && (
                <ImportModal
                    projectId={projectId}
                    modules={p.modules}
                    onClose={() => p.setImportModal(false)}
                    onImported={async (endpointId?: string) => {
                        p.setImportModal(false);
                        await p.refreshEndpoints();
                        await reloadProjectScope();
                        if (endpointId) p.setSelectedId(endpointId);
                    }}
                />
            )}

            {p.projectModal.open && (
                <ProjectModal
                    project={p.projectModal.project}
                    initialSection={p.tplFocusSection}
                    onClose={() => {
                        p.setTplFocusSection(null);
                        p.setProjectModal({ open: false, project: null });
                    }}
                    onSave={p.saveProjectModal}
                />
            )}

            {p.moduleModal.open && (
                <ModuleModal
                    module={p.moduleModal.module}
                    onClose={() => p.setModuleModal({ open: false, module: null })}
                    onSave={p.saveModuleModal}
                />
            )}

            {p.confirm && (
                <ConfirmDialog
                    open
                    title={p.confirm.title}
                    desc={p.confirm.message}
                    danger={p.confirm.danger}
                    confirmText={p.confirm.confirmText}
                    onConfirm={() => { const fn = p.confirm!.onConfirm; p.setConfirm(null); fn(); }}
                    onCancel={() => p.setConfirm(null)}
                />
            )}

            {p.moveModule && (
                <MoveModuleModal
                    module={p.moveModule}
                    modules={p.modules}
                    onClose={() => p.setMoveModule(null)}
                    onMoved={p.doMoveModule}
                />
            )}
        </>
    );
}
