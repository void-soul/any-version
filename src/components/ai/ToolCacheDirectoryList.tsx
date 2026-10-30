import type { AiToolCacheInfo } from "../../contracts/ai";
import { useTranslation } from "react-i18next";
import { FolderOpen, HardDrive, Trash2, FolderSync } from "lucide-react";

interface ToolCacheDirectoryListProps {
  cacheInfos: AiToolCacheInfo[];
  selectedToolCaches: AiToolCacheInfo[];
  handleOpenCacheDir: (fullPath: string) => Promise<void>;
  handleMigrateCache: (toolId: string, dirName: string, _fullPath: string) => Promise<void>;
  migratingCache: string | null;
  handleCleanCache: (toolId: string, dirName: string) => Promise<void>;
  cleaningCache: string | null;
}

/** Pure ToolCacheDirectoryList view; its owning panel retains side effects and state. */
export function ToolCacheDirectoryList({ cacheInfos, selectedToolCaches, handleOpenCacheDir, handleMigrateCache, migratingCache, handleCleanCache, cleaningCache }: ToolCacheDirectoryListProps) {
  const { t } = useTranslation();
  return (
    <div className="mt-2 rounded-ctl border border-white/5 bg-slate-900/30 overflow-hidden">
      <div className="max-h-56 overflow-y-auto divide-y divide-white/[0.03]">
        {cacheInfos.length === 0 ? (
          <div className="px-3 py-4 text-tiny text-slate-600 text-center">{t("toollaunch.loading")}</div>
        ) : selectedToolCaches.length === 0 ? (
          <div className="px-3 py-4 text-tiny text-slate-600 text-center">{t("toollaunch.noCache")}</div>
        ) : (
          selectedToolCaches.map(cache => (
            <div key={`${cache.tool_id}:${cache.dir_name}`} className="px-3 py-2 flex items-center gap-3">
              <HardDrive className="w-3 h-3 text-slate-600 flex-shrink-0" />
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="text-tiny text-slate-300 font-mono truncate">{cache.dir_name}</span>
                  {cache.is_junction && (
                    <span className="text-[8px] text-blue-400 bg-blue-500/10 px-1 rounded">JUNCTION</span>
                  )}
                </div>
                <div className="text-micro text-slate-500 font-mono truncate mt-0.5" title={cache.full_path}>
                  {cache.exists ? cache.full_path : t("toollaunch.notExists")}
                </div>
                {cache.is_junction && cache.junction_target && (
                  <div className="text-[8px] text-blue-400/70 font-mono truncate mt-0.5" title={cache.junction_target}>
                    ↳ {cache.junction_target}
                  </div>
                )}
                <div className="text-[8px] text-slate-600 mt-0.5">{cache.exists ? cache.size : "0 B"}</div>
              </div>
              {cache.exists && (
                <div className="flex items-center gap-1 flex-shrink-0">
                  <button onClick={() => handleOpenCacheDir(cache.full_path)}
                    className="p-1 rounded text-slate-600 hover:text-[var(--module-accent)] hover:bg-blue-500/10 cursor-pointer"
                    title={t("toollaunch.openDir")}>
                    <FolderOpen className="w-3 h-3" />
                  </button>
                  <button onClick={() => handleMigrateCache(cache.tool_id, cache.dir_name, cache.full_path)}
                    disabled={migratingCache === `${cache.tool_id}:${cache.dir_name}`}
                    className="p-1 rounded text-slate-600 hover:text-emerald-400 hover:bg-emerald-500/10 cursor-pointer disabled:opacity-50"
                    title={t("toollaunch.migrateCache")}>
                    <FolderSync className="w-3 h-3" />
                  </button>
                  <button onClick={() => handleCleanCache(cache.tool_id, cache.dir_name)}
                    disabled={cleaningCache === `${cache.tool_id}:${cache.dir_name}`}
                    className="p-1 rounded text-slate-600 hover:text-red-400 hover:bg-red-500/10 cursor-pointer disabled:opacity-50"
                    title={t("toollaunch.clearCache")}>
                    <Trash2 className="w-3 h-3" />
                  </button>
                </div>
              )}
            </div>
          ))
        )}
      </div>
    </div>
  );
}
