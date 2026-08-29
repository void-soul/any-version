import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  Send, Play, Plus, Trash2, Save, FolderPlus, FilePlus2, Settings2,
  ChevronDown, ChevronLeft, ChevronRight, Download, Upload, FlaskConical, Gauge, Database,
  BookOpen, ListChecks, Copy, Check, Loader2, Pencil, Folder,
  KeyRound, Cookie, SlidersHorizontal, FileText, TestTube2, Braces,
  Link2, StickyNote, Star, History, Eraser, AlertTriangle, FolderInput, Lock, X,
} from "lucide-react";
import type {
  ApiProject, ApiEnvironment, ApiModule, ApiEndpoint, KeyValueItem,
  PresetHeaderSet, SendRequestInput, SendRequestOutput,
  UnitTest, UnitTestRunOutput, LoadTestConfig, LoadTestRun, LoadRunStatus,
  ApiHistoryEntry,
} from "./types";
import {
  METHODS, BODY_TYPES, COMMON_AUTO_HEADERS,
  defaultAuthorization, defaultSettings,
} from "./types";
import {
  methodIcon, emptyEndpoint, fmtTime, prettyJson, ResponseBody,
  KvEditor, FormDataEditor, AuthPanel, SettingsPanel, ACCENT, VarInput,
} from "./panelParts";
import { PresetHeadersModal, EnvModal, ProjectModal, ModuleModal } from "./panelModals";
import { LoadReportView } from "./panelReport";
import { EndpointRow, UnitTestsPanel, DocsPanel, ImportModal } from "./panelSubs";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import { SharedModal } from "../shared/Modal";
import { SharedButton } from "../shared/Button";

// 记住当前项目/环境/激活接口（模块卸载重挂载后恢复）
const API_CTX_KEY = "any_version_api_ctx";

