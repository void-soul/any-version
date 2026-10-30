import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { theamedAlert } from "../shared/ThemedAlert";

/** Owns the ApiCollectionExport feature, independently of page rendering. */
export function useApiCollectionExport({ activeProjectId }: {
  activeProjectId: string | null;
}) {

  const [exportPop, setExportPop] = useState(false);

  const exportPostman = async () => {
    if (!activeProjectId) return;
    try {
      const json = await invoke<string>("api_export_postman", { projectId: activeProjectId });
      const target = await saveDialog({ defaultPath: "postman_collection.json", filters: [{ name: "JSON", extensions: ["json"] }] });
      if (target) {
        await invoke("write_text_file", { path: target, content: json });
      }
    } catch (e) {
      theamedAlert(String(e));
    }
  };

  const exportOpenapi = async () => {
    if (!activeProjectId) return;
    try {
      const json = await invoke<string>("api_export_openapi", { projectId: activeProjectId });
      const target = await saveDialog({ defaultPath: "openapi.json", filters: [{ name: "OpenAPI JSON", extensions: ["json"] }] });
      if (target) {
        await invoke("write_text_file", { path: target, content: json });
      }
    } catch (e) {
      theamedAlert(String(e));
    }
  };
  return {
    exportPop,
    setExportPop,
    exportPostman,
    exportOpenapi,
  };
}
