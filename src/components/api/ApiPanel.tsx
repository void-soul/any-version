import { useApiCollectionExport } from "./useApiCollectionExport";
import { ApiResponseToolbar } from "./ApiResponseToolbar";
import { ApiRequestHeadersEditor } from "./ApiRequestHeadersEditor";
import { ApiEndpointMetadata } from "./ApiEndpointMetadata";
import { ApiSidebarResizeHandle } from "./ApiSidebarResizeHandle";
import { ApiSidebarContent } from "./ApiSidebarContent";
import { ApiSidebarTabs } from "./ApiSidebarTabs";
import { ApiEndpointTree } from "./ApiEndpointTree";
import { useApiProjectActions } from "./useApiProjectActions";
import { ApiExtractRulesEditor } from "./ApiExtractRulesEditor";
import { ApiRequestBodyEditor } from "./ApiRequestBodyEditor";
import { ApiTemplateInheritanceMenu } from "./ApiTemplateInheritanceMenu";
import { ApiCollectionActions } from "./ApiCollectionActions";
import { ApiEnvironmentSelector } from "./ApiEnvironmentSelector";
import { ApiProjectSelector } from "./ApiProjectSelector";
import { useApiRequest } from "./useApiRequest";
import { useApiLoadTests } from "./useApiLoadTests";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Send, Plus, Trash2, Save, ChevronRight, FlaskConical, Gauge, BookOpen, ListChecks, Loader2, Pencil, KeyRound, Cookie, SlidersHorizontal, FileText, TestTube2, Braces, Link2, Star, Square, Wand2, ListOrdered } from "lucide-react";
import type { ApiProject, ApiEnvironment, ApiModule, ApiEndpoint, KeyValueItem, PresetHeaderSet, UnitTest, UnitTestRunOutput, LoadTestConfig, LoadTestRun, ApiHistoryEntry } from "./types";
import { METHODS, defaultAuthorization, defaultSettings } from "./types";
import { emptyEndpoint, fmtTime, prettyJson, KvEditor, AuthPanel, SettingsPanel, ACCENT, VarInput } from "./panelParts";
import { PresetHeadersModal, EnvModal, ProjectModal, ModuleModal } from "./panelModals";
import VexEmptyState from "../VexEmptyState";
import { UnitTestsPanel, DocsPanel, ImportModal } from "./panelSubs";
import { RunnerPanel } from "./panelRunner";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import { theamedAlert } from "../shared/ThemedAlert";
import { API_CTX_KEY } from "./ApiPanel.model";
import { MoveModuleModal } from "./ApiPanel.MoveModuleModal";

