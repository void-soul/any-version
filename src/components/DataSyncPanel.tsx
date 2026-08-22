// any-version 统一数据配置备份与同步面板（内嵌于「设置」模块）：
// 1) 本地快照：一键把全部模块配置/数据库打包为单个压缩文件（gzip），可导出/导入。
// 2) S3 同步：把该快照备份到 S3 / 从 S3 恢复（独立于 Picky 的同步，配置与对象 key 均独立）。
import React, { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  DatabaseBackup,
  Cloud,
  Upload,
  Download,
  Save,
  Loader2,
  FolderDown,
  FolderUp,
  Sliders,
  X,
} from "lucide-react";

interface StateSyncConfig {
  endpoint?: string | null;
  region: string;
  accessKeyId: string;
  secretAccessKey: string;
  bucketName: string;
  prefix?: string | null;
  addressingStyle: string;
  tlsVerify: boolean;
  timeoutSeconds: number;
  enabled: boolean;
  lastSyncAt?: string | null;
  // 打包范围（可选目录开关）
  includeClipboardImages: boolean;
  includeMihomo: boolean;
  includeFonts: boolean;
  includeBackup: boolean;
}

/** 打包范围选项：是否包含某个可选目录 */
const SCOPE_OPTIONS: {
  key: "includeClipboardImages" | "includeMihomo" | "includeFonts" | "includeBackup";
  label: string;
  desc: string;
}[] = [
  { key: "includeClipboardImages", label: "剪贴板图片", desc: "clipboard/images（可能较大）" },
  { key: "includeMihomo", label: "mihomo 代理配置", desc: "订阅、覆写与配置文件" },
  { key: "includeFonts", label: "自定义字体", desc: "fonts 目录" },
  { key: "includeBackup", label: "环境备份目录", desc: "backup（环境变量/托管备份）" },
];

interface ExportResult {
  path: string;
  fileCount: number;
  sizeBytes: number;
  compressedBytes: number;
  createdAt: string;
}

interface SnapshotFileInfo {
  relPath: string;
  sizeBytes: number;
}

interface PickState {
  kind: "local" | "s3";
  path?: string;
  files: SnapshotFileInfo[];
  checked: Set<string>;
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
  const [cfg, setCfg] = useState<StateSyncConfig | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  const [lastExport, setLastExport] = useState<ExportResult | null>(null);
  const [importPath, setImportPath] = useState("");
  const [pick, setPick] = useState<PickState | null>(null);

  const loadCfg = async () => {
    try {
      const c = await invoke<StateSyncConfig>("state_sync_get_config");
      setCfg(c);
    } catch (e) {
      console.error("读取同步配置失败", e);
    }
  };

  useEffect(() => {
    loadCfg();
  }, []);

  const set = (k: keyof StateSyncConfig, v: unknown) => setCfg((c) => (c ? { ...c, [k]: v } : c));

  // 切换打包范围开关（自动保存，导出/云端备份立即生效）
  const toggleScope = async (key: (typeof SCOPE_OPTIONS)[number]["key"], v: boolean) => {
    if (!cfg) return;
    const next = { ...cfg, [key]: v };
    setCfg(next);
    try {
      await invoke("state_sync_save_config", { config: next });
      setMsg("打包范围已保存，下次导出/云端备份生效");
    } catch (e) {
      setMsg(`保存打包范围失败：${e}`);
    }
  };

  const exportSnapshot = async () => {
    try {
      const filePath = await saveDialog({
        title: "导出 AnyVersion 数据快照",
        defaultPath: `any-version-state-${new Date().toISOString().slice(0, 10)}.json.gz`,
        filters: [{ name: "AnyVersion 数据快照 (压缩)", extensions: ["gz"] }],
      });
      if (!filePath || typeof filePath !== "string") return;
      setBusy(true);
      setMsg("");
      const res = await invoke<ExportResult>("state_sync_export", { targetPath: filePath });
      setLastExport(res);
      setMsg(
        `已导出快照：${res.fileCount} 个文件 / ${fmtSize(res.sizeBytes)}（压缩后 ${fmtSize(res.compressedBytes)}）`,
      );
    } catch (e) {
      setMsg(`导出失败：${e}`);
    } finally {
      setBusy(false);
    }
  };