// ─── 模块转移弹窗（选择目标模块） ───
function MoveModuleModal({ module, modules, onClose, onMoved }: {
  module: ApiModule;
  modules: ApiModule[];
  onClose: () => void;
  onMoved: (targetId: string) => void;
}) {
  const [targetId, setTargetId] = useState("");
  const targets = modules.filter((m) => m.id !== module.id);
  return (
    <SharedModal
      open
      onClose={onClose}
      width={420}
      title={
        <span className="inline-flex items-center gap-2">
          <FolderInput className="w-4 h-4" style={{ color: ACCENT }} /> 移动模块内容
        </span>
      }
      footer={
        <>
          <SharedButton onClick={onClose}>取消</SharedButton>
          <SharedButton onClick={() => onMoved(targetId)} variant="primary" disabled={!targetId}>
            移动
          </SharedButton>
        </>
      }
    >
      <p className="text-xs leading-relaxed text-slate-400 mb-3">
        将「{module.name}」下的全部接口转移到其他模块。
      </p>
      <select
        value={targetId}
        onChange={(e) => setTargetId(e.target.value)}
        className="w-full rounded-lg border border-white/10 bg-black/30 px-3 py-2 text-xs text-slate-200 focus:outline-none focus:border-[var(--module-accent)]/60 cursor-pointer"
      >
        <option value="">请选择目标模块…</option>
        {targets.map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
      </select>
    </SharedModal>
  );
}

export default function ApiPanel() {
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
  const [loadRuns, setLoadRuns] = useState<LoadTestRun[]>([]);
  const [response, setResponse] = useState<SendRequestOutput | null>(null);
  const [sending, setSending] = useState(false);
  const [activeTab, setActiveTab] = useState<"request" | "tests" | "load" | "docs">("request");
  const [subTab, setSubTab] = useState<"params" | "auth" | "headers" | "body" | "settings" | "cookies">("params");
  const [envModal, setEnvModal] = useState(false);
  const [importModal, setImportModal] = useState(false);
  const [presetModal, setPresetModal] = useState(false);
  const [presetSets, setPresetSets] = useState<PresetHeaderSet[]>([]);
  const [hideCommonHeaders, setHideCommonHeaders] = useState(false);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [copied, setCopied] = useState(false);
  const [bodyMode, setBodyMode] = useState<"pretty" | "raw">("pretty");
  const [loadConfig, setLoadConfig] = useState<LoadTestConfig>({ concurrency: 10, duration_secs: 10, ramp_up_secs: 0, rps_limit: 0 });
  const [runningRunId, setRunningRunId] = useState<string | null>(null);
  const [loadStatus, setLoadStatus] = useState<LoadRunStatus | null>(null);
  const [commentDraft, setCommentDraft] = useState("");
  // 美化确认弹窗状态：{ title, message, danger, confirmText, onConfirm }
  const [confirm, setConfirm] = useState<{ title: string; message: string; danger?: boolean; confirmText?: string; onConfirm: () => void } | null>(null);
  // 模块转移弹窗状态
  const [moveModule, setMoveModule] = useState<ApiModule | null>(null);
  // 模板继承项概览弹层
  const [showTplPanel, setShowTplPanel] = useState(false);
  // 从模板继承面板打开项目弹窗时，定位到哪个模板编辑区
  const [tplFocusSection, setTplFocusSection] = useState<"headers" | "params" | "body" | null>(null);
  const pollRef = useRef<number | null>(null);

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
  const reloadCurrent = async () => {
    if (!selectedId) return;
    const ep = await invoke<ApiEndpoint>("api_get_endpoint", { endpointId: selectedId });
    setDraft(ep);
    setCommentDraft(ep.response_comment ?? "");
    setResponse(null);
  };

  const variables = useMemo(() => {
    const env = envs.find((e) => e.id === activeEnvId);
    return env?.variables ?? {};
  }, [envs, activeEnvId]);

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
      setActiveTab("request");
    })();
  }, [selectedId]);

  // 压测轮询
  useEffect(() => {
    if (!runningRunId) return;
    let alive = true;
    const tick = async () => {
      try {
        const st = await invoke<LoadRunStatus>("api_load_run_status", { runId: runningRunId });
        if (!alive) return;
        setLoadStatus(st);
        if (!st.running && st.report) {
          setRunningRunId(null);
          const runs = await invoke<LoadTestRun[]>("api_list_load_runs", { endpointId: selectedId });
          if (alive) setLoadRuns(runs);
        }
      } catch {
        if (alive) setRunningRunId(null);
      }
    };
    tick();
    pollRef.current = window.setInterval(tick, 1000);
    return () => {
      alive = false;
      if (pollRef.current) window.clearInterval(pollRef.current);
    };
  }, [runningRunId, selectedId]);

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

  // 当前请求配置 → SendRequestInput（供发送/单测/压测复用）
  const currentInput = useMemo((): SendRequestInput | null => {
    if (!draft) return null;
    return {
      method: draft.method,
      url: draft.url,
      headers: draft.headers,
      query_params: draft.query_params,
      path_params: draft.path_params,
      body: draft.body,
      body_type: draft.body_type,
      body_form: draft.body_form,
      body_urlencoded: draft.body_urlencoded,
      body_graphql_query: draft.body_graphql_query,
      body_graphql_variables: draft.body_graphql_variables,
      authorization: draft.authorization,
      cookies: draft.cookies,
      settings: draft.settings,
      timeout_ms: draft.timeout_ms,
      variables,
    };
  }, [draft, variables]);

  const sendRequest = async () => {
    if (!draft || !currentInput) return;
    setSending(true);
    try {
      const out = await invoke<SendRequestOutput>("api_send_request", { input: currentInput });
      setResponse(out);
      // 记录请求历史
      if (activeProjectId) {
        try {
          await invoke("api_add_history", {
            projectId: activeProjectId,
            endpointId: selectedId,
            name: draft.name,
            input: currentInput,
          });
          loadHistory();
        } catch { /* 历史记录失败不影响请求 */ }
      }
    } catch (e) {
      setResponse({ ok: false, status: 0, status_text: "错误", headers: [], body: String(e), body_truncated: false, time_ms: 0, size_bytes: 0 });
    } finally {
      setSending(false);
    }
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
      response_comment: "", is_favorite: false, description: "", docs_md: "",
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
      window.alert(String(e));
    } finally {
      setTesting(false);
    }
  };

  const startLoadTest = async () => {
    if (!selectedId || !draft) return;
    try {
      const runId = await invoke<string>("api_start_load_test", {
        endpointId: selectedId,
        name: draft.name,
        config: loadConfig,
        variables,
        inputOverride: currentInput,
      });
      setLoadStatus({ running: true, elapsed_secs: 0, total: 0, success: 0, failed: 0, qps: 0, latency_avg_ms: 0, latency_p95_ms: 0, report: null });
      setRunningRunId(runId);
    } catch (e) {
      window.alert(String(e));
    }
  };

  // 衍生：把当前请求 + 响应保存为文档
  const saveAsDoc = async () => {
    if (!draft) return;
    const ep = draft;
    let md = `# ${ep.name}\n\n> \`${ep.method} ${ep.url}\`\n`;
    if (ep.authorization.type !== "none") md += `\n**认证：** ${ep.authorization.type}\n`;
    const kvMd = (list: KeyValueItem[], title: string) => {
      const rows = list.filter((k) => k.enabled && k.key).map((k) => `| \`${k.key}\` | \`${k.value}\` | ${k.description ?? ""} |`);
      if (rows.length) md += `\n### ${title}\n\n| 参数 | 值 | 说明 |\n|------|------|------|\n${rows.join("\n")}\n`;
    };
    kvMd(ep.query_params, "查询参数");
    kvMd(ep.headers, "请求头");
    if (ep.body_type !== "none" && ep.body.trim()) {
      md += `\n### Body（${ep.body_type}）\n\n\`\`\`\n${ep.body}\n\`\`\`\n`;
    }
    if (response) {
      md += `\n## 响应示例（HTTP ${response.status} · ${fmtTime(response.time_ms)}）\n\n\`\`\`json\n${prettyJson(response.body)}\n\`\`\`\n`;
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
      window.alert("请先发送请求，以便从响应生成断言");
      return;
    }
    const test: UnitTest = {
      id: "", endpoint_id: selectedId, name: `从请求衍生 ${new Date().toLocaleTimeString()}`,
      assertions, created_at: "",
    };
    await invoke("api_save_unit_test", { test });
    const t = await invoke<UnitTest[]>("api_list_unit_tests", { endpointId: selectedId });
    setTests(t);
    setActiveTab("tests");
    window.alert("已从当前请求/响应生成单测断言");
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

  // 保存响应注释
  const saveComment = async () => {
    if (!draft) return;
    await saveDraft({ response_comment: commentDraft });
  };

  const toggleExpand = (id: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };

  const copyBody = async () => {
    if (!response) return;
    await navigator.clipboard.writeText(response.body);
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  };

  const [projectModal, setProjectModal] = useState<{ open: boolean; project: ApiProject | null }>({ open: false, project: null });  const openCreateProject = () => {
    setTplFocusSection(null);
    setProjectModal({ open: true, project: null });
  };
  const openEditProject = (p: ApiProject) => {
    setTplFocusSection(null);
    setProjectModal({ open: true, project: p });
  };
  // 模板继承面板的「打开项目模板」：定位到未同步项最多的分区
  const openProjectTpl = () => {
    const proj = projects.find((p) => p.id === activeProjectId);
    if (!proj) return;
    setShowTplPanel(false);
    const labelToSec = (label: string): "headers" | "params" | "body" | null =>
      label === "Headers" ? "headers" : label === "Query Params" ? "params" : label.startsWith("Body") ? "body" : null;
    const unsynced = tplItems.groups.filter((g) => g.items.some((it) => !it.synced));
    const primary = (unsynced.length > 0 ? unsynced : tplItems.groups)
      .map((g) => labelToSec(g.label))
      .find((s): s is "headers" | "params" | "body" => s !== null) ?? "headers";
    setTplFocusSection(primary);
    setProjectModal({ open: true, project: proj });
  };

  const saveProjectModal = async (name: string, description: string, commonHeaders: KeyValueItem[], commonParams: KeyValueItem[], commonBody: KeyValueItem[]) => {
    if (projectModal.project) {
      const updated = { ...projectModal.project, name, description, common_headers: commonHeaders, common_params: commonParams, common_body: commonBody };
      await invoke("api_update_project", { project: updated });
      setProjects(projects.map((p) => (p.id === updated.id ? updated : p)));
    } else {
      await invoke("api_create_project", { name, description });
      const list = await invoke<ApiProject[]>("api_list_projects");
      setProjects(list);
      setActiveProjectId(list[list.length - 1].id);
      // 新项目模板字段由后端默认空；若用户在创建时就填了模板，则再更新一次
      if (commonHeaders.length > 0 || commonParams.length > 0 || commonBody.length > 0) {
        const created = list[list.length - 1];
        await invoke("api_update_project", { project: { ...created, common_headers: commonHeaders, common_params: commonParams, common_body: commonBody } });
        setProjects(await invoke<ApiProject[]>("api_list_projects"));
      }
    }
    setTplFocusSection(null);
    setProjectModal({ open: false, project: null });
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
      title: "删除接口",
      message: `确定删除接口「${ep?.name ?? ""}」？其单测、压测记录将一并删除，且不可恢复。`,
      danger: true,
      confirmText: "删除",
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
      title: "删除模块",
      message: `确定删除模块「${mod?.name ?? ""}」？其下 ${count} 个接口将一并删除，且不可恢复。`,
      danger: true,
      confirmText: "删除",
      onConfirm: async () => {
        await invoke("api_delete_module", { moduleId: id });
        setModules(modules.filter((m) => m.id !== id));
        if (selectedId && endpoints.find((e) => e.id === selectedId)?.module_id === id) setSelectedId(null);
        refreshEndpoints();
      },
    });
  };

  const deleteProject = async (id: string) => {
    const p = projects.find((x) => x.id === id);
    setConfirm({
      title: "删除项目",
      message: `确定删除项目「${p?.name ?? ""}」？项目下所有模块、接口、变量集合与历史记录将一并删除，且不可恢复。`,
      danger: true,
      confirmText: "删除",
      onConfirm: async () => {
        await invoke("api_delete_project", { projectId: id });
        const list = projects.filter((x) => x.id !== id);
        setProjects(list);
        if (activeProjectId === id) setActiveProjectId(list[0]?.id ?? null);
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
        title: "移动完成",
        message: `已将 ${n} 个接口从「${moveModule.name}」转移到目标模块。`,
        confirmText: "知道了",
        onConfirm: () => setConfirm(null),
      });
    } catch (e) {
      setConfirm({
        title: "移动失败",
        message: String(e),
        danger: true,
        confirmText: "知道了",
        onConfirm: () => setConfirm(null),
      });
    }
  };

  const exportPostman = async () => {
    if (!activeProjectId) return;
    try {
      const json = await invoke<string>("api_export_postman", { projectId: activeProjectId });
      const target = await saveDialog({ defaultPath: "postman_collection.json", filters: [{ name: "JSON", extensions: ["json"] }] });
      if (target) {
        await invoke("write_text_file", { path: target, content: json });
      }
    } catch (e) {
      window.alert(String(e));
    }
  };

  // 渲染树（项目图标 + 模块文件夹图标 + 各方法图标）
  const renderTree = () => {
    const loose = endpoints.filter((e) => !e.module_id);
    const favs = endpoints.filter((e) => e.is_favorite);
    const row = (ep: ApiEndpoint) => (
      <EndpointRow
        key={ep.id}
        ep={ep}
        selected={selectedId === ep.id}
        onSelect={() => setSelectedId(ep.id)}
        onDelete={() => deleteEndpoint(ep.id)}
        onToggleFavorite={() => toggleFavorite(ep)}
      />
    );
    return (
      <div className="space-y-0.5">
        {favs.length > 0 && (
          <div className="mb-1">
            <div className="flex items-center gap-1 px-1.5 pt-1 pb-0.5">
              <Star className="w-3 h-3 text-amber-400 fill-amber-400" />
              <span className="text-[10px] font-semibold text-slate-500">收藏</span>
            </div>
            {favs.map(row)}
          </div>
        )}
        {modules.map((m) => {
          const children = endpoints.filter((e) => e.module_id === m.id);
          const isOpen = expanded.has(m.id);
          return (
            <div key={m.id}>
              <div className="group flex items-center gap-1 rounded-md px-1.5 py-1 hover:bg-white/5 cursor-pointer" onClick={() => toggleExpand(m.id)}>
                {isOpen ? <ChevronDown className="w-3.5 h-3.5 text-slate-500" /> : <ChevronRight className="w-3.5 h-3.5 text-slate-500" />}
                <Folder className="w-3.5 h-3.5 text-amber-400/80 shrink-0" />
                <span className="flex-1 text-xs text-slate-300 truncate">{m.name}</span>
                {m.description && <span className="text-[10px] text-slate-600 truncate max-w-28 hidden group-hover:block" title={m.description}>{m.description}</span>}
                <span className="text-[10px] px-1.5 py-px rounded-full bg-white/5 border border-white/5 text-slate-500 tabular-nums">{children.length}</span>
                <button
                  onClick={(e) => { e.stopPropagation(); openEditModule(m); }}
                  className="hidden group-hover:block p-0.5 text-slate-500 hover:text-[var(--module-accent)] cursor-pointer"
                  title="编辑模块"
                >
                  <Pencil className="w-3 h-3" />
                </button>
                <button
                  onClick={(e) => { e.stopPropagation(); createEndpoint(m.id); }}
                  className="hidden group-hover:block p-0.5 text-slate-500 hover:text-[var(--module-accent)] cursor-pointer"
                  title="新增接口"
                >
                  <Plus className="w-3 h-3" />
                </button>
                <button
                  onClick={(e) => { e.stopPropagation(); setMoveModule(m); }}
                  className="hidden group-hover:block p-0.5 text-slate-500 hover:text-[var(--module-accent)] cursor-pointer"
                  title="移动到其他模块"
                >
                  <FolderInput className="w-3 h-3" />
                </button>
                <button
                  onClick={(e) => { e.stopPropagation(); deleteModule(m.id); }}
                  className="hidden group-hover:block p-0.5 text-slate-500 hover:text-rose-400 cursor-pointer"
                  title="删除模块（其下接口一并删除）"
                >
                  <Trash2 className="w-3 h-3" />
                </button>
              </div>
              {/* 折叠动画容器（grid-rows 过渡） */}
              <div
                className="ml-4 grid transition-[grid-template-rows] duration-200 ease-out"
                style={{ gridTemplateRows: isOpen ? "1fr" : "0fr" }}
              >
                <div className="overflow-hidden min-h-0">
                  <div className="space-y-0.5 py-0.5">
                    {m.description && (
                      <div className="text-[10px] text-slate-500/90 leading-snug px-1.5 py-0.5 border-l border-white/10 ml-1">{m.description}</div>
                    )}
                    {children.map(row)}
                    {children.length === 0 && <div className="text-[10px] text-slate-600 px-2 py-0.5">（空模块）</div>}
                  </div>
                </div>
              </div>
            </div>
          );
        })}
        {loose.map(row)}
        {modules.length === 0 && endpoints.length === 0 && (
          <div className="text-[10px] text-slate-600 px-2 py-2">暂无接口，点击下方按钮新建</div>
        )}
      </div>
    );
  };

  const statusBadge = response ? (
    <span className={`text-xs font-bold px-2 py-0.5 rounded-md border ${response.status === 0 ? "text-rose-300 border-rose-500/40 bg-rose-500/10" : response.status < 300 ? "text-emerald-300 border-emerald-500/40 bg-emerald-500/10" : response.status < 500 ? "text-amber-300 border-amber-500/40 bg-amber-500/10" : "text-rose-300 border-rose-500/40 bg-rose-500/10"}`}>
      {response.status === 0 ? "错误" : response.status}
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
              <div className="relative flex-1 min-w-0">
                <button
                  onClick={() => { setProjectPop((v) => !v); setEnvPop(false); }}
                  className="flex w-full items-center gap-1.5 rounded-md border border-white/10 bg-black/30 px-2 py-1.5 text-xs text-slate-200 hover:border-white/25 cursor-pointer"
                  title="点击切换项目"
                >
                  <FlaskConical className="w-3.5 h-3.5 shrink-0" style={{ color: ACCENT }} />
                  <span className="flex-1 truncate text-left">{currentProject?.name ?? "选择项目"}</span>
                  <ChevronDown className="w-3 h-3 shrink-0 text-slate-500" />
                </button>
                {projectPop && (
                  <>
                    <div className="fixed inset-0 z-30" onClick={() => setProjectPop(false)} />
                    <div className="absolute left-0 top-full z-40 mt-1.5 w-56 overflow-hidden rounded-xl border border-white/10 shadow-2xl" style={{ background: "linear-gradient(160deg, rgba(13,21,36,0.99), rgba(13,21,36,0.95))" }}>
                      <div className="flex items-center justify-between border-b border-white/10 px-3 py-1.5">
                        <span className="text-[10px] font-semibold text-slate-500">API 项目</span>
                        <button onClick={openCreateProject} className="p-0.5 text-slate-500 hover:text-[var(--module-accent)] cursor-pointer" title="新建项目">
                          <Plus className="w-3 h-3" />
                        </button>
                      </div>
                      <div className="max-h-56 overflow-y-auto p-1 space-y-0.5">
                        {projects.length === 0 && <div className="px-2 py-1 text-[10px] text-slate-600">暂无项目</div>}
                        {projects.map((p) => (
                          <div
                            key={p.id}
                            onClick={() => { setActiveProjectId(p.id); setProjectPop(false); }}
                            className={`flex items-center gap-1.5 rounded-md px-2 py-1 cursor-pointer ${p.id === activeProjectId ? "bg-[color-mix(in_srgb,var(--module-accent)_15%,transparent)] text-white" : "text-slate-300 hover:bg-white/5"}`}
                          >
                            <FlaskConical className="w-3 h-3 shrink-0" style={{ color: p.id === activeProjectId ? ACCENT : undefined }} />
                            <span className="flex-1 text-xs truncate" title={p.description || undefined}>{p.name}</span>
                            {p.id === activeProjectId && (
                              <span className="rounded-full px-1.5 py-px text-[9px] font-semibold" style={{ background: "color-mix(in srgb, var(--module-accent) 25%, transparent)", color: ACCENT }}>当前</span>
                            )}
                          </div>
                        ))}
                      </div>
                    </div>
                  </>
                )}
              </div>
              {currentProject && (
                <>
                  <button onClick={() => openEditProject(currentProject)} className="p-1.5 text-slate-500 hover:text-[var(--module-accent)] cursor-pointer" title="编辑项目">
                    <Pencil className="w-3.5 h-3.5" />
                  </button>
                  <button onClick={() => deleteProject(currentProject.id)} className="p-1.5 text-slate-500 hover:text-rose-400 cursor-pointer" title="删除项目">
                    <Trash2 className="w-3.5 h-3.5" />
                  </button>
                </>
              )}
              <button onClick={openCreateProject} className="p-1.5 text-slate-500 hover:text-[var(--module-accent)] cursor-pointer" title="新建项目">
                <Plus className="w-3.5 h-3.5" />
              </button>
            </div>
          </div>

          {currentProject ? (
            <>
              {/* 环境切换（气泡）+ 变量维护 */}
              <div className="px-2 pt-2">
                <div className="flex items-center gap-1">
                  <div className="relative flex-1 min-w-0">
                    <button
                      onClick={() => { setEnvPop((v) => !v); setProjectPop(false); }}
                      className="flex w-full items-center gap-1.5 rounded-md border border-white/10 bg-black/30 px-2 py-1 text-[11px] text-slate-200 hover:border-white/25 cursor-pointer"
                      title="点击切换变量集合"
                    >
                      <Database className="w-3 h-3 shrink-0 text-slate-500" />
                      <span className="flex-1 truncate text-left">{activeEnv?.name ?? "无环境"}</span>
                      <ChevronDown className="w-3 h-3 shrink-0 text-slate-500" />
                    </button>
                    {envPop && (
                      <>
                        <div className="fixed inset-0 z-30" onClick={() => setEnvPop(false)} />
                        <div className="absolute left-0 top-full z-40 mt-1.5 min-w-full w-max max-w-[260px] overflow-hidden rounded-xl border border-white/10 shadow-2xl" style={{ background: "linear-gradient(160deg, rgba(13,21,36,0.99), rgba(13,21,36,0.95))" }}>
                          <div className="px-3 py-1.5 text-[10px] font-semibold text-slate-500">变量集合（环境）· 点击切换</div>
                          <div className="max-h-52 overflow-y-auto p-1 space-y-0.5">
                            {envs.length === 0 && <div className="px-2 py-1 text-[10px] text-slate-600">暂无环境，点「变量维护」新建</div>}
                            {envs.map((e) => (
                              <button
                                key={e.id}
                                onClick={() => { setActiveEnvId(e.id); setEnvPop(false); invoke("api_set_active_env", { projectId: activeProjectId, envId: e.id }); }}
                                className={`flex w-full items-center gap-1.5 rounded-md px-2 py-1 text-[11px] text-left cursor-pointer transition-colors ${e.id === activeEnvId ? "bg-[color-mix(in_srgb,var(--module-accent)_15%,transparent)] text-white" : "text-slate-300 hover:bg-white/5"}`}
                              >
                                {e.id === activeEnvId && <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: "var(--module-accent)" }} />}
                                <span className={`flex-1 truncate ${e.id === activeEnvId ? "" : "pl-3"}`}>{e.name}</span>
                              </button>
                            ))}
                          </div>
                        </div>
                      </>
                    )}
                  </div>
                  <button onClick={() => setEnvModal(true)} className="flex items-center gap-1 rounded-md border border-white/10 bg-black/30 px-1.5 py-1.5 text-[10px] text-slate-400 hover:text-white hover:border-white/25 cursor-pointer" title="管理变量集合">
                    <Settings2 className="w-3 h-3" />
                    变量维护
                  </button>
                </div>
              </div>

              {/* 接口 / 历史 选项卡（共享空间，各自主滚动，不会互相挤压） */}
              <div className="flex items-center gap-0.5 px-2 pt-2">
                <button
                  onClick={() => setSideTab("tree")}
                  className={`flex flex-1 items-center gap-1 rounded-md px-2 py-1 text-[11px] cursor-pointer transition-colors ${sideTab === "tree" ? "bg-[color-mix(in_srgb,var(--module-accent)_14%,transparent)] text-white" : "text-slate-400 hover:bg-white/5 hover:text-slate-300"}`}
                >
                  <Braces className="w-3 h-3" style={sideTab === "tree" ? { color: ACCENT } : undefined} />
                  接口
                  <span className="ml-auto text-[9px] tabular-nums text-slate-500">{endpoints.length}</span>
                </button>
                <button
                  onClick={() => setSideTab("history")}
                  className={`flex flex-1 items-center gap-1 rounded-md px-2 py-1 text-[11px] cursor-pointer transition-colors ${sideTab === "history" ? "bg-[color-mix(in_srgb,var(--module-accent)_14%,transparent)] text-white" : "text-slate-400 hover:bg-white/5 hover:text-slate-300"}`}
                >
                  <History className="w-3 h-3" style={sideTab === "history" ? { color: ACCENT } : undefined} />
                  历史
                  <span className="ml-auto text-[9px] tabular-nums text-slate-500">{history.length}</span>
                </button>
                {sideTab === "history" && history.length > 0 && (
                  <button
                    onClick={async () => {
                      if (!window.confirm("清空该项目全部请求历史？")) return;
                      await invoke("api_clear_history", { projectId: activeProjectId });
                      setHistory([]);
                    }}
                    className="p-1 text-slate-600 hover:text-rose-400 cursor-pointer"
                    title="清空历史"
                  >
                    <Eraser className="w-3 h-3" />
                  </button>
                )}
              </div>

              {/* 内容区：接口树 / 历史 */}
              <div className="flex-1 min-h-0 overflow-y-auto p-2 mt-1">
                {sideTab === "tree" ? (
                  renderTree()
                ) : (
                  <div className="space-y-0.5">
                    {history.slice(0, 50).map((h) => {
                      const Icon = methodIcon(h.method);
                      return (
                        <button
                          key={h.id}
                          onClick={() => replayHistory(h)}
                          className="group flex w-full items-center gap-1.5 rounded-md px-1.5 py-1 text-left hover:bg-white/5 cursor-pointer"
                          title={`${h.method} ${h.url}\n${h.created_at.replace("T", " ").slice(0, 19)}`}
                        >
                          <Icon className={`w-3 h-3 shrink-0 ${h.method === "GET" ? "text-emerald-400" : h.method === "POST" ? "text-amber-400" : h.method === "DELETE" ? "text-rose-400" : "text-slate-400"}`} />
                          <span className="flex-1 text-[11px] text-slate-400 truncate">{h.name || h.url}</span>
                        </button>
                      );
                    })}
                    {history.length === 0 && <div className="text-[10px] text-slate-600 px-1.5 py-1">发送请求后自动记录</div>}
                  </div>
                )}
              </div>

              {/* 底部工具栏（布局对齐思维导图：2×2 网格） */}
              <div className="p-2 border-t border-white/10 space-y-1.5">
                <div className="grid grid-cols-2 gap-1.5">
                  <button onClick={openCreateModule} className="flex items-center justify-center gap-1.5 px-2 py-1.5 text-[11px] text-slate-400 hover:text-white hover:bg-white/5 border border-white/10 rounded-md cursor-pointer" title="新建模块">
                    <FolderPlus className="w-3.5 h-3.5" /> 模块
                  </button>
                  <button onClick={() => createEndpoint(null)} className="flex items-center justify-center gap-1.5 px-2 py-1.5 text-[11px] text-slate-400 hover:text-white hover:bg-white/5 border border-white/10 rounded-md cursor-pointer" title="新建接口">
                    <FilePlus2 className="w-3.5 h-3.5" /> 接口
                  </button>
                </div>
                <div className="grid grid-cols-2 gap-1.5">
                  <button onClick={() => setImportModal(true)} className="flex items-center justify-center gap-1.5 px-2 py-1.5 text-[11px] text-slate-400 hover:text-white hover:bg-white/5 border border-white/10 rounded-md cursor-pointer" title="导入 (Postman/cURL)">
                    <Upload className="w-3.5 h-3.5" /> 导入
                  </button>
                  <button onClick={exportPostman} className="flex items-center justify-center gap-1.5 px-2 py-1.5 text-[11px] text-slate-400 hover:text-white hover:bg-white/5 border border-white/10 rounded-md cursor-pointer" title="导出 Postman 集合">
                    <Download className="w-3.5 h-3.5" /> 导出
                  </button>
                </div>
              </div>
            </>
          ) : (
            <div className="flex-1 flex items-center justify-center text-[11px] text-slate-600 px-4 text-center">
              暂无项目，点击上方「+」新建
            </div>
          )}

          {/* 宽度拖拽把手 + 收起按钮（侧边栏右侧） */}
          <div
            className="absolute -right-1 top-0 z-10 flex h-full w-2.5 cursor-col-resize items-center justify-center hover:bg-white/[0.06]"
            title="拖动调整宽度"
            onMouseDown={(e) => {
              if (e.button !== 0) return;
              e.preventDefault();
              sbResizeRef.current.moved = false;
              const startX = e.clientX; const startW = sidebarW;
              const onMove = (ev: MouseEvent) => { if (Math.abs(ev.clientX - startX) > 2) sbResizeRef.current.moved = true; setSidebarW(Math.min(460, Math.max(170, startW + (ev.clientX - startX)))); };
              const onUp = () => { window.removeEventListener("mousemove", onMove); window.removeEventListener("mouseup", onUp); };
              window.addEventListener("mousemove", onMove);
              window.addEventListener("mouseup", onUp);
            }}
          >
            <button
              type="button"
              className="flex h-6 w-2.5 items-center justify-center rounded-l bg-slate-800/80 text-slate-400 opacity-0 transition group-hover/sb:opacity-100 pointer-events-none group-hover/sb:pointer-events-auto hover:text-white cursor-pointer"
              title="收起侧栏"
              onClick={(e) => { e.stopPropagation(); if (!sbResizeRef.current.moved) setSidebarCollapsed(true); }}
            >
              <ChevronLeft className="h-3 w-3" />
            </button>
          </div>
        </aside>
      )}
      {sidebarCollapsed && (
        <button
          type="button"
          className="absolute left-2 top-2 z-20 inline-flex h-8 w-8 items-center justify-center rounded-md border border-white/10 bg-slate-900/90 text-slate-300 shadow-lg transition hover:text-white cursor-pointer"
          onClick={() => setSidebarCollapsed(false)}
          title="展开侧栏"
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
              className="bg-black/30 border border-white/10 rounded-md px-1.5 py-1.5 text-xs font-bold cursor-pointer focus:outline-none"
            >
              {METHODS.map((m) => <option key={m} value={m}>{m}</option>)}
            </select>
            <input
              value={draft.name}
              onChange={(e) => updateDraft({ name: e.target.value })}
              className="w-40 bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200 focus:outline-none"
              placeholder="接口名称"
            />
            <VarInput
              value={draft.url}
              envVars={variables}
              onChange={(v) => updateDraft({ url: v })}
              className="flex-1 bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]/60"
              placeholder="https://api.example.com/users/{{userId}}"
              onKeyDown={(e) => e.key === "Enter" && sendRequest()}
            />
            {tplItems.total > 0 && (
              <div className="relative">
                <button
                  onClick={() => setShowTplPanel((v) => !v)}
                  className={`flex items-center gap-1.5 rounded-md border px-2.5 py-1.5 text-xs cursor-pointer transition-colors ${showTplPanel ? "border-[var(--module-accent)]/50 bg-[color-mix(in_srgb,var(--module-accent)_12%,transparent)] text-[var(--module-accent)]" : "border-[color-mix(in_srgb,var(--module-accent)_35%,transparent)] bg-[color-mix(in_srgb,var(--module-accent)_8%,transparent)] text-[var(--module-accent)] hover:bg-[color-mix(in_srgb,var(--module-accent)_14%,transparent)]"} ${tplItems.outOfSync > 0 ? "!border-rose-500/60" : ""}`}
                  title={tplItems.outOfSync > 0 ? `${tplItems.outOfSync} 项模板参数已变化，重新加载接口后同步` : "点击查看继承自项目模板的参数"}
                >
                  <Link2 className="w-3.5 h-3.5" />
                  模板继承
                  <span
                    className="rounded-full px-1.5 py-px text-[10px] font-bold text-white tabular-nums"
                    style={{ background: tplItems.outOfSync > 0 ? "#f43f5e" : "var(--module-accent)" }}
                  >
                    {tplItems.total}
                  </span>
                  {tplItems.outOfSync > 0 && (
                    <span
                      className="absolute -right-1.5 -top-1.5 flex h-4 min-w-4 items-center justify-center rounded-full px-1 text-[9px] font-bold text-white shadow"
                      style={{ background: "#e11d48" }}
                      title={`${tplItems.outOfSync} 项待同步`}
                    >
                      {tplItems.outOfSync}
                    </span>
                  )}
                </button>
                {showTplPanel && (
                  <>
                    <div className="fixed inset-0 z-30" onClick={() => setShowTplPanel(false)} />
                    <div className="absolute right-0 top-full z-40 mt-1.5 w-80 overflow-hidden rounded-xl border border-white/10 shadow-2xl" style={{ background: "linear-gradient(160deg, rgba(13,21,36,0.99), rgba(13,21,36,0.95))" }}>
                      <div className="flex items-center justify-between border-b border-white/10 px-3 py-2">
                        <span className="flex items-center gap-1.5 text-[11px] font-semibold text-white">
                          <Link2 className="w-3 h-3" style={{ color: "var(--module-accent)" }} />
                          继承自项目模板的参数（只读）
                        </span>
                        <button onClick={() => setShowTplPanel(false)} className="p-0.5 text-slate-500 hover:text-white cursor-pointer">
                          <X className="w-3.5 h-3.5" />
                        </button>
                      </div>
                      <div className="max-h-64 overflow-y-auto p-2 space-y-2.5">
                        {tplItems.outOfSync > 0 && (
                          <div className="flex items-center gap-2 rounded-md border border-rose-500/40 bg-rose-500/10 px-2.5 py-2">
                            <AlertTriangle className="w-3.5 h-3.5 shrink-0 text-rose-400" />
                            <span className="flex-1 text-[10px] leading-snug text-rose-300">
                              项目模板已修改，{tplItems.outOfSync} 项尚未同步到本接口。
                            </span>
                            <button
                              onClick={reloadCurrent}
                              className="shrink-0 rounded-md px-2 py-1 text-[10px] font-semibold text-white cursor-pointer hover:opacity-85"
                              style={{ background: "#e11d48" }}
                            >
                              重新加载
                            </button>
                          </div>
                        )}
                        {tplItems.groups.map((g) => (
                          <div key={g.label}>
                            <div className="px-1 pb-1 text-[9px] font-semibold uppercase tracking-wider text-slate-500">{g.label}</div>
                            <div className="space-y-1">
                              {g.items.map((it, i) => (
                                <div key={i} className={`flex items-center gap-1.5 rounded-md border px-2 py-1 ${it.synced ? "border-white/5 bg-black/25" : "border-rose-500/40 bg-rose-500/10"}`}>
                                  <Lock className={`w-3 h-3 shrink-0 ${it.synced ? "text-[var(--module-accent)]" : "text-rose-400"}`} />
                                  <code className="font-mono text-[10px] text-slate-200 truncate">{it.key}</code>
                                  <span className="text-[10px] text-slate-500">=</span>
                                  <code className={`font-mono text-[10px] truncate ${it.synced ? "text-[var(--module-accent)]/90" : "text-rose-300"}`} title={it.value}>{it.value || "（空）"}</code>
                                  {!it.synced && <span className="ml-auto shrink-0 text-[9px] font-semibold text-rose-400">待同步</span>}
                                </div>
                              ))}
                            </div>
                          </div>
                        ))}
                        <div className="px-1 pt-1 space-y-1.5">
                          <button
                            onClick={openProjectTpl}
                            className="flex w-full items-center justify-center gap-1.5 rounded-lg border border-white/10 bg-white/5 px-2 py-1.5 text-[10px] font-semibold text-slate-200 hover:bg-white/10 hover:text-white cursor-pointer transition-colors"
                          >
                            <Settings2 className="w-3 h-3" style={{ color: "var(--module-accent)" }} />
                            打开项目模板
                          </button>
                          <div className="text-center text-[9px] text-slate-500">在模板中修改后，所有接口自动同步。</div>
                        </div>
                      </div>
                    </div>
                  </>
                )}
              </div>
            )}
            <button
              onClick={sendRequest}
              disabled={sending}
              className="flex items-center gap-1.5 px-4 py-1.5 rounded-md text-xs font-semibold text-white cursor-pointer disabled:opacity-50"
              style={{ background: ACCENT }}
            >
              {sending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Send className="w-3.5 h-3.5" />}
              {sending ? "发送中" : "发送"}
            </button>
            <button onClick={() => saveDraft()} className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
              <Save className="w-3.5 h-3.5" /> 保存
            </button>
            {selectedId && (
              <button
                onClick={() => toggleFavorite(draft)}
                title={draft.is_favorite ? "取消收藏" : "收藏此接口"}
                className={`flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-xs cursor-pointer ${draft.is_favorite ? "text-amber-400 bg-amber-500/10" : "text-slate-400 bg-white/5 hover:bg-white/10"}`}
              >
                <Star className={`w-3.5 h-3.5 ${draft.is_favorite ? "fill-amber-400" : ""}`} /> {draft.is_favorite ? "已收藏" : "收藏"}
              </button>
            )}
            <button onClick={saveAsDoc} title="把当前请求参数 + 返回结果保存为 Markdown 文档" className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-xs bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
              <BookOpen className="w-3.5 h-3.5" /> 存为文档
            </button>
            <button onClick={saveAsTest} title="根据当前请求与返回结果自动生成单测断言" className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-xs bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
              <TestTube2 className="w-3.5 h-3.5" /> 存为单测
            </button>
          </div>

          {/* 层级说明：模块简介 + 接口简介 */}
          <div className="flex items-center gap-2 px-3 py-1.5 border-b border-white/5 bg-black/10">
            {(() => {
              const mod = modules.find((m) => m.id === draft.module_id);
              return (
                <>
                  {mod && (
                    <div className="flex items-center gap-1 shrink-0 text-[10px] text-amber-300/80">
                      <Folder className="w-3 h-3" />
                      <span className="font-semibold">{mod.name}</span>
                      {mod.description && <span className="text-slate-500 max-w-40 truncate" title={mod.description}>· {mod.description}</span>}
                    </div>
                  )}
                  <input
                    value={draft.description}
                    onChange={(e) => updateDraft({ description: e.target.value })}
                    placeholder="接口简介：这个接口做什么？用途、注意事项…（随保存持久化）"
                    className="flex-1 min-w-0 bg-transparent border border-transparent hover:border-white/10 focus:border-[var(--module-accent)]/40 rounded-md px-2 py-1 text-[11px] text-slate-300 placeholder:text-slate-600 focus:outline-none"
                  />
                </>
              );
            })()}
          </div>

          {/* 功能页签 */}
          <div className="flex items-center gap-1 px-3 pt-1.5">
            {([
              ["request", "请求", Send],
              ["tests", "单测", ListChecks],
              ["load", "压测", Gauge],
              ["docs", "文档", BookOpen],
            ] as const).map(([key, label, Icon]) => (
              <button
                key={key}
                onClick={() => setActiveTab(key)}
                className={`flex items-center gap-1.5 px-3 py-1.5 rounded-t-lg text-xs cursor-pointer border-b-2 ${activeTab === key ? "text-white border-[var(--module-accent)] bg-white/5" : "text-slate-500 border-transparent hover:text-slate-300"}`}
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
                    ["settings", "设置", SlidersHorizontal],
                    ["cookies", "Cookies", Cookie],
                  ] as const).map(([key, label, Icon]) => (
                    <button
                      key={key}
                      onClick={() => setSubTab(key)}
                      className={`flex items-center gap-1 px-2.5 py-1 text-[11px] rounded-md cursor-pointer ${subTab === key ? "bg-white/10 text-white" : "text-slate-500 hover:text-slate-300"}`}
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
                    placeholderValue="值（支持 {{baseUrl}} / {{$guid}}）"
                  />
                )}
                {subTab === "auth" && (
                  <AuthPanel auth={draft.authorization} onChange={(a) => updateDraft({ authorization: a })} />
                )}
                {subTab === "headers" && (
                  <div className="space-y-2">
                    <div className="flex items-center gap-2 flex-wrap">
                      <label className="flex items-center gap-1.5 text-[10px] text-slate-400 cursor-pointer">
                        <input type="checkbox" checked={hideCommonHeaders} onChange={(e) => setHideCommonHeaders(e.target.checked)} className="accent-[var(--module-accent)]" />
                        隐藏常见自动附加头（Content-Type/Length、Host、User-Agent、Accept…）
                      </label>
                      <select
                        value=""
                        onChange={(e) => { if (e.target.value) applyPreset(e.target.value); }}
                        className="bg-black/30 border border-white/10 rounded-md px-1.5 py-1 text-[10px] text-slate-300 cursor-pointer"
                      >
                        <option value="">应用预设 Headers…</option>
                        {presetSets.map((s) => <option key={s.id} value={s.id}>{s.name}</option>)}
                      </select>
                      <button onClick={() => setPresetModal(true)} className="flex items-center gap-1 text-[10px] px-2 py-1 rounded bg-white/5 hover:bg-white/10 text-slate-400 cursor-pointer">
                        <Settings2 className="w-3 h-3" /> 管理预设
                      </button>
                    </div>
                    <KvEditor
                      items={hideCommonHeaders ? draft.headers.filter((h) => !COMMON_AUTO_HEADERS.includes(h.key.toLowerCase())) : draft.headers}
                      onChange={(v) => {
                        // 隐藏模式下把被隐藏的常见头合并回去
                        if (hideCommonHeaders) {
                          const hidden = draft.headers.filter((h) => COMMON_AUTO_HEADERS.includes(h.key.toLowerCase()));
                          updateDraft({ headers: [...v, ...hidden] });
                        } else {
                          updateDraft({ headers: v });
                        }
                      }}
                      placeholderKey="Header 名"
                      envVars={variables}
                      placeholderValue="值（如 Bearer {{token}}）"
                    />
                  </div>
                )}
                {subTab === "body" && (
                  <div className="space-y-2">
                    <div className="flex gap-1 flex-wrap">
                      {BODY_TYPES.map((b) => (
                        <button
                          key={b.value}
                          onClick={() => updateDraft({ body_type: b.value })}
                          className={`px-2.5 py-1 text-[11px] rounded-md cursor-pointer ${draft.body_type === b.value ? "bg-white/10 text-white" : "text-slate-500 hover:text-slate-300"}`}
                        >
                          {b.label}
                        </button>
                      ))}
                    </div>
                    {draft.body_type === "formdata" && (
                      <FormDataEditor items={draft.body_form} onChange={(v) => updateDraft({ body_form: v })} envVars={variables} />
                    )}
                    {draft.body_type === "form" && (
                      <KvEditor items={draft.body_urlencoded} onChange={(v) => updateDraft({ body_urlencoded: v })} envVars={variables} placeholderValue="值" />
                    )}
                    {draft.body_type === "graphql" && (
                      <div className="space-y-2">
                        <VarInput
                          multiline
                          rows={6}
                          value={draft.body_graphql_query}
                          envVars={variables}
                          onChange={(v) => updateDraft({ body_graphql_query: v })}
                          placeholder={"query GetUser($id: ID!) {\n  user(id: $id) { id name }\n}"}
                          className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs font-mono text-slate-200 focus:outline-none"
                        />
                        <VarInput
                          multiline
                          rows={3}
                          value={draft.body_graphql_variables}
                          envVars={variables}
                          onChange={(v) => updateDraft({ body_graphql_variables: v })}
                          placeholder='{"id": "{{random:int:1:100}}"}'
                          className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs font-mono text-slate-200 focus:outline-none"
                        />
                      </div>
                    )}
                    {draft.body_type === "binary" && (
                      <div className="space-y-1.5">
                        <input
                          value={draft.body}
                          onChange={(e) => updateDraft({ body: e.target.value })}
                          placeholder="本地文件路径"
                          className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200"
                        />
                        <button
                          onClick={async () => {
                            const f = await openDialog({ multiple: false });
                            if (f) updateDraft({ body: String(f) });
                          }}
                          className="text-[11px] px-2 py-1 rounded bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer"
                        >
                          选择文件
                        </button>
                      </div>
                    )}
                    {(draft.body_type === "raw" || draft.body_type === "json") && (
                      <VarInput
                        multiline
                        rows={7}
                        value={draft.body}
                        envVars={variables}
                        onChange={(v) => updateDraft({ body: v })}
                        className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs font-mono text-slate-200 focus:outline-none focus:border-[var(--module-accent)]/60"
                        placeholder={draft.body_type === "json" ? '{"name": "{{random:string:6}}", "age": {{random:int:18:60}}}' : "原始内容"}
                      />
                    )}
                  </div>
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
                    <KvEditor items={draft.cookies} onChange={(v) => updateDraft({ cookies: v })} envVars={variables} placeholderKey="Cookie 名" placeholderValue="值" />
                    <div className="text-[10px] text-slate-500">独立设置的 Cookie 会随请求发送。</div>
                  </div>
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

            {activeTab === "load" && (
              <div className="space-y-3">
                <div className="grid grid-cols-4 gap-2">
                  {([
                    ["concurrency", "并发数", loadConfig.concurrency, (v: number) => setLoadConfig({ ...loadConfig, concurrency: v })],
                    ["duration_secs", "时长(秒)", loadConfig.duration_secs, (v: number) => setLoadConfig({ ...loadConfig, duration_secs: v })],
                    ["ramp_up_secs", "Ramp-up(秒)", loadConfig.ramp_up_secs, (v: number) => setLoadConfig({ ...loadConfig, ramp_up_secs: v })],
                    ["rps_limit", "RPS 上限(0=不限)", loadConfig.rps_limit, (v: number) => setLoadConfig({ ...loadConfig, rps_limit: v })],
                  ] as const).map(([key, label, value, set]) => (
                    <label key={key} className="block">
                      <span className="text-[10px] text-slate-500">{label}</span>
                      <input
                        type="number"
                        min={0}
                        value={value}
                        onChange={(e) => set(Number(e.target.value))}
                        className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200 focus:outline-none"
                      />
                    </label>
                  ))}
                </div>
                <div className="text-[10px] text-slate-500">压测使用当前请求配置（含认证/Cookie/随机变量，每次请求重新生成随机值）。</div>
                <div className="flex items-center gap-2">
                  <button
                    onClick={startLoadTest}
                    disabled={!!runningRunId || !draft.url}
                    className="flex items-center gap-1.5 px-4 py-1.5 rounded-md text-xs font-semibold text-white cursor-pointer disabled:opacity-50"
                    style={{ background: ACCENT }}
                  >
                    <Play className="w-3.5 h-3.5" />
                    {runningRunId ? "压测进行中…" : "开始压测"}
                  </button>
                  {runningRunId && loadStatus && (
                    <div className="flex-1 text-[11px] text-slate-300">
                      已运行 {loadStatus.elapsed_secs}s · 请求 {loadStatus.total} · 成功 {loadStatus.success} · 失败 {loadStatus.failed} · QPS {loadStatus.qps.toFixed(1)} · p95 {loadStatus.latency_p95_ms.toFixed(1)}ms
                    </div>
                  )}
                </div>
                {!runningRunId && loadStatus?.report && <LoadReportView report={loadStatus.report} />}
                {runningRunId && <div className="h-1.5 rounded-full bg-white/10 overflow-hidden"><div className="h-full bg-[var(--module-accent)]" style={{ width: `${Math.min(100, (loadStatus?.elapsed_secs ?? 0) / Math.max(1, loadConfig.duration_secs) * 100)}%` }} /></div>}
                {loadRuns.length > 0 && !runningRunId && (
                  <div className="space-y-1.5">
                    <div className="text-[11px] font-semibold text-slate-400">历史压测报告</div>
                    {loadRuns.map((run) => (
                      <div key={run.id} className="rounded-lg border border-white/10 bg-black/20 px-3 py-2">
                        <div className="flex items-center justify-between">
                          <div className="flex items-center gap-2 text-xs text-slate-300">
                            <span className="font-semibold">{run.name || draft.name}</span>
                            <span className="text-[10px] text-slate-500">{run.created_at.replace("T", " ").slice(0, 19)}</span>
                            <span className="text-[10px] text-slate-500">并发 {run.config.concurrency} · {run.config.duration_secs}s</span>
                          </div>
                          <button
                            onClick={async () => {
                              await invoke("api_delete_load_run", { runId: run.id });
                              setLoadRuns(loadRuns.filter((r) => r.id !== run.id));
                            }}
                            className="p-1 text-slate-600 hover:text-rose-400 cursor-pointer"
                          >
                            <Trash2 className="w-3.5 h-3.5" />
                          </button>
                        </div>
                        {run.report && <LoadReportView report={run.report} />}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}

            {activeTab === "docs" && (
              <DocsPanel draft={draft} onSave={(md) => saveDraft({ docs_md: md })} />
            )}
          </div>

          {/* 响应区 */}
          <div className="shrink-0 h-64 border-t border-white/10 flex flex-col">
            <div className="flex items-center gap-2 px-3 py-1.5 border-b border-white/10">
              <span className="text-[11px] font-semibold text-slate-400">响应</span>
              {statusBadge}                  {response && (
                    <>
                      <span className="text-[10px] text-slate-500">{fmtTime(response.time_ms)}</span>
                  <span className="text-[10px] text-slate-500">{response.size_bytes > 1024 * 1024 ? `${(response.size_bytes / 1024 / 1024).toFixed(1)}MB` : `${(response.size_bytes / 1024).toFixed(1)}KB`}</span>
                  {response.body_truncated && <span className="text-[10px] text-amber-400">Body 已截断（2MB 上限）</span>}
                  <div className="ml-auto flex items-center gap-2">
                    <div className="flex gap-0.5 bg-black/30 rounded p-0.5">
                      {(["pretty", "raw"] as const).map((m) => (
                        <button
                          key={m}
                          onClick={() => setBodyMode(m)}
                          className={`px-1.5 py-0.5 rounded text-[10px] cursor-pointer ${bodyMode === m ? "bg-white/10 text-cyan-300" : "text-slate-500 hover:text-slate-300"}`}
                        >
                          {m === "pretty" ? "美化" : "原始"}
                        </button>
                      ))}
                    </div>
                    <button onClick={copyBody} className="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded bg-white/5 hover:bg-white/10 text-slate-400 cursor-pointer">
                      {copied ? <Check className="w-3 h-3 text-emerald-400" /> : <Copy className="w-3 h-3" />} {copied ? "已复制" : "复制 Body"}
                    </button>
                  </div>
                </>
              )}
            </div>
            <div className="flex-1 min-h-0 grid grid-cols-[minmax(180px,24%)_1fr_220px]">
              <div className="border-r border-white/10 overflow-y-auto p-2">
                {response?.headers.map((h, i) => (
                  <div key={i} className="flex text-[10px] py-0.5">
                    <span className="w-1/2 text-slate-500 truncate">{h.key}</span>
                    <span className="w-1/2 text-slate-300 truncate" title={h.value}>{h.value}</span>
                  </div>
                ))}
                {response && response.headers.length === 0 && <div className="text-[10px] text-slate-600">无响应头</div>}
              </div>
              {sending ? (
                <div className="flex flex-col items-center justify-center h-full gap-2 text-slate-500 select-none">
                  <Loader2 className="w-5 h-5 animate-spin text-cyan-400" />
                  <span className="text-[11px]">请求发送中…</span>
                </div>
              ) : response ? (
                <ResponseBody body={response.body} mode={bodyMode} />
              ) : (
                <div className="h-full flex flex-col items-center justify-center text-slate-600 gap-2 select-none">
                  <Send className="w-6 h-6 opacity-40" />
                  <span className="text-[11px]">发送请求后在此查看响应</span>
                </div>
              )}
              {/* 响应注释 */}
              <div className="border-l border-white/10 flex flex-col">
                <div className="flex items-center gap-1 px-2 py-1 border-b border-white/10">
                  <StickyNote className="w-3 h-3 text-slate-500" />
                  <span className="text-[10px] text-slate-500">响应注释</span>
                  <button onClick={saveComment} className="ml-auto text-[10px] px-1.5 py-0.5 rounded bg-white/5 hover:bg-white/10 text-slate-400 cursor-pointer">保存</button>
                </div>
                <textarea
                  value={commentDraft}
                  onChange={(e) => setCommentDraft(e.target.value)}
                  placeholder="记录本次返回结果的含义、注意事项…"
                  className="flex-1 bg-transparent p-2 text-[11px] text-slate-300 resize-none focus:outline-none"
                />
              </div>
            </div>
          </div>
        </div>
      ) : (
        <div className="flex-1 flex items-center justify-center text-sm text-slate-500">
          <div className="text-center space-y-2">
            <FlaskConical className="w-10 h-10 mx-auto opacity-40" />
            <p>选择左侧接口开始调试，或新建项目/接口</p>
            <p className="text-[10px] text-slate-600">支持 {'{{\"变量名\"}}'}、{'{{$guid}}'} 等随机变量 · 单测断言 · 并发压测 · Markdown 文档 · Postman 导入导出</p>
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
          onImported={async () => {
            setImportModal(false);
            refreshEndpoints();
            const [modList, envList, presetList] = await Promise.all([
              invoke<ApiModule[]>("api_list_modules", { projectId: activeProjectId }),
              invoke<ApiEnvironment[]>("api_list_environments", { projectId: activeProjectId }),
              invoke<PresetHeaderSet[]>("api_list_preset_headers", { projectId: activeProjectId }),
            ]);
            setModules(modList);
            setEnvs(envList);
            setPresetSets(presetList);
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
