// any-version 统一数据配置备份面板（内嵌于「设置」模块）：
// 一键把全部模块配置/数据库（含 picky 数据与其云同步版本号）打包为单个压缩文件（gzip），
// 全量导出到本地 / 从备份文件全量导入恢复，无任何勾选环节。
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { DatabaseBackup, Loader2, FolderDown, FolderUp } from "lucide-react";
import { vexSay } from "../utils/vexSay";

interface ExportResult {
  path: string;
  fileCount: number;
  sizeBytes: number;
  compressedBytes: number;
  createdAt: string;
}

function fmtSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return (bytes / 1024 / 1024).toFixed(1) + " MB";
  if (bytes >= 1024) return (bytes / 1024).toFixed(1) + " KB";
  return bytes + " B";
}

function fmtTime(iso?: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  return d.toLocaleString("zh-CN");
}

export default function DataSyncPanel() {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  const [lastExport, setLastExport] = useState<ExportResult | null>(null);
  const [importPath, setImportPath] = useState("");

  const exportSnapshot = async () => {
    try {
      const filePath = await saveDialog({
        title: t("datasync.exportTitle"),
        defaultPath: `kira-state-${new Date().toISOString().slice(0, 10)}.json.gz`,
        filters: [{ name: t("datasync.exportFilter"), extensions: ["gz"] }],
      });
      if (!filePath || typeof filePath !== "string") return;
      setBusy(true);
      setMsg("");
      const res = await invoke<ExportResult>("state_sync_export", { targetPath: filePath });
      setLastExport(res);
      setMsg(
        t("datasync.exported", { count: res.fileCount, size: fmtSize(res.sizeBytes), comp: fmtSize(res.compressedBytes) }),
      );
      vexSay(t("datasync.vexExported"), "success");
    } catch (e) {
      setMsg(t("datasync.exportFail", { err: String(e) }));
      vexSay(t("datasync.vexExportErr"), "error");
    } finally {
      setBusy(false);
    }
  };

  // 全量导入：选择快照文件后直接整体恢复（不做勾选，所有数据都会被覆盖）
  const importAll = async () => {
    try {
      const selected = await openDialog({
        title: t("datasync.importTitle"),
        filters: [
          { name: t("datasync.importFilterAll"), extensions: ["gz", "json"] },
          { name: t("datasync.importFilterGz"), extensions: ["gz"] },
          { name: t("datasync.importFilterJson"), extensions: ["json"] },
        ],
      });
      if (!selected || typeof selected !== "string") return;
      setImportPath(selected);
      setBusy(true);
      setMsg("");
      const res = await invoke<string>("state_sync_import", { path: selected });
      setMsg(res);
      vexSay(t("datasync.vexRestored"), "success");
    } catch (e) {
      setMsg(t("datasync.importFail", { err: String(e) }));
      vexSay(t("datasync.vexRestoreErr"), "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-5">
      {/* 头部 */}
      <div className="flex items-center justify-between pb-3 border-b border-white/5">
        <div className="flex items-center gap-2">
          <DatabaseBackup className="w-4 h-4 text-[var(--module-accent)]" />
          <div>
            <h3 className="text-xs font-semibold text-white">{t("datasync.title")}</h3>
            <p className="text-[9px] text-slate-500 mt-0.5">
              {t("datasync.desc")}
            </p>
          </div>
        </div>
      </div>

      {/* 操作结果（完整展示，不截断） */}
      {msg && (
        <div className="flex items-start gap-2 rounded-xl border border-[var(--module-accent)]/30 bg-[var(--module-accent)]/10 px-3.5 py-2.5 text-[11px] text-slate-200 leading-relaxed">
          <span className="text-[var(--module-accent)] mt-0.5 shrink-0">✓</span>
          <span className="break-all">{msg}</span>
        </div>
      )}

      <div className="space-y-4">
        {/* 说明 */}
        <div className="bg-white/[0.03] border border-white/10 rounded-xl p-3.5 text-[11px] text-slate-400 leading-relaxed">
          {t("datasync.snapshotHint1")}<b className="text-slate-200">{t("datasync.snapshotHint2")}</b>{t("datasync.snapshotHint3")}{" "}
          <b className="text-slate-200">{t("datasync.snapshotHintPicky")}</b>{t("datasync.snapshotHintTail")}
        </div>

        {/* 导出 */}
        <div className="bg-white/[0.03] border border-white/10 rounded-xl p-4 space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-xs font-bold text-white flex items-center gap-1.5">
              <FolderDown className="w-3.5 h-3.5 text-[var(--module-accent)]" /> {t("datasync.exportAll")}
            </h3>
            <button
              onClick={exportSnapshot}
              disabled={busy}
              className="px-3 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] text-white font-semibold cursor-pointer hover:opacity-85 disabled:opacity-50 flex items-center gap-1"
            >
              {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <FolderDown className="w-3 h-3" />} {t("datasync.exportSnapshot")}
            </button>
          </div>
          <p className="text-[9px] text-slate-600">
            {t("datasync.exportHint")}
          </p>
          {lastExport && (
            <p className="text-[10px] text-slate-500 break-all">
              {t("datasync.exportedPath", { path: lastExport.path })}
              <br />
              {t("datasync.filesInfo", { count: lastExport.fileCount, size: fmtSize(lastExport.sizeBytes), comp: fmtSize(lastExport.compressedBytes), time: fmtTime(lastExport.createdAt) })}
            </p>
          )}
        </div>

        {/* 导入 */}
        <div className="bg-white/[0.03] border border-white/10 rounded-xl p-4 space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-xs font-bold text-white flex items-center gap-1.5">
              <FolderUp className="w-3.5 h-3.5 text-[var(--module-accent)]" /> {t("datasync.importAll")}
            </h3>
            <button
              onClick={importAll}
              disabled={busy}
              className="px-3 py-1.5 rounded-lg text-[11px] bg-white/5 hover:bg-white/10 border border-white/10 text-slate-300 cursor-pointer disabled:opacity-50 flex items-center gap-1"
            >
              {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <FolderUp className="w-3 h-3" />} {t("datasync.chooseAndRestore")}
            </button>
          </div>
          {importPath && <p className="text-[10px] text-slate-500 break-all">{t("datasync.selectedPath", { path: importPath })}</p>}
          <p className="text-[9px] text-amber-400/80">
            {t("datasync.importWarn")}
          </p>
        </div>
      </div>
    </div>
  );
}
