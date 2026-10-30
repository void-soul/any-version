import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, Trash2, Download, Globe, FolderOpen, Loader } from "lucide-react";
// ═══════════════════════════════════════
//  旧版数据（托管前备份的安装信息）
// ═══════════════════════════════════════
export function LegacyTab({ projectId }: { projectId: string }) {
  const { t } = useTranslation();
  const [data, setData] = useState<{
    install_source?: string;
    install_root?: string;
    version?: string;
    backed_env_vars: Record<string, string>;
    removed_path_entries: string[];
    timestamp: number;
  } | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    invoke<{
      project_id: string;
      install_source?: string;
      install_root?: string;
      version?: string;
      backed_env_vars: Record<string, string>;
      removed_path_entries: string[];
      timestamp: number;
    } | null>("get_legacy_backup", { id: projectId })
      .then((info) => setData(info))
      .catch(() => setData(null))
      .finally(() => setLoading(false));
  }, [projectId]);

  if (loading) {
    return (
      <div className="flex items-center justify-center gap-2 text-body text-slate-400 py-8">
        <Loader className="w-4 h-4 animate-spin text-[var(--module-accent)]" /> {t("projsub.loadingOldData")}
      </div>
    );
  }

  if (!data) {
    return (
      <div className="p-8 text-center text-slate-500">
        <FolderOpen className="w-10 h-10 mx-auto text-slate-600 mb-3" />
        <p className="text-body">{t("projsub.noOldData")}</p>
        <p className="text-title text-slate-500 mt-1">{t("projsub.noOldDataDesc")}</p>
      </div>
    );
  }

  const envVarEntries = Object.entries(data.backed_env_vars || {});

  return (
    <div className="space-y-5">
      {/* 标题说明 */}
      <div className="glass-panel rounded-panel p-4 border border-amber-500/10 bg-amber-500/3">
        <div className="flex items-start gap-3">
          <div className="w-9 h-9 rounded-card flex items-center justify-center bg-amber-500/10 flex-shrink-0 mt-0.5">
            <AlertTriangle className="w-4.5 h-4.5 text-amber-400" />
          </div>
          <div>
            <h4 className="text-body font-semibold text-amber-300">{t("projsub.legacyTitle")}</h4>
            <p className="text-title text-amber-400/60 mt-0.5">
              {t("projsub.legacyDesc")}
            </p>
          </div>
        </div>
      </div>

      {/* 旧版安装信息 */}
      {(data.install_source || data.install_root || data.version) && (
        <div className="glass-panel rounded-panel p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center gap-2">
            <Download className="w-4 h-4 text-slate-400" />
            <h4 className="text-body font-semibold text-white">{t("projsub.oldInstallInfo")}</h4>
          </div>
          <div className="grid grid-cols-1 md:grid-cols-3 gap-3 text-title">
            {data.version && (
              <div className="p-2.5 bg-black/20 rounded-card border border-white/5">
                <span className="text-slate-500 block mb-0.5">{t("projsub.thVersion")}</span>
                <span className="font-mono text-slate-200 font-semibold">{data.version}</span>
              </div>
            )}
            {data.install_source && (
              <div className="p-2.5 bg-black/20 rounded-card border border-white/5">
                <span className="text-slate-500 block mb-0.5">{t("projsub.thInstallMethod")}</span>
                <span className="font-mono text-slate-200">{data.install_source}</span>
              </div>
            )}
            {data.install_root && (
              <div className="p-2.5 bg-black/20 rounded-card border border-white/5">
                <span className="text-slate-500 block mb-0.5">{t("projsub.thInstallPath")}</span>
                <span className="font-mono text-slate-200 text-body break-all">{data.install_root}</span>
              </div>
            )}
          </div>
        </div>
      )}

      {/* 备份的环境变量 */}
      {envVarEntries.length > 0 && (
        <div className="glass-panel rounded-panel p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center gap-2">
            <Globe className="w-4 h-4 text-[var(--module-accent)]" />
            <h4 className="text-body font-semibold text-white">{t("projsub.backedEnv")}</h4>
            <span className="text-body text-slate-500">{t("projsub.envCount", { count: envVarEntries.length })}</span>
          </div>
          <div className="w-full">
            <table className="w-full text-left text-title">
              <thead><tr className="text-slate-500 border-b border-white/5"><th className="p-2 w-48">{t("projsub.thVarName")}</th><th className="p-2">{t("projsub.thOriginalVal")}</th></tr></thead>
              <tbody className="divide-y divide-white/5">
                {envVarEntries.map(([name, val]) => (
                  <tr key={name} className="hover:bg-white/2 text-slate-300">
                    <td className="p-2 font-mono font-semibold">{name}</td>
                    <td className="p-2 font-mono text-body break-all text-slate-400">{val || t("projsub.emptyVal")}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {/* 移除的 PATH 条目 */}
      {data.removed_path_entries.length > 0 && (
        <div className="glass-panel rounded-panel p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center gap-2">
            <Trash2 className="w-4 h-4 text-red-400" />
            <h4 className="text-body font-semibold text-white">{t("projsub.removedPathEntries")}</h4>
            <span className="text-body text-slate-500">{t("projsub.entryCount", { count: data.removed_path_entries.length })}</span>
          </div>
          <div className="w-full space-y-1">
            {data.removed_path_entries.map((entry, idx) => (
              <div key={idx} className="p-2 bg-black/20 rounded-ctl border border-white/5 text-body font-mono text-slate-400 break-all">
                {entry}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
