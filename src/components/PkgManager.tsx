import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  ArrowUpCircle,
  CheckCircle,
  RefreshCw,
  Terminal,
  Box,
  HelpCircle,
  TrendingUp,
  ExternalLink
} from "lucide-react";

interface PackageInfo {
  name: string;
  current_version: string;
  latest_version: string;
  status: string; // "latest" | "outdated"
  homepage: string;
}

export default function PkgManager() {
  const [activeSdk, setActiveSdk] = useState<"nodejs" | "python">("nodejs");
  const [packages, setPackages] = useState<PackageInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [upgradingName, setUpgradingName] = useState<string | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const fetchPackages = async (sdk: "nodejs" | "python") => {
    setLoading(true);
    setErrorMsg(null);
    try {
      const list = await invoke<PackageInfo[]>("get_global_packages", { sdkName: sdk });
      setPackages(list);
    } catch (e: any) {
      setErrorMsg(e);
      setPackages([]);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchPackages(activeSdk);
  }, [activeSdk]);

  const handleUpgrade = async (pkgName: string) => {
    setUpgradingName(pkgName);
    setErrorMsg(null);
    try {
      await invoke("upgrade_global_package", { sdkName: activeSdk, pkgName });
      alert(`包 ${pkgName} 升级成功！`);
      await fetchPackages(activeSdk);
    } catch (e: any) {
      setErrorMsg(`升级 ${pkgName} 失败: ${e}`);
    } finally {
      setUpgradingName(null);
    }
  };

  const getStatusBadge = (pkg: PackageInfo) => {
    if (pkg.status === "outdated") {
      return (
        <span className="px-2 py-0.5 rounded-md text-[10px] font-semibold bg-amber-500/10 text-amber-400 border border-amber-500/20 flex items-center gap-0.5 w-max">
          可升级
        </span>
      );
    }
    return (
      <span className="px-2 py-0.5 rounded-md text-[10px] font-semibold bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 flex items-center gap-0.5 w-max">
        已最新
      </span>
    );
  };

  return (
    <div className="flex-1 p-8 overflow-y-auto space-y-6 h-screen select-none flex flex-col">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-semibold text-white tracking-wide">全局包管理</h2>
          <p className="text-xs text-slate-400 mt-1">列出并升级当前已安装的全局 NPM 包或 Pip 包依赖版本。点击包名即可在浏览器打开它的官网，查看文档与源代码。</p>
        </div>

        <div className="flex items-center gap-2">
          {/* Toggle SDK */}
          <div className="flex bg-white/5 border border-white/5 rounded-xl p-0.5">
            <button
              onClick={() => setActiveSdk("nodejs")}
              className={`px-3.5 py-1.5 rounded-lg text-xs font-semibold flex items-center gap-1.5 transition-all cursor-pointer ${
                activeSdk === "nodejs" ? "bg-blue-600 text-white" : "text-slate-400 hover:text-slate-200"
              }`}
            >
              <Box className="w-3.5 h-3.5" />
              Node.js (NPM)
            </button>
            <button
              onClick={() => setActiveSdk("python")}
              className={`px-3.5 py-1.5 rounded-lg text-xs font-semibold flex items-center gap-1.5 transition-all cursor-pointer ${
                activeSdk === "python" ? "bg-blue-600 text-white" : "text-slate-400 hover:text-slate-200"
              }`}
            >
              <TrendingUp className="w-3.5 h-3.5" />
              Python (Pip)
            </button>
          </div>

          <button
            onClick={() => fetchPackages(activeSdk)}
            disabled={loading}
            className="flex items-center gap-2 px-3.5 py-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-xl text-xs border border-white/5 cursor-pointer transition-all"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${loading ? "animate-spin" : ""}`} />
            刷新
          </button>
        </div>
      </div>

      {errorMsg && (
        <div className="p-3.5 bg-red-500/10 border border-red-500/20 text-red-400 rounded-xl text-xs flex items-center gap-1.5 font-medium">
          <Terminal className="w-4 h-4 text-red-400" />
          {errorMsg}
        </div>
      )}

      {/* Packages Table */}
      <div className="flex-1 min-h-0 glass-panel border border-white/5 rounded-2xl overflow-hidden flex flex-col h-[480px]">
        <div className="flex-1 overflow-y-auto">
          <table className="w-full text-left border-collapse text-xs">
            <thead>
              <tr className="bg-white/3 border-b border-white/5 text-slate-400 font-semibold">
                <th className="p-4">依赖包名称</th>
                <th className="p-4 w-32">当前安装版本</th>
                <th className="p-4 w-32">最新可用版本</th>
                <th className="p-4 w-28">更新状态</th>
                <th className="p-4 w-28 text-center">操作</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-white/5">
              {loading ? (
                <tr>
                  <td colSpan={5} className="p-12 text-center text-slate-500 font-medium">
                    <RefreshCw className="w-6 h-6 animate-spin text-blue-400 mx-auto mb-3" />
                    正在扫描全局依赖包列表...
                  </td>
                </tr>
              ) : packages.length === 0 ? (
                <tr>
                  <td colSpan={5} className="p-12 text-center text-slate-500">
                    无全局包依赖或环境未就绪
                  </td>
                </tr>
              ) : (
                packages.map((pkg) => {
                  const isUpgrading = upgradingName === pkg.name;
                  return (
                    <tr 
                      key={pkg.name}
                      className="hover:bg-white/2 text-slate-300"
                    >
                      <td className="p-4 font-semibold text-slate-200">
                        <button
                          onClick={() => openUrl(pkg.homepage)}
                          title={`在浏览器打开官网: ${pkg.homepage}`}
                          className="inline-flex items-center gap-1.5 hover:text-blue-400 transition-colors cursor-pointer group"
                        >
                          {pkg.name}
                          <ExternalLink className="w-3 h-3 text-slate-500 group-hover:text-blue-400 opacity-0 group-hover:opacity-100 transition-all" />
                        </button>
                      </td>
                      <td className="p-4 font-mono">{pkg.current_version}</td>
                      <td className="p-4 font-mono text-slate-400">{pkg.latest_version}</td>
                      <td className="p-4">{getStatusBadge(pkg)}</td>
                      <td className="p-4 text-center">
                        {pkg.status === "outdated" ? (
                          <button
                            onClick={() => handleUpgrade(pkg.name)}
                            disabled={isUpgrading}
                            className="px-3 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-lg text-[10px] font-semibold cursor-pointer transition-all flex items-center justify-center gap-1 mx-auto"
                          >
                            <ArrowUpCircle className="w-3.5 h-3.5" />
                            {isUpgrading ? "升级中..." : "升级包"}
                          </button>
                        ) : (
                          <span className="text-[10px] text-slate-600">无需更新</span>
                        )}
                      </td>
                    </tr>
                  );
                })
              )}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  );
}
