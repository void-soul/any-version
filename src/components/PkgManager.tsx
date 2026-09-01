import { useState, useEffect, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  ArrowUpCircle,
  RefreshCw,
  Terminal,
  Box,
  TrendingUp,
  ExternalLink,
  Rocket,
} from "lucide-react";

interface PackageInfo {
  name: string;
  current_version: string;
  latest_version: string;
  status: string; // "latest" | "outdated"
  homepage: string;
}

const SDK_OPTIONS: { id: string; label: string; icon: React.ReactNode }[] = [
  { id: "nodejs", label: "Node.js (NPM)", icon: <Box className="w-3.5 h-3.5" /> },
  { id: "python", label: "Python (Pip)", icon: <TrendingUp className="w-3.5 h-3.5" /> },
];

export default function PkgManager() {
  const { t } = useTranslation();
  const [activeSdk, setActiveSdk] = useState<"nodejs" | "python">("nodejs");

  // Package state
  const [packages, setPackages] = useState<PackageInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [upgradingName, setUpgradingName] = useState<string | null>(null);
  const [upgradingAll, setUpgradingAll] = useState(false);

  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const outdatedCount = useMemo(() => packages.filter(p => p.status === "outdated").length, [packages]);

  // ─── Package functions ───────────────────────────────

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
      alert(t("pkgmgr2.upgraded", { name: pkgName }));
      await fetchPackages(activeSdk);
    } catch (e: any) {
      setErrorMsg(t("pkgmgr2.upgradeFail", { name: pkgName, err: String(e) }));
    } finally {
      setUpgradingName(null);
    }
  };

  const handleUpgradeAll = async () => {
    if (!confirm(t("pkgmgr2.upgradeAllConfirm", { count: outdatedCount }))) return;
    setUpgradingAll(true);
    setErrorMsg(null);
    try {
      const results = await invoke<Array<{ name: string; success: boolean; error: string | null }>>(
        "upgrade_all_global_packages",
        { sdkName: activeSdk }
      );
      const failed = results.filter(r => !r.success);
      if (failed.length > 0) {
        setErrorMsg(t("pkgmgr2.partialFail", { names: failed.map(f => `${f.name}(${f.error})`).join("、") }));
      }
      await fetchPackages(activeSdk);
    } catch (e: any) {
      setErrorMsg(t("pkgmgr2.batchFail", { err: String(e) }));
    } finally {
      setUpgradingAll(false);
    }
  };

  const getStatusBadge = (pkg: PackageInfo) => {
    if (pkg.status === "outdated") {
      return (
        <span className="px-2 py-0.5 rounded-md text-[10px] font-semibold bg-amber-500/10 text-amber-400 border border-amber-500/20 flex items-center gap-0.5 w-max">
          {t("pkgmgr2.upgradable")}
        </span>
      );
    }
    return (
      <span className="px-2 py-0.5 rounded-md text-[10px] font-semibold bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 flex items-center gap-0.5 w-max">
        {t("pkgmgr2.latest")}
      </span>
    );
  };

  return (
    <div className="flex-1 p-8 overflow-y-auto space-y-6 h-screen select-none flex flex-col">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-semibold text-white tracking-wide">{t("pkgmgr2.title")}</h2>
          <p className="text-xs text-slate-400 mt-1">
            {t("pkgmgr2.subtitle")}
          </p>
        </div>

        <div className="flex items-center gap-2">
          {/* SDK Toggle */}
          <div className="flex bg-white/5 border border-white/5 rounded-xl p-0.5">
            {SDK_OPTIONS.map(opt => (
              <button
                key={opt.id}
                onClick={() => setActiveSdk(opt.id as "nodejs" | "python")}
                className={`px-3.5 py-1.5 rounded-lg text-xs font-semibold flex items-center gap-1.5 transition-all cursor-pointer ${
                  activeSdk === opt.id
                    ? "bg-blue-600 text-white"
                    : "text-slate-400 hover:text-slate-200"
                }`}
              >
                {opt.icon}
                {opt.label}
              </button>
            ))}
          </div>

          {outdatedCount > 0 && (
            <button
              onClick={handleUpgradeAll}
              disabled={upgradingAll || loading}
              className="flex items-center gap-2 px-3.5 py-2 bg-amber-600 hover:bg-amber-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold cursor-pointer transition-all shadow-lg shadow-amber-500/10"
            >
              <Rocket className={`w-3.5 h-3.5 ${upgradingAll ? "animate-pulse" : ""}`} />
              {upgradingAll ? t("pkgmgr2.upgradingAll") : t("pkgmgr2.upgradeAll", { count: outdatedCount })}
            </button>
          )}

          <button
            onClick={() => fetchPackages(activeSdk)}
            disabled={loading}
            className="flex items-center gap-2 px-3.5 py-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-xl text-xs border border-white/5 cursor-pointer transition-all"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${loading ? "animate-spin" : ""}`} />
            {t("pkgmgr2.refresh")}
          </button>
        </div>
      </div>

      {/* Error Message */}
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
                <th className="p-4">{t("pkgmgr2.colPkg")}</th>
                <th className="p-4 w-32">{t("pkgmgr2.colCurrent")}</th>
                <th className="p-4 w-32">{t("pkgmgr2.colLatest")}</th>
                <th className="p-4 w-28">{t("pkgmgr2.colStatus")}</th>
                <th className="p-4 w-28 text-center">{t("pkgmgr2.colOps")}</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-white/5">
              {loading ? (
                <tr>
                  <td colSpan={5} className="p-12 text-center text-slate-500 font-medium">
                    <RefreshCw className="w-6 h-6 animate-spin text-blue-400 mx-auto mb-3" />
                    {t("pkgmgr2.scanning")}
                  </td>
                </tr>
              ) : packages.length === 0 ? (
                <tr>
                  <td colSpan={5} className="p-12 text-center text-slate-500">
                    {t("pkgmgr2.noPkgs")}
                  </td>
                </tr>
              ) : (
                packages.map(pkg => {
                  const isUpgrading = upgradingName === pkg.name;
                  return (
                    <tr key={pkg.name} className="hover:bg-white/2 text-slate-300">
                      <td className="p-4 font-semibold text-slate-200">
                        <button
                          onClick={() => openUrl(pkg.homepage)}
                          title={t("pkgmgr2.openHome", { name: pkg.homepage })}
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
                            {isUpgrading ? t("pkgmgr2.upgrading") : t("pkgmgr2.upgradeBtn")}
                          </button>
                        ) : (
                          <span className="text-[10px] text-slate-600">{t("pkgmgr2.noUpdate")}</span>
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
