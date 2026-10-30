import { getSkillFacets } from "./skillSelectors";
import { SkillMetadataFields } from "./SkillMetadataFields";
import { SkillDirectoryDialog } from "./SkillDirectoryDialog";
import { BuiltinSkillLibrary } from "./BuiltinSkillLibrary";
import { SkillMarketplaceDirectory } from "./SkillMarketplaceDirectory";
import { SkillLibraryList } from "./SkillLibraryList";
import { SkillLibraryFilters } from "./SkillLibraryFilters";
import { SkillPreviewDialog } from "./SkillPreviewDialog";
import { ProjectSkillDetails } from "./ProjectSkillDetails";
import { SkillLibraryPicker } from "./SkillLibraryPicker";
import { SkillScanDirectoryOptions } from "./SkillScanDirectoryOptions";
import { SkillManagerHeader } from "./SkillManagerHeader";
import { useSkillDirectoryActions } from "./useSkillDirectoryActions";
import { useSkillPreview } from "./useSkillPreview";
import { useSkillInstallation } from "./useSkillInstallation";
import { useSkillMetadataEditor } from "./useSkillMetadataEditor";
import { useState, useEffect, useMemo } from 'react';
import { useProjects } from './useProjects';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { openPath } from '@tauri-apps/plugin-opener';
import { open } from '@tauri-apps/plugin-dialog';
import { listen as listenEvent } from '@tauri-apps/api/event';
import { Search, Tag, Store, Download, Trash2, CheckCircle, AlertTriangle, X, Loader2, ChevronDown, Filter, Sparkles, FolderOpen, Plus, Link2, RotateCw } from 'lucide-react';
import { ConfirmDialogHost, type ConfirmRequest } from "../shared/ConfirmDialog";
import { SharedModal } from "../shared/Modal";
import { type BuiltinSkillView, type InstallOutcome, type ProjectSkillEntry, type SkillEntry, type SkillProject, type TabKey } from "./SkillManager.model";

