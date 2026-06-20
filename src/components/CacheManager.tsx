import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { 
  Trash2, 
  ArrowRight, 
  FolderSync, 
  RefreshCw, 
  Link,
  ShieldAlert,
  HardDrive
} from "lucide-react";

interface CacheInfo {
  name: string;
  installed: boolean;
  path: string;
  size: string;
  is_link: boolean;
  real_target: string;
  detect_source: string;
  detect_content: string;
}

export default function CacheManager() {
  const [caches, setCaches] = useState<CacheInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [migratingName, setMigratingName] = useState<string | null>(null);

  // Custom targets state
  const [customPaths, setCustomPaths] = useState<Record<string, string>>({});

  const fetchCaches = async () => {
    setLoading(true);
    try {
      const list = await invoke<CacheInfo[]>("get_caches_list");
      setCaches(list);
      
      // Suggest a default non-C drive path for non-redirected caches
      const paths: Record<string, string> = {};
      list.forEach(c => {
        if (!c.is_link) {
          // Default suggestion: D:\any-version-caches\<name>
          paths[c.name] = `D:\\any-version-caches\\${c.name}`;
        } else {
          paths[c.name] = c.real_target;
        }
      });
      setCustomPaths(paths);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchCaches();
  }, []);

  const handleMigrate = async (name: string) => {
    const target = customPaths[name];
    if (!target) return;
    
    if (target.toLowerCase().startsWith("c:")) {
      alert("错误: 目标重定向目录必须位于非 C 盘 (例如 D:\\...)，以腾出 C 盘空间。");
      return;
    }

    setMigratingName(name);
    try {
      await invoke("migrate_cache_path", { name, newPath: target });
      await fetchCaches();
    } catch (e: any) {
      alert(`重定向失败: ${e}`);
    } finally {
      setMigratingName(null);
    }
  };

  return (
    <div className="flex-1 p-8 overflow-y-auto space-y-6 h-screen select-none">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-semibold text-white tracking-wide">开发缓存清理</h2>
          <p className="text-xs text-slate-400 mt-1">迁移 C 盘下的包管理器全局缓存到其他磁盘（NTFS 目录联接映射），拯救 C 盘空间。每一项都会告诉你「依据哪个配置检测到的」以及「将要怎么迁移」。</p>
        </div>

        <button
          onClick={fetchCaches}
          disabled={loading}
          className="flex items-center gap-2 px-3.5 py-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-xl text-xs border border-white/5 cursor-pointer transition-all"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${loading ? "animate-spin" : ""}`} />
          刷新体积
        </button>
      </div>

      {/* Caches list */}
      <div className="space-y-4">
        {caches.map((cache) => {
          const isMigrating = migratingName === cache.name;
          const target = customPaths[cache.name] || "";
          return (
            <div
              key={cache.name}
              className="glass-panel rounded-2xl p-5 border border-white/5 space-y-4"
            >
              <div className="flex flex-col md:flex-row md:items-center justify-between gap-4">
                {/* Left side info */}
                <div className="space-y-1.5 flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <h3 className="text-xs font-semibold text-white uppercase">{cache.name} 缓存</h3>
                    {cache.is_link && (
                      <span className="px-2 py-0.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[9px] font-medium flex items-center gap-1 font-mono">
                        <Link className="w-3 h-3" />
                        已重定向
                      </span>
                    )}
                    {!cache.installed && (
                      <span className="px-2 py-0.5 rounded bg-slate-500/10 text-slate-500 border border-slate-500/20 text-[9px] font-medium">
                        未检测到客户端
                      </span>
                    )}
                  </div>

                  <div className="font-mono text-[10px] text-slate-400 space-y-1 truncate">
                    <p>原位置: {cache.path}</p>
                    {cache.is_link && (
                      <p className="text-blue-400 flex items-center gap-1.5 font-semibold">
                        <ArrowRight className="w-3.5 h-3.5" />
                        真实物理位置: {cache.real_target}
                      </p>
                    )}
                  </div>
                </div>

                {/* Middle: Size display */}
                <div className="flex items-center gap-4 text-right px-4">
                  <div className="flex items-center gap-1.5 bg-black/20 p-3.5 rounded-xl border border-white/5 min-w-[90px] justify-center">
                    <HardDrive className="w-4 h-4 text-slate-500" />
                    <span className="font-mono font-bold text-xs text-white">{cache.size}</span>
                  </div>
                </div>

                {/* Right: Action form */}
                <div className="flex items-center gap-2">
                  {!cache.is_link ? (
                    <>
                      <input
                        type="text"
                        value={customPaths[cache.name] || ""}
                        onChange={(e) => setCustomPaths({ ...customPaths, [cache.name]: e.target.value })}
                        className="glass-input px-3 py-2 text-xs w-48 font-mono"
                        placeholder="e.g. D:\caches"
                      />
                      <button
                        onClick={() => handleMigrate(cache.name)}
                        disabled={isMigrating || !customPaths[cache.name]}
                        className="px-4 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-lg text-xs font-semibold cursor-pointer transition-all flex items-center gap-1 shadow-lg shadow-blue-500/10"
                      >
                        <FolderSync className="w-3.5 h-3.5" />
                        {isMigrating ? "正在迁移..." : "迁移"}
                      </button>
                    </>
                  ) : (
                    <button
                      disabled
                      className="px-4 py-2 bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 rounded-lg text-xs font-semibold"
                    >
                      已完成重定向
                    </button>
                  )}
                </div>
              </div>

              {/* 检测依据：透明展示我们是怎么找到这个缓存目录的 */}
              <div className="grid grid-cols-1 md:grid-cols-2 gap-3 pt-3 border-t border-white/5">
                <div className="p-3 rounded-xl bg-amber-500/5 border border-amber-500/15 space-y-1.5">
                  <span className="text-[10px] font-semibold text-amber-400 uppercase tracking-wide">检测依据（如何找到）</span>
                  <p className="text-[10px] text-slate-300 leading-relaxed">{cache.detect_source}</p>
                  <p className="text-[10px] font-mono text-slate-400 break-all bg-black/20 rounded p-1.5 border border-white/5">{cache.detect_content}</p>
                </div>

                {/* 迁移方案：透明展示将要怎么做 */}
                {!cache.is_link ? (
                  <div className="p-3 rounded-xl bg-blue-500/5 border border-blue-500/15 space-y-1.5">
                    <span className="text-[10px] font-semibold text-blue-400 uppercase tracking-wide">迁移方案（将要怎么做）</span>
                    <p className="text-[10px] text-slate-300 leading-relaxed">
                      将缓存从 <span className="font-mono text-slate-200">{cache.path}</span> 整体移动到
                      <span className="font-mono text-emerald-300"> {target || "（请填写目标路径）"}</span>，
                      并在原位置创建 NTFS 目录联接（mklink /J）。工具仍按原路径访问，文件却存到非 C 盘，使用无感。
                    </p>
                  </div>
                ) : (
                  <div className="p-3 rounded-xl bg-emerald-500/5 border border-emerald-500/15 space-y-1.5">
                    <span className="text-[10px] font-semibold text-emerald-400 uppercase tracking-wide">当前状态</span>
                    <p className="text-[10px] text-slate-300 leading-relaxed">
                      已通过目录联接重定向到 <span className="font-mono text-emerald-300 break-all">{cache.real_target}</span>，不再占用 C 盘空间。
                    </p>
                  </div>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
