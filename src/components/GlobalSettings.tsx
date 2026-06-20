import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { 
  FolderKanban, 
  Save, 
  RefreshCw, 
  Info,
  CheckCircle2,
  ExternalLink,
  FolderOpen
} from "lucide-react";

interface Config {
  versions_dir: string;
  links_dir: string;
}

export default function GlobalSettings() {
  const [versionsDir, setVersionsDir] = useState("");
  const [linksDir, setLinksDir] = useState("");
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [success, setSuccess] = useState(false);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [latestVersion, setLatestVersion] = useState<string | null>(null);
  const [updateBody, setUpdateBody] = useState<string | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);

  const fetchConfig = async () => {
    setLoading(true);
    setSuccess(false);
    try {
      const config = await invoke<Config>("get_config");
      setVersionsDir(config.versions_dir);
      setLinksDir(config.links_dir);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchConfig();
  }, []);

  const handleSave = async () => {
    if (!versionsDir || !linksDir) return;
    setSaving(true);
    setSuccess(false);
    try {
      await invoke("update_config", { versionsDir, linksDir });
      setSuccess(true);
      await fetchConfig();
    } catch (e: any) {
      alert(`保存配置失败: ${e}`);
    } finally {
      setSaving(false);
    }
  };

  const handleCheckUpdate = async () => {
    setCheckingUpdate(true);
    setUpdateError(null);
    setLatestVersion(null);
    try {
      const resp = await fetch("https://api.github.com/repos/anyversion/anyversion/releases/latest", {
        headers: { "Accept": "application/vnd.github.v3+json" }
      });
      if (!resp.ok) throw new Error("检查失败: " + resp.status);
      const data = await resp.json();
      const tag = data.tag_name?.replace(/^v/, "") ?? "";
      if (tag && tag !== "1.0.0") {
        setLatestVersion(tag);
        setUpdateBody(data.body ?? null);
      } else {
        setLatestVersion(null);
        setUpdateError(null);
        // Show "already latest" feedback
        alert("当前已是最新版本！");
      }
    } catch (e: any) {
      setUpdateError(e.message || "检查更新失败");
    } finally {
      setCheckingUpdate(false);
    }
  };

  const handleBrowseFolder = async (setter: (v: string) => void) => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, title: "选择文件夹" });
      if (selected) setter(selected as string);
    } catch {
      alert("文件夹选择器不可用，请手动输入路径。");
    }
  };

  const handleDownloadUpdate = () => {
    window.open("https://github.com/anyversion/anyversion/releases/latest", "_blank");
  };

  return (
    <div className="flex-1 p-8 overflow-y-auto space-y-6 h-full select-none max-w-3xl mx-auto">
      {/* Header */}
      <div>
        <h2 className="text-xl font-semibold text-white tracking-wide">设置</h2>
        <p className="text-xs text-slate-400 mt-1">配置工作目录、版本检查与应用升级</p>
      </div>

      <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-6">
        <div className="flex items-center gap-2 pb-3 border-b border-white/5">
          <FolderKanban className="w-4 h-4 text-blue-400" />
          <h3 className="text-xs font-semibold text-white">AnyVersion 工作目录说明</h3>
        </div>

        <div className="p-4 bg-indigo-500/5 border border-indigo-500/15 rounded-xl space-y-2 text-[10px] text-slate-300 leading-relaxed">
          <p className="font-semibold text-indigo-300 text-[11px]">这两个目录分别做什么？</p>
          <p>• <span className="font-mono text-slate-200">SDK 存储目录</span>：所有下载的 SDK（如 Node.js、Go、Python）会存放在这里，按「工具名/版本号」归类，例如 <span className="font-mono">versions/nodejs/20.11.1</span>。</p>
          <p>• <span className="font-mono text-slate-200">链接映射目录</span>：每种工具对应一个固定路径（如 <span className="font-mono">links/nodejs</span>），通过 NTFS 目录联接指向当前激活的版本。切换版本只需改变这个联接的指向，毫秒级完成，不需要改任何环境变量。</p>
        </div>

        {loading ? (
          <div className="text-xs text-slate-400 py-6 flex items-center gap-2">
            <RefreshCw className="w-4 h-4 animate-spin text-blue-400" />
            正在读取系统配置...
          </div>
        ) : (
          <div className="space-y-4">
            <div className="space-y-1.5">
              <label className="text-[10px] text-slate-500 uppercase font-semibold">SDK 存储目录 (versions_dir)</label>
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  value={versionsDir}
                  onChange={(e) => setVersionsDir(e.target.value)}
                  className="flex-1 glass-input px-3.5 py-2.5 text-xs font-mono"
                  placeholder="e.g. C:\Users\Admin\.any-version\versions"
                />
                <button onClick={() => handleBrowseFolder(setVersionsDir)} className="p-2.5 bg-white/5 hover:bg-white/10 text-slate-400 hover:text-slate-200 rounded-lg border border-white/5 cursor-pointer transition-all flex-shrink-0" title="选择文件夹">
                  <FolderOpen className="w-4 h-4" />
                </button>
              </div>
              <p className="text-[9px] text-slate-500">此目录存储所有下载和手动安装的 SDK 和本地数据库包文件。</p>
            </div>

            <div className="space-y-1.5">
              <label className="text-[10px] text-slate-500 uppercase font-semibold">链接映射目录 (links_dir)</label>
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  value={linksDir}
                  onChange={(e) => setLinksDir(e.target.value)}
                  className="flex-1 glass-input px-3.5 py-2.5 text-xs font-mono"
                  placeholder="e.g. C:\Users\Admin\.any-version\links"
                />
                <button onClick={() => handleBrowseFolder(setLinksDir)} className="p-2.5 bg-white/5 hover:bg-white/10 text-slate-400 hover:text-slate-200 rounded-lg border border-white/5 cursor-pointer transition-all flex-shrink-0" title="选择文件夹">
                  <FolderOpen className="w-4 h-4" />
                </button>
              </div>
              <p className="text-[9px] text-slate-500">此目录存放各个工具的固定快捷链接文件夹（会自动加入系统 PATH），切换版本即是秒级修改其底层指向。</p>
            </div>

            <div className="p-4 bg-blue-950/10 border border-blue-500/20 rounded-xl space-y-2">
              <h4 className="text-xs font-semibold text-blue-400 flex items-center gap-1.5">
                <Info className="w-4 h-4" />
                警告与提示
              </h4>
              <p className="text-[10px] text-slate-400 leading-relaxed">
                更新路径后，AnyVersion 将自动移除旧路径在 PATH 中的环境变量，并将新路径重新注册。已存在的 SDK 链接关系也将自动转移。
              </p>
            </div>

            <div className="flex items-center justify-between pt-4 border-t border-white/5">
              <div>
                {success && (
                  <span className="text-xs font-medium text-emerald-400 flex items-center gap-1.5">
                    <CheckCircle2 className="w-4 h-4" />
                    路径配置已保存，环境变量已成功同步！
                  </span>
                )}
              </div>

              <button
                onClick={handleSave}
                disabled={saving || !versionsDir || !linksDir}
                className="px-6 py-2.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold shadow-lg shadow-blue-500/10 cursor-pointer transition-all flex items-center gap-1.5"
              >
                <Save className="w-3.5 h-3.5" />
                {saving ? "正在保存..." : "保存配置"}
              </button>
            </div>
          </div>
        )}
      </div>

      {/* 版本检查与升级 */}
      <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-4">
        <div className="flex items-center justify-between pb-3 border-b border-white/5">
          <div className="flex items-center gap-2">
            <RefreshCw className="w-4 h-4 text-blue-400" />
            <h3 className="text-xs font-semibold text-white">版本检查与升级</h3>
          </div>
          <button
            onClick={handleCheckUpdate}
            disabled={checkingUpdate}
            className="flex items-center gap-1.5 px-3 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg text-[10px] border border-white/5 cursor-pointer"
          >
            <RefreshCw className={`w-3 h-3 ${checkingUpdate ? "animate-spin" : ""}`} />
            {checkingUpdate ? "检查中..." : "检查更新"}
          </button>
        </div>

        <div className="flex items-center gap-3 text-xs">
          <span className="text-slate-400">当前版本:</span>
          <span className="font-mono text-slate-200 bg-black/20 px-2 py-0.5 rounded">v1.0.0</span>
        </div>

        {updateError && (
          <div className="p-3 bg-red-500/10 border border-red-500/20 rounded-xl text-[10px] text-red-400">
            {updateError}
          </div>
        )}

        {latestVersion && (
          <div className="p-4 bg-emerald-500/5 border border-emerald-500/15 rounded-xl space-y-2">
            <div className="flex items-center gap-2">
              <span className="text-xs font-semibold text-emerald-300">发现新版本: v{latestVersion}</span>
              {updateBody && (
                <span className="text-[10px] text-slate-400">({updateBody.substring(0, 80)}...)</span>
              )}
            </div>
            <button
              onClick={handleDownloadUpdate}
              className="px-4 py-2 bg-emerald-600 hover:bg-emerald-500 text-white rounded-lg text-xs font-semibold cursor-pointer transition-all flex items-center gap-1.5"
            >
              <ExternalLink className="w-3 h-3" />
              前往下载页面
            </button>
          </div>
        )}

        {latestVersion === null && !checkingUpdate && !updateError && (
          <p className="text-[10px] text-slate-500">点击「检查更新」查看是否有新版本可用。</p>
        )}
      </div>

      {/* 路径迁移说明 */}
      <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-4">
        <div className="flex items-center gap-2 pb-3 border-b border-white/5">
          <FolderKanban className="w-4 h-4 text-blue-400" />
          <h3 className="text-xs font-semibold text-white">路径迁移</h3>
        </div>
        <div className="p-4 bg-amber-500/5 border border-amber-500/15 rounded-xl text-[10px] text-slate-300 leading-relaxed space-y-2">
          <p className="font-semibold text-amber-300">修改路径后会自动迁移</p>
          <p>当您修改上方的 SDK 存储目录或链接映射目录并保存后，AnyVersion 会自动：</p>
          <p>1. 将旧目录下的所有已安装版本文件移动到新目录</p>
          <p>2. 更新所有 junction 链接的指向</p>
          <p>3. 更新 PATH 环境变量中的旧路径为新路径</p>
          <p>整个过程无需手动操作，已安装的 SDK 不会丢失。</p>
        </div>
      </div>
    </div>
 