export default function SkillManager() {
  const { t } = useTranslation();
  const [tab, setTab] = useState<TabKey>('skills');

  // ── 技能数据 ──
  const [skills, setSkills] = useState<SkillEntry[]>([]);
  const [skillLoading, setSkillLoading] = useState(true);
  const [search, setSearch] = useState('');
  const [selCat, setSelCat] = useState<string | null>(null);
  const [selTags, setSelTags] = useState<string[]>([]);
  const {
    editing,
    setEditing,
    edCat,
    setEdCat,
    edTags,
    tagInput,
    setTagInput,
    savingMeta,
    saveMeta,
    openEdit,
    addTag,
    removeTag,
  } = useSkillMetadataEditor({
    loadSkills: () => loadSkills(),
  });
  const [skillMsg, setSkillMsg] = useState<{ id: string; msg: string; ok: boolean } | null>(null);

  // ── 市场安装 ──
  const {
    installInput,
    setInstallInput,
    installing,
    setInstalling,
    installLog,
    setInstallLog,
    installErr,
    sources,
    updates,
    checkingUpdates,
    builtins,
    setBuiltins,
    builtinBusy,
    builtinResult,
    checkUpdates,
    startInstall,
    updateBuiltin,
    installBuiltin,
  } = useSkillInstallation({
    loadSkills: () => loadSkills(),
  });
  // 市场 / 内置技能收进「添加技能」弹框（它们只是「获取技能的渠道」，不是并列的管理维度）
  const [showAdd, setShowAdd] = useState(false);
  const [showMarket, setShowMarket] = useState(false);
  const [showBuiltin, setShowBuiltin] = useState(false);

  // ── 项目（技能的作用域） ──
  /**
    * 项目清单来自**共享 store**（./useProjects）—— MCP 页读的是同一份快照。
    * 之前只是共用 hook 代码、各存一份 useState，加上两页 keep-alive 常驻，
    * 于是「在技能页删掉的项目在 MCP 页还在」——后端早就改了，只有前端不同步。
    */
    const {
      projects,
      selProject: selProjectRaw,
      setSelProject: setSelProjectRaw,
      picked: pickedProjects,
      setPicked: setPickedProjects,
      counts: projCounts,
      reloadCounts: loadProjCounts,
      reload: reloadProjects,
      removeProject: removeProjectShared,
    } = useProjects();
  /** 右侧正在查看的项目（左侧点谁看谁）。空串 = 还没选，界面给引导而不是空白 */
  const selProject: string | null = selProjectRaw || null;
  const setSelProject = (id: string | null) => setSelProjectRaw(id ?? '');
  /** 技能库实际路径（如实显示，别让用户猜装到哪了） */
  const [skillsDir, setSkillsDir] = useState('');
  /** 技能库位置设置弹窗 */
  const [dirDialog, setDirDialog] = useState<{ path: string; busy: boolean; msg: string | null } | null>(null);
  /** 技能库网格是否折叠（折叠后只留标题栏 + 操作条，勾选与批量安装照常可用） */
  const [libCollapsed, setLibCollapsed] = useState(false);
  /** 项目页技能库：只看尚未装到（勾选的）项目里的技能 */
  const [onlyMissing, setOnlyMissing] = useState(false);

  /** 各AI 工具的项目级扫描目录（后端 SKILL_SCAN_DIRS，取自 CodeBuddy 服务端下发的 skillScanDirs）。
   *  一个技能会在**每个勾选的目录**下各建一个 junction 指向库里同一份实体 ——
   *  不复制内容，所以库一改所有目录同时生效。 */
  const SCAN_DIRS = ['.agents', '.codebuddy', '.claude', '.workbuddy', '.openclaw'];
  const [pickedDirs, setPickedDirs] = useState<string[]>(() => {
    try {
      const saved = localStorage.getItem('kira.skill.scanDirs');
      const parsed = saved ? (JSON.parse(saved) as string[]) : null;
      // 只认仍在清单里的目录名：清单变了以后旧的 localStorage 不能把新目录全否掉
      return parsed?.filter((d) => SCAN_DIRS.includes(d)) ?? [...SCAN_DIRS];
    } catch {
      return [...SCAN_DIRS];
    }
  });
  /** 安装后自动把这些目录写进项目的 .gitignore（junction 会被 git 跟随进内容） */
  const [autoGitignore, setAutoGitignore] = useState(
    () => localStorage.getItem('kira.skill.autoGitignore') === '1',
  );
  useEffect(() => {
    localStorage.setItem('kira.skill.scanDirs', JSON.stringify(pickedDirs));
  }, [pickedDirs]);
  useEffect(() => {
    localStorage.setItem('kira.skill.autoGitignore', autoGitignore ? '1' : '0');
  }, [autoGitignore]);

  const toggleScanDir = (d: string) => {
    setPickedDirs((prev) => {
      const next = prev.includes(d) ? prev.filter((x) => x !== d) : [...prev, d];
      // 至少留一个：全不勾等于「什么都不装」，那不如不给这个选项
      return next.length ? next : prev;
    });
  };
  const [projMsg, setProjMsg] = useState<{ id: string; msg: string; ok: boolean } | null>(null);
  /** 危险操作二次确认（删项目等）—— 走共享弹框，而不是原生 confirm */
  const [confirmReq, setConfirmReq] = useState<ConfirmRequest | null>(null);
  /** 右侧正在查看的项目（左侧点谁看谁） */
  /** projectId -> 该项目的技能清单（查看时才拉） */
  const [projSkills, setProjSkills] = useState<Record<string, ProjectSkillEntry[]>>({});
  /** 右侧勾选的「技能库技能」= 要安装的技能 */
  const [pickedLib, setPickedLib] = useState<Set<string>>(new Set());
  /** 右侧勾选的「项目内已装技能」= 要删除的技能 */
  /** SKILL.md 预览弹窗 */
  const {
    preview,
    setPreview,
    previewBusy,
    openPreview,
    openLibPreview,
  } = useSkillPreview({
    selProject,
  });
  const [projBusy, setProjBusy] = useState<string | null>(null);

  // ── 加载 ──
  const loadSkills = async () => {
    setSkillLoading(true);
    try {
      const data = await invoke<SkillEntry[]>('get_skill_overview');
      setSkills(data);
    } catch (e) {
      console.error("加载 AI 技能概览失败:", e);
    }
    try {
      setSkillsDir(await invoke<string>('get_skills_dir'));
    } catch {
      setSkillsDir('');
    }
    setSkillLoading(false);
  };

  const loadProjects = reloadProjects;

  /** 查看项目时按需拉它的技能清单（不预先拉所有项目的，避免无用 IO） */
  const loadProjectSkills = async (pid: string) => {
    try {
      const list = await invoke<ProjectSkillEntry[]>('list_project_skills', { projectId: pid });
      setProjSkills((m) => ({ ...m, [pid]: list }));
    } catch (e: any) {
      setProjMsg({ id: pid, msg: String(e), ok: false });
    }
  };

  useEffect(() => {
    loadSkills();
    loadProjects();
    invoke<BuiltinSkillView[]>('list_builtin_skills')
      .then(setBuiltins)
      .catch(() => setBuiltins([]));
  }, []);

  // 监听安装进度
  useEffect(() => {
    const unlisten = listenEvent<{ stage: string; message: string }>('skill-install-progress', (e) => {
      setInstallLog((prev) => prev + `[${e.payload.stage}] ${e.payload.message}\n`);
      if (e.payload.stage === 'done') {
        setInstalling(false);
        loadSkills();
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  const { allCats, allTags } = useMemo(() => getSkillFacets(skills), [skills]);

  // 筛选技能列表
  const filtered = useMemo(() => {
    return skills.filter((s) => {
      if (search.trim()) {
        const q = search.toLowerCase();
        const m = s.id.toLowerCase().includes(q) || s.name.toLowerCase().includes(q) || s.description.toLowerCase().includes(q);
        if (!m) return false;
      }
      if (selCat && s.category !== selCat) return false;
      if (selTags.length > 0 && !selTags.every((t) => s.tags.includes(t))) return false;
      return true;
    });
  }, [skills, search, selCat, selTags]);

  const hasFilter = search || selCat || selTags.length > 0;
  const clearFilter = () => { setSearch(''); setSelCat(null); setSelTags([]); };

  /**
   * 项目页技能库列表的最终结果：先按搜索/分类/标签筛（与技能库页共享），
   * 再按「只看未装」过滤掉已经装到目标项目里的技能。
   */
  const libFiltered = useMemo(() => {
    if (!onlyMissing) return filtered;
    // 目标 = 当前勾选的项目；没勾就以正在查看的项目为准（还没勾时也能筛）
    const targets = pickedProjects.size > 0 ? [...pickedProjects] : selProject ? [selProject] : [];
    if (targets.length === 0) return filtered;
    const installed = new Set<string>();
    for (const pid of targets) {
      for (const e of projSkills[pid] || []) {
        if (e.state === 'linked') installed.add(e.skillId);
      }
    }
    return filtered.filter((s) => !installed.has(s.id));
  }, [filtered, onlyMissing, pickedProjects, selProject, projSkills]);

  // 卸载技能（从技能库里删掉，不影响已装到项目里的 junction 之外的东西）
  const removeSkill = (id: string) => {
    setConfirmReq({
      title: t("common.delete"),
      desc: t("skillmgr.uninstallConfirm", { id }),
      confirmText: t("common.delete"),
      danger: true,
      onConfirm: async () => {
        try {
          await invoke('uninstall_skill', { skillId: id });
          setSkillMsg({ id, msg: t("skillmgr.deleted"), ok: true });
          loadSkills();
        } catch (e: any) {
          setSkillMsg({ id, msg: String(e), ok: false });
        }
      },
    });
  };

// ── 项目操作 ──

  /**
   * 改技能库位置。
   *
   * 后端会连着搬三样东西：技能实体目录、skills.json 里记录的 directory、
   * 以及各项目里指向旧库的 junction（目录联接记绝对路径，不重建就全成悬空）。
   * 所以这里成功提示要报迁移数量，让用户知道发生了什么。
   */
  const {
    saveSkillsDir,
    pickSkillsDir,
  } = useSkillDirectoryActions({
    dirDialog,
    setDirDialog,
    setSkillsDir,
    loadSkills,
    loadProjects,
    setProjSkills,
    setProjMsg,
  });

  /** 添加项目：让用户直接选一个目录 */
  const addProject = async () => {
    try {
      const pickedPath = await open({ directory: true, multiple: false, title: t("skillmgr.projPickTitle") });
      if (typeof pickedPath !== 'string' || !pickedPath) return;
      await invoke('add_project', { path: pickedPath });
      await loadProjects();
      setProjMsg({ id: '*', msg: t("skillmgr.projAdded", { name: pickedPath.split(/[\\/]/).filter(Boolean).pop() }), ok: true });
    } catch (e: any) {
      setProjMsg({ id: '*', msg: String(e), ok: false });
    }
  };

  /**
  * 移除项目条目（只删记录，项目目录里的 .agents/skills 一概不动）。
  *
  * 二次确认走共享 ConfirmDialogHost：原生 `confirm` 的样式跟随系统、还阻塞事件循环，
  * 而这里需要把「只删清单、不动项目里的文件」讲清楚。
  */
  const removeProject = (p: SkillProject) => {
    setConfirmReq({
      title: t("skillmgr.projRemove"),
      desc: t("skillmgr.projRemoveConfirm", { name: p.name || p.path }),
      confirmText: t("common.delete"),
      danger: true,
      onConfirm: async () => {
        try {
          await removeProjectShared(p.id);
          setProjMsg({ id: p.id, msg: t("skillmgr.projRemoved"), ok: true });
          // 选中项交给 store 的 reload 兜底（它会落到第一个项目上）
        } catch (e: any) {
          setProjMsg({ id: p.id, msg: String(e), ok: false });
        }
      },
    });
  };

  /** 切换右侧正在查看的项目，并按需拉它的技能清单 */
  const selectProject = async (id: string) => {
    setSelProject(id);
    if (!projSkills[id]) await loadProjectSkills(id);
  };

  const toggleProjectPick = (id: string) => {
    setPickedProjects((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });
  };

  const toggleLibPick = (skillId: string) => {
    setPickedLib((s) => {
      const n = new Set(s);
      if (n.has(skillId)) n.delete(skillId);
      else n.add(skillId);
      return n;
    });
  };

  /**
   * 从**某一个目录**卸载技能，并按需清理空掉的壳目录。
   *
   * 交互是「在技能这一行里点掉 .codebuddy 或 .agents 其中一个」：一个技能常同时
   * 装在几个目录，用户要能逐个去掉，而不是只有「全卸」和「都不卸」两个极端。
   * 全部目录都卸掉后这一行自然从列表消失 —— 列表是扫出来的，不是存出来的。
   *
   * `state === 'local'`（项目自带的真实目录）会**整个目录删除**，不可逆，
   * 所以那种情况要先确认；链接只删链接本身，技能库里那份不受影响。
   */
  const removeSkillFromDir = (entry: ProjectSkillEntry, dirName: string) => {
    if (!selProject) return;
    const isRealDir = entry.state === "local";
    const run = async () => {
   setProjBusy(selProject);
   try {
     await invoke("remove_skill_from_project", {
       skillId: entry.skillId,
       projectId: selProject,
       dirName,
     });
     await loadProjectSkills(selProject);
     void loadProjCounts();
     setProjMsg({ id: selProject, msg: t("skillmgr.projRemovedFromDir", { name: entry.name || entry.skillId, dir: dirName }), ok: true });
   } catch (e: any) {
     setProjMsg({ id: selProject, msg: String(e), ok: false });
   }
   setProjBusy(null);
    };
    if (!isRealDir) {
   void run();
   return;
    }
    setConfirmReq({
   title: t("skillmgr.projRemoveDirTitle"),
   danger: true,
   confirmText: t("common.delete"),
   desc: (
     <div className="space-y-1.5">
       <p>{t("skillmgr.projRemoveDirConfirm", { name: entry.name || entry.skillId, dir: dirName })}</p>
       <p className="text-micro text-slate-500">{t("skillmgr.projPruneEmptyDirsHint")}</p>
     </div>
   ),
   onConfirm: run,
    });
  };

  /** 批量勾选/取消技能库里的全部技能（筛选后一键全选很常用） */
  const toggleAllLib = (list: SkillEntry[]) => {
    setPickedLib((s) => (list.every((x) => s.has(x.id)) ? new Set() : new Set(list.map((x) => x.id))));
  };

  /**
   * 把勾选的技能装到勾选的项目上：**有则覆盖，无则新增**。
   *
   * 复选框勾的是「期望状态」，后端 install 对已存在的链接幂等、对同名真实目录
   * 覆盖，所以重复应用是安全的 —— 不用先算差集。
   */
  const applyToPicked = async () => {
    const targets = [...pickedProjects];
    const wanted = [...pickedLib];
    if (targets.length === 0 || wanted.length === 0) {
      setProjMsg({ id: '*', msg: t("skillmgr.projApplyNeedBoth"), ok: false });
      return;
    }
    setProjBusy('*');
    const errors: string[] = [];
    let n = 0;
    for (const pid of targets) {
      for (const sid of wanted) {
        try {
          const r = await invoke<InstallOutcome>('install_skill_to_project', {
            skillId: sid,
            projectId: pid,
            dirs: pickedDirs,
            gitignore: autoGitignore,
          });
          n += 1;
          // 逐目录结果必须如实上报：5 个目录里 1 个失败时报「装好了」是假的，
          // 用户会在那个目录里找不到技能，又不知道发生了什么
          for (const f of r.failed) {
            errors.push(`${sid} → ${projects.find((p) => p.id === pid)?.name || pid}: ${f}`);
          }
          for (const rp of r.replaced) {
            errors.push(`${sid} → ${rp}: ${t("skillmgr.replacedRealDir")}`);
          }
        } catch (e: any) {
          errors.push(`${sid} → ${projects.find((p) => p.id === pid)?.name || pid}: ${String(e)}`);
        }
      }
    }
    setProjBusy(null);
    // 只刷新右侧正在看的那个项目，不必全量重拉
    if (selProject) await loadProjectSkills(selProject);
    // 左侧的技能数是实时的：装完就该变，否则用户会以为「装了没生效」
    void loadProjCounts();
    setPickedLib(new Set());
    if (errors.length === 0) {
      setProjMsg({ id: '*', msg: t("skillmgr.projApplied", { add: n }), ok: true });
    } else {
      setProjMsg({ id: '*', msg: errors.join('；'), ok: false });
    }
  };

  /** 清掉悬空链接（技能库里的技能已被删除，junction 还留在项目里） */
  const cleanupBroken = async (pid: string) => {
    const broken = (projSkills[pid] || []).filter((e) => e.state === 'broken');
    if (broken.length === 0) return;
    setProjBusy(pid);
    for (const e of broken) {
      try {
        await invoke('remove_skill_from_project', { skillId: e.skillId, projectId: pid });
      } catch (err: any) {
        setProjMsg({ id: pid, msg: String(err), ok: false });
      }
    }
    setProjBusy(null);
    await loadProjectSkills(pid);
    void loadProjCounts();
  };

  return (
    <div className="flex flex-col h-full text-slate-200 select-none">
      {/* 顶部导航 Tab */}
      <SkillManagerHeader
        setTab={setTab}
        tab={tab}
        skillsDir={skillsDir}
        setDirDialog={setDirDialog}
      />

      <div className="flex-1 overflow-y-auto p-4">
{/* ════════ 项目 Tab ════════ */}
        {tab === 'projects' && (
          <div className="space-y-3">{projMsg?.id === '*' && (
              <div className={`p-2.5 rounded-ctl text-body flex items-center gap-2 ${projMsg.ok ? 'bg-emerald-500/10 text-emerald-400' : 'bg-red-500/10 text-red-400'}`}>
                {projMsg.ok ? <CheckCircle className="w-3.5 h-3.5 flex-shrink-0" /> : <AlertTriangle className="w-3.5 h-3.5 flex-shrink-0" />}
                <span className="break-all">{projMsg.msg}</span>
              </div>
            )}

            {/* ────────── 技能库（顶部通栏，可折叠） ────────── */}
            <div className="rounded-card bg-white/[0.02] border border-white/10 p-3 space-y-2">
              <div className="flex items-center justify-between gap-2">
                <button
                  onClick={() => setLibCollapsed((v) => !v)}
                  className="flex items-center gap-1.5 cursor-pointer min-w-0"
                  title={libCollapsed ? t("skillmgr.libExpand") : t("skillmgr.libCollapse")}
                >
                  <ChevronDown className={`w-3.5 h-3.5 text-slate-400 flex-shrink-0 transition-transform ${libCollapsed ? '' : 'rotate-180'}`} />
                  <span className="text-body font-bold text-slate-300">{t("skillmgr.libTitle")}</span>
                  <span className="text-micro text-slate-500 truncate">
                    {libCollapsed && pickedLib.size > 0
                      ? t("skillmgr.libPickedBadge", { n: pickedLib.size })
                      : t("skillmgr.libHint")}
                  </span>
                </button>
                {!libCollapsed && (
                  <button
                    onClick={() => toggleAllLib(libFiltered)}
                    className="px-2 py-0.5 rounded text-micro bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer flex-shrink-0"
                  >
                    {t("skillmgr.projSelectAll")}
                  </button>
                )}
              </div>

              {!libCollapsed && (
                skills.length === 0 ? (
                  <div className="text-tiny text-slate-500 py-4 text-center">{t("skillmgr.projNoSkills")}</div>
                ) : (
                  <>
                    {/* 筛选：搜索与技能库页共享同一份状态，两边筛选结果一致 */}
                    <div className="flex flex-wrap items-center gap-2">
                      <div className="relative flex-1 min-w-[160px]">
                        <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-slate-500" />
                        <input
                          value={search}
                          onChange={(e) => setSearch(e.target.value)}
                          placeholder={t("skillmgr.searchPh")}
                          className="w-full pl-8 pr-2 py-1.5 rounded-ctl bg-white/5 border border-white/10 text-body text-slate-200 placeholder-slate-500 focus:outline-none focus:border-[var(--module-accent-ring)]"
                        />
                      </div>
                      <button
                        onClick={() => setOnlyMissing((v) => !v)}
                        title={t("skillmgr.libOnlyMissing")}
                        className={`flex items-center gap-1 px-2 py-1.5 rounded-ctl text-tiny cursor-pointer transition-colors ${
                          onlyMissing
                            ? 'bg-[var(--module-accent-soft)] text-[var(--module-accent)]'
                            : 'bg-white/5 hover:bg-white/10 text-slate-300'
                        }`}
                      >
                        <Filter className="w-3 h-3" /> {t("skillmgr.libOnlyMissing")}
                      </button>
                      {hasFilter && (
                        <button
                          onClick={clearFilter}
                          className="px-2 py-1.5 rounded-ctl bg-white/5 hover:bg-white/10 text-tiny text-slate-400 flex items-center gap-1 cursor-pointer"
                        >
                          <X className="w-3 h-3" /> {t("skillmgr.clearFilter")}
                        </button>
                      )}
                    </div>

                    {/* 装到哪些扫描目录：决定这个技能出现在哪些 AI 工具的视野里。
                        每个勾选的目录下各建一个 junction 指向库里同一份实体，不复制内容。 */}
                    <SkillScanDirectoryOptions
                      SCAN_DIRS={SCAN_DIRS}
                      pickedDirs={pickedDirs}
                      toggleScanDir={toggleScanDir}
                      setPickedDirs={setPickedDirs}
                      autoGitignore={autoGitignore}
                      setAutoGitignore={setAutoGitignore}
                    />

                    {libFiltered.length === 0 ? (
                      <div className="text-tiny text-slate-500 py-6 text-center">{t("skillmgr.emptyFiltered")}</div>
                    ) : (
                      <SkillLibraryPicker
                        libFiltered={libFiltered}
                        pickedLib={pickedLib}
                        toggleLibPick={toggleLibPick}
                        openLibPreview={openLibPreview}
                      />
                    )}
                  </>
                )
              )}

              {/* 操作条放在折叠区之外：折叠后仍看得见勾了多少、也照样点得了批量安装 */}
              <div className="flex items-center justify-end gap-2 pt-1 border-t border-white/5">
                <span className="text-micro text-slate-500">
                  {t("skillmgr.projPickedMatrix", { n: pickedLib.size, m: pickedProjects.size })}
                </span>
                <button
                  onClick={() => void applyToPicked()}
                  disabled={projBusy === '*' || pickedLib.size === 0 || pickedProjects.size === 0}
                  className="flex items-center gap-1 px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-40 disabled:bg-slate-700 text-body font-semibold text-white cursor-pointer transition-all"
                >
                  {projBusy === '*' ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Link2 className="w-3.5 h-3.5" />}
                  {t("skillmgr.projApplyTo")}
                </button>
              </div>
            </div>

            {/* ────────── 下：左项目 / 右项目技能 ────────── */}
            <div className="grid grid-cols-1 lg:grid-cols-[264px_1fr] gap-3 items-start">
              {/* 左：项目列表（复选框 = 安装目标） */}
              <div className="rounded-card bg-white/[0.02] border border-white/10 p-2 space-y-1.5">
                <div className="flex items-center justify-between px-1">
                  <span className="text-body font-bold text-slate-300">{t("skillmgr.projTitle")}</span>
                  <div className="flex items-center gap-1">
                    {/* 全选/全不选：装一批技能时最烦的就是一个个勾 */}
                    <button
                      onClick={() => {
                        const all = pickedProjects.size === projects.length && projects.length > 0;
                        setPickedProjects(all ? new Set() : new Set(projects.map((p) => p.id)));
                      }}
                      className="px-1.5 py-0.5 rounded text-micro bg-white/5 hover:bg-white/10 text-slate-400 cursor-pointer"
                      title={t("skillmgr.projSelectAll")}
                    >
                      {pickedProjects.size === projects.length && projects.length > 0
                        ? t("skillmgr.projUnselectAll")
                        : t("skillmgr.projSelectAll")}
                    </button>
                    {/* 计数是实时扫盘的结果，装完/删完技能数字会过期，给一个手动刷新 */}
                    <button
                      onClick={() => void loadProjCounts()}
                      className="p-1 rounded hover:bg-white/10 text-slate-400 hover:text-slate-200 cursor-pointer"
                      title={t("skillmgr.projCountRefresh")}
                    >
                      <RotateCw className="w-3 h-3" />
                    </button>
                    <button
                      onClick={() => void addProject()}
                      className="flex items-center gap-1 px-2 py-1 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-tiny font-semibold text-white cursor-pointer"
                    >
                      <Plus className="w-3 h-3" /> {t("skillmgr.projAddShort")}
                    </button>
                  </div>
                </div>

                {projects.length === 0 ? (
                  <div className="text-center py-10 text-slate-500 text-tiny">{t("skillmgr.projEmpty")}</div>
                ) : (
                  projects.map((p) => {
                    const on = pickedProjects.has(p.id);
                    const viewing = selProject === p.id;
                    // 数字来自「一次扫完所有项目」的计数，不再依赖「点过这个项目」
                    const cnt = projCounts[p.id];
                    return (
                      <div
                        key={p.id}
                        className={`flex items-center gap-1.5 px-1.5 py-1.5 rounded-ctl border transition-colors ${
                          viewing ? 'border-[var(--module-accent-ring)] bg-[var(--module-accent-soft)]' : 'border-transparent hover:bg-white/5'
                        }`}
                      >
                        <input
                          type="checkbox"
                          checked={on}
                          onChange={() => toggleProjectPick(p.id)}
                          title={t("skillmgr.projPickAsTarget")}
                          className="w-3 h-3 accent-[var(--module-accent)] flex-shrink-0 cursor-pointer"
                        />
                        <button onClick={() => void selectProject(p.id)} className="flex-1 min-w-0 text-left cursor-pointer">
                          <div className="text-tiny font-semibold text-slate-200 truncate">{p.name || p.path}</div>
                          <div className="text-micro text-slate-500 font-mono truncate">{p.path}</div>
                        </button>
                        {/* 打开项目目录：装完技能想去项目里看一眼 junction 落在哪。
                            失败时**透出原始错误**，不要换成「项目可能已被移动或删除」那种
                            猜测 —— 实测路径全部存在时，那句提示会把排查方向带偏
                            （真实原因可能是权限、路径在非挂载盘、或 shell 调用失败）。
                            与 McpManager 的同名按钮保持一致的做法。 */}
                        <button
                          onClick={() => void openPath(p.path).catch((e) => setProjMsg({ id: p.id, msg: String(e), ok: false }))}
                          className="text-slate-600 hover:text-[var(--module-accent)] cursor-pointer flex-shrink-0"
                          title={t("skillmgr.projOpenDir")}
                        >
                          <FolderOpen className="w-3.5 h-3.5" />
                        </button>
                        {/* 技能数常驻显示；悬停看拆分。悬空链接单独标出来——那是最容易漏掉的一类 */}
                        {cnt && (
                          <span className="flex items-center gap-1 flex-shrink-0">
                            <span
                              className="px-1.5 py-0.5 rounded text-micro font-semibold bg-emerald-500/15 text-emerald-400"
                              title={t("skillmgr.projCountHint", {
                                linked: cnt.linked,
                                local: cnt.local,
                              })}
                            >
                              {cnt.total}
                            </span>
                            {cnt.broken > 0 && (
                              <span
                                className="px-1 py-0.5 rounded text-micro font-semibold bg-amber-500/20 text-amber-400"
                                title={t("skillmgr.projCountBrokenHint", { n: cnt.broken })}
                              >
                                ⚠{cnt.broken}
                              </span>
                            )}
                          </span>
                        )}
                        <button
                          onClick={() => void removeProject(p)}
                          className="text-slate-600 hover:text-red-400 cursor-pointer flex-shrink-0"
                          title={t("skillmgr.projRemove")}
                        >
                          <Trash2 className="w-3 h-3" />
                        </button>
                      </div>
                    );
                  })
                )}
              </div>

              {/* 右：当前项目的技能 */}
              <ProjectSkillDetails
                selProject={selProject}
                projSkills={projSkills}
                projects={projects}
                cleanupBroken={cleanupBroken}
                projBusy={projBusy}
                openPreview={openPreview}
                removeSkillFromDir={removeSkillFromDir}
                projMsg={projMsg}
              />
            </div>
          </div>
        )}

        {/* SKILL.md 预览弹窗 */}
        {preview && (
          <SkillPreviewDialog
            preview={preview}
            setPreview={setPreview}
            previewBusy={previewBusy}
          />
        )}

        {/* ════════ 技能 Tab ════════ */}
        {tab === 'skills' && (
          <div className="space-y-3">
            {/* 检索筛选栏 */}
            <SkillLibraryFilters
              search={search}
              setSearch={setSearch}
              selCat={selCat}
              setSelCat={setSelCat}
              allCats={allCats}
              hasFilter={hasFilter}
              clearFilter={clearFilter}
            />

            {/* 标签 chips */}
            {allTags.length > 0 && (
              <div className="flex flex-wrap items-center gap-1.5">
                <Tag className="w-3.5 h-3.5 text-slate-500" />
                {allTags.map((t) => {
                  const active = selTags.includes(t);
                  return (
                    <button
                      key={t}
                      onClick={() => setSelTags(active ? selTags.filter((x) => x !== t) : [...selTags, t])}
                      className={`vex-chip px-2 py-0.5 rounded-full text-tiny font-medium transition-all cursor-pointer ${
                        active
                          ? 'vex-chip-active bg-[color-mix(in_srgb,var(--module-accent)_30%,transparent)] text-[var(--module-accent)] border border-[var(--module-accent-ring)]'
                          : 'bg-white/5 text-slate-400 hover:bg-white/10 border border-transparent'
                      }`}
                    >
                      {t}
                    </button>
                  );
                })}
              </div>
            )}

            {/* 概览数据提示 + 获取渠道入口 */}
            <div className="flex items-center justify-between gap-2 text-tiny text-slate-500">
              <span>{skillLoading ? t("skillmgr.loading") : `${t("skillmgr.skillsCount", { count: filtered.length })}${hasFilter ? t("skillmgr.filteredCount", { total: skills.length }) : ''}`}</span>
              <div className="flex items-center gap-2">
                <span className="truncate max-w-[240px]" title={skillsDir}>{t("skillmgr.source")}<code className="text-slate-400">{skillsDir || '…'}</code></span>
                {/* 市场与内置是「获取技能的渠道」，不是管理维度 —— 页头只留一个「添加技能」入口，
                    两条渠道收进弹框。原先它们是页头两个 toggle，点开是一大片面板占掉整屏，
                    而技能列表才是这个页面的正事。 */}
                <button
                  onClick={() => { setShowAdd(true); setShowMarket(true); setShowBuiltin(false); }}
                  className="flex items-center gap-1 px-2 py-1 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white font-semibold cursor-pointer transition-colors"
                >
                  <Plus className="w-3 h-3" /> {t("skillmgr.tabAdd")}
                </button>
              </div>
            </div>

            {/* 技能网格 */}
            {!skillLoading && filtered.length === 0 && (
              <div className="text-center py-16 text-slate-500 text-sm">
                {skills.length === 0
                  ? t("skillmgr.emptyAll")
                  : t("skillmgr.emptyFiltered")}
              </div>
            )}

            {/*
              技能列表：**一行一个**，不是卡片网格。

              卡片式排布在这个页面尤其不合适：技能数量多、单个信息量小（名字 + 一句描述），
              三列网格把每条撑成两块屏幕高，扫一眼只能看到 9 个名字，而这里要做的是
              **对比**（哪些装过、名字叫什么）。单行能让一屏列出三倍数量。
              描述与标签移进 hover 的原生 title（信息不丢，只是不再占版面）。
            */}
            <SkillLibraryList
              filtered={filtered}
              skillMsg={skillMsg}
              openLibPreview={openLibPreview}
              openEdit={openEdit}
              removeSkill={removeSkill}
            />
        {/* 获取渠道（市场 / 内置）：内容一字未改，只是从页头 toggle 收进了这个弹框 */}
      <SharedModal
        open={showAdd}
        onClose={() => setShowAdd(false)}
        title={t("skillmgr.tabAdd")}
        width={720}
        headerActions={
          <div className="flex items-center gap-1">
            <button
              onClick={() => { setShowMarket(true); setShowBuiltin(false); }}
              className={`flex items-center gap-1 px-2 py-1 rounded-ctl text-tiny cursor-pointer transition-colors ${showMarket ? 'bg-[var(--module-accent-soft)] text-[var(--module-accent)]' : 'bg-white/5 hover:bg-white/10 text-slate-300'}`}
            >
              <Store className="w-3 h-3" /> {t("skillmgr.tabMarket")}
            </button>
            <button
              onClick={() => { setShowBuiltin(true); setShowMarket(false); }}
              className={`flex items-center gap-1 px-2 py-1 rounded-ctl text-tiny cursor-pointer transition-colors ${showBuiltin ? 'bg-[var(--module-accent-soft)] text-[var(--module-accent)]' : 'bg-white/5 hover:bg-white/10 text-slate-300'}`}
            >
              <Sparkles className="w-3 h-3" /> {t("skillmgr.tabBuiltin")}
            </button>
          </div>
        }
      >
        {/* ════════ 从市场获取 ════════ */}
        {showMarket && (
          <div className="space-y-4 max-w-3xl">
            <div className="rounded-card bg-[color-mix(in_srgb,var(--module-accent)_5%,transparent)] border border-[var(--module-accent-ring)] p-3.5 text-caption text-[color-mix(in_srgb,var(--module-accent)_80%,transparent)] leading-relaxed">
              {t("skillmgr.marketHint1")}
              {t("skillmgr.marketHint2")}
            </div>

            {/* 来源列表 */}
            <SkillMarketplaceDirectory

            />

            {/* 在线安装 */}
            <div>
              <div className="text-body font-semibold text-slate-300 mb-2 flex items-center gap-1.5">
                <Download className="w-3.5 h-3.5 text-[var(--module-accent)]" /> {t("skillmgr.installFrom")}
              </div>
              <div className="rounded-card bg-white/[0.03] border border-white/10 p-3.5 space-y-2.5">
                <input
                  value={installInput}
                  onChange={(e) => setInstallInput(e.target.value)}
                  placeholder="如: owner/repo / https://github.com/... / 本地技能路径"
                  className="w-full px-3 py-2 rounded-ctl bg-white/5 border border-white/10 text-body text-slate-200 placeholder-slate-500 focus:outline-none focus:border-[var(--module-accent-ring)]"
                />
                <button
                  onClick={startInstall}
                  disabled={installing || !installInput.trim()}
                  className="w-full px-3 py-2.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-body font-semibold text-white flex items-center justify-center gap-2 transition-all cursor-pointer"
                >
                  {installing ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Download className="w-3.5 h-3.5" />}
                  {installing ? t("skillmgr.installing") : t("skillmgr.installTo")}
                </button>
                {/* 预置来源：点一下把 owner/repo 填进输入框 */}
                {sources.length > 0 && (
                  <div className="flex flex-wrap items-center gap-1.5">
                    <span className="text-micro text-slate-500">{t("skillmgr.presetSources")}</span>
                    {sources.map((s) => (
                      <button key={s.repo} onClick={() => setInstallInput(s.repo)} title={`${s.repo} — ${s.hint}`}
                        className="px-2 py-0.5 rounded-md text-micro bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer transition-colors">
                        {s.label}
                      </button>
                    ))}
                  </div>
                )}
                {/* 检查更新：只对「安装时记了来源」的技能有效 */}
                <div className="flex items-center gap-2 pt-0.5">
                  <button onClick={() => void checkUpdates()} disabled={checkingUpdates}
                    className="px-2.5 py-1 rounded-md text-tiny bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer disabled:opacity-40 transition-colors">
                    {checkingUpdates ? <Loader2 className="w-3 h-3 inline animate-spin" /> : null} {t("skillmgr.checkUpdates")}
                  </button>
                  <span className="text-micro text-slate-500">
                    {updates.length > 0
                      ? t("skillmgr.updatesAvailable", { count: updates.length })
                      : t("skillmgr.noUpdates")}
                  </span>
                </div>
                {updates.length > 0 && (
                  <div className="rounded-ctl border border-white/10 divide-y divide-white/5">
                    {updates.map((u) => (
                      <div key={u.skillId} className="px-2.5 py-1.5 flex items-center gap-2 text-micro">
                        <span className="text-slate-300 truncate">{u.name}</span>
                        <span className="text-slate-600 font-mono truncate">{u.source}</span>
                        <span className="flex-1" />
                        <span className="text-amber-400/80 font-mono">{u.current.slice(0, 7) || "—"} → {u.latest.slice(0, 7)}</span>
                        <button onClick={() => setInstallInput(u.source)}
                          className="px-1.5 py-0.5 rounded bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
                          {t("skillmgr.updateNow")}
                        </button>
                      </div>
                    ))}
                  </div>
                )}
                {installErr && (
                  <div className="p-2.5 rounded-ctl text-body flex items-center gap-2 bg-red-500/10 text-red-400">
                    <AlertTriangle className="w-3.5 h-3.5" /> {installErr}
                  </div>
                )}
                {installLog && (
                  <pre className="text-tiny text-slate-400 bg-black/30 rounded-ctl p-2.5 max-h-40 overflow-y-auto whitespace-pre-wrap font-mono border border-white/5">{installLog}</pre>
                )}
              </div>
            </div>
          </div>
        )}

        {/* ════════ 内置技能 ════════ */}
        {showBuiltin && (
          <BuiltinSkillLibrary
            builtins={builtins}
            skills={skills}
            updateBuiltin={updateBuiltin}
            installBuiltin={installBuiltin}
            builtinBusy={builtinBusy}
            builtinResult={builtinResult}
          />
        )}
      </SharedModal>

          </div>
        )}

      </div>

      {/* 技能库位置设置弹窗 */}
      {dirDialog && (
        <SkillDirectoryDialog
          setDirDialog={setDirDialog}
          skillsDir={skillsDir}
          dirDialog={dirDialog}
          pickSkillsDir={pickSkillsDir}
          saveSkillsDir={saveSkillsDir}
        />
      )}

      {/* 编辑分类/标签弹窗 */}
      {editing && (
        <div className="fixed inset-0 z-50 modal-mask bg-black/60 backdrop-blur-xs flex items-center justify-center p-4">
          <div className="rounded-card ui-input p-4 w-full max-w-md space-y-3 shadow-2xl">
            <div className="flex items-center justify-between border-b border-white/5 pb-2">
              <span className="text-body font-bold text-slate-200">{t("skillmgr.editAttr", { id: editing.id })}</span>
              <button onClick={() => setEditing(null)} className="text-slate-500 hover:text-slate-300">
                <X className="w-4 h-4" />
              </button>
            </div>

            <SkillMetadataFields
              edCat={edCat}
              setEdCat={setEdCat}
              edTags={edTags}
              removeTag={removeTag}
              tagInput={tagInput}
              setTagInput={setTagInput}
              addTag={addTag}
            />

            <div className="flex items-center justify-end gap-2 pt-2 border-t border-white/5">
              <button onClick={() => setEditing(null)} className="px-3 py-1.5 rounded text-body text-slate-400 hover:text-slate-200 cursor-pointer">
                {t("skillmgr.cancel")}
              </button>
              <button
                onClick={saveMeta}
                disabled={savingMeta}
                className="px-3 py-1.5 rounded bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-body font-semibold text-white flex items-center gap-1.5 cursor-pointer"
              >
                {savingMeta ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : null}
                {t("skillmgr.save")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* 危险操作二次确认（删项目等） */}
      <ConfirmDialogHost request={confirmReq} onClose={() => setConfirmReq(null)} />
    </div>
  );
}