  const pickImportFile = async () => {
    const selected = await openDialog({
      title: "选择 any-version 数据快照文件",
      filters: [
        { name: "AnyVersion 数据快照 (*.gz, *.json)", extensions: ["gz", "json"] },
        { name: "压缩快照 (*.gz)", extensions: ["gz"] },
        { name: "旧版 JSON 快照 (*.json)", extensions: ["json"] },
      ],
    });
    if (selected && typeof selected === "string") {
      setImportPath(selected);
      setBusy(true);
      setMsg("");
      try {
        const files = await invoke<SnapshotFileInfo[]>("state_sync_peek", { path: selected });
        setPick({ kind: "local", path: selected, files, checked: new Set(files.map((f) => f.relPath)) });
      } catch (e) {
        setMsg(`读取快照失败：${e}`);
      } finally {
        setBusy(false);
      }
    }
  };

  const peekCloud = async () => {
    setBusy(true);
    setMsg("");
    try {
      const files = await invoke<SnapshotFileInfo[]>("state_sync_s3_peek");
      setPick({ kind: "s3", files, checked: new Set(files.map((f) => f.relPath)) });
    } catch (e) {
      setMsg(`读取云端快照失败：${e}`);
    } finally {
      setBusy(false);
    }
  };

  const togglePick = (relPath: string) =>
    setPick((p) => {
      if (!p) return p;
      const next = new Set(p.checked);
      if (next.has(relPath)) next.delete(relPath);
      else next.add(relPath);
      return { ...p, checked: next };
    });

  const togglePickAll = (on: boolean) =>
    setPick((p) => {
      if (!p) return p;
      return { ...p, checked: on ? new Set(p.files.map((f) => f.relPath)) : new Set() };
    });

  const confirmRestore = async () => {
    if (!pick || pick.checked.size === 0) return;
    setBusy(true);
    setMsg("");
    const files = [...pick.checked];
    try {
      const res =
        pick.kind === "local"
          ? await invoke<string>("state_sync_import", { path: pick.path, files })
          : await invoke<string>("state_sync_s3_pull", { files });
      setMsg(res);
      setPick(null);
      loadCfg();
    } catch (e) {
      setMsg(`${pick.kind === "local" ? "导入" : "恢复"}失败：${e}`);
    } finally {
      setBusy(false);
    }
  };

  const saveCfg = async () => {
    if (!cfg) return;
    setBusy(true);
    setMsg("");
    try {
      await invoke("state_sync_save_config", { config: cfg });
      setMsg("同步配置已保存");
    } catch (e) {
      setMsg(`保存失败：${e}`);
    } finally {
      setBusy(false);
    }
  };

  const push = async () => {
    if (!cfg) return;
    setBusy(true);
    setMsg("");
    try {
      await invoke("state_sync_save_config", { config: cfg });
      const res = await invoke<string>("state_sync_s3_push");
      setMsg(res);
    } catch (e) {
      setMsg(`备份失败：${e}`);
    } finally {
      setBusy(false);
    }
  };

  const pull = async () => {
    if (!cfg) return;
    try {
      await invoke("state_sync_save_config", { config: cfg });
      await peekCloud();
    } catch (e) {
      setMsg(`保存配置失败：${e}`);
    }
  };

  const field =
    "w-full glass-input px-3 py-2 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none focus:border-[var(--module-accent)]/50";

