import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Shield,
  ShieldAlert,
  ShieldCheck,
  Activity,
  AlertTriangle,
  Info,
  CheckSquare,
  Square,
  RefreshCw,
  Zap
} from "lucide-react";

interface Problem {
  id: string;
  problem_type: string;
  description: string;
  detail: string;
  severity: string; // "严重" | "警告" | "建议"
  fix_type: string;
  fix_target: string;
  // 检测依据
  evidence_source: string;
  evidence_content: string;
  evidence_reason: string;
  // 修复方案
  fix_plan: string;
  fix_file: string;
  fix_source_path: string;
  fix_dest_path: string;
}

export default function EnvDiagnostics() {
  const [scanning, setScanning] = useState(false);
  const [problems, setProblems] = useState<Problem[]>([]);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [selectedProblem, setSelectedProblem] = useState<Problem | null>(null);
  const [hasScanned, setHasScanned] = useState(false);
  const [repairing, setRepairing] = useState(false);
  const [repairSuccess, setRepairSuccess] = useState<string | null>(null);

  const startScan = async () => {
    setScanning(true);
    setRepairSuccess(null);
    try {
      // Simulate scan animation for better premium feel
      await new Promise((resolve) => setTimeout(resolve, 1500));
      const res = await invoke<Problem[]>("scan_environment");
      setProblems(res);
      // Auto-select all by default
      setSelectedIds(new Set(res.map(p => p.id)));
      if (res.length > 0) {
        setSelectedProblem(res[0]);
      } else {
        setSelectedProblem(null);
      }
      setHasScanned(true);
    } catch (e: any) {
      console.error(e);
    } finally {
      setScanning(false);
    }
  };

  const handleResolve = async () => {
    if (selectedIds.size === 0) return;
    setRepairing(true);
    setRepairSuccess(null);
    try {
      const toResolve = problems.filter(p => selectedIds.has(p.id));
      await invoke("resolve_problems", { problems: toResolve });
      setRepairSuccess(`成功修复了 ${toResolve.length} 个环境问题！`);
      // Re-scan
      const res = await invoke<Problem[]>("scan_environment");
      setProblems(res);
      setSelectedIds(new Set(res.map(p => p.id)));
      if (res.length > 0) {
        setSelectedProblem(res[0]);
      } else {
        setSelectedProblem(null);
      }
    } catch (e: any) {
      setRepairSuccess(`修复失败: ${e}`);
    } finally {
      setRepairing(false);
    }
  };

  const toggleSelect = (id: string) => {
    const next = new Set(selectedIds);
    if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    setSelectedIds(next);
  };

  const toggleSelectAll = () => {
    if (selectedIds.size === problems.length) {
      setSelectedIds(new Set());
    } else {
      setSelectedIds(new Set(problems.map(p => p.id)));
    }
  };

  const getSeverityColor = (sev: string) => {
    switch (sev) {
      case "严重": return "bg-red-500/10 text-red-400 border border-red-500/20";
      case "警告": return "bg-amber-500/10 text-amber-400 border border-amber-500/20";
      default: return "bg-blue-500/10 text-blue-400 border border-blue-500/20";
    }
  };

  const criticalCount = problems.filter(p => p.severity === "严重").length;
  const warningCount = problems.filter(p => p.severity === "警告").length;
  const suggestionCount = problems.filter(p => p.severity === "建议").length;

  const rules = [
    {
      id: "c_drive_cache",
      title: "C盘缓存占用检测",
      desc: "依据 npm/yarn/pnpm 的 config 命令、pip.ini、.m2/settings.xml 等定位缓存目录，判断是否仍在 C 盘且未重定向",
    },
    {
      id: "dead_env_path",
      title: "无效环境变量检测",
      desc: "读取注册表 HKCU\\Environment 的 PATH 及 JAVA_HOME 等变量，逐条核对磁盘上是否真实存在",
    },
    {
      id: "conflict_env",
      title: "环境优先级冲突检测",
      desc: "扫描 PATH 顺序，检测是否存在排在 Any-Version 之前的外部 go/node/python 等，导致版本切换不生效",
    },
    {
      id: "residue_files",
      title: "服务残留数据检测",
      desc: "扫描 MySQL/MongoDB/PostgreSQL 常见默认数据目录，找出已停止服务遗留的数据文件夹",
    },
  ];

  const renderRuleStatus = (ruleId: string) => {
    if (scanning) {
      return (
        <span className="flex items-center gap-1 px-2 py-0.5 rounded-md text-[10px] font-semibold bg-blue-500/10 text-blue-400 border border-blue-500/20">
          <RefreshCw className="w-3 h-3 animate-spin" />
          检测中...
        </span>
      );
    }
    if (!hasScanned) {
      return (
        <span className="px-2 py-0.5 rounded-md text-[10px] font-semibold bg-slate-500/10 text-slate-400 border border-slate-500/20">
          待检测
        </span>
      );
    }
    const count = problems.filter(p => p.problem_type === ruleId).length;
    if (count > 0) {
      return (
        <span className="px-2 py-0.5 rounded-md text-[10px] font-semibold bg-amber-500/10 text-amber-400 border border-amber-500/20">
          发现 {count} 个问题
        </span>
      );
    }
    return (
      <span className="px-2 py-0.5 rounded-md text-[10px] font-semibold bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 flex items-center gap-0.5">
        <ShieldCheck className="w-3 h-3" />
        正常
      </span>
    );
  };

  return (
    <div className="flex-1 p-8 overflow-y-auto space-y-6 h-screen select-none">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-semibold text-white tracking-wide">集成环境体检</h2>
          <p className="text-xs text-slate-400 mt-1">扫描环境变量、路径冲突、文件残留及本地缓存配置并一键优化修复。每一项都会说明「依据哪个配置文件检测」和「将要如何修复」。</p>
        </div>

        {hasScanned && (
          <button
            onClick={startScan}
            disabled={scanning || repairing}
            className="flex items-center gap-2 px-4 py-2 bg-white/5 hover:bg-white/10 text-slate-200 rounded-xl text-xs font-medium border border-white/10 transition-all cursor-pointer disabled:opacity-50"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${scanning ? "animate-spin" : ""}`} />
            重新体检
          </button>
        )}
      </div>

      <div className="grid grid-cols-1 md:grid-cols-4 gap-4">
        {rules.map((rule) => (
          <div key={rule.id} className="glass-panel rounded-xl p-4 border border-white/5 flex flex-col justify-between gap-3 bg-white/1">
            <div>
              <div className="flex items-center justify-between gap-2">
                <span className="text-xs font-semibold text-white">{rule.title}</span>
                {renderRuleStatus(rule.id)}
              </div>
              <p className="text-[10px] text-slate-400 mt-1.5 leading-relaxed">{rule.desc}</p>
            </div>
          </div>
        ))}
      </div>

      {!hasScanned ? (
        <div className="glass-panel rounded-2xl p-12 flex flex-col items-center justify-center text-center max-w-2xl mx-auto mt-4 border border-white/5">
          <div className="w-24 h-24 rounded-full bg-blue-600/10 flex items-center justify-center text-blue-400 relative">
            <div className="absolute inset-0 rounded-full border border-blue-500/20 animate-ping"></div>
            <Shield className="w-12 h-12" />
          </div>

          <h3 className="text-base font-medium text-white mt-6">您尚未进行系统环境体检</h3>
          <p className="text-xs text-slate-400 mt-2 max-w-sm">体检将扫描死链 PATH、多外部开发包冲突、缓存位置冗余等系统问题，确保多语言开发环境干净、配置正确。</p>

          <button
            onClick={startScan}
            disabled={scanning}
            className="mt-8 px-8 py-3.5 bg-blue-600 hover:bg-blue-500 text-white rounded-xl text-xs font-semibold shadow-lg shadow-blue-500/20 transition-all cursor-pointer flex items-center gap-2 hover:scale-[1.02] active:scale-[0.98]"
          >
            {scanning ? (
              <>
                <RefreshCw className="w-4 h-4 animate-spin" />
                正在深度扫描...
              </>
            ) : (
              <>
                <Activity className="w-4 h-4" />
                一键深度体检
              </>
            )}
          </button>
        </div>
      ) : (
        <div className="grid grid-cols-1 lg:grid-cols-3 gap-6 animate-fadeIn">
          <div className="lg:col-span-2 space-y-6">
            <div className="glass-panel rounded-2xl p-6 flex items-center justify-between border border-white/5 bg-white/2">
              <div className="flex items-center gap-4">
                <div className={`w-14 h-14 rounded-full flex items-center justify-center ${problems.length > 0 ? "bg-red-500/10 text-red-400" : "bg-emerald-500/10 text-emerald-400"
                  }`}>
                  {problems.length > 0 ? <ShieldAlert className="w-7 h-7" /> : <ShieldCheck className="w-7 h-7" />}
                </div>
                <div>
                  <h4 className="font-semibold text-white text-sm">
                    {problems.length > 0 ? `检测到 ${problems.length} 个环境隐患` : "系统环境良好"}
                  </h4>
                  <p className="text-[11px] text-slate-400 mt-0.5">
                    {problems.length > 0 ? "建议勾选问题后进行一键修复" : "所有检测项目已全部通过！未发现异常。"}
                  </p>
                </div>
              </div>

              <div className="flex gap-2">
                <span className="px-2.5 py-1 bg-red-500/10 border border-red-500/20 rounded-md text-[10px] text-red-400">
                  严重: {criticalCount}
                </span>
                <span className="px-2.5 py-1 bg-amber-500/10 border border-amber-500/20 rounded-md text-[10px] text-amber-400">
                  警告: {warningCount}
                </span>
                <span className="px-2.5 py-1 bg-blue-500/10 border border-blue-500/20 rounded-md text-[10px] text-blue-400">
                  建议: {suggestionCount}
                </span>
              </div>
            </div>

            <div className="glass-panel rounded-2xl border border-white/5 overflow-hidden">
              <div className="p-4 bg-white/3 border-b border-white/5 flex items-center justify-between">
                <button
                  onClick={toggleSelectAll}
                  className="flex items-center gap-2 text-[11px] text-slate-400 hover:text-slate-200 transition-all cursor-pointer"
                >
                  {selectedIds.size === problems.length && problems.length > 0 ? (
                    <CheckSquare className="w-4 h-4 text-blue-400" />
                  ) : (
                    <Square className="w-4 h-4" />
                  )}
                  全选/全不选
                </button>
                <span className="text-[11px] text-slate-400">已选中 {selectedIds.size} 项</span>
              </div>

              {problems.length === 0 ? (
                <div className="p-12 text-center text-slate-400">
                  <ShieldCheck className="w-12 h-12 text-emerald-400 mx-auto mb-4" />
                  <p className="text-xs font-medium text-white">没有需要解决的环境问题</p>
                </div>
              ) : (
                <div className="divide-y divide-white/5 max-h-[420px] overflow-y-auto">
                  {problems.map((p) => {
                    const isSelected = selectedIds.has(p.id);
                    return (
                      <div
                        key={p.id}
                        onClick={() => setSelectedProblem(p)}
                        className={`p-4 flex items-start gap-3 hover:bg-white/2 transition-all cursor-pointer ${selectedProblem?.id === p.id ? "bg-blue-600/5 border-l-2 border-blue-500" : ""
                          }`}
                      >
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            toggleSelect(p.id);
                          }}
                          className="mt-0.5 text-slate-400 hover:text-slate-200 transition-all"
                        >
                          {isSelected ? (
                            <CheckSquare className="w-4 h-4 text-blue-400" />
                          ) : (
                            <Square className="w-4 h-4" />
                          )}
                        </button>

                        <div className="flex-1 min-w-0">
                          <div className="flex items-center gap-2">
                            <span className={`px-1.5 py-0.5 rounded text-[9px] font-semibold uppercase ${getSeverityColor(p.severity)}`}>
                              {p.severity}
                            </span>
                            <h5 className="font-medium text-white text-xs truncate">{p.description}</h5>
                          </div>
                          <p className="text-[10px] text-slate-400 mt-1 font-mono truncate">{p.detail}</p>
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>

            {problems.length > 0 && (
              <div className="flex items-center justify-between">
                <div>
                  {repairSuccess && (
                    <span className="text-xs font-medium text-emerald-400 flex items-center gap-1.5">
                      <ShieldCheck className="w-4 h-4" />
                      {repairSuccess}
                    </span>
                  )}
                </div>
                <button
                  onClick={handleResolve}
                  disabled={selectedIds.size === 0 || repairing}
                  className="px-6 py-3 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold shadow-lg shadow-blue-500/20 transition-all cursor-pointer flex items-center gap-2 hover:scale-[1.02] active:scale-[0.98]"
                >
                  <Zap className="w-3.5 h-3.5" />
                  {repairing ? "正在一键修复..." : "一键修复已选问题"}
                </button>
              </div>
            )}
          </div>

          <div className="lg:col-span-1">
            <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-4 h-[520px] flex flex-col">
              <div className="flex items-center gap-2 pb-3 border-b border-white/5">
                <Info className="w-4 h-4 text-blue-400" />
                <h4 className="font-semibold text-white text-xs">诊断建议详情</h4>
              </div>

              {selectedProblem ? (
                <div className="flex-1 flex flex-col justify-between overflow-y-auto -mr-2 pr-2">
                  <div className="space-y-4 min-w-0">
                    <div>
                      <span className="text-[10px] text-slate-400">问题</span>
                      <p className="text-xs text-white font-medium mt-1">{selectedProblem.description}</p>
                    </div>

                    {/* 检测依据：为什么判定为问题 */}
                    <div className="p-3 rounded-xl bg-amber-500/5 border border-amber-500/15 space-y-2.5">
                      <div className="flex items-center gap-1.5">
                        <Info className="w-3.5 h-3.5 text-amber-400" />
                        <span className="text-[10px] font-semibold text-amber-400 uppercase tracking-wide">检测依据（为什么）</span>
                      </div>
                      <div>
                        <span className="text-[9px] text-slate-500">检测来源</span>
                        <p className="text-[11px] text-slate-200 mt-0.5 break-all">{selectedProblem.evidence_source}</p>
                      </div>
                      <div>
                        <span className="text-[9px] text-slate-500">读取到的内容</span>
                        <p className="text-[11px] font-mono text-slate-300 mt-0.5 whitespace-pre-wrap break-all p-2 bg-black/30 rounded-lg border border-white/5">
                          {selectedProblem.evidence_content || selectedProblem.detail}
                        </p>
                      </div>
                      <div>
                        <span className="text-[9px] text-slate-500">判定逻辑</span>
                        <p className="text-[11px] text-slate-300 mt-0.5 leading-relaxed">{selectedProblem.evidence_reason}</p>
                      </div>
                    </div>

                    {/* 修复方案：将要怎么做、做什么 */}
                    <div className="p-3 rounded-xl bg-blue-500/5 border border-blue-500/15 space-y-2.5">
                      <div className="flex items-center gap-1.5">
                        <Zap className="w-3.5 h-3.5 text-blue-400" />
                        <span className="text-[10px] font-semibold text-blue-400 uppercase tracking-wide">修复方案（怎么做）</span>
                      </div>
                      <p className="text-[11px] text-slate-200 leading-relaxed">{selectedProblem.fix_plan}</p>

                      {selectedProblem.fix_file && (
                        <div>
                          <span className="text-[9px] text-slate-500">将修改的文件 / 位置</span>
                          <p className="text-[11px] font-mono text-slate-300 mt-0.5 break-all">{selectedProblem.fix_file}</p>
                        </div>
                      )}
                      {selectedProblem.fix_source_path && (
                        <div>
                          <span className="text-[9px] text-slate-500">源路径（从）</span>
                          <p className="text-[11px] font-mono text-slate-300 mt-0.5 break-all">{selectedProblem.fix_source_path}</p>
                        </div>
                      )}
                      {selectedProblem.fix_dest_path && (
                        <div>
                          <span className="text-[9px] text-slate-500">目标位置（到）</span>
                          <p className="text-[11px] font-mono text-emerald-300 mt-0.5 break-all">{selectedProblem.fix_dest_path}</p>
                        </div>
                      )}
                    </div>
                  </div>

                  <div className="pt-4 mt-2 border-t border-white/5">
                    <span className="text-[10px] text-slate-500">
                      修复类型: <span className="font-mono text-slate-400">{selectedProblem.fix_type}</span>
                    </span>
                  </div>
                </div>
              ) : (
                <div className="flex-1 flex items-center justify-center text-center text-slate-500 text-xs">
                  请选择左侧列表中的诊断项，查看「为什么」和「将如何修复」
                </div>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
