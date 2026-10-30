import { useTranslation } from "react-i18next";
import { FolderPlus, FilePlus2, Download, Upload, FileText } from "lucide-react";
import { POPOVER_BG } from "./ApiPanel.model";
import type { Dispatch, SetStateAction } from "react";

interface ApiCollectionActionsProps {
  openCreateModule: () => void;
  createEndpoint: (moduleId: string | null) => Promise<void>;
  setImportModal: Dispatch<SetStateAction<boolean>>;
  setExportPop: Dispatch<SetStateAction<boolean>>;
  exportPop: boolean;
  exportPostman: () => Promise<void>;
  exportOpenapi: () => Promise<void>;
}

/** Pure ApiCollectionActions view; its owning panel retains side effects and state. */
export function ApiCollectionActions({ openCreateModule, createEndpoint, setImportModal, setExportPop, exportPop, exportPostman, exportOpenapi }: ApiCollectionActionsProps) {
  const { t } = useTranslation();
  return (
    <div className="p-2 border-t border-white/10 space-y-1.5">
      <div className="grid grid-cols-2 gap-1.5">
        <button onClick={openCreateModule} className="flex items-center justify-center gap-1.5 px-2 py-1.5 text-caption text-slate-400 hover:text-white hover:bg-white/5 border border-white/10 rounded-md cursor-pointer" title={t("pmodals.newModule")}>
          <FolderPlus className="w-3.5 h-3.5" /> {t("api.newModuleBtn")}
        </button>
        <button onClick={() => createEndpoint(null)} className="flex items-center justify-center gap-1.5 px-2 py-1.5 text-caption text-slate-400 hover:text-white hover:bg-white/5 border border-white/10 rounded-md cursor-pointer" title={t("api.addEndpoint")}>
          <FilePlus2 className="w-3.5 h-3.5" /> {t("api.newEndpointBtn")}
        </button>
      </div>
      <div className="grid grid-cols-2 gap-1.5">
        <button onClick={() => setImportModal(true)} className="flex items-center justify-center gap-1.5 px-2 py-1.5 text-caption text-slate-400 hover:text-white hover:bg-white/5 border border-white/10 rounded-md cursor-pointer" title={t("api.importTip")}>
          <Upload className="w-3.5 h-3.5" /> {t("api.importBtn")}
        </button>
        <div className="relative">
          <button
            onClick={() => setExportPop((v) => !v)}
            className="flex w-full items-center justify-center gap-1.5 px-2 py-1.5 text-caption text-slate-400 hover:text-white hover:bg-white/5 border border-white/10 rounded-md cursor-pointer"
            title={t("api.exportTip")}
          >
            <Download className="w-3.5 h-3.5" /> {t("api.exportBtn")}
          </button>
          {exportPop && (
            <>
              <div className="fixed inset-0 z-30" onClick={() => setExportPop(false)} />
              <div className="absolute bottom-full right-0 z-40 mb-1.5 w-44 overflow-hidden rounded-card border border-white/10 shadow-2xl" style={{ background: POPOVER_BG }}>
                <div className="px-2.5 py-1.5 text-micro font-semibold text-slate-500 border-b border-white/10">{t("api.exportFormat")}</div>
                <button
                  onClick={() => { setExportPop(false); exportPostman(); }}
                  className="flex w-full items-center gap-2 px-2.5 py-1.5 text-caption text-slate-300 hover:bg-white/5 hover:text-white cursor-pointer"
                >
                  <Download className="w-3 h-3" /> {t("api.exportPostmanOpt")}
                </button>
                <button
                  onClick={() => { setExportPop(false); exportOpenapi(); }}
                  className="flex w-full items-center gap-2 px-2.5 py-1.5 text-caption text-slate-300 hover:bg-white/5 hover:text-white cursor-pointer"
                >
                  <FileText className="w-3 h-3" /> {t("api.exportOpenapiOpt")}
                </button>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
