import React, { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  HardDrive,
  FolderOpen,
  FolderSync,
  Link2,
  RefreshCw,
  CheckCircle,
} from "lucide-react";
import type { DetectedAiTool, AiToolCacheInfo } from "./types";

export default function AiCacheManager() {
  const [cacheInfos, setCacheInfos] = useState<AiToolCacheInfo[]>([]);
  const [tools, setTools] = useState<DetectedAiTool[]>([]);
  const [loading, setLoading] = useState(true);
  const [migrating, setMigrating] = useState<string | null>(null);
  const [migrateProgress, setMigrateProgress] = useState<{
    stage: string;
    current: number;
    total: number;
    file_name: string;
  } | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [ci, t] = await Promise.all([
        invoke<AiToolCacheInfo[]>("get_ai_tool_cache_info").catch(() => []),
        invoke<DetectedAiTool[]>("detect_ai_tools").catch(() => []),
      ]);
      setCacheInfos(ci);
      setTools(t);
    } catch (e) { console.error(e); }
    finally { setLoading(false); }
  }, []);

  const normalizePath = (p: string) =>
    p.replace(/\\/g, "/").replace(/\/$/, "").toLowerCase();

  useEffect(() => { load(); }, [load]);

  const getDisplayName = (toolId: string) =>
    tools.find(t => t.id === toolId)?.display_name || toolId;

  const handleMigrate = async (toolId: string, dirName: string, fullPath: string) => {
    try {
      const selected = await open({ directory: true, title: "选择新的缓存目录（仅支持 Junction）" });
      if (!selected) return;
      const targetPath = selected as string;
      if (targetPath.toLowerCase().startsWith("c:")) {
        alert("错误：AI 工具缓存只能迁移到非 C 盘，禁止直接指向 C 盘目录。");
        return;
      }
      if (normalizePath(targetPath) === normalizePath(fullPath)) {
        alert("错误：目标路径与原路径相同。");
        return;
      }
      const key = `${toolId}:${dirName}`;
      setMigrating(key);
      setMigrateProgress(null);

      // 监听 SDK 统一缓存迁移进度事件
      const unlisten = await listen<{ stage: string; current: number; total: number; file_name: string }>(
        "migrate-storage-progress",
        (event) => setMigrateProgress(event.payload)
      );

      try {
        await invoke("migrate_ai_tool_cache", { toolId, dirName, newPath: targetPath });
        await load();
      } finally {
        unlisten();
        setMigrating(null);
        setMigrateProgress(null);
      }
    } catch (e: any) {
      setMigrating(null);
      setMigrateProgress(null);
      alert(`迁移失败: ${e}`);
    }
  };

  const handleOpen = async (dirName: string) => {
    try { await invoke("open_ai_tool_cache_dir", { dirName }); }
    catch (e) { console.error(e); }
  };

  // 按工具分组
  const groupedByTool = React.useMemo(() => {
    const map = new Map<string, { displayName: string; caches: AiToolCacheInfo[] }>();
    for (const ci of cacheInfos) {
      if (!map.has(ci.tool_id)) {
        map.set(ci.tool_id, { displayName: getDisplayName(ci.tool_id), caches: [] });
      }
      map.get(ci.tool_id)!.caches.push(ci);
    }
    // 按总大小排序
    const entries = Array.from(map.entries());
    entries.sort((a, b) => {
      const sizeA = a[1].caches.reduce((s, c) => s + c.size_bytes, 0);
      const sizeB = b[1].caches.reduce((s, c) => s + c.size_bytes, 0);
      return sizeB - sizeA;
    });
    return entries;
  }, [cacheInfos, tools]);

  // 总计
  const totalSize = React.useMemo(() => {
    const sum = cacheInfos.reduce((s, c) => s + c.size_bytes, 0);
    if (sum >= 1024 * 1024 * 1024) return `${(sum / 1024 / 1024 / 1024).toFixed(1)} GB`;
    if (sum >= 1024 * 1024) return `${(sum / 1024 / 1024).toFixed(1)} MB`;
    if (sum >= 1024) return `${(sum / 1024).toFixed(1)} KB`;
    return `${sum} B`;
  }, [cacheInfos]);

  const junctionCount = cacheInfos.filter(c => c.is_junction).length;

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center text-slate-500">
        <RefreshCw className="w-5 h-5 animate-spin mr-2" />
        <span className="text-xs">加载中...</span>
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto p-6 space-y-5">
      {migrating && (
        <div className="p-4 rounded-xl border border-emerald-500/20 bg-emerald-600/5 space-y-3 animate-fadeIn">
          <div className="flex items-center justify-between">
            <span className="text-[11px] font-semibold text-emerald-300 flex items-center gap-1.5">
              <FolderSync className="w-3.5 h-3.5 animate-spin" />
              {migrateProgress?.stage || "正在迁移..."}
            </span>
            {migrateProgress && migrateProgress.total > 0 && (
              <span className="text-[11px] font-mono text-emerald-400">
                {Math.round((migrateProgress.current / migrateProgress.total) * 100)}%
              </span>
            )}
            {migrateProgress?.stage === "已完成" && (
              <CheckCircle className="w-4 h-4 text-emerald-400" />
            )}
          </div>

          {migrateProgress && migrateProgress.total > 0 && (
            <>
              <div className="w-full bg-white/5 rounded-full h-1.5 overflow-hidden border border-white/5">
                <div
                  className="bg-emerald-500 h-1.5 rounded-full transition-all duration-200"
                  style={{ width: `${Math.min(Math.round((migrateProgress.current / migrateProgress.total) * 100), 100)}%` }}
                />
              </div>
              <div className="text-[10px] text-slate-500 font-mono flex justify-between">
                <span className="truncate max-w-[60%]">{migrateProgress.file_name || "-"}</span>
                <span>{migrateProgress.current} / {migrateProgress.total}</span>
              </div>
            </>
          )}
          {!migrateProgress && (
            <div className="flex items-center gap-2 text-[10px] text-slate-500">
              <RefreshCw className="w-3 h-3 animate-spin" />
              准备中...
            </div>
          )}
        </div>
      )}

      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-bold text-white">缓存路径</h3>
          <p className="text-[10px] text-slate-500 mt-0.5">
            {cacheInfos.length} 个缓存目录 · 总计 {totalSize}
            {junctionCount > 0 && <span className="text-blue-400"> · {junctionCount} 个 JUNCTION</span>}
          </p>
        </div>
        <button onClick={load} className="px-2.5 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-400 hover:text-white cursor-pointer transition-all flex items-center gap-1">
          <RefreshCw className="w-3 h-3" /> 刷新
        </button>
      </div>

      {cacheInfos.length === 0 ? (
        <div className="h-48 border border-dashed border-white/5 rounded-2xl flex flex-col items-center justify-center text-slate-500">
          <HardDrive className="w-8 h-8 text-slate-700 mb-2" />
          <span className="text-xs font-bold text-slate-400">暂无缓存数据</span>
          <span className="text-[10px] text-slate-600 mt-1">AI 工具启动后会自动生成缓存目录</span>
        </div>
      ) : (
        <div className="space-y-4">
          {groupedByTool.map(([toolId, group]) => {
            const toolTotal = group.caches.reduce((s, c) => s + c.size_bytes, 0);
            const toolTotalStr =
              toolTotal >= 1024 * 1024 * 1024 ? `${(toolTotal / 1024 / 1024 / 1024).toFixed(1)} GB` :
              toolTotal >= 1024 * 1024 ? `${(toolTotal / 1024 / 1024).toFixed(0)} MB` :
              toolTotal >= 1024 ? `${(toolTotal / 1024).toFixed(0)} KB` : `${toolTotal} B`;

            return (
              <div key={toolId} className="rounded-xl border border-white/5 bg-slate-900/30 overflow-hidden">
                {/* 工具名头 */}
                <div className="px-4 py-2.5 bg-white/[0.02] border-b border-white/5 flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <HardDrive className="w-3.5 h-3.5 text-violet-400" />
                    <span className="text-xs font-bold text-slate-200">{group.displayName}</span>
                  </div>
                  <span className="text-[9px] text-slate-500">{toolTotalStr}</span>
                </div>

                {/* 缓存条目 */}
                <div className="divide-y divide-white/[0.03]">
                  {group.caches.map(cache => {
                    const key = `${cache.tool_id}:${cache.dir_name}`;
                    return (
                      <div key={key} className="px-4 py-3 flex items-center gap-3 hover:bg-white/[0.01]">
                        <FolderOpen className="w-3.5 h-3.5 text-slate-600 flex-shrink-0" />
                        <div className="flex-1 min-w-0">
                          <div className="flex items-center gap-2">
                            <span className="text-[11px] text-slate-300 font-mono truncate">{cache.dir_name}</span>
                            {cache.is_junction && (
                              <span
                                className="text-[8px] font-bold bg-blue-500/15 text-blue-400 px-1.5 py-0.5 rounded cursor-help"
                                title={`JUNCTION → ${cache.junction_target}`}
                              >
                                <Link2 className="w-2.5 h-2.5 inline mr-0.5" />
                                JUNCTION
                              </span>
                            )}
                          </div>
                          <div className="flex items-center gap-2 mt-0.5">
                            <span className="text-[9px] text-slate-500 font-mono truncate max-w-[280px]">
                              {cache.full_path}
                            </span>
                            <span className={`text-[9px] font-semibold ${cache.size_bytes > 0 ? "text-amber-400" : "text-slate-600"}`}>
                              {cache.exists ? cache.size : "不存在"}
                            </span>
                          </div>
                          {/* Junction 目标路径 */}
                          {cache.is_junction && cache.junction_target && (
                            <div className="text-[8px] text-blue-400/60 mt-0.5">
                              → {cache.junction_target}
                            </div>
                          )}
                        </div>

                        {/* 操作按钮 */}
                        {cache.exists && (
                          <div className="flex items-center gap-1 flex-shrink-0">
                            <button
                              onClick={() => handleOpen(cache.dir_name)}
                              className="p-1.5 rounded text-slate-600 hover:text-blue-400 hover:bg-blue-500/10 cursor-pointer transition-all"
                              title="打开目录"
                            >
                              <FolderOpen className="w-3.5 h-3.5" />
                            </button>
                            <button
                              onClick={() => handleMigrate(cache.tool_id, cache.dir_name, cache.full_path)}
                              disabled={migrating === key}
                              className="p-1.5 rounded text-slate-600 hover:text-emerald-400 hover:bg-emerald-500/10 cursor-pointer transition-all disabled:opacity-50"
                              title="迁移到新位置（JUNCTION）"
                            >
                              {migrating === key ? (
                                <RefreshCw className="w-3.5 h-3.5 animate-spin" />
                              ) : (
                                <FolderSync className="w-3.5 h-3.5" />
                              )}
                            </button>

                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* 说明 */}
      <div className="p-3 rounded-xl bg-blue-500/5 border border-blue-500/10 text-[10px] text-slate-400 space-y-1">
        <p className="font-semibold text-blue-300 flex items-center gap-1">
          <Link2 className="w-3 h-3" /> 仅支持 Junction 迁移
        </p>
        <p>迁移会在新位置保存数据，原位置替换为 NTFS JUNCTION 目录链接。AI 工具仍访问原路径，数据实际存储在新位置。</p>
        <p className="text-amber-400/80">禁止直接指向目录：请勿通过修改配置文件把缓存路径直接改到新目录，必须保持原路径并通过 Junction 重定向。</p>
      </div>
    </div>
  );
}
