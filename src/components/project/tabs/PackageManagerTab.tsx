import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  ExternalLink,
  CheckCircle,
  AlertTriangle,
  RefreshCw,
  Trash2,
  Download,
  Globe,
  HardDrive,
  FolderOpen,
  FolderSync,
  Package,
  Loader,
  Wifi,
  WifiOff,
  Info,
} from "lucide-react";
import type { ProjectStatus, ProjectDef, PackageManagerDef } from "../types";

const pmDetectionCache: Record<string, any> = {};

export function PackageManagerTab({ 
  projectId, 
  pm, 
  hidden, 
  installRoot, 
  installSource,
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
  const [checkingGitRepo, setCheckingGitRepo] = useState(false);
  const [bootstrapping, setBootstrapping] = useState(false);
  const [updatingGitRepo, setUpdatingGitRepo] = useState(false);

  // 缓存 & 数据存储管理
  type ParentLink = { parent_path: string; parent_target: string; child_rel: string };
  const [cacheInfo, setCacheInfo] = useState<any>(isCached ? cachedData.cacheInfo : null);
  const [dataInfo, setDataInfo] = useState<any>(isCached ? cachedData.dataInfo : null);

  const updatePmCache = (data: Partial<typeof pmDetectionCache[string]>) => {
    const key = `${projectId}:${pm.id}`;
    if (!pmDetectionCache[key]) {
      pmDetectionCache[key] = {
        installed: false,
        version: null,
        latestVersion: null,
        cacheInfo: null,
        dataInfo: null,
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

  // ── 工作流（缓存/数据变更） ──
  // null=关闭, 'cache'=缓存变更, 'data'=数据迁移
  const [workflowType, setWorkflowType] = useState<"cache" | "data" | null>(null);
  // method / paths / confirm / executing / done
  const [workflowStep, setWorkflowStep] = useState<"method" | "paths" | "confirm" | "executing" | "done">("method");
  const [workflowMethod, setWorkflowMethod] = useState<"junction" | "point">("junction");
  // junction: linkPath=形式路径(链接所在), actualPath=实际路径(数据所在)
  const [workflowLinkPath, setWorkflowLinkPath] = useState("");
  const [workflowActualPath, setWorkflowActualPath] = useState("");
  // point: 指向路径
  const [workflowPointPath, setWorkflowPointPath] = useState("");
  // 旧文件处理方式: delete=删除, move=移动到新目录, keep=不做改动
  const [workflowFileAction, setWorkflowFileAction] = useState<"delete" | "move" | "keep">("keep");
  // 执行阶段
  const [workflowExecuting, setWorkflowExecuting] = useState(false);
  const [workflowProgress, setWorkflowProgress] = useState<{ stage: string; current: number; total: number; file_name: string } | null>(null);

  // 关闭工作流，重置所有状态
  const closeWorkflow = () => {
    setWorkflowType(null);
    setWorkflowStep("method");
    setWorkflowMethod("junction");
    setWorkflowLinkPath("");
    setWorkflowActualPath("");
    setWorkflowPointPath("");
    setWorkflowFileAction("keep");
    setWorkflowExecuting(false);
    setWorkflowProgress(null);
  };

  // 打开工作流
  const openWorkflow = (type: "cache" | "data") => {
    closeWorkflow();
    setWorkflowType(type);
    setWorkflowStep("method");
    // 预填默认值
    if (type === "cache" && cacheInfo) {
      setWorkflowLinkPath(cacheInfo.path);
      if (cacheInfo.real_target) {
        setWorkflowActualPath(cacheInfo.real_target);
      } else {
        const drive = cacheInfo.path.match(/^([A-Za-z]):\\/);
        if (drive && drive[1].toUpperCase() === "C") {
          setWorkflowActualPath(`D:\\any-version-caches\\${pm.id}`);
        }
      }
    }
    if (type === "data") {
      if (dataInfo) {
        setWorkflowLinkPath(dataInfo.path);
        if (dataInfo.real_target) {
          setWorkflowActualPath(dataInfo.real_target);
        } else {
          const drive = dataInfo.path.match(/^([A-Za-z]):\\/);
          if (drive && drive[1].toUpperCase() === "C") {
            setWorkflowActualPath(`D:\\any-version-data\\${pm.id}`);
          }
        }
      } else {
        // 未检测到数据路径，预填一个建议目标路径，源路径留空由用户填写
        setWorkflowActualPath(`D:\\any-version-data\\${pm.id}`);
      }
    }
  };

  // 工作流下一步
  const workflowNext = () => {
    if (workflowStep === "method") {
      setWorkflowStep("paths");
    } else if (workflowStep === "paths") {
      setWorkflowStep("confirm");
    } else if (workflowStep === "confirm") {
      executeWorkflow();
    }
  };

  // 工作流上一步
  const workflowPrev = () => {
    if (workflowStep === "paths") {
      setWorkflowStep("method");
    } else if (workflowStep === "confirm") {
      setWorkflowStep("paths");
    }
  };

  // 浏览文件夹
  const browseWorkflowPath = async (setter: (v: string) => void) => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, title: "选择文件夹" });
      if (selected) setter(selected as string);
    } catch { alert("文件夹选择器不可用，请手动输入路径。"); }
  };

  // 执行工作流
  const executeWorkflow = async () => {
    // 检查是否向同一目录移动文件
    const pathsSame = workflowMethod === "junction"
      && workflowLinkPath.toLowerCase().replace(/[\\/]+$/, "")
      === workflowActualPath.toLowerCase().replace(/[\\/]+$/, "");

    if (workflowFileAction === "move" && pathsSame) {
      // 同目录，提示用户无需移动
      if (!confirm("源路径和目标路径相同，无需移动文件。将直接创建链接，继续？")) {
        return;
      }
    }

    setWorkflowStep("executing");
    setWorkflowExecuting(true);
    setWorkflowProgress(null);

    const unlisten = await listen<{ stage: string; current: number; total: number; file_name: string }>(
      "migrate-storage-progress",
      (event) => setWorkflowProgress(event.payload)
    );

    try {
      if (workflowMethod === "junction") {
        // Junction 模式：调用 migrate_pkg_storage
        // delete: deleteOldFirst=true, move: deleteOldFirst=false(copy then junction), keep: deleteOldFirst=false
        const deleteOldFirst = workflowFileAction === "delete";
        await invoke("migrate_pkg_storage", {
          projectId,
          pmId: pm.id,
          newPath: workflowActualPath,
          storageKind: workflowType as string,
          deleteOldFirst,
          origPath: workflowLinkPath || undefined,
        });
      } else {
        // Point 模式：修改配置（仅缓存支持）
        if (!pm.cache_set_cmd_template && !pm.cache_env_var) {
          throw new Error("该项目不支持配置指向");
        }
        // 处理旧文件
        const oldPath = workflowType === "cache" ? cacheInfo?.path : dataInfo?.path;
        if (oldPath && workflowFileAction !== "keep") {
          await invoke("handle_point_storage_files", {
            oldPath,
            newPath: workflowPointPath,
            action: workflowFileAction,
          });
        }
        await invoke("project_set_cache_path", {
          projectId,
          pmId: pm.id,
          newPath: workflowPointPath,
        });
      }
      await runDetection();
      setWorkflowStep("done");
    } catch (e: unknown) {
      alert(`操作失败: ${e}`);
      setWorkflowStep("confirm"); // 回到确认步骤
    } finally {
      unlisten();
      setWorkflowExecuting(false);
      setWorkflowProgress(null);
    }
  };

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
    setDetectStep(`正在检测 ${pm.display_name}...`);

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
        label: `正在检测 ${pm.display_name} 初始化状态与更新...`,
        run: async () => {
          try {
            setCheckingGitRepo(true);
            const status = await invoke<any>("check_git_repo_status", { path: installRoot, exeName, bootstrapCmd });
            setGitRepoStatus(status);
            cachedData.gitRepoStatus = status;

            // 自动初始化逻辑：符合 git 仓库且没有可执行文件，则自动执行初始化脚本
            if (status.is_git && !status.has_exe && !bootstrapping && bootstrapCmd) {
              setBootstrapping(true);
              setDetectStep(`检测到 ${pm.display_name} 尚未初始化，正在自动编译...`);
              try {
                await invoke("bootstrap_git_repo", { path: installRoot, cmd: bootstrapCmd });
                // 初始化成功后，重新获取状态
                const newStatus = await invoke<any>("check_git_repo_status", { path: installRoot, exeName, bootstrapCmd });
                setGitRepoStatus(newStatus);
                cachedData.gitRepoStatus = newStatus;
              } catch (err) {
                alert(`自动初始化 ${pm.display_name} 失败: ${err}`);
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
      label: `正在检测 ${pm.display_name} 版本...`,
      run: async () => {
        if (pm.version_cmd) {
          try {
            const out = await invoke<string>("run_cmd_capture", { cmd: pm.version_cmd, projectId });
            setInstalled(true);
            setVersion(out.trim());
            cachedData.installed = true;
            cachedData.version = out.trim();
          } catch {
            setInstalled(false);
            setVersion(null);
            cachedData.installed = false;
            cachedData.version = null;
          }
        }
      },
    });

    // Step 1b: 检测最新版本（仅在已安装时）
    if (pm.latest_version_cmd) {
      steps.push({
        label: `正在检查 ${pm.display_name} 最新版本...`,
        run: async () => {
          try {
            const out = await invoke<string>("run_cmd_capture", { cmd: pm.latest_version_cmd!, projectId });
            setLatestVersion(out.trim());
            cachedData.latestVersion = out.trim();
          } catch {
            setLatestVersion(null);
            cachedData.latestVersion = null;
          }
        },
      });
    }

    // Step 2: cache
    if (pm.cache_detect_cmd || pm.cache_default_path || pm.cache_env_var) {
      steps.push({
        label: `正在检测 ${pm.display_name} 缓存路径...`,
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

    // Step 2b: data
    if (pm.data_detect_cmd || pm.data_default_path || pm.data_env_var) {
      steps.push({
        label: `正在检测 ${pm.display_name} 数据路径...`,
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
        label: `正在检测 ${pm.display_name} 代理配置...`,
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
        label: `正在检测 ${pm.display_name} 当前镜像源...`,
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
    
    // 检查缓存是否存在，且有效期在 5 分钟内
    if (cached && Date.now() - cached.timestamp < 5 * 60 * 1000) {
      setInstalled(cached.installed);
      setVersion(cached.version);
      setLatestVersion(cached.latestVersion);
      setCacheInfo(cached.cacheInfo);
      setDataInfo(cached.dataInfo);
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
      alert(`安装 ${pm.display_name} 失败: ${e}`);
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
      alert(`升级 ${pm.display_name} 失败: ${e}`);
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
      alert(`切换镜像失败: ${e}`);
    } finally {
      setSwitchingMirror(null);
    }
  };

  // ── 清理缓存（带进度条） ──
  const handleCleanCache = async () => {
    if (!pm.cache_detect_cmd && !pm.cache_default_path && !pm.cache_env_var) return;
    if (!confirm(`将删除所有缓存文件（约 ${cacheInfo?.size || "?"}），确定继续？`)) return;
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
      alert(`清理缓存失败: ${e}`);
    } finally {
      unlisten();
      setCleaningCache(false);
      setCleanProgress(null);
    }
  };

  // 设置代理
  const handleSetProxy = async () => {
    if (!pm.proxy_set_cmd_template) return;
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
      alert(`设置代理失败: ${e}`);
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
      alert(`升级 ${pkgName} 失败: ${e}`);
    } finally {
      setUpgradingPkg(null);
    }
  };

  // ── 工作流 UI 渲染函数 ──
  const renderWorkflow = () => {
    const isData = workflowType === "data";
    const accentBg = isData ? "bg-red-500/10" : "bg-amber-500/10";
    const accentBorder = isData ? "border-red-500/20" : "border-amber-500/20";
    const accentText = isData ? "text-red-400" : "text-amber-400";
    const btnBg = isData ? "bg-red-600 hover:bg-red-500" : "bg-amber-600 hover:bg-amber-500";
    const progressBarColor = isData ? "bg-red-500/60" : "bg-amber-500/60";

    const stepLabels: Record<string, string> = {
      method: "选择方式",
      paths: "配置路径",
      confirm: "确认预览",
      executing: "执行中",
      done: "已完成",
    };

    const totalSteps = 4;

    // ── Step: 选择方式 ──
    if (workflowStep === "method") {
      return (
        <div className={`mt-3 p-3 rounded-xl border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className={`text-[12px] font-semibold ${accentText}`}>
              变更{isData ? "数据" : "缓存"}配置 · Step 1/{totalSteps} · {stepLabels.method}
            </span>
            <button onClick={closeWorkflow} className="text-[11px] text-slate-500 hover:text-slate-300 cursor-pointer">✕ 取消</button>
          </div>
          <div className="space-y-1.5">
            <p className="text-[12px] text-slate-300">请选择变更方式：</p>
            <label className={`flex items-start gap-2 p-2.5 rounded-lg cursor-pointer transition-all border ${workflowMethod === "junction"
              ? `${accentBorder} bg-white/5`
              : "border-white/5 hover:bg-white/[0.02]"
              }`}>
              <input type="radio" name="wf_method" value="junction" checked={workflowMethod === "junction"}
                onChange={() => setWorkflowMethod("junction")} className="mt-0.5" />
              <div>
                <span className="text-[12px] font-semibold text-slate-200">A. Junction 链接</span>
                <p className="text-[13px] text-slate-500 mt-0.5">
                  创建一个目录链接，将{isData ? "数据" : "缓存"}目录指向新位置。文件实际存储在新位置，原位置通过链接访问。
                </p>
              </div>
            </label>
            {!isData && (pm.cache_set_cmd_template || pm.cache_env_var) && (
              <label className={`flex items-start gap-2 p-2.5 rounded-lg cursor-pointer transition-all border ${workflowMethod === "point"
                ? `${accentBorder} bg-white/5`
                : "border-white/5 hover:bg-white/[0.02]"
                }`}>
                <input type="radio" name="wf_method" value="point" checked={workflowMethod === "point"}
                  onChange={() => setWorkflowMethod("point")} className="mt-0.5" />
                <div>
                  <span className="text-[12px] font-semibold text-purple-300">B. 指向配置</span>
                  <p className="text-[13px] text-slate-500 mt-0.5">
                    直接修改 {pm.display_name} 的配置或环境变量，更改{isData ? "数据" : "缓存"}目录路径。不改动已有文件。
                  </p>
                </div>
              </label>
            )}
          </div>
          <div className="flex justify-end">
            <button onClick={workflowNext}
              className={`px-3 py-1 ${btnBg} text-white rounded text-[11px] font-semibold cursor-pointer transition-colors`}>
              下一步 →
            </button>
          </div>
        </div>
      );
    }

    // ── Step: 配置路径 ──
    if (workflowStep === "paths") {
      return (
        <div className={`mt-3 p-3 rounded-xl border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className={`text-[12px] font-semibold ${accentText}`}>
              变更{isData ? "数据" : "缓存"}配置 · Step 2/{totalSteps} · {stepLabels.paths}
            </span>
            <button onClick={closeWorkflow} className="text-[11px] text-slate-500 hover:text-slate-300 cursor-pointer">✕ 取消</button>
          </div>

          {workflowMethod === "junction" ? (
            <>
              <p className="text-[11px] text-slate-400">
                <span className="font-semibold text-slate-300">Junction 链接模式</span> — ① 形式路径（链接所在位置）→ ② 实际路径（数据存放位置）
              </p>
              <div className="space-y-1.5">
                <div>
                  <label className="text-[13px] text-slate-500 block mb-0.5">① 形式路径（链接创建位置，即 {pm.display_name} {isData ? "数据" : "缓存"}的原始路径）</label>
                  <div className="flex items-center gap-1">
                    <input type="text" value={workflowLinkPath} onChange={(e) => setWorkflowLinkPath(e.target.value)}
                      className="flex-1 glass-input px-1.5 py-1 text-[12px] font-mono" placeholder={`${isData ? "数据" : "缓存"}源路径`} />
                    <button onClick={() => browseWorkflowPath(setWorkflowLinkPath)}
                      className="p-1 bg-white/5 hover:bg-white/10 text-slate-400 rounded border border-white/5 cursor-pointer">
                      <FolderOpen className="w-3 h-3" />
                    </button>
                  </div>
                </div>
                <div>
                  <label className="text-[13px] text-slate-500 block mb-0.5">② 实际路径（数据真实存放位置，建议选非 C 盘）</label>
                  <div className="flex items-center gap-1">
                    <input type="text" value={workflowActualPath} onChange={(e) => setWorkflowActualPath(e.target.value)}
                      className="flex-1 glass-input px-1.5 py-1 text-[12px] font-mono" placeholder="目标路径（如 D:\any-version-caches\npm）" />
                    <button onClick={() => browseWorkflowPath(setWorkflowActualPath)}
                      className="p-1 bg-white/5 hover:bg-white/10 text-slate-400 rounded border border-white/5 cursor-pointer">
                      <FolderOpen className="w-3 h-3" />
                    </button>
                  </div>
                </div>
              </div>
            </>
          ) : (
            <>
              <p className="text-[11px] text-slate-400">
                <span className="font-semibold text-purple-300">指向配置模式</span> — 直接修改 {pm.display_name} 配置，指向新路径
              </p>
              <div>
                <label className="text-[13px] text-slate-500 block mb-0.5">指向路径（设置 {pm.display_name} 的{isData ? "数据" : "缓存"}目录）</label>
                <div className="flex items-center gap-1">
                  <input type="text" value={workflowPointPath} onChange={(e) => setWorkflowPointPath(e.target.value)}
                    className="flex-1 glass-input px-1.5 py-1 text-[12px] font-mono"
                    placeholder={pm.cache_default_path || "新路径"} />
                  <button onClick={() => browseWorkflowPath(setWorkflowPointPath)}
                    className="p-1 bg-white/5 hover:bg-white/10 text-slate-400 rounded border border-white/5 cursor-pointer">
                    <FolderOpen className="w-3 h-3" />
                  </button>
                </div>
              </div>
            </>
          )}

          {/* 旧文件处理方式（Junction 和 Point 共用） */}
          <div className="pt-1 space-y-1">
            <p className="text-[13px] text-slate-400 font-semibold">旧文件处理方式：</p>
            {/* 移动旧文件 */}
            <label className={`flex items-start gap-2 p-2 rounded-lg cursor-pointer border transition-all ${workflowFileAction === "move" ? "border-blue-500/30 bg-blue-500/5" : "border-white/5 hover:bg-white/[0.02]"}`}>
              <input type="radio" name="wf_file_action" value="move" checked={workflowFileAction === "move"}
                onChange={() => setWorkflowFileAction("move")} className="mt-0.5" />
              <div>
                <span className="text-[13px] font-semibold text-blue-300">移动旧文件到新目录</span>
                <p className="text-[11px] text-slate-500 mt-0.5">将现有文件整体复制到新位置，完成后{workflowMethod === "junction" ? "创建链接" : "修改配置指向"}。保留所有已有数据。</p>
              </div>
            </label>
            {/* 删除旧文件 */}
            <label className={`flex items-start gap-2 p-2 rounded-lg cursor-pointer border transition-all ${workflowFileAction === "delete" ? "border-red-500/30 bg-red-500/5" : "border-white/5 hover:bg-white/[0.02]"}`}>
              <input type="radio" name="wf_file_action" value="delete" checked={workflowFileAction === "delete"}
                disabled={isData} className="mt-0.5" />
              <div>
                <span className={`text-[13px] font-semibold ${isData ? "text-slate-600" : "text-red-300"}`}>删除旧文件</span>
                <p className="text-[11px] text-slate-500 mt-0.5">
                  {isData ? "数据文件不可直接删除以保证安全性" : "直接删除旧文件。（缓存可从网络重新下载，适合清空重建）"}
                </p>
              </div>
            </label>
            {/* 不做改动 */}
            <label className={`flex items-start gap-2 p-2 rounded-lg cursor-pointer border transition-all ${workflowFileAction === "keep" ? "border-slate-500/30 bg-slate-500/5" : "border-white/5 hover:bg-white/[0.02]"}`}>
              <input type="radio" name="wf_file_action" value="keep" checked={workflowFileAction === "keep"}
                onChange={() => setWorkflowFileAction("keep")} className="mt-0.5" />
              <div>
                <span className="text-[13px] font-semibold text-slate-300">不做改动</span>
                <p className="text-[11px] text-slate-500 mt-0.5">仅{workflowMethod === "junction" ? "创建链接指向新目录" : "修改配置指向新路径"}，旧目录中的文件保持原样不动。</p>
              </div>
            </label>
          </div>

          <div className="flex justify-between">
            <button onClick={workflowPrev}
              className="px-3 py-1 bg-white/5 hover:bg-white/10 text-slate-300 rounded text-[11px] font-semibold cursor-pointer transition-colors">
              ← 上一步
            </button>
            <button onClick={workflowNext}
              disabled={workflowMethod === "junction"
                ? (!workflowLinkPath || !workflowActualPath || workflowLinkPath === workflowActualPath)
                : !workflowPointPath}
              className={`px-3 py-1 ${btnBg} text-white rounded text-[11px] font-semibold cursor-pointer transition-colors disabled:opacity-40 disabled:cursor-not-allowed`}>
              预览 →
            </button>
          </div>
        </div>
      );
    }

    // ── Step: 确认预览 ──
    if (workflowStep === "confirm") {
      return (
        <div className={`mt-3 p-3 rounded-xl border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className={`text-[12px] font-semibold ${accentText}`}>
              变更{isData ? "数据" : "缓存"}配置 · Step 3/{totalSteps} · {stepLabels.confirm}
            </span>
            <button onClick={closeWorkflow} className="text-[11px] text-slate-500 hover:text-slate-300 cursor-pointer">✕ 取消</button>
          </div>

          {/* 预览卡片 */}
          <div className="p-3 bg-black/20 rounded-lg border border-white/5 space-y-2">
            <p className="text-[11px] text-slate-400 font-semibold uppercase tracking-wider">操作预览</p>
            <div className="space-y-1.5">
              <div className="flex items-center gap-2 text-[12px]">
                <span className={`px-1.5 py-0.5 rounded text-[13px] font-semibold ${workflowMethod === "junction" ? "bg-blue-500/10 text-blue-400" : "bg-purple-500/10 text-purple-400"
                  }`}>
                  {workflowMethod === "junction" ? "Junction" : "指向"}
                </span>
                {workflowMethod === "junction" ? (
                  <div className="font-mono text-slate-300 space-y-0.5">
                    <p className="flex items-center gap-1">
                      <span className="text-[13px] text-slate-500 flex-shrink-0">形式路径：</span>
                      <span className="text-[11px] break-all">{workflowLinkPath}</span>
                    </p>
                    <p className="flex items-center gap-1">
                      <span className="text-[13px] text-blue-400 flex-shrink-0">↓ 链接到</span>
                      <span className="text-[11px] text-blue-300 break-all">{workflowActualPath}</span>
                    </p>
                  </div>
                ) : (
                  <p className="font-mono text-slate-300 text-[12px] break-all">
                    配置指向：{workflowPointPath || "(未设置)"}
                  </p>
                )}
              </div>
              <div className="flex items-center gap-2 text-[12px]">
                <span className="text-slate-500">旧文件处理：</span>
                <span className={
                  workflowFileAction === "delete" ? "text-red-400 font-semibold" :
                    workflowFileAction === "move" ? "text-blue-400 font-semibold" :
                      "text-slate-400"
                }>
                  {workflowFileAction === "delete" ? "🗑 删除旧文件" :
                    workflowFileAction === "move" ? "📦 移动到新目录" : "📌 不做改动"}
                </span>
              </div>
              {(workflowMethod === "junction" && workflowLinkPath.toLowerCase().startsWith("c:")) && (
                <p className="text-[13px] text-red-400/80 flex items-center gap-1">
                  <AlertTriangle className="w-2.5 h-2.5" />当前路径在 C 盘，建议迁移到非系统盘
                </p>
              )}
            </div>
          </div>

          <div className="flex justify-between">
            <button onClick={workflowPrev}
              className="px-3 py-1 bg-white/5 hover:bg-white/10 text-slate-300 rounded text-[11px] font-semibold cursor-pointer transition-colors">
              ← 上一步
            </button>
            <button onClick={workflowNext} disabled={workflowExecuting}
              className={`px-3 py-1 ${btnBg} text-white rounded text-[11px] font-semibold cursor-pointer transition-colors disabled:opacity-40`}>
              确认执行
            </button>
          </div>
        </div>
      );
    }

    // ── Step: 执行中 ──
    if (workflowStep === "executing") {
      return (
        <div className={`mt-3 p-3 rounded-xl border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center gap-2">
            <Loader className="w-3.5 h-3.5 animate-spin text-blue-400" />
            <span className={`text-[12px] font-semibold ${accentText}`}>
              正在执行 · {workflowProgress?.stage || "准备中..."}
            </span>
          </div>
          {workflowProgress && (
            <div className="space-y-1.5">
              <div className="flex items-center justify-between text-[13px] text-slate-400">
                <span>{workflowProgress.stage}</span>
                <span className="font-mono">{workflowProgress.current}/{workflowProgress.total}</span>
              </div>
              <div className="w-full h-1.5 bg-white/5 rounded-full overflow-hidden">
                <div
                  className={`h-full ${progressBarColor} rounded-full transition-all duration-200`}
                  style={{ width: `${workflowProgress.total > 0 ? (workflowProgress.current / workflowProgress.total) * 100 : 0}%` }}
                />
              </div>
              {workflowProgress.file_name && (
                <p className="text-[13px] text-slate-500 truncate font-mono">{workflowProgress.file_name}</p>
              )}
            </div>
          )}
          {!workflowProgress && (
            <p className="text-[11px] text-slate-400 flex items-center gap-1">
              <Loader className="w-3 h-3 animate-spin" />正在启动操作...
            </p>
          )}
        </div>
      );
    }

    // ── Step: 完成 ──
    if (workflowStep === "done") {
      return (
        <div className={`mt-3 p-3 rounded-xl border border-emerald-500/20 bg-emerald-500/5 space-y-3 animate-fadeIn`}>
          <div className="flex items-center gap-2">
            <CheckCircle className="w-4 h-4 text-emerald-400" />
            <span className="text-[12px] font-semibold text-emerald-300">操作成功！</span>
          </div>
          <p className="text-[11px] text-emerald-400/70">
            {isData ? "数据" : "缓存"}已成功{workflowMethod === "junction" ? "迁移" : "重新配置"}，现状已更新。
          </p>
          <div className="flex justify-end">
            <button onClick={closeWorkflow}
              className="px-3 py-1 bg-emerald-600/50 hover:bg-emerald-600 text-white rounded text-[11px] font-semibold cursor-pointer transition-colors">
              关闭
            </button>
          </div>
        </div>
      );
    }

    return null;
  };

  // 隐藏时跳过渲染，但保持组件挂载和状态（避免切换回此 tab 时重新检测）
  if (hidden) return null;

  return (
    <div className="space-y-5">
      {projectStatus && !projectStatus.managed && (
        <div className="flex items-start gap-2.5 p-3 rounded-xl border border-blue-500/20 bg-blue-500/10 text-[12.5px] text-blue-200 animate-fadeIn">
          <Info className="w-4.5 h-4.5 text-blue-400 flex-shrink-0 mt-0.5" />
          <span>
            <strong>只读模式提示：</strong>当前项目未开启托管。缓存路径、数据目录及附带工具等参数仅支持查看，不能执行修改、配置、迁移或清理操作。若需修改，请先在当前页下方开启【托管项目】。
          </span>
        </div>
      )}
      {/* 头部状态栏 */}
      <div className="glass-panel rounded-2xl p-4 border border-white/5 bg-white/2">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className={`w-9 h-9 rounded-xl flex items-center justify-center ${installed ? "bg-emerald-500/10" : "bg-slate-500/10"}`}>
              <Package className={`w-4.5 h-4.5 ${installed ? "text-emerald-400" : "text-slate-500"}`} />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h3 className="text-sm font-bold text-white">{pm.display_name}</h3>
                {pm.built_in && (
                  <span className="px-1.5 py-0.5 rounded text-[11px] bg-purple-500/10 text-purple-400 border border-purple-500/20 font-semibold">内置</span>
                )}
              </div>
              {installed ? (
                <span className="text-[13px] text-emerald-400 font-mono">{version || "已安装"}</span>
              ) : checking ? (
                <span className="text-[13px] text-blue-400 flex items-center gap-1"><Loader className="w-3 h-3 animate-spin" />检测中...</span>
              ) : (
                <span className="text-[13px] text-slate-400">未安装</span>
              )}
            </div>
          </div>
          <div className="flex items-center gap-2">
            <button onClick={runDetection} disabled={checking || installing || upgrading} className="p-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg border border-white/5 cursor-pointer transition-all" title="刷新检测">
              <RefreshCw className={`w-3.5 h-3.5 ${checking ? "animate-spin" : ""}`} />
            </button>
            {installed && !pm.built_in && pm.install_cmd && (
              <>
                {latestVersion && version && versionGt(latestVersion, version) ? (
                  <button onClick={handleUpgrade} disabled={!projectStatus?.managed || upgrading || installing} className="px-4 py-1.5 bg-amber-600 hover:bg-amber-500 disabled:opacity-50 text-white rounded-lg text-[11px] font-semibold cursor-pointer transition-all flex items-center gap-1.5" title={!projectStatus?.managed ? "请先托管项目" : ""}>
                    <Download className="w-3.5 h-3.5" />{upgrading ? "升级中..." : `升级至 ${latestVersion}`}
                  </button>
                ) : latestVersion && version && !versionGt(latestVersion, version) ? (
                  <span className="text-[11px] text-emerald-400 px-2 py-1 rounded bg-emerald-500/10 border border-emerald-500/20 font-semibold">已是最新</span>
                ) : null}
              </>
            )}
            {!installed && pm.install_cmd && (
              <button onClick={handleInstall} disabled={!projectStatus?.managed || installing || upgrading} className="px-4 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-lg text-[11px] font-semibold cursor-pointer transition-all flex items-center gap-1.5" title={!projectStatus?.managed ? "请先托管项目" : ""}>
                <Download className="w-3.5 h-3.5" />{installing ? "安装中..." : "安装"}
              </button>
            )}
          </div>
        </div>
        {/* 安装/升级进度条 */}
        {installProgress && (
          <div className="mt-3 space-y-1.5 animate-fadeIn">
            <div className="flex items-center gap-2 text-[13px] text-blue-300">
              <Loader className="w-3 h-3 animate-spin" />
              {upgrading ? `正在升级 ${pm.display_name}...` : `正在安装 ${pm.display_name}...`}
            </div>
            <div className="w-full h-1.5 bg-white/5 rounded-full overflow-hidden">
              <div className="h-full bg-blue-500/60 rounded-full animate-pulse" style={{ width: "100%" }} />
            </div>
          </div>
        )}
        {detectStep && (
          <div className="mt-3 flex items-center gap-2 text-[13px] text-blue-300">
            <Loader className="w-3 h-3 animate-spin" />{detectStep}
          </div>
        )}
      </div>

      {/* Git Update Notification (通用 is_git_repo) */}
      {projectDef?.is_git_repo && gitRepoStatus?.has_update && (
        <div className="glass-panel rounded-2xl p-4 border border-amber-500/15 bg-amber-500/5 flex items-center justify-between animate-fadeIn mb-4">
          <div className="flex items-center gap-2.5 flex-1 min-w-0">
            <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0" />
            <div className="min-w-0">
              <p className="text-xs font-bold text-amber-300">检测到 {pm.display_name} 有新的 Git 更新</p>
              <p className="text-[11px] text-amber-400/80 mt-0.5 truncate">
                当前版本: <span className="font-mono">{gitRepoStatus.current_commit}</span> &rarr; 最新版本: <span className="font-mono">{gitRepoStatus.latest_commit}</span>
              </p>
            </div>
          </div>
          <button
            onClick={async () => {
              if (!installRoot) return;
              setUpdatingGitRepo(true);
              setDetectStep(`正在从 Git 拉取最新代码并重新初始化 ${pm.display_name}...`);
              try {
                await invoke("update_git_repo", { path: installRoot, bootstrapCmd: projectDef.bootstrap_cmd ?? null });
                await runDetection();
              } catch (e) {
                alert(`更新 ${pm.display_name} 失败: ${e}`);
              } finally {
                setUpdatingGitRepo(false);
                setDetectStep("");
              }
            }}
            disabled={updatingGitRepo}
            className="px-3 py-1.5 bg-amber-600 hover:bg-amber-500 disabled:opacity-50 text-white rounded-lg text-[10px] font-semibold cursor-pointer transition-all flex items-center gap-1 flex-shrink-0 ml-2"
          >
            {updatingGitRepo ? (
              <><Loader className="w-3 h-3 animate-spin" />正在更新...</>
            ) : (
              `更新 ${pm.display_name}`
            )}
          </button>
        </div>
      )}

      {/* Git 仓库尚未初始化提示 (通用 is_git_repo) */}
      {projectDef?.is_git_repo && gitRepoStatus?.is_git && !gitRepoStatus?.has_exe && !checking && (
        <div className="glass-panel rounded-2xl p-6 border border-blue-500/15 bg-blue-500/5 text-center space-y-4 animate-fadeIn">
          <Package className="w-10 h-10 text-blue-400 mx-auto opacity-70 animate-pulse" />
          <div>
            <p className="text-blue-300 text-sm font-semibold">{pm.display_name} 尚未初始化</p>
            <p className="text-[12px] text-blue-400/80 mt-1 max-w-md mx-auto leading-relaxed">
              检测到您指定的目录为 {pm.display_name} 仓库，但尚未生成可执行文件。您需要运行初始化脚本进行编译。
            </p>
          </div>
          <button
            onClick={async () => {
              if (!installRoot || !projectDef.bootstrap_cmd) return;
              setBootstrapping(true);
              setDetectStep(`正在编译并初始化 ${pm.display_name}...`);
              try {
                await invoke("bootstrap_git_repo", { path: installRoot, cmd: projectDef.bootstrap_cmd });
                await runDetection();
              } catch (e) {
                alert(`初始化 ${pm.display_name} 失败: ${e}`);
              } finally {
                setBootstrapping(false);
                setDetectStep("");
              }
            }}
            disabled={bootstrapping}
            className="px-5 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold cursor-pointer transition-all inline-flex items-center gap-1.5"
          >
            {bootstrapping ? (
              <><Loader className="w-3.5 h-3.5 animate-spin" />正在编译初始化...</>
            ) : (
              "编译并初始化"
            )}
          </button>
        </div>
      )}

      {/* 未安装提示 */}
      {hasChecked && !installed && !checking && !(projectDef?.is_git_repo && gitRepoStatus?.is_git && !gitRepoStatus?.has_exe) && (
        <div className="glass-panel rounded-2xl p-6 border border-white/5 bg-white/2 text-center animate-fadeIn">
          <Package className="w-10 h-10 text-slate-500 mx-auto mb-3 opacity-50" />
          <p className="text-slate-400 text-sm font-semibold">{pm.display_name} 未安装</p>
          <p className="text-[13px] text-slate-500 mt-1">安装后可管理缓存、数据、镜像、代理和全局依赖包。</p>
        </div>
      )}

      {/* 缓存管理 */}
      {hasChecked && installed && (pm.cache_detect_cmd || pm.cache_default_path) && (
        <div className="glass-panel rounded-2xl p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center gap-2">
            <HardDrive className="w-4 h-4 text-amber-400" />
            <h4 className="text-xs font-semibold text-white">缓存管理</h4>
            <span className="text-[11px] px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20">缓存</span>
          </div>
          {cacheInfo ? (
            <div className="p-4 bg-black/20 rounded-xl border border-white/5 space-y-3">
              <div className="flex items-start justify-between">
                <div className="space-y-1 flex-1 min-w-0">
                  <div className="flex items-center gap-2 flex-wrap">
                    {cacheInfo.detect_source && (
                      <span className="px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[10px] inline-flex items-center font-mono">
                        {cacheInfo.detect_source}
                      </span>
                    )}
                    {cacheInfo.real_target ? (
                      <span className="px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 text-[10px] inline-flex items-center font-semibold">
                        已迁移 (Junction)
                      </span>
                    ) : (
                      <span className="px-1.5 py-0.5 rounded bg-slate-500/10 text-slate-400 border border-slate-500/20 text-[10px] inline-flex items-center">
                        默认路径
                      </span>
                    )}
                  </div>
                  <p className="font-mono text-[12px] text-slate-400 break-all">{cacheInfo.path}</p>
                  {cacheInfo.real_target && (
                    <p className="font-mono text-[11px] text-slate-500 break-all">
                      ↳ 实际指向: {cacheInfo.real_target}
                    </p>
                  )}
                </div>
                <div className="flex items-center gap-2 flex-shrink-0">
                  <span className="text-slate-300 font-mono text-[13px] font-semibold bg-white/5 px-2.5 py-1 rounded-lg">
                    {cacheInfo.size}
                  </span>
                </div>
              </div>

              {/* 操作行 */}
              <div className="pt-2 border-t border-white/5 flex items-center gap-2">
                <button onClick={() => openWorkflow("cache")} disabled={!projectStatus?.managed || workflowType !== null}
                  className="px-3 py-1.5 bg-amber-600/80 hover:bg-amber-600 disabled:opacity-40 text-white rounded-lg text-[12px] font-semibold cursor-pointer flex items-center gap-1 transition-all" title={!projectStatus?.managed ? "请先托管项目" : ""}>
                  <FolderSync className="w-3.5 h-3.5" />开始变更
                </button>
                <button onClick={handleCleanCache} disabled={!projectStatus?.managed || cleaningCache || workflowType !== null}
                  className="px-3 py-1.5 bg-red-600/10 hover:bg-red-600/20 text-red-400 disabled:opacity-40 text-white rounded-lg text-[12px] font-semibold cursor-pointer flex items-center gap-1 transition-all" title={!projectStatus?.managed ? "请先托管项目" : ""}>
                  <Trash2 className="w-3.5 h-3.5" />{cleaningCache ? "清理中" : "清理缓存"}
                </button>
              </div>
              {cleanProgress && (
                <div className="space-y-1 pt-1">
                  <div className="flex items-center justify-between text-[13px] text-slate-400">
                    <span>{cleanProgress.stage}</span>
                    <span>{cleanProgress.current}/{cleanProgress.total}</span>
                  </div>
                  <div className="w-full h-1 bg-white/5 rounded-full overflow-hidden">
                    <div className="h-full bg-red-500/60 rounded-full transition-all duration-200"
                      style={{ width: `${cleanProgress.total > 0 ? (cleanProgress.current / cleanProgress.total) * 100 : 0}%` }} />
                  </div>
                  {cleanProgress.file_name && (
                    <p className="text-[13px] text-slate-500 truncate font-mono">{cleanProgress.file_name}</p>
                  )}
                </div>
              )}

              {/* 工作流面板 — 缓存变更 */}
              {workflowType === "cache" && renderWorkflow()}
            </div>
          ) : (
            <p className="text-[13px] text-slate-500">默认路径: <span className="font-mono text-slate-400">{pm.cache_default_path || "未配置"}</span></p>
          )}
        </div>
      )}

      {/* 数据管理 — 安全迁移（必须拷贝，不可删） */}
      {hasChecked && installed && pm.data_detect_cmd && (
        <div className="glass-panel rounded-2xl p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center gap-2">
            <HardDrive className="w-4 h-4 text-red-400" />
            <h4 className="text-xs font-semibold text-white">数据管理</h4>
            <span className="text-[11px] px-1.5 py-0.5 rounded bg-red-500/10 text-red-400 border border-red-500/20">数据</span>
          </div>
          <div className="p-4 bg-black/20 rounded-xl border border-white/5 space-y-3">
            {/* 数据状态 */}
            {dataInfo ? (
              <div className="flex items-start justify-between">
                <div className="space-y-1 flex-1 min-w-0">
                  <div className="flex items-center gap-2 flex-wrap">
                    {dataInfo.detect_source && (
                      <span className="px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[10px] inline-flex items-center font-mono">
                        {dataInfo.detect_source}
                      </span>
                    )}
                    {dataInfo.real_target ? (
                      <span className="px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 text-[10px] inline-flex items-center font-semibold">
                        已迁移 (Junction)
                      </span>
                    ) : (
                      <span className="px-1.5 py-0.5 rounded bg-slate-500/10 text-slate-400 border border-slate-500/20 text-[10px] inline-flex items-center">
                        默认路径
                      </span>
                    )}
                  </div>
                  <p className="font-mono text-[12px] text-slate-400 break-all">{dataInfo.path}</p>
                  {dataInfo.real_target && (
                    <p className="font-mono text-[11px] text-slate-500 break-all">
                      ↳ 实际指向: {dataInfo.real_target}
                    </p>
                  )}
                </div>
                <div className="flex items-center gap-2 flex-shrink-0">
                  <span className="text-slate-300 font-mono text-[13px] font-semibold bg-white/5 px-2.5 py-1 rounded-lg">
                    {dataInfo.size}
                  </span>
                </div>
              </div>
            ) : (
              <div className="space-y-1">
                <p className="text-[12px] text-slate-500 flex items-center gap-1.5">
                  <AlertTriangle className="w-3 h-3 text-yellow-400" />
                  <span>未设置 — 执行检测命令 </span>
                  <span className="text-[11px] font-mono text-slate-400">{pm.data_detect_cmd}</span>
                  <span> 未返回有效路径</span>
                </p>
              </div>
            )}

            <p className="text-[11px] text-red-400/70">⚠ 数据文件必须拷贝后迁移，不可直接删除（保证安全性）。</p>

            {/* 操作行 */}
            <div className="pt-2 border-t border-white/5 flex items-center gap-2">
              <button onClick={() => openWorkflow("data")} disabled={!projectStatus?.managed || workflowType !== null}
                className="px-3 py-1.5 bg-red-600 hover:bg-red-500 disabled:opacity-40 text-white rounded-lg text-[12px] font-semibold cursor-pointer flex items-center gap-1 transition-colors" title={!projectStatus?.managed ? "请先托管项目" : ""}>
                <FolderSync className="w-3.5 h-3.5" />{dataInfo ? "开始迁移" : "设置数据目录"}
              </button>
            </div>

            {/* 工作流面板 — 数据迁移 */}
            {workflowType === "data" && renderWorkflow()}
          </div>
        </div>
      )}

      {/* 镜像配置 */}
      {hasChecked && installed && pm.mirror_options && pm.mirror_options.length > 0 && (
        <div className="glass-panel rounded-2xl p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center gap-2">
            <Globe className="w-4 h-4 text-blue-400" />
            <h4 className="text-xs font-semibold text-white">镜像配置</h4>
            <span className="ml-auto text-[11px] text-slate-400 font-mono bg-black/20 px-2 py-0.5 rounded border border-white/5 break-all max-w-[400px]">
              当前: {currentMirror || "官方源（默认）"}
            </span>
          </div>
          <div className="grid grid-cols-1 gap-1.5">
            {pm.mirror_options.map((opt) => {
              const isCurrent = opt.url === "" ? !currentMirror : currentMirror === opt.url;
              return (
                <button key={opt.mirror_type} onClick={() => handleSwitchMirror(opt.url, opt.mirror_type)} disabled={!projectStatus?.managed || switchingMirror !== null || isCurrent}
                  className={`flex items-center justify-between px-3 py-2 rounded-lg text-[13px] font-medium cursor-pointer transition-all border
                    ${isCurrent ? "bg-emerald-500/10 border-emerald-500/30 text-emerald-300" : "bg-black/20 border-white/5 text-slate-300 hover:bg-white/5"}`} title={!projectStatus?.managed ? "请先托管项目" : ""}>
                  <span>{opt.name}</span>
                  <div className="flex items-center gap-1.5 ml-auto">
                    <span className={`text-[12px] ${isCurrent ? "text-emerald-400" : "text-slate-500"} font-mono`}>
                      {opt.url || "默认"}
                    </span>
                    {switchingMirror === opt.url ? <Loader className="w-3 h-3 animate-spin text-blue-400" /> : isCurrent && <CheckCircle className="w-3 h-3 text-emerald-400" />}
                  </div>
                </button>
              );
            })}
          </div>
        </div>
      )}

      {/* 代理配置 */}
      {hasChecked && installed && pm.proxy_detect_cmd && (
        <div className="glass-panel rounded-2xl p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center gap-2">
            {proxyDetected ? <Wifi className="w-4 h-4 text-emerald-400" /> : <WifiOff className="w-4 h-4 text-slate-500" />}
            <h4 className="text-xs font-semibold text-white">代理配置</h4>
            {proxyDetected && <span className="text-[12px] text-emerald-400 font-mono">已配置</span>}
          </div>
          {proxyDetected && (
            <p className="font-mono text-[13px] text-slate-300 truncate" title={proxyDetected}>当前代理: {proxyDetected}</p>
          )}
          <div className="flex items-center gap-1.5">
            <input type="text" value={proxyInput} onChange={(e) => setProxyInput(e.target.value)} disabled={!projectStatus?.managed}
              className="flex-1 glass-input px-3 py-1.5 text-[13px] font-mono disabled:opacity-50 disabled:cursor-not-allowed" placeholder="http://proxy.example.com:8080" />
            <button onClick={handleSetProxy} disabled={!projectStatus?.managed || settingProxy}
              className="px-3 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 disabled:cursor-not-allowed text-white rounded-lg text-[13px] font-semibold cursor-pointer flex-shrink-0" title={!projectStatus?.managed ? "请先托管项目" : ""}>
              {settingProxy ? "设置中..." : proxyInput ? "设置代理" : "清除代理"}
            </button>
          </div>
          <p className="text-[12px] text-slate-500">设置 HTTP/HTTPS 代理，留空并点击清除可移除代理配置。</p>
        </div>
      )}

      {/* 全局包 */}
      {hasChecked && installed && pm.pkg_list_cmd && (
        <div className="glass-panel rounded-2xl p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Package className="w-4 h-4 text-blue-400" />
              <h4 className="text-xs font-semibold text-white">全局依赖包</h4>
            </div>
            <button onClick={loadPackages} disabled={loadingPackages} className="flex items-center gap-1 px-2.5 py-1 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg text-[13px] border border-white/5 cursor-pointer">
              <RefreshCw className={`w-3 h-3 ${loadingPackages ? "animate-spin" : ""}`} />刷新
            </button>
          </div>
          {loadingPackages ? (
            <div className="flex items-center gap-2 text-[13px] text-slate-400 py-2"><Loader className="w-3 h-3 animate-spin text-blue-400" />正在扫描...</div>
          ) : packages.length === 0 ? (
            <p className="text-[13px] text-slate-500">无全局依赖包，或无法获取列表。</p>
          ) : (
            <div className="max-h-[250px] overflow-y-auto">
              <table className="w-full text-left text-[13px]">
                <thead><tr className="text-slate-500 border-b border-white/5"><th className="p-2">包名</th><th className="p-2 w-20">当前</th><th className="p-2 w-20">最新</th><th className="p-2 w-16">状态</th><th className="p-2 w-16 text-center">操作</th></tr></thead>
                <tbody className="divide-y divide-white/5">
                  {packages.map((p) => (
                    <tr key={p.name} className="hover:bg-white/2 text-slate-300">
                      <td className="p-2 font-medium">
                        <button onClick={() => openUrl(p.homepage)} className="hover:text-blue-400 transition-colors cursor-pointer group">{p.name}<ExternalLink className="w-2.5 h-2.5 inline ml-0.5 text-slate-600 group-hover:text-blue-400 opacity-0 group-hover:opacity-100" /></button>
                      </td>
                      <td className="p-2 font-mono">{p.current_version}</td>
                      <td className="p-2 font-mono text-slate-400">{p.latest_version}</td>
                      <td className="p-2">{p.status === "outdated" ? <span className="text-[11px] px-1 py-0.5 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20 font-semibold">可升级</span> : <span className="text-[11px] px-1 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-semibold">最新</span>}</td>
                      <td className="p-2 text-center">
                        {p.status === "outdated" && <button onClick={() => handleUpgradePackage(p.name)} disabled={!projectStatus?.managed || upgradingPkg === p.name} className="px-2 py-0.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded text-[11px] font-semibold cursor-pointer" title={!projectStatus?.managed ? "请先托管项目" : ""}>{upgradingPkg === p.name ? "升级中" : "升级"}</button>}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
