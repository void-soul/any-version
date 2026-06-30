import React, { useState, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { 
  ArrowUp, 
  ArrowDown, 
  Trash2, 
  Plus, 
  RefreshCw, 
  Save, 
  ShieldAlert, 
  Folder, 
  CheckCircle, 
  AlertTriangle, 
  ChevronsUp, 
  ChevronsDown,
  Info
} from "lucide-react";

interface PathDirectoryInfo {
  path: string;
  source: string; // "HKCU" | "HKLM"
  exists: boolean;
  executables: string[];
}

export default function PathEnvManager() {
  const [paths, setPaths] = useState<PathDirectoryInfo[]>([]);
  const [isAdmin, setIsAdmin] = useState<boolean>(false);
  const [loading, setLoading] = useState<boolean>(false);
  const [saving, setSaving] = useState<boolean>(false);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [newPathInput, setNewPathInput] = useState<string>("");
  const [newPathSource, setNewPathSource] = useState<"HKCU" | "HKLM">("HKCU");
  const [message, setMessage] = useState<{ text: string; type: "success" | "error" | "info" } | null>(null);

  const fetchPaths = async () => {
    setLoading(true);
    setMessage(null);
    try {
      const adminStatus = await invoke<boolean>("is_admin");
      setIsAdmin(adminStatus);
      const data = await invoke<PathDirectoryInfo[]>("get_path_directories");
      setPaths(data);
      if (data.length > 0 && !selectedPath) {
        setSelectedPath(data[0].path);
      }
    } catch (err: any) {
      setMessage({ text: `加载失败: ${err.message || err}`, type: "error" });
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchPaths();
  }, []);

  // Compute conflict map: exe_name (lowercase) -> list of path directories
  const conflictMap = useMemo(() => {
    const map: Record<string, string[]> = {};
    paths.forEach((p) => {
      if (!p.exists) return;
      p.executables.forEach((exe) => {
        const exeLower = exe.toLowerCase();
        if (!map[exeLower]) {
          map[exeLower] = [];
        }
        map[exeLower].push(p.path);
      });
    });
    return map;
  }, [paths]);

  // Check if a path has any duplicate executables
  const getPathConflicts = (pInfo: PathDirectoryInfo) => {
    if (!pInfo.exists) return [];
    return pInfo.executables.filter((exe) => {
      const exeLower = exe.toLowerCase();
      return conflictMap[exeLower] && conflictMap[exeLower].length > 1;
    });
  };

  const handleMove = (index: number, direction: "up" | "down" | "top" | "bottom") => {
    const list = [...paths];
    const item = list[index];
    const source = item.source;

    // Filter paths by the same source (only move within HKCU or HKLM)
    const sourceIndices = list
      .map((p, idx) => (p.source === source ? idx : -1))
      .filter((idx) => idx !== -1);

    const sourcePosition = sourceIndices.indexOf(index);
    if (sourcePosition === -1) return;

    let targetSourcePosition = sourcePosition;
    if (direction === "up" && sourcePosition > 0) {
      targetSourcePosition = sourcePosition - 1;
    } else if (direction === "down" && sourcePosition < sourceIndices.length - 1) {
      targetSourcePosition = sourcePosition + 1;
    } else if (direction === "top") {
      targetSourcePosition = 0;
    } else if (direction === "bottom") {
      targetSourcePosition = sourceIndices.length - 1;
    }

    if (targetSourcePosition === sourcePosition) return;

    // Rearrange within the combined list
    const targetIndex = sourceIndices[targetSourcePosition];
    list.splice(index, 1);
    list.splice(targetIndex, 0, item);
    setPaths(list);
  };

  const handleDelete = (index: number) => {
    const deletedPath = paths[index].path;
    const list = paths.filter((_, idx) => idx !== index);
    setPaths(list);
    if (selectedPath === deletedPath) {
      setSelectedPath(list.length > 0 ? list[0].path : null);
    }
  };

  const handleAddPath = () => {
    const trimmed = newPathInput.trim();
    if (!trimmed) return;
    
    // Check if duplicate in the same source
    if (paths.some((p) => p.path.toLowerCase() === trimmed.toLowerCase() && p.source === newPathSource)) {
      setMessage({ text: "该路径已存在于所选的变量源中", type: "error" });
      return;
    }

    // Prepare path item
    const newItem: PathDirectoryInfo = {
      path: trimmed,
      source: newPathSource,
      exists: false, // We'll let backend recheck it on reload or default to false
      executables: []
    };

    // Add to the end of that source
    const list = [...paths];
    const lastIndex = list.map((p, i) => p.source === newPathSource ? i : -1).reduce((acc, curr) => Math.max(acc, curr), -1);
    
    if (lastIndex === -1) {
      list.push(newItem);
    } else {
      list.splice(lastIndex + 1, 0, newItem);
    }

    setPaths(list);
    setSelectedPath(trimmed);
    setNewPathInput("");
    setMessage({ text: "已添加路径（保存后生效，建议刷新以检测其是否有效）", type: "success" });
  };

  const handleSave = async (saveAll: boolean) => {
    setSaving(true);
    setMessage(null);
    try {
      const userPaths = paths.filter((p) => p.source === "HKCU").map((p) => p.path);
      const systemPaths = paths.filter((p) => p.source === "HKLM").map((p) => p.path);

      if (saveAll && systemPaths.length > 0 && !isAdmin) {
        throw new Error("修改系统环境变量 (HKLM) 需要管理员权限，请重新以管理员身份运行 AnyVersion，或者仅保存用户级 PATH");
      }

      await invoke("save_path_directories", { 
        userPaths, 
        systemPaths: saveAll ? systemPaths : [], 
        saveSystem: saveAll 
      });
      setMessage({ 
        text: saveAll 
          ? "全部环境变量 PATH 保存并备份成功！新的顺序已生效。" 
          : "用户级环境变量 PATH 保存并备份成功！新的顺序已生效。", 
        type: "success" 
      });
      // Fetch again to refresh exists/executable states
      await fetchPaths();
    } catch (err: any) {
      setMessage({ text: `保存失败: ${err.message || err}`, type: "error" });
    } finally {
      setSaving(false);
    }
  };

  // Group paths
  const hkcuPaths = useMemo(() => paths.filter((p) => p.source === "HKCU"), [paths]);
  const hklmPaths = useMemo(() => paths.filter((p) => p.source === "HKLM"), [paths]);

  const selectedPathInfo = useMemo(() => {
    return paths.find((p) => p.path === selectedPath) || null;
  }, [paths, selectedPath]);

  const selectedConflicts = useMemo(() => {
    if (!selectedPathInfo) return [];
    return getPathConflicts(selectedPathInfo);
  }, [selectedPathInfo, conflictMap]);

  return (
    <div className="flex-grow flex flex-col min-h-0 bg-slate-950/20 text-slate-100 rounded-xl overflow-hidden border border-white/5">
      {/* 顶部工具栏 */}
      <div className="p-4 border-b border-white/5 bg-slate-900/40 backdrop-blur-md flex flex-wrap items-center justify-between gap-4">
        <div>
          <h2 className="text-sm font-bold text-slate-200 font-sans">PATH 环境变量管理与排列</h2>
          <p className="text-[10px] text-slate-400 mt-0.5">
            对系统的 PATH 物理顺序进行排序调整。排在越上方，其可执行程序（.exe/.cmd/.bat）的解析优先级越高。保存前将自动创建环境备份。
          </p>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={fetchPaths}
            disabled={loading || saving}
            className="px-2.5 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] font-semibold text-slate-300 hover:text-white hover:bg-white/10 transition-all flex items-center gap-1 cursor-pointer disabled:opacity-50"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${loading ? "animate-spin" : ""}`} />
            刷新
          </button>
          
          <button
            onClick={() => handleSave(false)}
            disabled={loading || saving}
            className="px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white text-[10px] font-semibold flex items-center gap-1 transition-all shadow-lg shadow-emerald-500/10 cursor-pointer disabled:opacity-50"
            title="仅保存修改后的用户级 PATH 变量，不需要管理员权限"
          >
            <Save className="w-3.5 h-3.5" />
            {saving ? "正在保存..." : "保存用户级变量"}
          </button>

          <button
            onClick={() => handleSave(true)}
            disabled={loading || saving}
            className="px-3 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-500 text-white text-[10px] font-semibold flex items-center gap-1 transition-all shadow-lg shadow-blue-500/10 cursor-pointer disabled:opacity-50"
            title="保存全部（用户级和系统级）PATH 变量，修改系统级变量需要管理员权限"
          >
            <Save className="w-3.5 h-3.5" />
            {saving ? "正在保存..." : "保存全部"}
          </button>
        </div>
      </div>

      {/* 管理员权限警告 */}
      {!isAdmin && (
        <div className="px-4 py-2 bg-yellow-500/10 border-b border-yellow-500/20 text-[10px] text-yellow-500 flex items-center gap-2">
          <ShieldAlert className="w-4 h-4 flex-shrink-0" />
          <span>当前未以管理员权限运行。您可以管理“用户级 PATH”，但保存“系统级 PATH”时可能会失败。</span>
        </div>
      )}

      {/* 提示信息 */}
      {message && (
        <div className={`px-4 py-2 border-b text-[10px] flex items-center gap-2 ${
          message.type === "success" 
            ? "bg-green-500/10 border-green-500/20 text-green-400" 
            : message.type === "error" 
            ? "bg-red-500/10 border-red-500/20 text-red-400" 
            : "bg-blue-500/10 border-blue-500/20 text-blue-400"
        }`}>
          {message.type === "success" ? (
            <CheckCircle className="w-4 h-4 flex-shrink-0" />
          ) : (
            <AlertTriangle className="w-4 h-4 flex-shrink-0" />
          )}
          <span>{message.text}</span>
        </div>
      )}

      {/* 左右分栏面板 */}
      <div className="flex-1 flex min-h-0 overflow-hidden">
        {/* 左栏：路径物理排序 */}
        <div className="flex-1 overflow-y-auto p-4 border-r border-white/5 min-w-[55%]">
          {/* 添加新路径栏 */}
          <div className="mb-4 p-3 bg-white/5 border border-white/5 rounded-xl flex flex-wrap items-center gap-2">
            <input
              type="text"
              placeholder="输入并添加新的 PATH 目录，例如 D:\tools\python"
              value={newPathInput}
              onChange={(e) => setNewPathInput(e.target.value)}
              className="flex-1 min-w-[200px] bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-[11px] text-slate-200 placeholder-slate-500 focus:outline-none focus:border-blue-500"
            />
            <div className="flex items-center gap-1">
              <select
                value={newPathSource}
                onChange={(e) => setNewPathSource(e.target.value as "HKCU" | "HKLM")}
                className="bg-slate-900 border border-white/10 rounded-lg px-2 py-1.5 text-[11px] text-slate-300 focus:outline-none"
              >
                <option value="HKCU">用户级 (HKCU)</option>
                <option value="HKLM">系统级 (HKLM)</option>
              </select>
              <button
                onClick={handleAddPath}
                className="p-1.5 rounded-lg bg-blue-600 hover:bg-blue-500 text-white transition-all cursor-pointer"
                title="添加路径"
              >
                <Plus className="w-4 h-4" />
              </button>
            </div>
          </div>

          {/* 渲染 HKEY_CURRENT_USER PATH 变量 */}
          <div className="mb-6">
            <div className="flex items-center justify-between mb-2">
              <span className="text-[11px] font-bold text-slate-400">用户级环境变量 PATH (HKEY_CURRENT_USER)</span>
              <span className="text-[10px] text-slate-500 bg-white/5 px-2 py-0.5 rounded-full">{hkcuPaths.length} 条</span>
            </div>
            <div className="space-y-1.5">
              {paths.map((pInfo, index) => {
                if (pInfo.source !== "HKCU") return null;
                const conflicts = getPathConflicts(pInfo);
                const hasConflicts = conflicts.length > 0;
                const isSelected = selectedPath === pInfo.path;

                return (
                  <div
                    key={`${pInfo.source}-${pInfo.path}-${index}`}
                    onClick={() => setSelectedPath(pInfo.path)}
                    className={`p-2.5 rounded-xl border flex items-center justify-between gap-3 cursor-pointer group transition-all ${
                      isSelected
                        ? "bg-blue-600/10 border-blue-500/40"
                        : "bg-slate-900/40 border-white/5 hover:border-white/10"
                    }`}
                  >
                    <div className="flex items-center gap-2.5 min-w-0">
                      {/* 排序操作区 */}
                      <div className="flex items-center gap-0.5 flex-shrink-0 opacity-45 group-hover:opacity-100 transition-all">
                        <button
                          onClick={(e) => { e.stopPropagation(); handleMove(index, "top"); }}
                          className="p-0.5 text-slate-400 hover:text-white rounded"
                          title="置顶"
                        >
                          <ChevronsUp className="w-3.5 h-3.5" />
                        </button>
                        <button
                          onClick={(e) => { e.stopPropagation(); handleMove(index, "up"); }}
                          className="p-0.5 text-slate-400 hover:text-white rounded"
                          title="上移"
                        >
                          <ArrowUp className="w-3.5 h-3.5" />
                        </button>
                        <button
                          onClick={(e) => { e.stopPropagation(); handleMove(index, "down"); }}
                          className="p-0.5 text-slate-400 hover:text-white rounded"
                          title="下移"
                        >
                          <ArrowDown className="w-3.5 h-3.5" />
                        </button>
                        <button
                          onClick={(e) => { e.stopPropagation(); handleMove(index, "bottom"); }}
                          className="p-0.5 text-slate-400 hover:text-white rounded"
                          title="置底"
                        >
                          <ChevronsDown className="w-3.5 h-3.5" />
                        </button>
                      </div>

                      {/* 目录名称 */}
                      <div className="min-w-0">
                        <div className={`text-[11px] truncate font-medium ${!pInfo.exists ? "text-red-400 line-through" : "text-slate-200"}`} title={pInfo.path}>
                          {pInfo.path}
                        </div>
                      </div>
                    </div>

                    {/* 状态徽章与删除 */}
                    <div className="flex items-center gap-2 flex-shrink-0">
                      {!pInfo.exists ? (
                        <span className="px-1.5 py-0.5 bg-red-500/10 border border-red-500/20 text-red-500 text-[8px] font-semibold rounded-md">不存在</span>
                      ) : hasConflicts ? (
                        <span className="px-1.5 py-0.5 bg-yellow-500/10 border border-yellow-500/20 text-yellow-500 text-[8px] font-semibold rounded-md flex items-center gap-0.5">
                          <AlertTriangle className="w-2.5 h-2.5" />
                          冲突 ({conflicts.length})
                        </span>
                      ) : (
                        <span className="px-1.5 py-0.5 bg-slate-800 border border-slate-700 text-slate-400 text-[8px] font-semibold rounded-md">正常</span>
                      )}

                      <button
                        onClick={(e) => { e.stopPropagation(); handleDelete(index); }}
                        className="p-1 text-slate-400 hover:text-red-400 hover:bg-red-500/10 rounded transition-all cursor-pointer opacity-0 group-hover:opacity-100"
                        title="删除路径"
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          </div>

          {/* 渲染 HKEY_LOCAL_MACHINE PATH 变量 */}
          <div>
            <div className="flex items-center justify-between mb-2">
              <span className="text-[11px] font-bold text-slate-400">系统级环境变量 PATH (HKEY_LOCAL_MACHINE)</span>
              <span className="text-[10px] text-slate-500 bg-white/5 px-2 py-0.5 rounded-full">{hklmPaths.length} 条</span>
            </div>
            <div className="space-y-1.5">
              {paths.map((pInfo, index) => {
                if (pInfo.source !== "HKLM") return null;
                const conflicts = getPathConflicts(pInfo);
                const hasConflicts = conflicts.length > 0;
                const isSelected = selectedPath === pInfo.path;

                return (
                  <div
                    key={`${pInfo.source}-${pInfo.path}-${index}`}
                    onClick={() => setSelectedPath(pInfo.path)}
                    className={`p-2.5 rounded-xl border flex items-center justify-between gap-3 cursor-pointer group transition-all ${
                      isSelected
                        ? "bg-blue-600/10 border-blue-500/40"
                        : "bg-slate-900/40 border-white/5 hover:border-white/10"
                    }`}
                  >
                    <div className="flex items-center gap-2.5 min-w-0">
                      {/* 排序操作区 */}
                      <div className="flex items-center gap-0.5 flex-shrink-0 opacity-45 group-hover:opacity-100 transition-all">
                        <button
                          onClick={(e) => { e.stopPropagation(); handleMove(index, "top"); }}
                          className="p-0.5 text-slate-400 hover:text-white rounded"
                          title="置顶"
                        >
                          <ChevronsUp className="w-3.5 h-3.5" />
                        </button>
                        <button
                          onClick={(e) => { e.stopPropagation(); handleMove(index, "up"); }}
                          className="p-0.5 text-slate-400 hover:text-white rounded"
                          title="上移"
                        >
                          <ArrowUp className="w-3.5 h-3.5" />
                        </button>
                        <button
                          onClick={(e) => { e.stopPropagation(); handleMove(index, "down"); }}
                          className="p-0.5 text-slate-400 hover:text-white rounded"
                          title="下移"
                        >
                          <ArrowDown className="w-3.5 h-3.5" />
                        </button>
                        <button
                          onClick={(e) => { e.stopPropagation(); handleMove(index, "bottom"); }}
                          className="p-0.5 text-slate-400 hover:text-white rounded"
                          title="置底"
                        >
                          <ChevronsDown className="w-3.5 h-3.5" />
                        </button>
                      </div>

                      {/* 目录名称 */}
                      <div className="min-w-0">
                        <div className={`text-[11px] truncate font-medium ${!pInfo.exists ? "text-red-400 line-through" : "text-slate-200"}`} title={pInfo.path}>
                          {pInfo.path}
                        </div>
                      </div>
                    </div>

                    {/* 状态徽章与删除 */}
                    <div className="flex items-center gap-2 flex-shrink-0">
                      {!pInfo.exists ? (
                        <span className="px-1.5 py-0.5 bg-red-500/10 border border-red-500/20 text-red-500 text-[8px] font-semibold rounded-md">不存在</span>
                      ) : hasConflicts ? (
                        <span className="px-1.5 py-0.5 bg-yellow-500/10 border border-yellow-500/20 text-yellow-500 text-[8px] font-semibold rounded-md flex items-center gap-0.5">
                          <AlertTriangle className="w-2.5 h-2.5" />
                          冲突 ({conflicts.length})
                        </span>
                      ) : (
                        <span className="px-1.5 py-0.5 bg-slate-800 border border-slate-700 text-slate-400 text-[8px] font-semibold rounded-md">正常</span>
                      )}

                      <button
                        onClick={(e) => { e.stopPropagation(); handleDelete(index); }}
                        className="p-1 text-slate-400 hover:text-red-400 hover:bg-red-500/10 rounded transition-all cursor-pointer opacity-0 group-hover:opacity-100"
                        title="删除路径"
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        </div>

        {/* 右栏：当前路径细节与可执行文件详情 */}
        <div className="w-[45%] bg-slate-900/20 flex flex-col min-h-0 overflow-y-auto p-4 border-l border-white/5">
          {selectedPathInfo ? (
            <div className="flex-grow flex flex-col min-h-0">
              <div className="flex items-center gap-1.5 text-[10px] text-blue-400 font-semibold mb-1">
                <Folder className="w-3.5 h-3.5" />
                {selectedPathInfo.source === "HKCU" ? "用户级 PATH 条目" : "系统级 PATH 条目"}
              </div>
              <h3 className="text-xs font-bold text-slate-200 break-all mb-4 bg-slate-900 border border-white/5 p-2.5 rounded-lg select-all">
                {selectedPathInfo.path}
              </h3>

              {!selectedPathInfo.exists ? (
                <div className="flex-1 flex flex-col items-center justify-center p-8 border border-dashed border-red-500/20 rounded-xl bg-red-500/5 text-center">
                  <AlertTriangle className="w-8 h-8 text-red-500 mb-2" />
                  <span className="text-[11px] font-bold text-red-400">该目录在本地硬盘上不存在</span>
                  <span className="text-[9px] text-slate-500 mt-1 max-w-[200px]">无效的 PATH 环境变量条目会延长命令行查询的耗时，若不需要可进行删除。</span>
                </div>
              ) : selectedPathInfo.executables.length === 0 ? (
                <div className="flex-1 flex flex-col items-center justify-center p-8 border border-dashed border-white/5 rounded-xl bg-white/5 text-center">
                  <Folder className="w-8 h-8 text-slate-600 mb-2" />
                  <span className="text-[11px] font-bold text-slate-400">未包含可执行文件</span>
                  <span className="text-[9px] text-slate-500 mt-1 max-w-[200px]">该目录下没有找到 `.exe`、`.cmd` 或 `.bat` 文件。</span>
                </div>
              ) : (
                <div className="flex-1 flex flex-col min-h-0">
                  <div className="text-[11px] font-bold text-slate-400 mb-2 flex items-center justify-between">
                    <span>可执行程序列表 ({selectedPathInfo.executables.length})</span>
                    {selectedConflicts.length > 0 && (
                      <span className="text-yellow-500 text-[10px] font-medium flex items-center gap-1">
                        <AlertTriangle className="w-3.5 h-3.5" />
                        存在 {selectedConflicts.length} 个冲突文件
                      </span>
                    )}
                  </div>
                  
                  <div className="flex-1 min-h-0 overflow-y-auto space-y-1.5 pr-1">
                    {selectedPathInfo.executables.map((exe) => {
                      const exeLower = exe.toLowerCase();
                      const otherPaths = conflictMap[exeLower] 
                        ? conflictMap[exeLower].filter((p) => p !== selectedPathInfo.path)
                        : [];
                      const isConflict = otherPaths.length > 0;

                      return (
                        <div
                          key={exe}
                          className={`p-2.5 rounded-lg border text-[10.5px] transition-all ${
                            isConflict
                              ? "bg-yellow-500/5 border-yellow-500/25 hover:bg-yellow-500/10"
                              : "bg-slate-900/60 border-white/5 hover:bg-slate-900"
                          }`}
                        >
                          <div className="flex items-center justify-between">
                            <span className={`font-mono font-medium ${isConflict ? "text-yellow-400" : "text-slate-300"}`}>
                              {exe}
                            </span>
                            {isConflict && (
                              <span className="px-1.5 py-0.5 bg-yellow-500/10 border border-yellow-500/20 text-yellow-500 text-[8px] font-bold rounded">
                                覆盖冲突
                              </span>
                            )}
                          </div>
                          
                          {isConflict && (
                            <div className="mt-1.5 pt-1.5 border-t border-yellow-500/10 text-[9.5px] text-slate-400">
                              <div className="flex items-center gap-1 font-semibold text-yellow-500/80 mb-1">
                                <Info className="w-3 h-3" />
                                冲突路径（列表中排在前面的目录将覆盖后面的）：
                              </div>
                              <ul className="list-disc pl-3.5 space-y-0.5 font-mono">
                                {otherPaths.map((op, idx) => (
                                  <li key={idx} className="break-all">
                                    {op}
                                  </li>
                                ))}
                              </ul>
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                </div>
              )}
            </div>
          ) : (
            <div className="flex-grow flex flex-col items-center justify-center p-8 text-center text-slate-500">
              <Folder className="w-10 h-10 text-slate-700 mb-2 animate-pulse" />
              <span className="text-[11px]">在左侧选择一个 PATH 条目查看其包含的可执行文件及冲突</span>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
