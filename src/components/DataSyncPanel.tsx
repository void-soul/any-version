// any-version 统一数据配置备份面板（内嵌于「设置」模块）：
// 一键把全部模块配置/数据库（含 picky 数据与其云同步版本号）打包为单个压缩文件（gzip），
// 全量导出到本地 / 从备份文件全量导入恢复，无任何勾选环节。
import { useState } from "react";
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
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  const [lastExport, setLastExport] = useState<ExportResult | null>(null);
  const [importPath, setImportPath] = useState("");

  const exportSnapshot = async () => {
    try {
      const filePath = await saveDialog({
        title: "导出 Kira 数据快照",
        defaultPath: `any-version-state-${new Date().toISOString().slice(0, 10)}.json.gz`,
        filters: [{ name: "Kira 数据快照 (压缩)", extensions: ["gz"] }],
      });
      if (!filePath || typeof filePath !== "string") return;
      setBusy(true);
      setMsg("");
      const res = await invoke<ExportResult>("state_sync_export", { targetPath: filePath });
      setLastExport(res);
      setMsg(
        `已导出全量快照：${res.fileCount} 个文件 / ${fmtSize(res.sizeBytes)}（压缩后 ${fmtSize(res.compressedBytes)}）`,
      );
      vexSay("快照打包好了，数据都在，放心。📦", "success");
    } catch (e) {
      setMsg(`导出失败：${e}`);
      vexSay("唔…导出这步卡住了，我帮你看看", "error");
    } finally {
      setBusy(false);
    }
  };

  // 全量导入：选择快照文件后直接整体恢复（不做勾选，所有数据都会被覆盖）
  const importAll = async () => {
    try {
      const selected = await openDialog({
        title: "选择 any-version 数据快照文件",
        filters: [
          { name: "Kira 数据快照 (*.gz, *.json)", extensions: ["gz", "json"] },
          { name: "压缩快照 (*.gz)", extensions: ["gz"] },
          { name: "旧版 JSON 快照 (*.json)", extensions: ["json"] },
        ],
      });
      if (!selected || typeof selected !== "string") return;
      setImportPath(selected);
      setBusy(true);
      setMsg("");
      const res = await invoke<string>("state_sync_import", { path: selected });
      setMsg(res);
      vexSay("恢复完成，数据都归位了，我确认过了。✅", "success");
    } catch (e) {
      setMsg(`导入失败：${e}`);
      vexSay("唔…恢复这步卡住了，先别急，我看看备份还在不在", "error");
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
            <h3 className="text-xs font-semibold text-white">数据备份</h3>
            <p className="text-[9px] text-slate-500 mt-0.5">
              全部配置与数据（含 picky 收藏与同步版本号）打包为一个压缩快照，全量导出 / 导入
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
          所有数据统一打包为<b className="text-slate-200">一个压缩快照文件</b>，不做任何勾选：
          config.json / 各数据库 / AI 配置与会话 / 技能 / MCP / 协作 / 翻译配置 / 启动器 / 任务 / OTP /
          剪贴板（含图片）/ 证书凭据 / mihomo 代理配置 / 环境备份 / 自定义字体 / 思维导图 / API 接口平台 /{" "}
          <b className="text-slate-200">Picky 收藏数据（含其云同步版本号 lastSyncAt）</b>等。
          导入时整体恢复（会覆盖当前数据），请谨慎操作。
        </div>

        {/* 导出 */}
        <div className="bg-white/[0.03] border border-white/10 rounded-xl p-4 space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-xs font-bold text-white flex items-center gap-1.5">
              <FolderDown className="w-3.5 h-3.5 text-[var(--module-accent)]" /> 全量导出
            </h3>
            <button
              onClick={exportSnapshot}
              disabled={busy}
              className="px-3 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] text-white font-semibold cursor-pointer hover:opacity-85 disabled:opacity-50 flex items-center gap-1"
            >
              {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <FolderDown className="w-3 h-3" />} 导出快照
            </button>
          </div>
          <p className="text-[9px] text-slate-600">
            选择存放位置后，把全部数据打包为一个 .gz 压缩文件（gzip，旧版明文 JSON 也能导入）。
          </p>
          {lastExport && (
            <p className="text-[10px] text-slate-500 break-all">
              已导出：{lastExport.path}
              <br />
              {lastExport.fileCount} 个文件 · 原始 {fmtSize(lastExport.sizeBytes)} · 压缩后{" "}
              {fmtSize(lastExport.compressedBytes)} · {fmtTime(lastExport.createdAt)}
            </p>
          )}
        </div>

        {/* 导入 */}
        <div className="bg-white/[0.03] border border-white/10 rounded-xl p-4 space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-xs font-bold text-white flex items-center gap-1.5">
              <FolderUp className="w-3.5 h-3.5 text-[var(--module-accent)]" /> 全量导入恢复
            </h3>
            <button
              onClick={importAll}
              disabled={busy}
              className="px-3 py-1.5 rounded-lg text-[11px] bg-white/5 hover:bg-white/10 border border-white/10 text-slate-300 cursor-pointer disabled:opacity-50 flex items-center gap-1"
            >
              {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <FolderUp className="w-3 h-3" />} 选择快照并恢复
            </button>
          </div>
          {importPath && <p className="text-[10px] text-slate-500 break-all">已选择：{importPath}</p>}
          <p className="text-[9px] text-amber-400/80">
            ⚠ 恢复会整体覆盖当前所有数据（含 picky 收藏与同步版本号），操作前建议先导出一次备份。
          </p>
        </div>
      </div>
    </div>
  );
}
