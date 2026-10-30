import { PackageDataStorageCard } from "./PackageDataStorageCard";
import { PackageCacheStorageCard } from "./PackageCacheStorageCard";
import { usePackageStorageWorkflow } from "./usePackageStorageWorkflow";
import { useState, useEffect } from "react";
import { theamedAlert, alertError } from "../../shared/ThemedAlert";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ExternalLink, CheckCircle, AlertTriangle, RefreshCw, Download, Globe, HardDrive, FolderSync, Package, Loader, Wifi, WifiOff, Info } from "lucide-react";
import type { ProjectStatus, ProjectDef, PackageManagerDef } from "../types";
import { ConfirmDialogHost } from "../../shared/ConfirmDialog";
import type { ConfirmRequest } from "../../shared/ConfirmDialog";
import { pmDetectionCache } from "./PackageManagerTab.model";

export function PackageManagerTab({
  projectId,
  pm,
  hidden,
  installRoot,
  projectDef,
  projectStatus
}: {
  projectId: string;
  pm: PackageManagerDef;
  hidden?: boolean;
  installRoot?: string | null;
  installSource?: string | null;
  projectDef?: ProjectDef | null;
  projectStatus?: ProjectStatus | null;
}) {
  const { t } = useTranslation();
  const cachedKey = `${projectId}:${pm.id}`;
  const cachedData = pmDetectionCache[cachedKey];
  const isCached = cachedData && (Date.now() - cachedData.timestamp < 5 * 60 * 1000);

  const [checking, setChecking] = useState(false);
  const [detectStep, setDetectStep] = useState("");
  const [installed, setInstalled] = useState(isCached ? cachedData.installed : false);
  const [version, setVersion] = useState<string | null>(isCached ? cachedData.version : null);
  const [installing, setInstalling] = useState(false);
  const [installProgress, setInstallProgress] = useState(false);
  const [upgrading, setUpgrading] = useState(false);
  const [latestVersion, setLatestVersion] = useState<string | null>(isCached ? cachedData.latestVersion : null);

  // Git repo states (通用 is_git_repo 驱动)
  const [gitRepoStatus, setGitRepoStatus] = useState<any>(isCached ? cachedData.gitRepoStatus : null);
  const [_checkingGitRepo, setCheckingGitRepo] = useState(false);
  const [bootstrapping, setBootstrapping] = useState(false);
  const [updatingGitRepo, setUpdatingGitRepo] = useState(false);

  // 缓存 & 数据存储管理
  type ParentLink = { parent_path: string; parent_target: string; child_rel: string };
  const [cacheInfo, setCacheInfo] = useState<any>(isCached ? cachedData.cacheInfo : null);
  const [dataInfo, setDataInfo] = useState<any>(isCached ? cachedData.dataInfo : null);
  // 附加缓存目录（pnpm.cache 等）：key = extra.id，value = 缓存信息
  const [extraCacheInfos, setExtraCacheInfos] = useState<Record<string, any>>(isCached ? cachedData.extraCacheInfos ?? {} : {});
  // 当前工作流针对的附加缓存 id（null = 主缓存 store）
  const {
    workflowExtraCacheId,
    workflowType,
    openWorkflow,
    renderWorkflow,
  } = usePackageStorageWorkflow({
    extraCacheInfos,
    cacheInfo,
    pm,
    dataInfo,
    projectId,
    runDetection: () => runDetection(),
    setConfirmRequest: request => setConfirmRequest(request),
  });
  // 统一确认弹窗请求（替代原生 window.confirm）
  const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null);

  const updatePmCache = (data: Partial<typeof pmDetectionCache[string]>) => {
    const key = `${projectId}:${pm.id}`;
    if (!pmDetectionCache[key]) {
      pmDetectionCache[key] = {
        installed: false,
        version: null,
        latestVersion: null,
        cacheInfo: null,
        dataInfo: null,
        extraCacheInfos: {},
        proxyDetected: null,
        proxyInput: "",
        currentMirror: null,
        packages: [],
        gitRepoStatus: null,
        timestamp: 0
      };
    }
    pmDetectionCache[key] = {
      ...pmDetectionCache[key],
      ...data,
      timestamp: Date.now()
    };
  };

  // 清理
  const [cleaningCache, setCleaningCache] = useState(false);
  const [cleanProgress, setCleanProgress] = useState<{ stage: string; current: number; total: number; file_name: string } | null>(null);

  // 镜像
  const [switchingMirror, setSwitchingMirror] = useState<string | null>(null);
  const [currentMirror, setCurrentMirror] = useState<string | null>(isCached ? cachedData.currentMirror : null);

  // 代理
  const [proxyDetected, setProxyDetected] = useState<string | null>(isCached ? cachedData.proxyDetected : null);
  const [proxyInput, setProxyInput] = useState(isCached ? cachedData.proxyInput : "");
  const [settingProxy, setSettingProxy] = useState(false);

  // 全局包
  const [packages, setPackages] = useState<Array<{ name: string; current_version: string; latest_version: string; status: string; homepage: string }>>(isCached ? cachedData.packages : []);
  const [loadingPackages, setLoadingPackages] = useState(false);
  const [upgradingPkg, setUpgradingPkg] = useState<string | null>(null);

  // 首次检测
  const [hasChecked, setHasChecked] = useState(isCached);

  const runDetection = async () => {
    setChecking(true);
    const steps: Array<{ label: string; run: () => Promise<void> }> = [];
    setDetectStep(t("pkgmgr.detectStep", { name: pm.display_name }));

    const cachedData: any = {
      installed: false,
      version: null,
      latestVersion: null,
      cacheInfo: null,
      dataInfo: null,
      proxyDetected: null,
      proxyInput: "",
      currentMirror: null,
      gitRepoStatus: null
    };

    // Step 0: git repo (is_git_repo) bootstrap & update status
    if (projectDef?.is_git_repo && installRoot) {
      const exeName = (projectDef as any).version_exe ?? pm.id;
      const bootstrapCmd = projectDef.bootstrap_cmd ?? null;
      steps.push({
        label: t("pkgmgr.detectInit", { name: pm.display_name }),
        run: async () => {
          try {
            setCheckingGitRepo(true);
            const status = await invoke<any>("check_git_repo_status", { path: installRoot, exeName, bootstrapCmd });
            setGitRepoStatus(status);
            cachedData.gitRepoStatus = status;

            // 自动初始化逻辑：符合 git 仓库且没有可执行文件，则自动执行初始化脚本
            if (status.is_git && !status.has_exe && !bootstrapping && bootstrapCmd) {
              setBootstrapping(true);
              setDetectStep(t("pkgmgr.autoInit", { name: pm.display_name }));
              try {
                await invoke("bootstrap_git_repo", { path: installRoot, cmd: bootstrapCmd });
                // 初始化成功后，重新获取状态
                const newStatus = await invoke<any>("check_git_repo_status", { path: installRoot, exeName, bootstrapCmd });
                setGitRepoStatus(newStatus);
                cachedData.gitRepoStatus = newStatus;
              } catch (err) {
                alertError(t("pkgmgr.autoInitFail", { name: pm.display_name, err: String(err) }));
              } finally {
                setBootstrapping(false);
              }
            }
          } catch (e) {
            console.error(e);
          } finally {
            setCheckingGitRepo(false);
          }
        }
      });
    }

    // Step 1: version
    steps.push({
      label: t("pkgmgr.detectVersion", { name: pm.display_name }),
      run: async () => {
        if (!pm.version_cmd) return;
        try {
          const out = await invoke<string>("run_cmd_capture", { cmd: pm.version_cmd, projectId });
          setInstalled(true);
          setVersion(out.trim());
          cachedData.installed = true;
          cachedData.version = out.trim();
        } catch {
          // 「工具自身」的内置条目（built_in 且 id 与项目 id 相同，如 go 的 Go Modules）：
          // 它的 version_exe 就是工具本身，命令能否跑通取决于 PATH 注入 / 安装位置，
          // 未必在本进程里解析得到。这时回退到应用自己的安装记录（与标题栏、版本管理页同源），
          // 否则会出现「版本管理页说装了、包管理页却说未安装」的矛盾。
          const fallbackVersion = projectStatus?.active_version ?? null;
          if (pm.built_in && pm.id === projectDef?.id && projectStatus?.installed) {
            setInstalled(true);
            setVersion(fallbackVersion);
            cachedData.installed = true;
            cachedData.version = fallbackVersion;
          } else {
            setInstalled(false);
            setVersion(null);
            cachedData.installed = false;
            cachedData.version = null;
          }
        }
      },
    });

    // 说明：此前这里有一段「检测包管理器自身最新版本」的步骤，读取的是
    // `pm.latest_version_cmd` —— 该字段在 Rust 的 PackageManagerDef 与所有
    // projects/*/package_managers.json 中都不存在（配置里用的是
    // pkg_upgrade_cmd_template / pkg_outdated_cmd），因此该分支永远不成立、
    // latestVersion 永远为 null（对应的「升级到 vX」按钮也就从未出现过）。
    // 依赖包的最新版本由全局依赖包列表（get_global_packages 的 latest_version）
    // 展示与升级，这里不再保留死分支。

    // Step 2: cache
    if (pm.cache_detect_cmd || pm.cache_default_path || pm.cache_env_var) {
      steps.push({
        label: t("pkgmgr.detectCache", { name: pm.display_name }),
        run: async () => {
          try {
            const info = await invoke<{ path: string; size: string; is_link: boolean; real_target: string; parent_link: ParentLink | null }>("get_pkg_cache_info", {
              projectId,
              pmId: pm.id,
              storageKind: "cache"
            });
            const newInfo = { ...info, detect_source: pm.cache_detect_cmd || pm.cache_env_var || pm.cache_default_path || "" };
            setCacheInfo(newInfo);
            cachedData.cacheInfo = newInfo;
          } catch { /* ignore */ }
        },
      });
    }

    // Step 2-extra: 附加缓存目录（pnpm.cache 等，与主缓存 store 平行可调整）
    if (pm.extra_caches && pm.extra_caches.length > 0) {
      steps.push({
        label: t("pkgmgr.detectExtraCache", { name: pm.display_name }),
        run: async () => {
          const extraInfos: Record<string, any> = {};
          for (const extra of pm.extra_caches!) {
            try {
              const info = await invoke<{ path: string; size: string; is_link: boolean; real_target: string; parent_link: ParentLink | null }>("get_pkg_cache_info", {
                projectId,
                pmId: pm.id,
                storageKind: "cache",
                extraCacheId: extra.id,
              });
              extraInfos[extra.id] = { ...info, display_name: extra.display_name, detect_source: extra.detect_cmd || extra.default_path || "" };
            } catch { /* 忽略未检测到的附加缓存 */ }
          }
          setExtraCacheInfos(extraInfos);
          cachedData.extraCacheInfos = extraInfos;
        },
      });
    }

    // Step 2b: data
    if (pm.data_detect_cmd || pm.data_default_path || pm.data_env_var) {
      steps.push({
        label: t("pkgmgr.detectData", { name: pm.display_name }),
        run: async () => {
          try {
            const info = await invoke<{ path: string; size: string; is_link: boolean; real_target: string; parent_link: ParentLink | null }>("get_pkg_cache_info", {
              projectId,
              pmId: pm.id,
              storageKind: "data"
            });
            const newInfo = { path: info.path, size: info.size, is_link: info.is_link, real_target: info.real_target, detect_source: pm.data_detect_cmd || pm.data_env_var || pm.data_default_path || "" };
            setDataInfo(newInfo);
            cachedData.dataInfo = newInfo;
          } catch { /* ignore */ }
        },
      });
    }

    // Step 3: proxy
    if (pm.proxy_detect_cmd) {
      steps.push({
        label: t("pkgmgr.detectProxy", { name: pm.display_name }),
        run: async () => {
          try {
            const out = await invoke<string>("run_cmd_capture", { cmd: pm.proxy_detect_cmd, projectId });
            const v = out.trim();
            if (v && v !== "null" && v !== "undefined") {
              setProxyDetected(v);
              setProxyInput(v);
              cachedData.proxyDetected = v;
              cachedData.proxyInput = v;
            }
          } catch { /* ignore */ }
        },
      });
    }
    // Step 4: current mirror
    if (pm.mirror_cmd_template || (pm.mirror_options && pm.mirror_options.length > 0)) {
      steps.push({
        label: t("pkgmgr.detectMirror", { name: pm.display_name }),
        run: async () => {
          try {
            if (pm.mirror_detect_cmd || pm.mirror_cmd_template) {
              const getCmd = pm.mirror_detect_cmd ?? pm.mirror_cmd_template!.replace("set ", "get ").replace("{url}", "");
              const out = await invoke<string>("run_cmd_capture", { cmd: getCmd, projectId });
              const v = out.trim();
              if (v && v !== "null" && v !== "undefined") {
                setCurrentMirror(v);
                cachedData.currentMirror = v;
              } else {
                setCurrentMirror(null);
                cachedData.currentMirror = null;
              }
            } else {
              const list = await invoke<Array<{ tool: string; current: string; mirror_name: string }>>("get_mirrors_list");
              const entry = list.find(m => m.tool.toLowerCase() === pm.id.toLowerCase() || (pm.id === "cargo" && m.tool === "rust"));
              if (entry) {
                setCurrentMirror(entry.current);
                cachedData.currentMirror = entry.current;
              }
            }
          } catch { /* ignore */ }
        },
      });
    }

    // 执行所有步骤
    for (let i = 0; i < steps.length; i++) {
      setDetectStep(steps[i].label);
      await steps[i].run();
    }

    setDetectStep("");
    setChecking(false);
    setHasChecked(true);

    // 将本次扫描完成的数据整体写入全局缓存 map
    updatePmCache({
      installed: cachedData.installed,
      version: cachedData.version,
      latestVersion: cachedData.latestVersion,
      cacheInfo: cachedData.cacheInfo,
      dataInfo: cachedData.dataInfo,
      extraCacheInfos: cachedData.extraCacheInfos,
      proxyDetected: cachedData.proxyDetected,
      proxyInput: cachedData.proxyInput,
      currentMirror: cachedData.currentMirror,
      gitRepoStatus: cachedData.gitRepoStatus
    });
  };

  // 统一的初始化、缓存读取与懒加载检测逻辑（合并以消除 React 异步状态竞态条件）
  useEffect(() => {
    const key = `${projectId}:${pm.id}`;
    const cached = pmDetectionCache[key];

    // 缓存有效性：5 分钟内视为新鲜；切换回本 tab 时若超过 30 秒也强制重检，
    // 避免长时间停留其他页面后看到陈旧的大小/路径/版本
    const CACHE_TTL_MS = 5 * 60 * 1000;
    const VISIBLE_REFRESH_MS = 30 * 1000;
    const cacheFresh = cached && Date.now() - cached.timestamp < CACHE_TTL_MS;
    const staleForTab = !hidden && cached && Date.now() - cached.timestamp > VISIBLE_REFRESH_MS;
    const useCache = cacheFresh && !staleForTab;

    if (useCache) {
      setInstalled(cached.installed);
      setVersion(cached.version);
      setLatestVersion(cached.latestVersion);
      setCacheInfo(cached.cacheInfo);
      setDataInfo(cached.dataInfo);
      setExtraCacheInfos(cached.extraCacheInfos ?? {});
      setProxyDetected(cached.proxyDetected);
      setProxyInput(cached.proxyInput);
      setCurrentMirror(cached.currentMirror);
      setPackages(cached.packages);
      setGitRepoStatus(cached.gitRepoStatus);

      setHasChecked(true);
      setChecking(false);
    } else {
      // 缓存过期或不存在，清空状态并按需触发检测
      setInstalled(false);
      setVersion(null);
      setLatestVersion(null);
      setCacheInfo(null);
      setDataInfo(null);
      setExtraCacheInfos({});
      setProxyDetected(null);
      setProxyInput("");
      setCurrentMirror(null);
      setPackages([]);
      setGitRepoStatus(null);

      setHasChecked(false);
      setChecking(false);

      if (!hidden) {
        runDetection();
      }
    }
  }, [projectId, pm.id, hidden]);

  // 安装
  const handleInstall = async () => {
    if (!pm.install_cmd) return;
    setInstalling(true);
    setInstallProgress(true);
    try {
      await invoke("run_cmd_capture", { cmd: pm.install_cmd, projectId });
      await runDetection();
    } catch (e: unknown) {
      alertError(t("pkgmgr.installFail", { name: pm.display_name, err: String(e) }));
    } finally {
      setInstalling(false);
      setInstallProgress(false);
    }
  };

  // 升级包管理器
  const handleUpgrade = async () => {
    if (!pm.install_cmd) return;
    setUpgrading(true);
    setInstallProgress(true);
    try {
      await invoke("run_cmd_capture", { cmd: pm.install_cmd, projectId });
      await runDetection();
    } catch (e: unknown) {
      alertError(t("pkgmgr.upgradeFail", { name: pm.display_name, err: String(e) }));
    } finally {
      setUpgrading(false);
      setInstallProgress(false);
    }
  };

  // 简单版本比较：返回 true 表示 a > b
  const versionGt = (a: string, b: string): boolean => {
    const pa = a.replace(/^v/, "").split(".").map(Number);
    const pb = b.replace(/^v/, "").split(".").map(Number);
    for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
      const va = pa[i] || 0;
      const vb = pb[i] || 0;
      if (va > vb) return true;
      if (va < vb) return false;
    }
    return false;
  };

  // 切换镜像
  const handleSwitchMirror = async (url: string, mirrorType: string) => {
    setSwitchingMirror(url);
    try {
      if (pm.mirror_cmd_template) {
        const cmd = pm.mirror_cmd_template.replace("{url}", url);
        await invoke("run_cmd_capture", { cmd, projectId });
      } else {
        await invoke("set_mirror", { tool: pm.id, mirrorType });
      }
      setCurrentMirror(url || null);
      updatePmCache({ currentMirror: url || null });
    } catch (e: unknown) {
      alertError(t("pkgmgr.switchMirrorFail", { err: String(e) }));
    } finally {
      setSwitchingMirror(null);
    }
  };

  // ── 清理缓存（带进度条） ──
  // 真正清理（由确认弹窗确认后调用）
  const doCleanCache = async () => {
    setCleaningCache(true);
    setCleanProgress(null);
    const unlisten = await listen<{ stage: string; current: number; total: number; file_name: string }>("clean-cache-progress", (event) => {
      setCleanProgress(event.payload);
    });
    try {
      await invoke("clean_pkg_cache", {
        projectId,
        pmId: pm.id,
        cachePath: cacheInfo?.path || null,
      });
      await runDetection();
    } catch (e: unknown) {
      alertError(t("pkgmgr.cleanCacheFail", { err: String(e) }));
    } finally {
      unlisten();
      setCleaningCache(false);
      setCleanProgress(null);
    }
  };

  /** 清理缓存入口：先弹统一确认弹窗（删除类操作，danger），确认后执行 */
  const handleCleanCache = async () => {
    if (!pm.cache_detect_cmd && !pm.cache_default_path && !pm.cache_env_var) return;
    setConfirmRequest({
      title: t("pkgmgr.cleanCache"),
      danger: true,
      confirmText: t("pkgmgr.cleanCache"),
      desc: t("pkgmgr.cleanCacheConfirm", { size: cacheInfo?.size || "?" }),
      onConfirm: () => {
        void doCleanCache();
      },
    });
  };

  // 设置代理
  const handleSetProxy = async () => {
    if (!pm.proxy_set_cmd_template) return;
    // 防注入：代理值会拼入 PowerShell/命令行模板执行，拒绝 shell 元字符
    const v = proxyInput.trim();
    if (v && /['";&|<>`$()\r\n]/.test(v)) {
      theamedAlert(t("pkgmgr.proxyInvalidChars"));
      return;
    }
    setSettingProxy(true);
    try {
      if (proxyInput.trim()) {
        const cmd = pm.proxy_set_cmd_template.replace("{url}", proxyInput.trim());
        await invoke("run_cmd_capture", { cmd, projectId });
      } else {
        if ((pm as any).proxy_clear_cmd) {
          await invoke("run_cmd_capture", { cmd: (pm as any).proxy_clear_cmd, projectId });
        } else {
          const cmd = pm.proxy_set_cmd_template.replace("{url}", "");
          await invoke("run_cmd_capture", { cmd, projectId });
        }
      }
      setProxyDetected(proxyInput.trim() || null);
      updatePmCache({ proxyDetected: proxyInput.trim() || null, proxyInput: proxyInput.trim() });
    } catch (e: unknown) {
      alertError(t("pkgmgr.setProxyFail", { err: String(e) }));
    } finally {
      setSettingProxy(false);
    }
  };

  // 全局包
  const loadPackages = async () => {
    if (!pm.pkg_list_cmd) return;
    setLoadingPackages(true);
    try {
      const list = await invoke<Array<{ name: string; current_version: string; latest_version: string; status: string; homepage: string }>>("get_global_packages", { sdkName: pm.id });
      setPackages(list);
      updatePmCache({ packages: list });
    } catch { /* ignore */ } finally {
      setLoadingPackages(false);
    }
  };

  useEffect(() => {
    if (hasChecked && installed && pm.pkg_list_cmd && packages.length === 0 && !loadingPackages) {
      loadPackages();
    }
  }, [hasChecked, installed]);

  const handleUpgradePackage = async (pkgName: string) => {
    if (!pm.id) return;
    setUpgradingPkg(pkgName);
    try {
      await invoke("upgrade_global_package", { sdkName: pm.id, pkgName });
      await loadPackages();
    } catch (e: unknown) {
      alertError(t("pkgmgr.upgradePkgFail", { name: pkgName, err: String(e) }));
    } finally {
      setUpgradingPkg(null);
    }
  };

  // 隐藏时跳过渲染，但保持组件挂载和状态（避免切换回此 tab 时重新检测）
  if (hidden) return null;

  return (
    <div className="space-y-5">
      {projectStatus && !projectStatus.managed && (
        <div className="flex items-start gap-2.5 p-3 rounded-card border border-[var(--module-accent-ring)] bg-[var(--module-accent-soft)] text-[12.5px] text-[var(--module-accent)] animate-fadeIn">
          <Info className="w-4.5 h-4.5 text-[var(--module-accent)] flex-shrink-0 mt-0.5" />
          <span>
            <strong>{t("pkgmgr.readonlyTitle")}</strong>{t("pkgmgr.readonlyDesc")}
          </span>
        </div>
      )}
      {/* 头部状态栏 */}
      <div className="glass-panel rounded-panel p-4 border border-white/5 bg-white/2">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className={`w-9 h-9 rounded-card flex items-center justify-center ${installed ? "bg-emerald-500/10" : "bg-slate-500/10"}`}>
              <Package className={`w-4.5 h-4.5 ${installed ? "text-emerald-400" : "text-slate-500"}`} />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h3 className="text-sm font-bold text-white">{pm.display_name}</h3>
                {pm.built_in && (
                  <span className="px-1.5 py-0.5 rounded text-caption bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] font-semibold">{t("pkgmgr.builtIn")}</span>
                )}
              </div>
              {installed ? (
                <span className="text-title text-emerald-400 font-mono">{version || t("pkgmgr.installed")}</span>
              ) : checking ? (
                <span className="text-title text-[var(--module-accent)] flex items-center gap-1"><Loader className="w-3 h-3 animate-spin" />{t("pkgmgr.checking")}</span>
              ) : (
                <span className="text-title text-slate-400">{t("pkgmgr.notInstalled")}</span>
              )}
            </div>
          </div>
          <div className="flex items-center gap-2">
            <button onClick={runDetection} disabled={checking || installing || upgrading} className="p-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-ctl border border-white/5 cursor-pointer transition-all" title={t("pkgmgr.refreshDetect")}>
              <RefreshCw className={`w-3.5 h-3.5 ${checking ? "animate-spin" : ""}`} />
            </button>
            {installed && !pm.built_in && pm.install_cmd && (
              <>
                {latestVersion && version && versionGt(latestVersion, version) ? (
                  <button onClick={handleUpgrade} disabled={!projectStatus?.managed || upgrading || installing} className="px-4 py-1.5 bg-amber-600 hover:bg-amber-500 disabled:opacity-50 text-white rounded-ctl text-caption font-semibold cursor-pointer transition-all flex items-center gap-1.5" title={!projectStatus?.managed ? t("pkgmgr.managedFirst") : ""}>
                    <Download className="w-3.5 h-3.5" />{upgrading ? t("pkgmgr.upgrading") : t("pkgmgr.upgradeTo", { version: latestVersion })}
                  </button>
                ) : latestVersion && version && !versionGt(latestVersion, version) ? (
                  <span className="text-caption text-emerald-400 px-2 py-1 rounded bg-emerald-500/10 border border-emerald-500/20 font-semibold">{t("pkgmgr.latest")}</span>
                ) : null}
              </>
            )}
            {!installed && pm.install_cmd && (
              <button onClick={handleInstall} disabled={!projectStatus?.managed || installing || upgrading} className="px-4 py-1.5 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-white rounded-ctl text-caption font-semibold cursor-pointer transition-all flex items-center gap-1.5" title={!projectStatus?.managed ? t("pkgmgr.managedFirst") : ""}>
                <Download className="w-3.5 h-3.5" />{installing ? t("pkgmgr.installing") : t("pkgmgr.install")}
              </button>
            )}
          </div>
        </div>
        {/* 安装/升级进度条 */}
        {installProgress && (
          <div className="mt-3 space-y-1.5 animate-fadeIn">
            <div className="flex items-center gap-2 text-title text-[var(--module-accent)]">
              <Loader className="w-3 h-3 animate-spin" />
              {upgrading ? t("pkgmgr.upgradeProgress", { name: pm.display_name }) : t("pkgmgr.installProgress", { name: pm.display_name })}
            </div>
            <div className="w-full h-1.5 bg-white/5 rounded-full overflow-hidden">
              <div className="h-full bg-[color-mix(in_srgb,var(--module-accent)_60%,transparent)] rounded-full animate-pulse" style={{ width: "100%" }} />
            </div>
          </div>
        )}
        {detectStep && (
          <div className="mt-3 flex items-center gap-2 text-title text-[var(--module-accent)]">
            <Loader className="w-3 h-3 animate-spin" />{detectStep}
          </div>
        )}
      </div>

      {/* Git Update Notification (通用 is_git_repo) */}
      {projectDef?.is_git_repo && gitRepoStatus?.has_update && (
        <div className="glass-panel rounded-panel p-4 border border-amber-500/15 bg-amber-500/5 flex items-center justify-between animate-fadeIn mb-4">
          <div className="flex items-center gap-2.5 flex-1 min-w-0">
            <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0" />
            <div className="min-w-0">
              <p className="text-body font-bold text-amber-300">{t("pkgmgr.gitNewUpdate", { name: pm.display_name })}</p>
              <p className="text-caption text-amber-400/80 mt-0.5 truncate">
                {t("pkgmgr.curVersionLabel")} <span className="font-mono">{gitRepoStatus.current_commit}</span> {t("pkgmgr.latestVersionLabel")} <span className="font-mono">{gitRepoStatus.latest_commit}</span>
              </p>
            </div>
          </div>
          <button
            onClick={async () => {
              if (!installRoot) return;
              setUpdatingGitRepo(true);
              setDetectStep(t("pkgmgr.gitPullStep", { name: pm.display_name }));
              try {
                await invoke("update_git_repo", { path: installRoot, bootstrapCmd: projectDef.bootstrap_cmd ?? null });
                await runDetection();
              } catch (e) {
                alertError(t("pkgmgr.gitUpdateFail", { name: pm.display_name, err: String(e) }));
              } finally {
                setUpdatingGitRepo(false);
                setDetectStep("");
              }
            }}
            disabled={updatingGitRepo}
            className="px-3 py-1.5 bg-amber-600 hover:bg-amber-500 disabled:opacity-50 text-white rounded-ctl text-tiny font-semibold cursor-pointer transition-all flex items-center gap-1 flex-shrink-0 ml-2"
          >
            {updatingGitRepo ? (
              <><Loader className="w-3 h-3 animate-spin" />{t("pkgmgr.updating")}</>
            ) : (
              t("pkgmgr.updateBtn", { name: pm.display_name })
            )}
          </button>
        </div>
      )}

      {/* Git 仓库尚未初始化提示 (通用 is_git_repo) */}
      {projectDef?.is_git_repo && gitRepoStatus?.is_git && !gitRepoStatus?.has_exe && !checking && (
        <div className="glass-panel rounded-panel p-6 border border-[var(--module-accent-ring)] bg-[color-mix(in_srgb,var(--module-accent)_5%,transparent)] text-center space-y-4 animate-fadeIn">
          <Package className="w-10 h-10 text-[var(--module-accent)] mx-auto opacity-70 animate-pulse" />
          <div>
            <p className="text-[var(--module-accent)] text-sm font-semibold">{t("pkgmgr.notInitTitle", { name: pm.display_name })}</p>
            <p className="text-body text-[color-mix(in_srgb,var(--module-accent)_80%,transparent)] mt-1 max-w-md mx-auto leading-relaxed">
              {t("pkgmgr.notInitDesc", { name: pm.display_name })}
            </p>
          </div>
          <button
            onClick={async () => {
              if (!installRoot || !projectDef.bootstrap_cmd) return;
              setBootstrapping(true);
              setDetectStep(t("pkgmgr.initStep", { name: pm.display_name }));
              try {
                await invoke("bootstrap_git_repo", { path: installRoot, cmd: projectDef.bootstrap_cmd });
                await runDetection();
              } catch (e) {
                alertError(t("pkgmgr.initFail", { name: pm.display_name, err: String(e) }));
              } finally {
                setBootstrapping(false);
                setDetectStep("");
              }
            }}
            disabled={bootstrapping}
            className="px-5 py-2 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-white rounded-card text-body font-semibold cursor-pointer transition-all inline-flex items-center gap-1.5"
          >
            {bootstrapping ? (
              <><Loader className="w-3.5 h-3.5 animate-spin" />{t("pkgmgr.compilingInit")}</>
            ) : (
              t("pkgmgr.compileInitBtn")
            )}
          </button>
        </div>
      )}

      {/* 未安装提示 */}
      {hasChecked && !installed && !checking && !(projectDef?.is_git_repo && gitRepoStatus?.is_git && !gitRepoStatus?.has_exe) && (
        <div className="glass-panel rounded-panel p-6 border border-white/5 bg-white/2 text-center animate-fadeIn">
          <Package className="w-10 h-10 text-slate-500 mx-auto mb-3 opacity-50" />
          <p className="text-slate-400 text-sm font-semibold">{t("pkgmgr.notInstalledTitle", { name: pm.display_name })}</p>
          <p className="text-title text-slate-500 mt-1">{t("pkgmgr.notInstalledDesc")}</p>
        </div>
      )}

      {/* 缓存管理 */}
      {/* 缓存管理 — 展示所有可配置缓存路径（主缓存 store + 附加缓存），即使无法直接配置也可用 junction */}
      {hasChecked && installed && (cacheInfo || pm.cache_default_path || pm.cache_detect_cmd || (pm.extra_caches && pm.extra_caches.length > 0)) && (
        <div className="glass-panel rounded-panel p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center gap-2">
            <HardDrive className="w-4 h-4 text-amber-400" />
            <h4 className="text-body font-semibold text-white">{t("pkgmgr.cacheTitle")}</h4>
            <span className="text-caption px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20">{t("pkgmgr.cacheTag")}</span>
          </div>

          {/* 主缓存（pnpm 的 store 等） */}
          {cacheInfo ? (
            <PackageCacheStorageCard
              cacheInfo={cacheInfo}
              openWorkflow={openWorkflow}
              projectStatus={projectStatus}
              workflowType={workflowType}
              handleCleanCache={handleCleanCache}
              cleaningCache={cleaningCache}
              cleanProgress={cleanProgress}
              workflowExtraCacheId={workflowExtraCacheId}
              renderWorkflow={renderWorkflow}
            />
          ) : (pm.cache_default_path || pm.cache_detect_cmd || pm.cache_env_var) ? (
            <p className="text-title text-slate-500">{t("pkgmgr.cacheNotDetected")} <span className="font-mono text-slate-400">{pm.cache_default_path || t("pkgmgr.notConfigured")}</span></p>
          ) : null}

          {/* 附加缓存目录（pnpm.cache 等），与主缓存平行独立展示 */}
          {pm.extra_caches && pm.extra_caches.length > 0 && (
            <div className="space-y-3">
              {pm.extra_caches.map(extra => {
                const eInfo = extraCacheInfos[extra.id];
                return (
                  <div key={extra.id} className="p-4 bg-black/20 rounded-card border border-white/5 space-y-2">
                    <div className="flex items-start justify-between gap-2">
                      <div className="space-y-1 flex-1 min-w-0">
                        <div className="flex items-center gap-2 flex-wrap">
                          <span className="text-body font-semibold text-slate-200">{extra.display_name}</span>
                          {eInfo?.real_target ? (
                            <span className="px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 text-tiny inline-flex items-center font-semibold">
                              {t("pkgmgr.migratedJunction")}
                            </span>
                          ) : (
                            <span className="px-1.5 py-0.5 rounded bg-slate-500/10 text-slate-400 border border-slate-500/20 text-tiny inline-flex items-center">
                              {t("pkgmgr.defaultPath")}
                            </span>
                          )}
                        </div>
                        <p className="font-mono text-body text-slate-400 break-all">{eInfo?.path || extra.default_path || t("pkgmgr.pathNotDetected")}</p>
                        {eInfo?.real_target && (
                          <p className="font-mono text-caption text-slate-500 break-all">{t("pkgmgr.actualTarget", { path: eInfo.real_target })}</p>
                        )}
                      </div>
                      <div className="flex items-center gap-2 flex-shrink-0">
                        {eInfo && (
                          <span className="text-slate-300 font-mono text-title font-semibold bg-white/5 px-2.5 py-1 rounded-ctl">{eInfo.size}</span>
                        )}
                        <button onClick={() => openWorkflow("cache", extra.id)} disabled={!projectStatus?.managed || workflowType !== null}
                          className="px-3 py-1.5 bg-amber-600/70 hover:bg-amber-600 disabled:opacity-40 text-white rounded-ctl text-body font-semibold cursor-pointer flex items-center gap-1 transition-all" title={!projectStatus?.managed ? t("pkgmgr.managedFirst") : ""}>
                          <FolderSync className="w-3.5 h-3.5" />{t("pkgmgr.adjustPath")}
                        </button>
                      </div>
                    </div>
                    {/* 附加缓存的工作流面板 */}
                    {workflowType === "cache" && workflowExtraCacheId === extra.id && renderWorkflow()}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}

      {/* 数据管理 — 安全迁移（必须拷贝，不可删） */}
      {hasChecked && installed && pm.data_detect_cmd && (
        <PackageDataStorageCard
          dataInfo={dataInfo}
          pm={pm}
          openWorkflow={openWorkflow}
          projectStatus={projectStatus}
          workflowType={workflowType}
          renderWorkflow={renderWorkflow}
        />
      )}

      {/* 镜像配置 */}
      {hasChecked && installed && pm.mirror_options && pm.mirror_options.length > 0 && (
        <div className="glass-panel rounded-panel p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center gap-2">
            <Globe className="w-4 h-4 text-[var(--module-accent)]" />
            <h4 className="text-body font-semibold text-white">{t("pkgmgr.mirrorTitle")}</h4>
            <span className="ml-auto text-caption text-slate-400 font-mono bg-black/20 px-2 py-0.5 rounded border border-white/5 break-all max-w-[400px]">
              {t("pkgmgr.currentLabel")} {currentMirror || t("pkgmgr.officialDefault")}
            </span>
          </div>
          <div className="grid grid-cols-1 gap-1.5">
            {pm.mirror_options.map((opt) => {
              const isCurrent = opt.url === "" ? !currentMirror : currentMirror === opt.url;
              return (
                <button key={opt.mirror_type} onClick={() => handleSwitchMirror(opt.url, opt.mirror_type)} disabled={!projectStatus?.managed || switchingMirror !== null || isCurrent}
                  className={`flex items-center justify-between px-3 py-2 rounded-ctl text-title font-medium cursor-pointer transition-all border
                    ${isCurrent ? "bg-emerald-500/10 border-emerald-500/30 text-emerald-300" : "bg-black/20 border-white/5 text-slate-300 hover:bg-white/5"}`} title={!projectStatus?.managed ? "请先托管项目" : ""}>
                  <span>{opt.name}</span>
                  <div className="flex items-center gap-1.5 ml-auto">
                    <span className={`text-body ${isCurrent ? "text-emerald-400" : "text-slate-500"} font-mono`}>
                      {opt.url || t("pkgmgr.default")}
                    </span>
                    {switchingMirror === opt.url ? <Loader className="w-3 h-3 animate-spin text-[var(--module-accent)]" /> : isCurrent && <CheckCircle className="w-3 h-3 text-emerald-400" />}
                  </div>
                </button>
              );
            })}
          </div>
        </div>
      )}

      {/* 代理配置 */}
      {hasChecked && installed && pm.proxy_detect_cmd && (
        <div className="glass-panel rounded-panel p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center gap-2">
            {proxyDetected ? <Wifi className="w-4 h-4 text-emerald-400" /> : <WifiOff className="w-4 h-4 text-slate-500" />}
            <h4 className="text-body font-semibold text-white">{t("pkgmgr.proxyTitle")}</h4>
            {proxyDetected && <span className="text-body text-emerald-400 font-mono">{t("pkgmgr.configured")}</span>}
          </div>
          {proxyDetected && (
            <p className="font-mono text-title text-slate-300 truncate" title={proxyDetected}>{t("pkgmgr.currentProxy")} {proxyDetected}</p>
          )}
          <div className="flex items-center gap-1.5">
            <input type="text" value={proxyInput} onChange={(e) => setProxyInput(e.target.value)} disabled={!projectStatus?.managed}
              className="flex-1 glass-input px-3 py-1.5 text-title font-mono disabled:opacity-50 disabled:cursor-not-allowed" placeholder="http://proxy.example.com:8080" />
            <button onClick={handleSetProxy} disabled={!projectStatus?.managed || settingProxy}
              className="px-3 py-1.5 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 disabled:cursor-not-allowed text-white rounded-ctl text-title font-semibold cursor-pointer flex-shrink-0" title={!projectStatus?.managed ? "请先托管项目" : ""}>
              {settingProxy ? t("pkgmgr.settingProxy") : proxyInput ? t("pkgmgr.setProxy") : t("pkgmgr.clearProxy")}
            </button>
          </div>
          <p className="text-body text-slate-500">{t("pkgmgr.proxyHint")}</p>
        </div>
      )}

      {/* 全局包 */}
      {hasChecked && installed && pm.pkg_list_cmd && (
        <div className="glass-panel rounded-panel p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Package className="w-4 h-4 text-[var(--module-accent)]" />
              <h4 className="text-body font-semibold text-white">{t("pkgmgr.packagesTitle")}</h4>
            </div>
            <button onClick={loadPackages} disabled={loadingPackages} className="flex items-center gap-1 px-2.5 py-1 bg-white/5 hover:bg-white/10 text-slate-300 rounded-ctl text-title border border-white/5 cursor-pointer">
              <RefreshCw className={`w-3 h-3 ${loadingPackages ? "animate-spin" : ""}`} />{t("pkgmgr.refresh")}
            </button>
          </div>
          {loadingPackages ? (
            <div className="flex items-center gap-2 text-title text-slate-400 py-2"><Loader className="w-3 h-3 animate-spin text-[var(--module-accent)]" />{t("pkgmgr.scanning")}</div>
          ) : packages.length === 0 ? (
            <p className="text-title text-slate-500">{t("pkgmgr.noPackages")}</p>
          ) : (
            <div className="max-h-[250px] overflow-y-auto">
              <table className="w-full text-left text-title">
                <thead><tr className="text-slate-500 border-b border-white/5"><th className="p-2">{t("pkgmgr.colName")}</th><th className="p-2 w-20">{t("pkgmgr.colCurrent")}</th><th className="p-2 w-20">{t("pkgmgr.colLatest")}</th><th className="p-2 w-16">{t("pkgmgr.colStatus")}</th><th className="p-2 w-16 text-center">{t("pkgmgr.colAction")}</th></tr></thead>
                <tbody className="divide-y divide-white/5">
                  {packages.map((p) => (
                    <tr key={p.name} className="hover:bg-white/2 text-slate-300">
                      <td className="p-2 font-medium">
                        <button onClick={() => openUrl(p.homepage)} className="hover:text-[var(--module-accent)] transition-colors cursor-pointer group">{p.name}<ExternalLink className="w-2.5 h-2.5 inline ml-0.5 text-slate-600 group-hover:text-[var(--module-accent)] opacity-0 group-hover:opacity-100" /></button>
                      </td>
                      <td className="p-2 font-mono">{p.current_version}</td>
                      <td className="p-2 font-mono text-slate-400">{p.latest_version}</td>
                      <td className="p-2">{p.status === "outdated" ? <span className="text-caption px-1 py-0.5 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20 font-semibold">{t("pkgmgr.upgradable")}</span> : <span className="text-caption px-1 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-semibold">{t("pkgmgr.latestShort")}</span>}</td>
                      <td className="p-2 text-center">
                        {p.status === "outdated" && <button onClick={() => handleUpgradePackage(p.name)} disabled={!projectStatus?.managed || upgradingPkg === p.name} className="px-2 py-0.5 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-white rounded text-caption font-semibold cursor-pointer" title={!projectStatus?.managed ? t("pkgmgr.managedFirst") : ""}>{upgradingPkg === p.name ? t("pkgmgr.upgradingPkg") : t("pkgmgr.upgradePkg")}</button>}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}

      {/* 统一确认弹窗（清理缓存 / 同路径直接建链） */}
      {<ConfirmDialogHost request={confirmRequest} onClose={() => setConfirmRequest(null)} />}
    </div>
  );
}
