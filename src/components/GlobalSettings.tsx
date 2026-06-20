import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { 
  FolderKanban, 
  Save, 
  RefreshCw, 
  Info,
  CheckCircle2
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

  return (
    <div className="flex-1 p-8 overflow-y-auto space-y-6 h-screen select-none max-w-3xl">
      {/* Header */}
      <div>
        <h2 className="text-xl font-semibold text-white tracking-wide">全局路径设置</h2>
        <p className="text-xs text-slate-400 mt-1">配置多版本 SDK 下载、缓存和系统软链接映射目录</p>
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
              <input 
                type="text"
                value={versionsDir}
                onChange={(e) => setVersionsDir(e.target.value)}
                className="w-full glass-input px-3.5 py-2.5 text-xs font-mono"
                placeholder="e.g. C:\Users\Admin\.any-version\versions"
              />
              <p className="text-[9px] text-slate-500">此目录存储所有下载和手动安装的 SDK 和本地数据库包文件。</p>
            </div>

            <div className="space-y-1.5">
              <label className="text-[10px] text-slate-500 uppercase font-semibold">链接映射目录 (links_dir)</label>
              <input 
                type="text"
                value={linksDir}
                onChange={(e) => setLinksDir(e.target.value)}
                className="w-full glass-input px-3.5 py-2.5 text-xs font-mono"
                placeholder="e.g. C:\Users\Admin\.any-version\links"
              />
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
    </div>
  );
}