  return (
    <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-5">
      {/* 头部 */}
      <div className="flex items-center justify-between pb-3 border-b border-white/5">
        <div className="flex items-center gap-2">
          <DatabaseBackup className="w-4 h-4 text-[var(--module-accent)]" />
          <div>
            <h3 className="text-xs font-semibold text-white">数据备份与同步</h3>
            <p className="text-[9px] text-slate-500 mt-0.5">
              把全部模块的配置与数据打包为一个压缩快照，可导出到本地或同步到 S3
            </p>
          </div>
        </div>
        {msg && <span className="text-[11px] text-[var(--module-accent)] max-w-[50%] truncate text-right">{msg}</span>}
      </div>

      <div className="space-y-4">
        {/* 说明 */}
        <div className="bg-white/[0.03] border border-white/10 rounded-xl p-3.5 text-[11px] text-slate-400 leading-relaxed">
          所有模块的配置与数据（config.json / ai_config.json / AI 会话与技能 / 启动器 / 任务 / OTP /
          剪贴板含图片 / 证书凭据 / 代理配置 / 环境备份 / 自定义字体等）统一打包为
          <b className="text-slate-200">一个压缩快照文件</b>，方便整体备份、迁移与云端同步。
          其中 Picky 收藏有独立的同步机制，不包含在此快照内（互不影响）。
        </div>

        {/* 本地快照 */}
        <div className="bg-white/[0.03] border border-white/10 rounded-xl p-4 space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-xs font-bold text-white flex items-center gap-1.5">
              <FolderDown className="w-3.5 h-3.5 text-[var(--module-accent)]" /> 本地快照
            </h3>
            <button
              onClick={exportSnapshot}
              disabled={busy}
              className="px-3 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] text-white font-semibold cursor-pointer hover:opacity-85 disabled:opacity-50 flex items-center gap-1"
            >
              {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <FolderDown className="w-3 h-3" />} 导出快照
            </button>
          </div>
          {lastExport && (
            <p className="text-[10px] text-slate-500 break-all">
              已导出：{lastExport.path}
              <br />
              {lastExport.fileCount} 个文件 · 原始 {fmtSize(lastExport.sizeBytes)} · 压缩后{" "}
              {fmtSize(lastExport.compressedBytes)} · {fmtTime(lastExport.createdAt)}
            </p>
          )}
          <div className="flex gap-2">
            <input
              value={importPath}
              onChange={(e) => setImportPath(e.target.value)}
              readOnly
              placeholder="选择要恢复的快照文件…"
              className={field + " flex-1 cursor-pointer"}
              onClick={pickImportFile}
            />
            <button
              onClick={pickImportFile}
              disabled={busy}
              className="px-3 py-1.5 rounded-lg text-[11px] bg-white/5 hover:bg-white/10 border border-white/10 text-slate-300 cursor-pointer disabled:opacity-50 flex items-center gap-1 flex-shrink-0"
            >
              {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <FolderUp className="w-3 h-3" />} 选择快照
            </button>
          </div>
          <p className="text-[9px] text-slate-600">
            选择快照后可在弹窗中勾选要恢复的文件（默认全选），只恢复选中的项，不会整体覆盖。
            支持本工具导出的压缩快照（.gz）与旧版 JSON 快照。
          </p>
        </div>

        {/* 打包范围 */}
        <div className="bg-white/[0.03] border border-white/10 rounded-xl p-4 space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-xs font-bold text-white flex items-center gap-1.5">
              <Sliders className="w-3.5 h-3.5 text-[var(--module-accent)]" /> 打包范围
            </h3>
            <span className="text-[9px] text-slate-500">勾选项决定导出 / 云端备份包含哪些可选目录</span>
          </div>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
            {SCOPE_OPTIONS.map(({ key, label, desc }) => (
              <label
                key={key}
                className="flex items-start gap-2 px-3 py-2 rounded-lg border border-white/10 bg-black/20 cursor-pointer hover:bg-white/5 text-[11px]"
              >
                <input
                  type="checkbox"
                  checked={!!cfg?.[key]}
                  onChange={(e) => toggleScope(key, e.target.checked)}
                  className="accent-[var(--module-accent)] mt-0.5"
                />
                <span className="min-w-0">
                  <span className="block text-slate-200 font-medium">{label}</span>
                  <span className="block text-[9px] text-slate-500 truncate">{desc}</span>
                </span>
              </label>
            ))}
          </div>
          <p className="text-[9px] text-slate-600">
            核心数据（config.json、各数据库、AI 配置/会话、证书目录等）始终包含，不可关闭。
          </p>
        </div>

        {/* S3 同步 */}
        <div className="bg-white/[0.03] border border-white/10 rounded-xl p-4 space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-xs font-bold text-white flex items-center gap-1.5">
              <Cloud className="w-3.5 h-3.5 text-[var(--module-accent)]" /> S3 云备份（独立于 Picky）
            </h3>
            <label className="flex items-center gap-1.5 text-[11px] text-slate-300 cursor-pointer">
              <input
                type="checkbox"
                checked={!!cfg?.enabled}
                onChange={(e) => set("enabled", e.target.checked)}
                className="accent-[var(--module-accent)]"
              />
              启用
            </label>
          </div>
          {cfg?.lastSyncAt && <p className="text-[10px] text-slate-500">上次同步：{fmtTime(cfg.lastSyncAt)}</p>}

          {cfg && (
            <>
              <div>
                <label className="text-[10px] text-slate-400 mb-1 block">
                  Endpoint（如 https://s3.amazonaws.com 或 MinIO 地址）
                </label>
                <input
                  value={cfg.endpoint || ""}
                  onChange={(e) => set("endpoint", e.target.value)}
                  placeholder="https://s3.example.com"
                  className={field}
                />
              </div>
              <div className="grid grid-cols-2 gap-2">
                <div>
                  <label className="text-[10px] text-slate-400 mb-1 block">Region</label>
                  <input value={cfg.region} onChange={(e) => set("region", e.target.value)} className={field} />
                </div>
                <div>
                  <label className="text-[10px] text-slate-400 mb-1 block">Bucket</label>
                  <input value={cfg.bucketName} onChange={(e) => set("bucketName", e.target.value)} className={field} />
                </div>
              </div>
              <div>
                <label className="text-[10px] text-slate-400 mb-1 block">AccessKey ID</label>
                <input value={cfg.accessKeyId} onChange={(e) => set("accessKeyId", e.target.value)} className={field} />
              </div>
              <div>
                <label className="text-[10px] text-slate-400 mb-1 block">SecretKey（加密存储）</label>
                <input
                  type="password"
                  value={cfg.secretAccessKey}
                  onChange={(e) => set("secretAccessKey", e.target.value)}
                  className={field}
                />
              </div>
              <div className="grid grid-cols-2 gap-2">
                <div>
                  <label className="text-[10px] text-slate-400 mb-1 block">前缀（可选，默认 any-version/）</label>
                  <input
                    value={cfg.prefix || ""}
                    onChange={(e) => set("prefix", e.target.value)}
                    placeholder="any-version/"
                    className={field}
                  />
                </div>
                <div>
                  <label className="text-[10px] text-slate-400 mb-1 block">寻址风格</label>
                  <select
                    value={cfg.addressingStyle}
                    onChange={(e) => set("addressingStyle", e.target.value)}
                    className={field}
                  >
                    <option value="auto">auto（自动）</option>
                    <option value="path">path（路径式）</option>
                    <option value="virtual-host">virtual-host（虚拟主机式）</option>
                  </select>
                </div>
              </div>
              <label className="flex items-center gap-2 text-[11px] text-slate-300 cursor-pointer">
                <input
                  type="checkbox"
                  checked={cfg.tlsVerify}
                  onChange={(e) => set("tlsVerify", e.target.checked)}
                  className="accent-[var(--module-accent)]"
                />
                校验 TLS 证书（自签名 / 内网 http 可关闭）
              </label>

              <div className="flex justify-end gap-2 pt-1">
                <button
                  onClick={saveCfg}
                  disabled={busy}
                  className="px-3 py-1.5 rounded-lg text-[11px] bg-white/5 hover:bg-white/10 border border-white/10 text-slate-300 cursor-pointer disabled:opacity-50 flex items-center gap-1"
                >
                  {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Save className="w-3 h-3" />} 保存配置
                </button>
                <button
                  onClick={pull}
                  disabled={busy}
                  className="px-3 py-1.5 rounded-lg text-[11px] bg-white/5 hover:bg-white/10 border border-white/10 text-slate-300 cursor-pointer disabled:opacity-50 flex items-center gap-1"
                >
                  {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Download className="w-3 h-3" />} 从云端恢复
                </button>
                <button
                  onClick={push}
                  disabled={busy}
                  className="px-4 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] text-white font-semibold cursor-pointer hover:opacity-85 disabled:opacity-50 flex items-center gap-1"
                >
                  {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Upload className="w-3 h-3" />} 备份到云端
                </button>
              </div>
              <p className="text-[9px] text-slate-600 leading-relaxed">
                快照对象为 <code className="text-slate-400">{cfg.prefix || "any-version/"}any-version-state.json</code>
                ，与 Picky 的 state.json 各自独立。云端对象包含证书主密钥，请确保该空间仅自己可访问。
              </p>
            </>
          )}
        </div>

        <div className="text-[9px] text-slate-600">
          始终包含：config.json、backups.json、AI 配置/会话/技能/MCP/协作、翻译与同步配置、任务/启动器/AI 用量/OTP/
          剪贴板数据库、证书目录。可选包含（按上方「打包范围」勾选）：剪贴板图片、mihomo 代理配置、环境备份、
          自定义字体。不包含：Picky 收藏（独立同步）、sdk 目录与版本缓存（体积大/可重建）、mihomo geo 与日志（可重新下载）。
        </div>
      </div>

      {/* 选择要恢复的文件弹窗 */}
      {pick && (
        <div
          className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-6"
          onClick={() => !busy && setPick(null)}
        >
          <div
            className="w-full max-w-lg bg-[#1a1d24] border border-white/10 rounded-xl shadow-2xl flex flex-col max-h-[80vh]"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center gap-2 px-4 py-3 border-b border-white/5">
              <span className="text-xs font-bold text-white">选择要恢复的文件</span>
              <span className="text-[10px] text-slate-500">{pick.kind === "local" ? "本地快照" : "云端快照"}</span>
              <div className="flex-1" />
              <button
                onClick={() => !busy && setPick(null)}
                disabled={busy}
                className="text-slate-400 hover:text-white disabled:opacity-40 cursor-pointer"
              >
                <X className="w-4 h-4" />
              </button>
            </div>
            <div className="flex items-center gap-3 px-4 py-2 border-b border-white/5 text-[11px]">
              <button
                onClick={() => togglePickAll(true)}
                disabled={busy}
                className="text-slate-300 hover:text-white disabled:opacity-40 cursor-pointer"
              >
                全选
              </button>
              <button
                onClick={() => togglePickAll(false)}
                disabled={busy}
                className="text-slate-300 hover:text-white disabled:opacity-40 cursor-pointer"
              >
                全不选
              </button>
              <div className="flex-1" />
              <span className="text-slate-400">已选 {pick.checked.size}/{pick.files.length}</span>
            </div>
            <div className="flex-1 overflow-y-auto p-2">
              {pick.files.map((f) => (
                <label
                  key={f.relPath}
                  className="flex items-center gap-2 px-2 py-1.5 rounded-lg hover:bg-white/5 cursor-pointer text-[11px]"
                >
                  <input
                    type="checkbox"
                    checked={pick.checked.has(f.relPath)}
                    onChange={() => togglePick(f.relPath)}
                    className="accent-[var(--module-accent)]"
                  />
                  <span className="flex-1 text-slate-200 font-mono truncate" title={f.relPath}>
                    {f.relPath}
                  </span>
                  <span className="text-[10px] text-slate-500 flex-shrink-0">{fmtSize(f.sizeBytes)}</span>
                </label>
              ))}
            </div>
            <div className="px-4 py-3 border-t border-white/5 flex items-center justify-end gap-2">
              <button
                onClick={() => !busy && setPick(null)}
                disabled={busy}
                className="px-3 py-1.5 rounded-lg text-[11px] bg-white/5 hover:bg-white/10 border border-white/10 text-slate-300 cursor-pointer disabled:opacity-50"
              >
                取消
              </button>
              <button
                onClick={confirmRestore}
                disabled={busy || pick.checked.size === 0}
                className="px-4 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] text-white font-semibold cursor-pointer hover:opacity-85 disabled:opacity-50 flex items-center gap-1"
              >
                {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <FolderUp className="w-3 h-3" />} 恢复选中（
                {pick.checked.size}）
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