export default function ApiPanel() {
  const { t } = useTranslation();
  const [projects, setProjects] = useState<ApiProject[]>([]);
  const [activeProjectId, setActiveProjectId] = useState<string | null>(null);
  const [envs, setEnvs] = useState<ApiEnvironment[]>([]);
  const [activeEnvId, setActiveEnvId] = useState<string | null>(null);
  const [modules, setModules] = useState<ApiModule[]>([]);
  const [endpoints, setEndpoints] = useState<ApiEndpoint[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [draft, setDraft] = useState<ApiEndpoint | null>(null);
  const [tests, setTests] = useState<UnitTest[]>([]);
  const [testResults, setTestResults] = useState<Record<string, UnitTestRunOutput>>({});
  const [testing, setTesting] = useState(false);

  const {
    response,
    setResponse,
    sending,
    copied,
    bodyMode,
    setBodyMode,
    setCommentDraft,
    extractResults,
    setExtractResults,
    setHeaderFilter,
    curlCopied,
    variables,
    currentInput,
    cancelRequest,
    sendRequest,
    saveResponseBody,
    copyAsCurl,
    addExtractRule,
    updateExtractRule,
    removeExtractRule,
    copyBody,
    renderResponseContent,
  } = useApiRequest({
    envs,
    activeEnvId,
    draft,
    setEnvs,
    activeProjectId,
    selectedId,
    loadHistory: () => loadHistory(),
    updateDraft: patch => updateDraft(patch),
    saveDraft: patch => saveDraft(patch),
  });
  const {
    setLoadRuns,
    setLoadConfig,
    renderLoadTestPanel,
  } = useApiLoadTests({
    selectedId,
    draft,
    currentInput,
    variables,
  });
  const [activeTab, setActiveTab] = useState<"request" | "tests" | "runner" | "load" | "docs">("request");
  const [subTab, setSubTab] = useState<"params" | "auth" | "headers" | "body" | "settings" | "cookies" | "extract">("params");
  const [envModal, setEnvModal] = useState(false);
  const [importModal, setImportModal] = useState(false);
  const [presetModal, setPresetModal] = useState(false);
  const [presetSets, setPresetSets] = useState<PresetHeaderSet[]>([]);
  const [hideCommonHeaders, setHideCommonHeaders] = useState(false);

  const {
    exportPop,
    setExportPop,
    exportPostman,
    exportOpenapi,
  } = useApiCollectionExport({
    activeProjectId,
  });
  // 美化确认弹窗状态：{ title, message, danger, confirmText, onConfirm }
  const [confirm, setConfirm] = useState<{ title: string; message: string; danger?: boolean; confirmText?: string; onConfirm: () => void } | null>(null);
  // 模块转移弹窗状态
  const [moveModule, setMoveModule] = useState<ApiModule | null>(null);
  // 模板继承项概览弹层
  const [showTplPanel, setShowTplPanel] = useState(false);
  // 从模板继承面板打开项目弹窗时，定位到哪个模板编辑区

  // 侧栏：可拖动宽度 / 收起 / 接口历史选项卡 / 项目环境气泡切换
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [sidebarW, setSidebarW] = useState(240);
  const sbResizeRef = useRef<{ moved: boolean }>({ moved: false });
  const [sideTab, setSideTab] = useState<"tree" | "history">("tree");
  const [projectPop, setProjectPop] = useState(false);
  const [envPop, setEnvPop] = useState(false);

  // 当前 draft 中继承自项目模板的条目（按来源分组），并对比项目最新模板判断是否待同步
  const tplItems = useMemo(() => {
    if (!draft) return { total: 0, outOfSync: 0, groups: [] as { label: string; items: (KeyValueItem & { synced: boolean })[] }[] };
    const project = projects.find((p) => p.id === activeProjectId);
    // 模板项（headers/params/body 三类合并，用于比对 key/value）
    const tplAll = [...(project?.common_headers ?? []), ...(project?.common_params ?? []), ...(project?.common_body ?? [])];
    const syncedOf = (item: { key: string; value: string; enabled: boolean }) => {
      const t = tplAll.find((x) => x.key === item.key);
      return !!t && t.value === item.value && t.enabled === item.enabled;
    };
    const kv = (arr: KeyValueItem[]) =>
      arr.filter((x) => x.from_template).map((x) => ({ ...x, synced: syncedOf(x) }));
    const groups: { label: string; items: (KeyValueItem & { synced: boolean })[] }[] = [];
    const push = (label: string, arr: (KeyValueItem & { synced: boolean })[]) => {
      if (arr.length > 0) groups.push({ label, items: arr });
    };
    push("Headers", kv(draft.headers));
    push("Query Params", kv(draft.query_params));
    push("Body (urlencoded)", kv(draft.body_urlencoded));
    push(
      "Body (form-data)",
      draft.body_form.filter((x) => x.from_template).map((f) => ({ key: f.key, value: f.value, enabled: f.enabled, from_template: true, synced: syncedOf(f) }))
    );
    push("Cookies", kv(draft.cookies));
    push("Path Params", kv(draft.path_params));
    const total = groups.reduce((n, g) => n + g.items.length, 0);
    const outOfSync = groups.reduce((n, g) => n + g.items.filter((it) => !it.synced).length, 0);
    return { total, outOfSync, groups };
  }, [draft, projects, activeProjectId]);

  // 重新加载当前接口（同步模板值后角标消失）
  const {
    tplFocusSection,
    setTplFocusSection,
    projectModal,
    setProjectModal,
    openCreateProject,
    openEditProject,
    openProjectTpl,
    saveProjectModal,
    deleteProject,
  } = useApiProjectActions({
    projects,
    activeProjectId,
    setShowTplPanel,
    tplItems,
    setProjects,
    setActiveProjectId,
    setConfirm,
  });

  const reloadCurrent = async () => {
    if (!selectedId) return;
    const ep = await invoke<ApiEndpoint>("api_get_endpoint", { endpointId: selectedId });
    setDraft(ep);
    setCommentDraft(ep.response_comment ?? "");
    setResponse(null);
  };

  const currentProject = useMemo(
    () => projects.find((p) => p.id === activeProjectId) ?? null,
    [projects, activeProjectId]
  );
  const activeEnv = useMemo(
    () => envs.find((e) => e.id === activeEnvId) ?? null,
    [envs, activeEnvId]
  );

  // 记住当前项目/环境/激活接口（模块卸载重挂载后恢复工作上下文）
  useEffect(() => {
    try {
      localStorage.setItem(API_CTX_KEY, JSON.stringify({
        projectId: activeProjectId, envId: activeEnvId, endpointId: selectedId,
      }));
    } catch {
      // 忽略写入失败
    }
  }, [activeProjectId, activeEnvId, selectedId]);

  // 初始化
  useEffect(() => {
    invoke("api_init").then(() => loadProjects());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const loadProjects = useCallback(async () => {
    const list = await invoke<ApiProject[]>("api_list_projects");
    setProjects(list);
    // 恢复上次选中的项目（若已被删除则落到第一个）
    const saved = (() => { try { return JSON.parse(localStorage.getItem(API_CTX_KEY) || "null"); } catch { return null; } })();
    const savedProj = saved?.projectId;
    if (savedProj && list.some((p) => p.id === savedProj)) setActiveProjectId(savedProj);
    else if (list.length > 0) setActiveProjectId(list[0].id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 切换项目：加载环境/模块/接口/预设
  useEffect(() => {
    if (!activeProjectId) return;
    (async () => {
      const [envList, modList, epList, presetList] = await Promise.all([
        invoke<ApiEnvironment[]>("api_list_environments", { projectId: activeProjectId }),
        invoke<ApiModule[]>("api_list_modules", { projectId: activeProjectId }),
        invoke<ApiEndpoint[]>("api_list_endpoints", { projectId: activeProjectId, moduleId: null }),
        invoke<PresetHeaderSet[]>("api_list_preset_headers", { projectId: activeProjectId }),
      ]);
      setEnvs(envList);
      setModules(modList);
      setEndpoints(epList);
      setPresetSets(presetList);
      const project = projects.find((p) => p.id === activeProjectId);
      // 恢复上次选中的环境与接口（模块卸载重挂载后保留工作上下文）
      const saved = (() => { try { return JSON.parse(localStorage.getItem(API_CTX_KEY) || "null"); } catch { return null; } })();
      const savedEnv = saved?.envId && envList.some((e) => e.id === saved.envId)
        ? saved.envId
        : project?.active_env_id && envList.some((e) => e.id === project.active_env_id)
          ? project.active_env_id
          : envList[0]?.id ?? null;
      setActiveEnvId(savedEnv);
      setSelectedId(saved?.endpointId && epList.some((e) => e.id === saved.endpointId) ? saved.endpointId : null);
      setDraft(null);
      setResponse(null);
      loadHistory();
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeProjectId]);

  // 选择接口
  useEffect(() => {
    if (!selectedId) return;
    (async () => {
      const ep = await invoke<ApiEndpoint>("api_get_endpoint", { endpointId: selectedId });
      setDraft(ep);
      setCommentDraft(ep.response_comment ?? "");
      const t = await invoke<UnitTest[]>("api_list_unit_tests", { endpointId: selectedId });
      setTests(t);
      const runs = await invoke<LoadTestRun[]>("api_list_load_runs", { endpointId: selectedId });
      setLoadRuns(runs);
      setResponse(null);
      setTestResults({});
      setExtractResults([]);
      setHeaderFilter("");
      // 恢复该接口上次的压测参数
      try {
        const raw = localStorage.getItem(`any_version_api_load_cfg_${selectedId}`);
        if (raw) setLoadConfig(JSON.parse(raw) as LoadTestConfig);
      } catch { /* 忽略损坏的本地配置 */ }
      setActiveTab("request");
    })();
  }, [selectedId]);

  const updateDraft = (patch: Partial<ApiEndpoint>) => {
    setDraft((d) => (d ? { ...d, ...patch } : d));
  };

  const saveDraft = async (patch?: Partial<ApiEndpoint>) => {
    if (!draft) return;
    const next = patch ? { ...draft, ...patch } : draft;
    if (!next.id) {
      const created = await invoke<ApiEndpoint>("api_create_endpoint", { ep: next });
      setDraft(created);
      setSelectedId(created.id);
      refreshEndpoints();
    } else {
      await invoke("api_update_endpoint", { ep: next });
      if (patch) setDraft(next);
      refreshEndpoints();
    }
  };

  const refreshEndpoints = async () => {
    if (!activeProjectId) return;
    const list = await invoke<ApiEndpoint[]>("api_list_endpoints", { projectId: activeProjectId, moduleId: null });
    setEndpoints(list);
  };

  const [history, setHistory] = useState<ApiHistoryEntry[]>([]);

  const loadHistory = useCallback(async () => {
    if (!activeProjectId) return;
    try {
      const list = await invoke<ApiHistoryEntry[]>("api_list_history", { projectId: activeProjectId });
      setHistory(list);
    } catch { /* 忽略 */ }
  }, [activeProjectId]);

  // 回放历史请求：填入编辑器（若关联接口仍存在则选中）
  const replayHistory = async (h: ApiHistoryEntry) => {
    if (h.endpoint_id && endpoints.some((e) => e.id === h.endpoint_id)) {
      setSelectedId(h.endpoint_id);
      return;
    }
    // 无关联接口：以临时草稿打开（不落库，避免污染接口树）
    const draftEp: ApiEndpoint = {
      id: "", project_id: activeProjectId ?? "", module_id: null,
      name: h.name, method: h.method, url: h.url,
      headers: h.input.headers ?? [], query_params: h.input.query_params ?? [], path_params: h.input.path_params ?? [],
      body: h.input.body ?? "", body_type: h.input.body_type ?? "none",
      body_form: h.input.body_form ?? [], body_urlencoded: h.input.body_urlencoded ?? [],
      body_graphql_query: h.input.body_graphql_query ?? "", body_graphql_variables: h.input.body_graphql_variables ?? "",
      authorization: h.input.authorization ?? defaultAuthorization(),
      cookies: h.input.cookies ?? [], settings: h.input.settings ?? defaultSettings(),
      response_comment: "", is_favorite: false, extract_rules: [], description: "", docs_md: "",
      timeout_ms: h.input.timeout_ms ?? 15000, created_at: "", updated_at: "",
    };
    setSelectedId(null);
    setDraft(draftEp);
    setResponse(null);
    setActiveTab("request");
  };

  // 收藏/取消收藏
  const toggleFavorite = async (ep: ApiEndpoint) => {
    const next = !ep.is_favorite;
    await invoke("api_set_favorite", { endpointId: ep.id, favorite: next });
    setEndpoints(endpoints.map((e) => (e.id === ep.id ? { ...e, is_favorite: next } : e)));
    if (selectedId === ep.id && draft) setDraft({ ...draft, is_favorite: next });
  };

  const runTests = async () => {
    if (!selectedId) return;
    setTesting(true);
    try {
      const outs = await invoke<UnitTestRunOutput[]>("api_run_unit_test", { endpointId: selectedId, variables, inputOverride: currentInput });
      const map: Record<string, UnitTestRunOutput> = {};
      tests.forEach((t, i) => {
        map[t.id] = outs[i];
      });
      setTestResults(map);
    } catch (e) {
      theamedAlert(String(e));
    } finally {
      setTesting(false);
    }
  };

  // 衍生：把当前请求 + 响应保存为文档
  const saveAsDoc = async () => {
    if (!draft) return;
    const ep = draft;
    let md = `# ${ep.name}\n\n> \`${ep.method} ${ep.url}\`\n`;
    if (ep.authorization.type !== "none") md += `\n**${t("apisubs.auth")}：** ${ep.authorization.type}\n`;
    const kvMd = (list: KeyValueItem[], title: string) => {
      const rows = list.filter((k) => k.enabled && k.key).map((k) => `| \`${k.key}\` | \`${k.value}\` | ${k.description ?? ""} |`);
      if (rows.length) md += `\n### ${title}\n\n| ${t("apisubs.param")} | ${t("apisubs.value")} | ${t("apisubs.desc")} |\n|------|------|------|\n${rows.join("\n")}\n`;
    };
    kvMd(ep.query_params, t("apisubs.queryParams"));
    kvMd(ep.headers, t("apisubs.reqHeaders"));
    if (ep.body_type !== "none" && ep.body.trim()) {
      md += `\n### Body（${ep.body_type}）\n\n\`\`\`\n${ep.body}\n\`\`\`\n`;
    }
    if (response) {
      md += `\n## ${t("apisubs.respExample", { status: response.status, time: fmtTime(response.time_ms) })}\n\n\`\`\`json\n${prettyJson(response.body)}\n\`\`\`\n`;
    }
    await saveDraft({ docs_md: md });
    setActiveTab("docs");
  };

  // 衍生：从响应自动生成单测断言并保存
  const saveAsTest = async () => {
    if (!draft || !selectedId) return;
    const assertions: UnitTest["assertions"] = [];
    if (response?.status) {
      assertions.push({ type: "status_eq", expected: response.status });
    }
    if (response) {
      try {
        const j = JSON.parse(response.body);
        if (j && typeof j === "object") {
          const keys = Object.keys(j).slice(0, 3);
          for (const k of keys) {
            const v = j[k];
            if (typeof v === "string") assertions.push({ type: "json_path", path: k, op: "eq", expected: v });
            else if (typeof v === "number" || typeof v === "boolean") assertions.push({ type: "json_path", path: k, op: "eq", expected: v });
          }
        }
      } catch { /* non-json body */ }
    }
    if (assertions.length === 0) {
      theamedAlert(t("api.alertSendFirst"));
      return;
    }
    const test: UnitTest = {
      id: "", endpoint_id: selectedId, name: t("apisubs.fromRequest", { time: new Date().toLocaleTimeString() }),
      assertions, created_at: "",
    };
    await invoke("api_save_unit_test", { test });
    const testList = await invoke<UnitTest[]>("api_list_unit_tests", { endpointId: selectedId });
    setTests(testList);
    setActiveTab("tests");
    theamedAlert(t("api.alertTestGenerated"));
  };

  // 应用预设 Headers 到当前接口
  const applyPreset = async (setId: string) => {
    const set = presetSets.find((s) => s.id === setId);
    if (!set || !draft) return;
    const merged = [...draft.headers];
    for (const h of set.headers) {
      if (!h.key) continue;
      const idx = merged.findIndex((x) => x.key.toLowerCase() === h.key.toLowerCase());
      if (idx >= 0) merged[idx] = h;
      else merged.push(h);
    }
    updateDraft({ headers: merged });
  };

  const [moduleModal, setModuleModal] = useState<{ open: boolean; module: ApiModule | null }>({ open: false, module: null });

  const openCreateModule = () => {
    if (!activeProjectId) return;
    setModuleModal({ open: true, module: null });
  };

  const openEditModule = (m: ApiModule) => {
    setModuleModal({ open: true, module: m });
  };

  const saveModuleModal = async (name: string, description: string) => {
    if (!activeProjectId) return;
    if (moduleModal.module) {
      const updated = { ...moduleModal.module, name, description };
      await invoke("api_update_module", { module: updated });
      setModules(modules.map((m) => (m.id === updated.id ? updated : m)));
    } else {
      const created = await invoke<ApiModule>("api_create_module", { projectId: activeProjectId, name, description });
      setModules([...modules, created]);
    }
    setModuleModal({ open: false, module: null });
  };

  const createEndpoint = async (moduleId: string | null) => {
    if (!activeProjectId) return;
    const ep = emptyEndpoint(activeProjectId, moduleId);
    const created = await invoke<ApiEndpoint>("api_create_endpoint", { ep });
    setSelectedId(created.id);
    refreshEndpoints();
  };

  const deleteEndpoint = async (id: string) => {
    const ep = endpoints.find((e) => e.id === id);
    setConfirm({
      title: t("api.confirmDeleteEndpointTitle"),
      message: t("api.confirmDeleteEndpointMsg", { name: ep?.name ?? "" }),
      danger: true,
      confirmText: t("common.delete"),
      onConfirm: async () => {
        await invoke("api_delete_endpoint", { endpointId: id });
        if (selectedId === id) {
          setSelectedId(null);
          setDraft(null);
        }
        refreshEndpoints();
      },
    });
  };

  const deleteModule = async (id: string) => {
    const mod = modules.find((m) => m.id === id);
    const count = endpoints.filter((e) => e.module_id === id).length;
    setConfirm({
      title: t("api.confirmDeleteModuleTitle"),
      message: t("api.confirmDeleteModuleMsg", { name: mod?.name ?? "", count }),
      danger: true,
      confirmText: t("common.delete"),
      onConfirm: async () => {
        await invoke("api_delete_module", { moduleId: id });
        setModules(modules.filter((m) => m.id !== id));
        if (selectedId && endpoints.find((e) => e.id === selectedId)?.module_id === id) setSelectedId(null);
        refreshEndpoints();
      },
    });
  };

  // 模块内容转移到其他模块
  const doMoveModule = async (targetId: string) => {
    if (!moveModule) return;
    try {
      const n = await invoke<number>("api_move_module_endpoints", {
        sourceModuleId: moveModule.id,
        targetModuleId: targetId,
      });
      setMoveModule(null);
      refreshEndpoints();
      setConfirm({
        title: t("api.moveDoneTitle"),
        message: t("api.moveDoneMsg", { n, name: moveModule.name }),
        confirmText: t("api.gotIt"),
        onConfirm: () => setConfirm(null),
      });
    } catch (e) {
      setConfirm({
        title: t("api.moveFailTitle"),
        message: String(e),
        danger: true,
        confirmText: t("api.gotIt"),
        onConfirm: () => setConfirm(null),
      });
    }
  };

  // 渲染树（项目图标 + 模块文件夹图标 + 各方法图标）
  const renderTree = () => <ApiEndpointTree
    endpoints={endpoints} modules={modules} selectedId={selectedId} setSelectedId={setSelectedId}
    onEndpointAction={(action, endpoint) => {
      if (action === "delete") void deleteEndpoint(endpoint.id);
      else void toggleFavorite(endpoint);
    }}
    onModuleAction={(action, module) => {
      if (action === "edit") openEditModule(module);
      else if (action === "createEndpoint") void createEndpoint(module.id);
      else if (action === "move") setMoveModule(module);
      else void deleteModule(module.id);
    }}
  />;

  const statusBadge = response ? (
    <span className={`text-body font-bold px-2 py-0.5 rounded-md border ${response.status === 0 ? "text-rose-300 border-rose-500/40 bg-rose-500/10" : response.status < 300 ? "text-emerald-300 border-emerald-500/40 bg-emerald-500/10" : response.status < 500 ? "text-amber-300 border-amber-500/40 bg-amber-500/10" : "text-rose-300 border-rose-500/40 bg-rose-500/10"}`}>
      {response.status === 0 ? t("api.statusError") : response.status}
    </span>
  ) : null;

  return (
    <div className="h-full flex relative" style={{ ["--module-accent" as string]: "#06b6d4" }}>
      {/* 左侧栏（可拖动改宽度 / 收起） */}
      {!sidebarCollapsed && (
        <aside className="relative shrink-0 border-r border-white/10 bg-black/20 flex flex-col group/sb" style={{ width: sidebarW }}>
          {/* 顶部：项目选择器（气泡切换）+ 编辑/删除/新建 */}
          <div className="p-2 border-b border-white/10">
            <div className="flex items-center gap-1">
              <ApiProjectSelector
                setProjectPop={setProjectPop}
                setEnvPop={setEnvPop}
                currentProject={currentProject}
                projectPop={projectPop}
                openCreateProject={openCreateProject}
                projects={projects}
                setActiveProjectId={setActiveProjectId}
                activeProjectId={activeProjectId}
              />
              {currentProject && (
                <>
                  <button onClick={() => openEditProject(currentProject)} className="p-1.5 text-slate-500 hover:text-[var(--module-accent)] cursor-pointer" title={t("api.editProjectTip")}>
                    <Pencil className="w-3.5 h-3.5" />
                  </button>
                  <button onClick={() => deleteProject(currentProject.id)} className="p-1.5 text-slate-500 hover:text-rose-400 cursor-pointer" title={t("api.deleteProjectTip")}>
                    <Trash2 className="w-3.5 h-3.5" />
                  </button>
                </>
              )}
              <button onClick={openCreateProject} className="p-1.5 text-slate-500 hover:text-[var(--module-accent)] cursor-pointer" title={t("api.newProjectTip")}>
                <Plus className="w-3.5 h-3.5" />
              </button>
            </div>
          </div>

          {currentProject ? (
            <>
              {/* 环境切换（气泡）+ 变量维护 */}
              <ApiEnvironmentSelector
                setEnvPop={setEnvPop}
                setProjectPop={setProjectPop}
                activeEnv={activeEnv}
                envPop={envPop}
                envs={envs}
                setActiveEnvId={setActiveEnvId}
                activeProjectId={activeProjectId}
                activeEnvId={activeEnvId}
                setEnvModal={setEnvModal}
              />

              {/* 接口 / 历史 选项卡（共享空间，各自主滚动，不会互相挤压） */}
              <ApiSidebarTabs
                setSideTab={setSideTab}
                sideTab={sideTab}
                endpoints={endpoints}
                history={history}
                activeProjectId={activeProjectId}
                setHistory={setHistory}
              />

              {/* 内容区：接口树 / 历史 */}
              <ApiSidebarContent
                sideTab={sideTab}
                renderTree={renderTree}
                history={history}
                replayHistory={replayHistory}
              />

              {/* 底部工具栏（布局对齐思维导图：2×2 网格） */}
              <ApiCollectionActions
                openCreateModule={openCreateModule}
                createEndpoint={createEndpoint}
                setImportModal={setImportModal}
                setExportPop={setExportPop}
                exportPop={exportPop}
                exportPostman={exportPostman}
                exportOpenapi={exportOpenapi}
              />
            </>
          ) : (
            <VexEmptyState
              title={t("api.emptyTitle")}
              desc={t("api.emptyDesc")}
              tick={t("api.emptyTick")}
              avatarSize={38}
              className="flex-1 !py-10"
            />
          )}

          {/* 宽度拖拽把手 + 收起按钮（侧边栏右侧） */}
          <ApiSidebarResizeHandle
            sbResizeRef={sbResizeRef}
            sidebarW={sidebarW}
            setSidebarW={setSidebarW}
            setSidebarCollapsed={setSidebarCollapsed}
          />
        </aside>
      )}
      {sidebarCollapsed && (
        <button
          type="button"
          className="absolute left-2 top-2 z-20 inline-flex h-8 w-8 items-center justify-center rounded-md border border-white/10 bg-slate-900/90 text-slate-300 shadow-lg transition hover:text-white cursor-pointer"
          onClick={() => setSidebarCollapsed(false)}
          title={t("api.expandSidebar")}
        >
          <ChevronRight className="h-4 w-4" />
        </button>
      )}

      {/* 主区域 */}
      {draft ? (
        <div className="flex-1 min-w-0 flex flex-col">
          {/* 顶部工具条 */}
          <div className="flex items-center gap-2 px-3 py-2 border-b border-white/10">
            <select
              value={draft.method}
              onChange={(e) => updateDraft({ method: e.target.value })}
              className="bg-black/30 border border-white/10 rounded-md px-1.5 py-1.5 text-body font-bold cursor-pointer focus:outline-none"
            >
              {METHODS.map((m) => <option key={m} value={m}>{m}</option>)}
            </select>
            <input
              value={draft.name}
              onChange={(e) => updateDraft({ name: e.target.value })}
              className="w-40 bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-body text-slate-200 focus:outline-none"
              placeholder={t("api.endpointNamePh")}
            />
            <VarInput
              value={draft.url}
              envVars={variables}
              onChange={(v) => updateDraft({ url: v })}
              className="flex-1 bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-body text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]/60"
              placeholder="https://api.example.com/users/{{userId}}"
              onKeyDown={(e) => e.key === "Enter" && sendRequest()}
            />
            {tplItems.total > 0 && (
              <ApiTemplateInheritanceMenu
                setShowTplPanel={setShowTplPanel}
                showTplPanel={showTplPanel}
                tplItems={tplItems}
                reloadCurrent={reloadCurrent}
                openProjectTpl={openProjectTpl}
              />
            )}
            <button
              onClick={sendRequest}
              disabled={sending}
              className="flex items-center gap-1.5 px-4 py-1.5 rounded-md text-body font-semibold text-white cursor-pointer disabled:opacity-50"
              style={{ background: ACCENT }}
            >
              {sending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Send className="w-3.5 h-3.5" />}
              {sending ? t("api.sending") : t("api.send")}
            </button>
            {sending && (
              <button
                onClick={cancelRequest}
                title={t("api.cancelRequestTip")}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-body bg-rose-500/15 text-rose-300 hover:bg-rose-500/25 border border-rose-500/30 cursor-pointer"
              >
                <Square className="w-3.5 h-3.5" /> {t("api.cancelRequest")}
              </button>
            )}
            <button onClick={() => saveDraft()} className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-body bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
              <Save className="w-3.5 h-3.5" /> {t("common.save")}
            </button>
            {selectedId && (
              <button
                onClick={() => toggleFavorite(draft)}
                title={draft.is_favorite ? t("api.unfavorite") : t("api.favThis")}
                className={`flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-body cursor-pointer ${draft.is_favorite ? "text-amber-400 bg-amber-500/10" : "text-slate-400 bg-white/5 hover:bg-white/10"}`}
              >
                <Star className={`w-3.5 h-3.5 ${draft.is_favorite ? "fill-amber-400" : ""}`} /> {draft.is_favorite ? t("api.favorited") : t("api.fav")}
              </button>
            )}
            <button onClick={saveAsDoc} title={t("api.saveAsDocTip")} className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-body bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
              <BookOpen className="w-3.5 h-3.5" /> {t("api.saveAsDoc")}
            </button>
            <button onClick={saveAsTest} title={t("api.saveAsTestTip")} className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-body bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
              <TestTube2 className="w-3.5 h-3.5" /> {t("api.saveAsTest")}
            </button>
          </div>

          {/* 层级说明：模块简介 + 接口简介 */}
          <ApiEndpointMetadata
            modules={modules}
            draft={draft}
            updateDraft={updateDraft}
          />

          {/* 功能页签 */}
          <div className="flex items-center gap-1 px-3 pt-1.5">
            {([
              ["request", t("api.tabRequest"), Send],
              ["tests", t("api.tabTests"), ListChecks],
              ["runner", t("api.tabRunner"), ListOrdered],
              ["load", t("api.tabLoad"), Gauge],
              ["docs", t("api.tabDocs"), BookOpen],
            ] as const).map(([key, label, Icon]) => (
              <button
                key={key}
                onClick={() => setActiveTab(key)}
                className={`flex items-center gap-1.5 px-3 py-1.5 rounded-t-ctl text-body cursor-pointer border-b-2 ${activeTab === key ? "text-white border-[var(--module-accent)] bg-white/5" : "text-slate-500 border-transparent hover:text-slate-300"}`}
              >
                <Icon className="w-3.5 h-3.5" /> {label}
              </button>
            ))}
          </div>

          <div className="flex-1 min-h-0 overflow-y-auto px-3 py-2">
            {activeTab === "request" && (
              <div className="space-y-2">
                <div className="flex gap-1 flex-wrap">
                  {([
                    ["params", "Params", Braces],
                    ["auth", "Authorization", KeyRound],
                    ["headers", "Headers", Link2],
                    ["body", "Body", FileText],
                    ["settings", t("api.subSettings"), SlidersHorizontal],
                    ["cookies", "Cookies", Cookie],
                    ["extract", t("api.extractTab"), Wand2],
                  ] as const).map(([key, label, Icon]) => (
                    <button
                      key={key}
                      onClick={() => setSubTab(key)}
                      className={`flex items-center gap-1 px-2.5 py-1 text-caption rounded-md cursor-pointer ${subTab === key ? "bg-white/10 text-white" : "text-slate-500 hover:text-slate-300"}`}
                    >
                      <Icon className="w-3 h-3" /> {label}
                    </button>
                  ))}
                </div>
                {subTab === "params" && (
                  <KvEditor
                    items={draft.query_params}
                    onChange={(v) => updateDraft({ query_params: v })}
                    envVars={variables}
                    placeholderValue={t("api.valuePh", { guid: "{{$guid}}" })}
                  />
                )}
                {subTab === "auth" && (
                  <AuthPanel auth={draft.authorization} onChange={(a) => updateDraft({ authorization: a })} />
                )}
                {subTab === "headers" && (
                  <ApiRequestHeadersEditor
                    hideCommonHeaders={hideCommonHeaders}
                    setHideCommonHeaders={setHideCommonHeaders}
                    applyPreset={applyPreset}
                    presetSets={presetSets}
                    setPresetModal={setPresetModal}
                    draft={draft}
                    updateDraft={updateDraft}
                    variables={variables}
                  />
                )}
                {subTab === "body" && (
                  <ApiRequestBodyEditor
                    updateDraft={updateDraft}
                    draft={draft}
                    variables={variables}
                  />
                )}
                {subTab === "settings" && (
                  <SettingsPanel
                    settings={draft.settings}
                    timeoutMs={draft.timeout_ms}
                    onChange={(s) => updateDraft({ settings: s })}
                    onTimeout={(ms) => updateDraft({ timeout_ms: ms })}
                  />
                )}
                {subTab === "cookies" && (
                  <div className="space-y-1.5">
                    <KvEditor items={draft.cookies} onChange={(v) => updateDraft({ cookies: v })} envVars={variables} placeholderKey={t("api.cookiePh")} placeholderValue={t("apiparts.kvValuePh")} />
                    <div className="text-tiny text-slate-500">{t("api.cookieHint")}</div>
                  </div>
                )}
                {subTab === "extract" && (
                  <ApiExtractRulesEditor
                    draft={draft}
                    updateExtractRule={updateExtractRule}
                    removeExtractRule={removeExtractRule}
                    addExtractRule={addExtractRule}
                    extractResults={extractResults}
                  />
                )}
              </div>
            )}

            {activeTab === "tests" && (
              <UnitTestsPanel
                endpointId={selectedId}
                tests={tests}
                setTests={setTests}
                results={testResults}
                running={testing}
                onRun={runTests}
              />
            )}

            {activeTab === "runner" && (
              <RunnerPanel
                endpoints={endpoints}
                modules={modules}
                variables={variables}
                onSelectEndpoint={setSelectedId}
              />
            )}

            {activeTab === "load" && (
              renderLoadTestPanel()
            )}

            {activeTab === "docs" && (
              <DocsPanel draft={draft} onSave={(md) => saveDraft({ docs_md: md })} />
            )}
          </div>

          {/* 响应区 */}
          <div className="shrink-0 h-64 border-t border-white/10 flex flex-col">
            <div className="flex items-center gap-2 px-3 py-1.5 border-b border-white/10">
              <span className="text-caption font-semibold text-slate-400">{t("api.response")}</span>
              {statusBadge}                  {response && (
                    <ApiResponseToolbar
                      response={response}
                      setBodyMode={setBodyMode}
                      bodyMode={bodyMode}
                      copyBody={copyBody}
                      copied={copied}
                      saveResponseBody={saveResponseBody}
                      copyAsCurl={copyAsCurl}
                      curlCopied={curlCopied}
                      extractResults={extractResults}
                    />
              )}
            </div>
            {renderResponseContent()}
          </div>
        </div>
      ) : (
        <div className="flex-1 flex items-center justify-center text-sm text-slate-500">
          <div className="text-center space-y-2">
            <FlaskConical className="w-10 h-10 mx-auto opacity-40" />
            <p>{t("api.emptyMain")}</p>
            <p className="text-tiny text-slate-600">{t("api.emptyHint2", { vars: '{{"变量名"}}', guid: "{{$guid}}" })}</p>
          </div>
        </div>
      )}

      {envModal && activeProjectId && (
        <EnvModal
          projectId={activeProjectId}
          envs={envs}
          activeEnvId={activeEnvId}
          onClose={() => setEnvModal(false)}
          onChanged={async () => {
            const envList = await invoke<ApiEnvironment[]>("api_list_environments", { projectId: activeProjectId });
            setEnvs(envList);
            const project = (await invoke<ApiProject[]>("api_list_projects")).find((p) => p.id === activeProjectId);
            setActiveEnvId(project?.active_env_id ?? envList[0]?.id ?? null);
          }}
        />
      )}

      {presetModal && activeProjectId && (
        <PresetHeadersModal
          projectId={activeProjectId}
          sets={presetSets}
          onClose={() => setPresetModal(false)}
          onChanged={async () => {
            const list = await invoke<PresetHeaderSet[]>("api_list_preset_headers", { projectId: activeProjectId });
            setPresetSets(list);
          }}
        />
      )}

      {importModal && activeProjectId && (
        <ImportModal
          projectId={activeProjectId}
          modules={modules}
          onClose={() => setImportModal(false)}
          onImported={async (endpointId?: string) => {
            setImportModal(false);
            await refreshEndpoints();
            const [modList, envList, presetList] = await Promise.all([
              invoke<ApiModule[]>("api_list_modules", { projectId: activeProjectId }),
              invoke<ApiEnvironment[]>("api_list_environments", { projectId: activeProjectId }),
              invoke<PresetHeaderSet[]>("api_list_preset_headers", { projectId: activeProjectId }),
            ]);
            setModules(modList);
            setEnvs(envList);
            setPresetSets(presetList);
            // cURL 导入后直接选中新接口
            if (endpointId) setSelectedId(endpointId);
          }}
        />
      )}

      {projectModal.open && (
        <ProjectModal
          project={projectModal.project}
          initialSection={tplFocusSection}
          onClose={() => {
            setTplFocusSection(null);
            setProjectModal({ open: false, project: null });
          }}
          onSave={saveProjectModal}
        />
      )}

      {moduleModal.open && (
        <ModuleModal
          module={moduleModal.module}
          onClose={() => setModuleModal({ open: false, module: null })}
          onSave={saveModuleModal}
        />
      )}

      {confirm && (
        <ConfirmDialog
          open
          title={confirm.title}
          desc={confirm.message}
          danger={confirm.danger}
          confirmText={confirm.confirmText}
          onConfirm={() => { const fn = confirm.onConfirm; setConfirm(null); fn(); }}
          onCancel={() => setConfirm(null)}
        />
      )}

      {moveModule && (
        <MoveModuleModal
          module={moveModule}
          modules={modules}
          onClose={() => setMoveModule(null)}
          onMoved={doMoveModule}
        />
      )}
    </div>
  );
}
