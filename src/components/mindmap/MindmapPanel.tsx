import { useMindmapHistory } from "./useMindmapHistory";
import { useMindmapDocumentLibrary } from "./useMindmapDocumentLibrary";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save } from "@tauri-apps/plugin-dialog";
import "@xyflow/react/dist/style.css";
import VexEmptyState from "../VexEmptyState";
import { Brain, Folder, Trash2, X, Plus, Pencil, ChevronRight, ChevronLeft, FolderPlus, Search, Minimize2, FileText, FileDown } from "lucide-react";
import type { AiConfig } from "../../contracts/ai";
import { AgentSessionRow, AiImportResult, DocumentFull, mmApi } from "./types";
import { useEventBufferSnapshot } from "../../utils/eventBuffer";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import { ModuleSettingsButton } from "../shared/ModuleSettings";
import { MindmapModuleSettings, type ExplorerSettings } from "./MindmapSettings";
import { AgentWorkbench, clearAnsweredAsks, clearAgentMessages, mmAiProgressBuffer, patchAgentTurn, pushAgentMessage } from "./agentShared";
import { partitionAgentOps, type AgentOp } from "./types";
import { usePaneWidth } from "./paneWidths";
import { ACCENT, DOC_SOURCE_ICONS, MM_LAST_DOC_KEY, button, formatTime, handledAgentOpsAt } from "./MindmapPanel.model";
import { Canvas } from "./MindmapPanel.Canvas";
import { AiRunningPill } from "./MindmapPanel.AiRunningPill";
import { CreateDocModal } from "./MindmapPanel.CreateDocModal";
import { ConfirmModal } from "./MindmapPanel.ConfirmModal";
import { AiImportReportModal } from "./MindmapPanel.AiImportReportModal";

export default function MindmapPanel() {
  const { t } = useTranslation();
  const [config, setConfig] = useState<AiConfig | null>(null);
  const [providerId, setProviderId] = useState<string>("");
  const [modelId, setModelId] = useState<string>("");
  // 记录「上次使用的模型」：思维导图不提供显式默认模型设置，选择即记住
  // （存 mindmap_settings.json；读-改-写以保留探索预算等其它字段）
  const rememberModel = useCallback(async (pid: string, mid: string) => {
    try {
      const cur = await invoke<ExplorerSettings>("mm_get_explorer_settings");
      await invoke("mm_save_explorer_settings", {
        settings: { ...cur, lastProviderId: pid, lastModelId: mid },
      });
    } catch (e) {
      console.error("记录思维导图模型失败:", e);
    }
  }, []);
  const [full, setFull] = useState<DocumentFull | null>(null);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const [showAi, setShowAi] = useState(false);
  // 对话是主模式（设计目标：AI 成为导图 Agent），导入两模式保留为面板内入口
  const [aiMode, setAiMode] = useState<"project" | "text" | "chat">("chat");
  const [targetDocumentId, setTargetDocumentId] = useState<string>("");
  const [textTitle, setTextTitle] = useState("");
  const [projectPath, setProjectPath] = useState("");
  // 最近一次 AI 运行结果：工作台结果卡展示（与校验报告弹窗 aiReport 分离——
  // 结果卡常驻工作台，报告弹窗仅在用户点「查看校验报告」时打开）
  const [lastAiResult, setLastAiResult] = useState<AiImportResult | null>(null);
  // AI 项目导入产物选项：深度（1 最浅清单 → 5 最深业务流）+ 视图开关（空 = AI 自动判断）
  const [aiDepth, setAiDepth] = useState(3);
  const [aiViews, setAiViews] = useState<string[]>([]);
  const aiHasRunRef = useRef(false);
  const [aiLoading, setAiLoading] = useState(false);
  const aiRunIdRef = useRef<string | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  // 第二栏（节点树）显隐：与第一栏共用一次操作 —— 第一栏与第二栏是一对「侧栏」，
  // 收起/展开本就应该同时发生（此前是两个组件里的独立 state，各自为政）
  const [treeOpen, setTreeOpen] = useState(true);
  /** 一次操作同时显隐第一栏与第二栏（两栏的头尾按钮、画布胶囊都走这里） */
  const setSidePanesOpen = (open: boolean) => { setSidebarCollapsed(!open); setTreeOpen(open); };
  const [sidebarW, setSidebarW] = usePaneWidth("sidebar");
  // 右栏 AI 对话面板宽度（与左栏把手同一套拖拽逻辑，方向相反）
  const [aiPanelW, setAiPanelW] = usePaneWidth("ai");
  // Agent 的写操作全部直接应用（不再有待确认清单）：回退走 Ctrl+Z 撤销快照
  // 当前文档的 Agent 会话（一个文档可有多个会话，各自独立历史）
  // ref 供异步回调读取最新值，state 供 UI 渲染
  const agentSessionRef = useRef<string | null>(null);
  const [agentSessionId, setAgentSessionId] = useState<string>("");
  const [agentSessions, setAgentSessions] = useState<AgentSessionRow[]>([]);
  /** 同时更新 ref 与 state（runAgentChat 等异步回调读 ref，UI 读 state） */
  const useAgentSession = useCallback((id: string) => {
    agentSessionRef.current = id || null;
    setAgentSessionId(id);
  }, []);

  /** 回放某会话的历史（带落库 id：用于「从这条分叉」定位分叉点） */
  const replaySession = useCallback(async (sid: string) => {
    const msgs = await mmApi.agentListMessages(sid);
    clearAgentMessages();
    for (const m of msgs) {
      if (m.content.trim()) pushAgentMessage(m.role === "user" ? "user" : "agent", m.content, m.id);
    }
  }, []);

  /** 重新拉取会话列表（新建/删除/重命名/分叉后刷新）；pick 非空时顺带切换当前会话 */
  const refreshSessions = useCallback(async (docId: string, pick?: string) => {
    const list = await mmApi.agentListSessions(docId);
    setAgentSessions(list);
    if (pick) useAgentSession(pick);
    return list;
  }, [useAgentSession]);
  // 绑定目录下的文件清单（@ 引用候选）
  const [projectFiles, setProjectFiles] = useState<string[]>([]);
  // 本轮对话 @ 引用的文件：模型生成的节点若没带 sources，用它兜底锚定
  // （「分析这个文件并画图」时，用户引用了谁就该锚谁，不能让节点凭空出现没有出处）
  const agentAttachmentsRef = useRef<string[]>([]);
  // AI 导入校验报告弹窗（导入完成后展示节点数/修复轮数/残留诊断）
  const [aiReport, setAiReport] = useState<AiImportResult | null>(null);
  // 侧栏宽度拖拽期间是否发生了位移（用于区分「点击收起」与「拖动调宽」）
  const sbResizeRef = useRef<{ moved: boolean }>({ moved: false });
  // AI 弹框关闭拦截：任务进行中先询问（确认后停止任务再关闭）
  const [aiCloseGuard, setAiCloseGuard] = useState(false);
  // AI 弹框最小化：任务后台继续跑，画布上只留一个可恢复的悬浮图标。
  // 关闭规则不变（仍只能经 requestCloseAi），最小化只是把弹框从视口挪开。
  const [aiMinimized, setAiMinimized] = useState(false);
  // 订阅模块级 AI 进度缓冲：驱动「边生成边绘制」effect 在每个 view_done 到达时触发
  const aiProgress = useEventBufferSnapshot(mmAiProgressBuffer);

  const flash = useCallback((m: string) => { setNotice(m); window.setTimeout(() => setNotice(""), 2600); }, []);

  // 稳定引用：避免每次父组件渲染都新建函数，导致 CanvasInner 的
  // flowNodes useMemo 反复重建、setNodes 连环触发造成卡顿。
  const {
    docs,
    setDocs,
    folders,
    setFolders,
    activeFolderId,
    setActiveFolderId,
    dragOverFolderId,
    ghost,
    showCreate,
    setShowCreate,
    showFolderCreate,
    setShowFolderCreate,
    editingFolder,
    setEditingFolder,
    folderName,
    setFolderName,
    renamingDocId,
    setRenamingDocId,
    renameDocName,
    setRenameDocName,
    search,
    setSearch,
    confirmState,
    setConfirmState,
    refreshFolders,
    createDoc,
    removeDoc,
    createFolder,
    updateFolder,
    startRenameDoc,
    saveRenameDoc,
    moveDoc,
    startDragHandle,
    renderFolderNodes,
    filteredDocs,
    treeRoots,
    folderPath,
  } = useMindmapDocumentLibrary({
    setFull,
    flash,
  });

  const onDocumentUpdated = useCallback((d: DocumentFull) => {
    setFull(d);
    setDocs(prev => prev.map(x => x.id === d.document.id ? d.document : x));
  }, []);

  // ── 撤销/重做（Ctrl+Z / Ctrl+Y）──
  // 快照式历史：每次离散操作前把当前 DocumentFull 压入撤销栈；撤销/重做时恢复快照并写回后端。
  const {
    historyVersion,
    commitHistory,
  } = useMindmapHistory({
    full,
    setFull,
    setDocs,
    setError,
  });

  useEffect(() => {
    void mmApi.init().then(async () => {
      const [ld, lf] = await Promise.all([mmApi.list(), mmApi.listFolders()]);
      setDocs(ld); setFolders(lf);
      // 挂载时恢复上次打开的思维导图（模块卸载重挂载后仍停留在原文档）
      try {
        const last = localStorage.getItem(MM_LAST_DOC_KEY);
        if (last && ld.some(d => d.id === last)) {
          const f = await mmApi.load(last);
          if (f) setFull(f);
        }
      } catch { /* 忽略恢复失败 */ }
    }).catch(() => {});
    // AI 供应商/模型预填：优先导图自己「上次使用的模型」，其次全局默认 AI 模型
    // （翻译模块的默认），失效或未设置时回退第一个可用供应商。
    void Promise.all([
      invoke<AiConfig>("get_ai_config"),
      invoke<ExplorerSettings>("mm_get_explorer_settings").catch(() => null),
      invoke<{ providerId: string | null; modelId: string | null }>("get_translate_config").catch(() => ({ providerId: null, modelId: null })),
    ]).then(([cfg, mmSet, gDef]) => {
      setConfig(cfg);
      const usable = cfg.providers.filter(x => x.api_key && x.openai_url);
      // 供应商优先级：导图上次用的 > 全局默认 > 首个可用 > 第一个
      const wantPid = mmSet?.lastProviderId || gDef.providerId;
      const p = (wantPid && cfg.providers.find(x => x.id === wantPid && x.api_key && x.openai_url))
        ?? usable[0]
        ?? cfg.providers[0];
      if (!p) return;
      setProviderId(p.id);
      // 模型优先级：导图上次用的（需同属该供应商）> 全局默认 > 供应商激活模型 > 第一个
      const wantMid = p.id === mmSet?.lastProviderId ? mmSet?.lastModelId : null;
      const mid = (wantMid && p.models.some(m => m.id === wantMid) ? wantMid : null)
        ?? (gDef.modelId && p.models.some(m => m.id === gDef.modelId) ? gDef.modelId : null)
        ?? p.active_model_id ?? p.models[0]?.id ?? "";
      setModelId(mid);
    }).catch(() => setError(t("mindmap.aiCfgFail")));
  }, []);

  // 当前文档变化时持久化 id，供挂载恢复
  useEffect(() => {
    if (full?.document.id) {
      try { localStorage.setItem(MM_LAST_DOC_KEY, full.document.id); } catch { /* 忽略 */ }
    }
  }, [full?.document.id]);

  const providers = useMemo(() => (config?.providers ?? []).filter(p => p.api_key && p.openai_url), [config]);

  // 统一 AI 入口：项目/文本是工作台内的任务类型，不再拆成两个入口。
  const openAiImport = useCallback(async () => {
    if (!full) {
      try {
        const doc = await mmApi.create({ name: t("mindmap.aiTextDoc"), description: "", sourceType: "ai_text", folderId: null });
        setDocs(prev => [doc, ...prev]);
        const f = await mmApi.load(doc.id);
        if (f) { setFull(f); setTargetDocumentId(f.document.id); }
        void refreshFolders();
      } catch (e) { flash(String(e)); return; }
    } else {
      setTargetDocumentId(full.document.id);
    }
    setAiMinimized(false);
    setShowAi(true);
  }, [full, flash, refreshFolders]);

  const selectAiTarget = useCallback(async (id: string) => {
    setTargetDocumentId(id);
    if (!id || full?.document.id === id) return;
    try {
      const f = await mmApi.load(id);
      if (f) setFull(f);
      else flash(t("mindmap.docNotFound"));
    } catch (e) { setError(String(e)); }
  }, [full?.document.id, flash]);

  const loadDocument = useCallback(async (id: string) => {
    setError("");
    try {
      const f = await mmApi.load(id);
      if (f) { setFull(f); setTargetDocumentId(id); } else { flash(t("mindmap.docNotFound")); }
    } catch (e) { setError(String(e)); }
  }, [flash]);
  // 从校验报告弹窗跳转到对应视图文档
  const openReportDoc = useCallback(async (id: string) => {
    setAiReport(null);
    setTargetDocumentId(id);
    try {
      const f = await mmApi.load(id);
      if (f) setFull(f); else flash(t("mindmap.docNotFound"));
    } catch (e) { setError(String(e)); }
  }, [flash]);

  // 应用 AI 类型路由导入结果：切到主文档，把新生成的视图文档并入侧栏列表。
  // （运行期间已通过 view_done 事件增量绘制过视图：这里避免把当前画布切回旧文档，
  //   只补齐尚未加载的新文档到侧栏。）
  const applyAiImport = useCallback((r: AiImportResult) => {
    setDocs(prev => {
      const existing = new Set(prev.map(d => d.id));
      const fresh = r.documents.map(d => d.document).filter(d => !existing.has(d.id));
      return [...fresh, ...prev];
    });
    const names = r.documents.map(d => d.document.name).join("、");
    const failTxt = r.failures.length ? t("mindmap.viewFailures", { count: r.failures.length, names: r.failures.map(f => f.view).join("、") }) : "";
    flash(t("mindmap.viewsGenerated", { count: r.documents.length, names: names ? `：${names}` : "", failures: failTxt }));
    // Agent 工作台保持打开：结果卡展示在本轮会话内（不自动关窗/自动弹报告）
    setLastAiResult(r);
    setAiMinimized(false);
    void refreshFolders();
  }, [flash, refreshFolders]);

  // ── 边生成边绘制 ──
  // 后端每落库一个视图文档就广播 view_done；这里增量拉取并渲染：
  // 首个完成视图直接切到该文档（用户立刻在画布上看到成果），
  // 后续视图并入侧栏列表（不打断用户正在看的文档，仅 toast 提示，点击即跳转）。
  const drawnDocIdsRef = useRef<Set<string>>(new Set());
  const firstViewDrawnRef = useRef(false);
  useEffect(() => {
    if (!aiLoading) return;
    const done = mmAiProgressBuffer.snapshot().filter(e => e.step === "view_done");
    for (const e of done) {
      const docId = e.docId ?? e.doc_id;
      if (!docId || drawnDocIdsRef.current.has(docId)) continue;
      drawnDocIdsRef.current.add(docId);
      void (async () => {
        try {
          const f = await mmApi.load(docId);
          if (!f) return;
          setDocs(prev => (prev.some(d => d.id === docId) ? prev : [f.document, ...prev]));
          if (!firstViewDrawnRef.current) {
            firstViewDrawnRef.current = true;
            setFull(f);
          }
        } catch { /* 拉取失败不致命：最终结果仍会整体应用 */ }
      })();
    }
  }, [aiLoading, aiProgress]);

  useEffect(() => {
    if (full?.document.id && !targetDocumentId) setTargetDocumentId(full.document.id);
  }, [full?.document.id, targetDocumentId]);

  // 新一轮 AI 导入开始：重置增量绘制标记
  useEffect(() => {
    if (aiLoading) { drawnDocIdsRef.current = new Set(); firstViewDrawnRef.current = false; }
  }, [aiLoading]);

  const runAiProject = useCallback(async (instruction: string) => {
    const targetId = targetDocumentId || full?.document.id;
    if (!targetId || !projectPath || !providerId || !modelId) return;
    const runId = crypto.randomUUID ? crypto.randomUUID() : `run-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    aiRunIdRef.current = runId;
    mmAiProgressBuffer.clear(); // 新一轮导入：清空进度缓冲，日志面板从零开始
    setAiLoading(true); setError("");
    clearAnsweredAsks(); // 每次请求重新启用新的询问表单
    const replaceExisting = aiHasRunRef.current;
    aiHasRunRef.current = true;
    try {
      // instruction 来自工作台指令框（首轮可空 = 纯配置驱动；追问轮 = 追加指令）
      const r = await mmApi.aiFromProject({ documentId: targetId, projectPath, providerId: providerId || null, modelId: modelId || null, userHint: instruction.trim() || null, depth: aiDepth, views: aiViews, replaceExisting, runId });
      applyAiImport(r);
    } catch (e) {
      const msg = String(e);
      // 用户主动停止：不当作错误红字提示，仅工作台展示
      if (msg.includes("已取消")) flash(t("mindmap.aiCancelled")); else setError(msg);
    } finally { aiRunIdRef.current = null; setAiLoading(false); }
  }, [full?.document.id, targetDocumentId, projectPath, providerId, modelId, aiDepth, aiViews, applyAiImport, flash, t]);

  const runAiText = useCallback(async (instruction: string) => {
    const targetId = targetDocumentId || full?.document.id;
    if (!targetId || !instruction.trim() || !providerId || !modelId) return;
    const runId = crypto.randomUUID ? crypto.randomUUID() : `run-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    aiRunIdRef.current = runId;
    mmAiProgressBuffer.clear(); // 新一轮导入：清空进度缓冲，日志面板从零开始
    setAiLoading(true); setError("");
    clearAnsweredAsks(); // 每次请求重新启用新的询问表单
    const replaceExisting = aiHasRunRef.current;
    aiHasRunRef.current = true;
    try {
      // instruction 是首轮需求或完成后的修改指令；后端会在追问轮替换目标文档节点。
      const r = await mmApi.aiFromText({ documentId: targetId, text: instruction, title: textTitle || full?.document.name || t("mindmap.aiTextDoc"), providerId: providerId || null, modelId: modelId || null, replaceExisting, runId });
      applyAiImport(r);
    } catch (e) {
      const msg = String(e);
      if (msg.includes("已取消")) flash(t("mindmap.aiCancelled")); else setError(msg);
    } finally { aiRunIdRef.current = null; setAiLoading(false); }
  }, [full?.document.id, full?.document.name, targetDocumentId, textTitle, providerId, modelId, applyAiImport, flash, t]);

  // IDE 式「停止」：给当前导入运行打取消标志，后端各循环/流式块边界随即中断
  const stopAi = useCallback(async () => {
    const rid = aiRunIdRef.current;
    if (!rid) return;
    try { await mmApi.aiCancel(rid); } catch { /* 取消请求本身失败可忽略：运行自然结束时 ref 会被清空 */ }
  }, []);

  // ─── 思维导图 Agent（右栏对话）───
  // 后端只裁决不执行：写 ops 经 agentOps 事件到达这里后**全部直接落图**
  // （删除/移动不再弹确认清单；想回退用 Ctrl+Z，落图前先压撤销快照）。

  const applyAgentOps = useCallback(async (_runId: string, ops: AgentOp[]) => {
    const docId = full?.document.id;
    if (!docId) return;
    const { auto } = partitionAgentOps(ops);
    if (!auto.length) return;
    commitHistory(); // 先压撤销快照，再写入
    let applied = 0;
    for (const op of auto) {
      try {
        if (op.action === "add" && op.id) {
          await mmApi.upsertNode({
            documentId: docId,
            node: {
              id: op.id, documentId: docId, parentId: op.parentId ?? null,
              name: op.name ?? "", detail: op.detail ?? "", kind: op.kind ?? "other",
              color: op.color ?? "#f59e0b",
              // 模型给了 sources 就用它的；没给但本轮引用了文件，则锚到被分析的文件上
              sources: op.sources?.length ? op.sources : agentAttachmentsRef.current.slice(0, 3),
              positionX: 0, positionY: 0,
            },
          });
          applied++;
        } else if (op.action === "update" && op.id) {
          // 只改 op 携带的字段：以库内现值为底，避免把未提及字段清空
          const f = await mmApi.load(docId);
          const n = f?.nodes.find((x) => x.id === op.id);
          if (n) {
            await mmApi.upsertNode({ documentId: docId, node: { ...n, name: op.name ?? n.name, detail: op.detail ?? n.detail, color: op.color ?? n.color } });
            applied++;
          }
        } else if (op.action === "delete" && op.id) {
          // 后端按子树级联删除
          await mmApi.deleteNode({ documentId: docId, nodeId: op.id });
          applied++;
        } else if (op.action === "move" && op.id) {
          const f = await mmApi.load(docId);
          const n = f?.nodes.find((x) => x.id === op.id);
          if (n) {
            await mmApi.upsertNode({ documentId: docId, node: { ...n, parentId: op.parentId ?? null } });
            applied++;
          }
        }
      } catch (e) { console.error("应用 Agent 变更失败:", e); }
    }
    const f = await mmApi.load(docId);
    if (f) onDocumentUpdated(f);
  }, [full?.document.id, commitHistory, onDocumentUpdated]);

  // 事件缓冲在面板重挂时会重放历史，用事件时间戳去重，避免重放导致重复应用/重复弹确认
  useEffect(() => {
    for (const e of aiProgress) {
      if (e.step !== "agentOps" || !e.ops?.length || !e.runId || !e.at) continue;
      if (handledAgentOpsAt.has(e.at)) continue;
      handledAgentOpsAt.add(e.at);
      void applyAgentOps(e.runId, e.ops);
    }
  }, [aiProgress, applyAgentOps]);

  const runAgentChat = useCallback(async (instruction: string) => {
    const docId = full?.document.id;
    if (!docId || !providerId || !modelId) return;
    const runId = crypto.randomUUID ? crypto.randomUUID() : `agent-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    aiRunIdRef.current = runId;
    // 新一轮：清空进度缓冲，思考过程/工具轨迹从零开始（跑完后仍留在面板里可回看）
    mmAiProgressBuffer.clear();
    setAiLoading(true); setError("");
    try {
      // @ 引用的文件：去重、封顶 8 个，后端读取内容进上下文
      const atRefs = [...new Set([...instruction.matchAll(/(?:^|\s)@([^\s@]+)/g)].map((m) => m[1]))].slice(0, 8);
      agentAttachmentsRef.current = atRefs;
      const r = await mmApi.agentChat({
        documentId: docId, sessionId: agentSessionRef.current ?? "", message: instruction,
        selectedNodeIds: [], attachedFiles: atRefs, providerId, modelId, runId,
      });
      if (r.sessionId !== agentSessionRef.current) useAgentSession(r.sessionId);
      // 落库后重新回放：让每条消息带上落库 id（「从这条分叉」要用），同时刷新会话列表排序
      await replaySession(r.sessionId);
      void refreshSessions(docId).catch(() => undefined);
    } catch (e) {
      // 写进**本轮**那条助手气泡（而不是再推一条）：一轮一轮看得见，
      // 失败原因才不会跟「它到底改了什么」分成两条、对不上号。
      patchAgentTurn({ status: "error", error: `${t("agent.runFailed")}：${String(e)}` });
    } finally { aiRunIdRef.current = null; setAiLoading(false); }
  }, [full?.document.id, providerId, modelId, t, useAgentSession, replaySession, refreshSessions]);

  // chat 模式绑定项目目录（一个导图至多绑定一个，重复绑定即替换）
  const pickAndBindDir = useCallback(async () => {
    const docId = targetDocumentId || full?.document.id;
    if (!docId) return;
    const d = await openDialog({ directory: true, multiple: false, title: t("mindmap.pickDir") });
    if (typeof d !== "string" || !d) return;
    try {
      await mmApi.bindDocumentDir(docId, d);
      const f = await mmApi.load(docId);
      if (f) onDocumentUpdated(f);
      flash(t("mindmap.agentDirBound"));
    } catch (e) { setError(String(e)); }
  }, [targetDocumentId, full?.document.id, onDocumentUpdated, flash, t]);

  // chat 模式：绑定目录（或换绑）后拉取文件清单作为 @ 候选
  useEffect(() => {
    if (aiMode !== "chat" || !full?.document.projectDir) { setProjectFiles([]); return; }
    let alive = true;
    void mmApi.listProjectFiles(full.document.id)
      .then((files) => { if (alive) setProjectFiles(files); })
      .catch(() => { if (alive) setProjectFiles([]); });
    return () => { alive = false; };
  }, [aiMode, full?.document.id, full?.document.projectDir]);

  // 打开右栏（或切换文档）时载入该文档的 Agent 会话：没有会话则建一条，默认选最近用过的
  useEffect(() => {
    if (!showAi || !full?.document.id) return;
    let alive = true;
    void (async () => {
      try {
        let list = await mmApi.agentListSessions(full.document.id);
        if (!alive) return;
        if (list.length === 0) {
          const sid = await mmApi.agentNewSession(full.document.id);
          if (!alive) return;
          list = await mmApi.agentListSessions(full.document.id);
          if (!alive) return;
          setAgentSessions(list);
          useAgentSession(sid);
          clearAgentMessages();
          return;
        }
        setAgentSessions(list);
        const sid = agentSessionRef.current && list.some((s) => s.id === agentSessionRef.current)
          ? (agentSessionRef.current as string)
          : list[0].id;
        useAgentSession(sid);
        if (alive) await replaySession(sid);
      } catch { /* 会话载入失败不阻断对话 */ }
    })();
    return () => { alive = false; };
  }, [showAi, full?.document.id, useAgentSession, replaySession]);

  // ─── 会话操作：切换 / 新建 / 重命名 / 删除 / 分叉 ───

  const selectAgentSession = useCallback(async (sid: string) => {
    if (!sid || sid === agentSessionRef.current) return;
    useAgentSession(sid);
    try { await replaySession(sid); } catch { /* 回放失败不阻断切换 */ }
  }, [useAgentSession, replaySession]);

  const newAgentSession = useCallback(async () => {
    const docId = full?.document.id;
    if (!docId) return;
    try {
      const sid = await mmApi.agentNewSession(docId);
      await refreshSessions(docId, sid);
      clearAgentMessages();
      aiHasRunRef.current = false;
      setLastAiResult(null);
      setError("");
    } catch (e) { setError(String(e)); }
  }, [full?.document.id, refreshSessions]);

  const renameAgentSession = useCallback(async (sid: string, title: string) => {
    const docId = full?.document.id;
    if (!docId || !sid) return;
    try {
      await mmApi.agentRenameSession(sid, title);
      await refreshSessions(docId);
    } catch (e) { setError(String(e)); }
  }, [full?.document.id, refreshSessions]);

  const deleteAgentSession = useCallback(async (sid: string) => {
    const docId = full?.document.id;
    if (!docId || !sid) return;
    try {
      await mmApi.agentDeleteSession(sid);
      const list = await refreshSessions(docId);
      // 删掉当前会话后自动切到剩下的最近一条（没有就建一条）
      if (list.length > 0) {
        useAgentSession(list[0].id);
        await replaySession(list[0].id);
      } else {
        const nid = await mmApi.agentNewSession(docId);
        await refreshSessions(docId, nid);
        clearAgentMessages();
      }
    } catch (e) { setError(String(e)); }
  }, [full?.document.id, refreshSessions, useAgentSession, replaySession]);

  /** 从某条消息分叉：复制该条及之前的历史到新会话并切过去（原会话原样保留） */
  const forkAgentSession = useCallback(async (messageId: string) => {
    const docId = full?.document.id;
    const sid = agentSessionRef.current;
    if (!docId || !sid) return;
    try {
      const nid = await mmApi.agentForkSession(sid, messageId || "");
      await refreshSessions(docId, nid);
      await replaySession(nid);
      flash(t("mindmap.agentForked"));
    } catch (e) { setError(String(e)); }
  }, [full?.document.id, refreshSessions, replaySession, flash, t]);

  // AI 弹框关闭：任务进行中先询问（确认后停止任务），空闲则直接关。
  // 只有右上角 ✕ / footer 按钮会走到这里；弹框本身无 Esc/遮罩关闭。
  const requestCloseAi = useCallback(() => {
    if (aiLoading) { setAiCloseGuard(true); return; }
    setShowAi(false);
    setAiMinimized(false);
  }, [aiLoading]);

  const confirmCloseAi = useCallback(async () => {
    setAiCloseGuard(false);
    await stopAi();
    setShowAi(false);
    setAiMinimized(false);
  }, [stopAi]);

  /** 真正执行导入。`target` 为空 = 新建文档；否则导入到该文档（replace 时先清空）。 */
  const runImportJson = useCallback(async (path: string, target: { id: string; replace: boolean } | null) => {
    const r = await invoke<{ documentId: string; nodeCount: number; warnings: string[] }>("mm_import_nodes", {
      input: {
        path,
        documentId: target?.id ?? null,
        folderId: null,
        replaceExisting: target?.replace ?? false,
      },
    });
    const f = await mmApi.load(r.documentId);
    if (f) onDocumentUpdated(f);
    flash(
      r.warnings.length > 0
        ? t("mindmap.importedWithWarnings", { count: r.nodeCount, n: r.warnings.length })
        : t("mindmap.imported", { count: r.nodeCount }),
    );
  }, [onDocumentUpdated, flash, t]);

  // 导入目标选择：{ 文件路径, 当前文档名 }（null = 不弹选择框）
  const [importPick, setImportPick] = useState<{ path: string; docName: string } | null>(null);

  /** 从 JSON 文件导入导图（文件由其它 Agent / 脚本生成，格式见 mindmap-import 技能）。
   *  有当前文档时先让用户选目标：新建 / 追加 / 替换（替换再加一次确认，它要先清空）。 */
  const importJsonFile = useCallback(async () => {
    const picked = await openDialog({
      multiple: false,
      directory: false,
      filters: [{ name: "Mindmap JSON", extensions: ["json", "mindmap.json"] }],
      title: t("mindmap.importJson"),
    });
    if (typeof picked !== "string" || !picked) return;
    // 没有打开的文档 → 只能是新建，不必多问一次
    if (!full) {
      try { await runImportJson(picked, null); } catch (e) { setError(String(e)); }
      return;
    }
    setImportPick({ path: picked, docName: full.document.name });
  }, [full, runImportJson, t]);

  /** 导入目标确认：追加 / 替换都动当前文档，替换尤其危险（先清空），要再确认一次。 */
  const confirmImportTarget = useCallback((replace: boolean) => {
    const pick = importPick;
    if (!pick || !full) return;
    const target = { id: full.document.id, replace };
    setImportPick(null);
    if (replace) {
      // 二次确认：替换会先清空当前文档的全部节点
      setConfirmState({
        title: t("mindmap.importReplaceTitle"),
        message: t("mindmap.importReplaceConfirm", { name: pick.docName }),
        action: () => { void runImportJson(pick.path, target).catch((e) => setError(String(e))); },
      });
      return;
    }
    void runImportJson(pick.path, target).catch((e) => setError(String(e)));
  }, [importPick, full, runImportJson, t]);

  const exportMd = useCallback(async () => {
    if (!full) return;
    try {
      const md = await mmApi.exportMd(full.document.id);
      const fp = await save({ defaultPath: `${full.document.name}.md`, filters: [{ name: "Markdown", extensions: ["md"] }] });
      if (!fp) return;
      await invoke("write_text_file", { path: fp, content: md });
      flash(t("mindmap.exportedTo", { path: fp }));
    } catch (e) { flash(String(e)); }
  }, [full, flash]);

  return (
    <div className="relative flex h-full min-h-0 flex-col bg-white/[0.01] text-slate-200">
      <div className="flex min-h-0 flex-1">
        {!sidebarCollapsed && (
          <aside className="group/sb relative flex shrink-0 flex-col border-r border-white/10 bg-white/[0.02]" style={{ width: sidebarW }}>
            {/* Search */}
            <div className="border-b border-white/10 px-2 py-1.5 flex items-center gap-1.5">
              <Search className="h-3 w-3 shrink-0 text-slate-600" />
              <input className="min-w-0 flex-1 bg-transparent text-tiny text-slate-300 outline-none placeholder:text-slate-700" value={search} onChange={(e) => setSearch(e.target.value)} placeholder={t("mindmap.searchPh")} />
              {search && <button type="button" className="text-slate-600 hover:text-white" onClick={() => setSearch("")}><X className="h-3 w-3" /></button>}
            </div>
            {!activeFolderId && folders.length === 0 && (
              <div className="px-3 pt-1 text-micro text-slate-500">{t("mindmap.noFolders")}</div>
            )}
            <div className="min-h-0 flex-1 overflow-y-auto p-1.5 select-none" data-drop-root>
              {/* 面包屑：路径上级均可点击进入；也是移回相应目录的投放目标 */}
              {activeFolderId && (
                <div className="mb-1 space-y-0.5">
                  <button type="button" className={`mb-0.5 flex w-full items-center gap-1.5 rounded-md px-2 py-1 text-tiny select-none ${dragOverFolderId === "__root" ? "ring-1 ring-cyan-400/70 bg-cyan-400/10 text-white" : "text-slate-400 hover:bg-white/[0.05] hover:text-white"}`}
                    onClick={() => setActiveFolderId(null)}
                    data-drop-root>
                    <ChevronRight className="h-3 w-3 -rotate-180" />{t("mindmap.allDocs")} <span className="text-[8px] text-slate-500">{t("mindmap.backToRoot")}</span>
                  </button>
                  {folderPath.map((f, i) => {
                    const isLast = i === folderPath.length - 1;
                    const over = dragOverFolderId === f.id;
                    if (isLast) {
                      return (
                        <div key={f.id} className={`mb-0.5 flex w-full items-center gap-1.5 rounded-md px-2 py-1 text-tiny ${over ? "ring-1 ring-cyan-400/70 bg-cyan-400/10 text-white" : "text-slate-400"}`}>
                          <Folder className="h-3 w-3 shrink-0 text-amber-400/70" />
                          <span className="truncate">{f.name}</span>
                          <span className="shrink-0 text-micro text-slate-500">{f.documentCount}</span>
                        </div>
                      );
                    }
                    return (
                      <button key={f.id} type="button" className={`mb-0.5 flex w-full items-center gap-1.5 rounded-md px-2 py-1 text-tiny cursor-pointer select-none ${over ? "ring-1 ring-cyan-400/70 bg-cyan-400/10 text-white" : "text-slate-400 hover:bg-white/[0.05] hover:text-white"}`}
                        onClick={() => setActiveFolderId(f.id)}
                        data-drop-folder={f.id}>
                        <Folder className="h-3 w-3 shrink-0 text-amber-400/70" />
                        <span className="truncate">{f.name}</span>
                        <span className="shrink-0 text-micro text-slate-500">{f.documentCount}</span>
                        <ChevronRight className="h-2.5 w-2.5 shrink-0 text-slate-600" />
                      </button>
                    );
                  })}
                </div>
              )}
              {/* 文件夹树 —— 可拖拽整理：文件↔目录、目录↔目录/根 */}
              {treeRoots.length > 0 && <div className="mb-1.5">{renderFolderNodes(treeRoots, 0)}</div>}
              {/* Documents */}
              {filteredDocs.length === 0 && <div className="py-8 text-center text-tiny text-slate-500">{search ? t("mindmap.noMatchDocs") : t("mindmap.noDocs")}</div>}
              {filteredDocs.map(d => {
                const IconFn = DOC_SOURCE_ICONS[d.sourceType] ?? DOC_SOURCE_ICONS.manual;
                return (
                <div key={d.id} className={`group mb-1 flex items-center gap-1.5 rounded-md border px-2.5 py-2 transition select-none ${full?.document.id === d.id ? "border-cyan-400/40 bg-cyan-400/10" : "border-white/10 bg-white/[0.02] hover:bg-white/[0.06]"}`}
                  title={t("mindmap.docHint")}
                  data-drop-root
                  onMouseDown={(e) => { if (renamingDocId === d.id) return; startDragHandle(e, "doc", d.id, d.name); }}
                  onDoubleClick={(e) => { if (renamingDocId === d.id) { e.stopPropagation(); return; } e.preventDefault(); e.stopPropagation(); void loadDocument(d.id); }}>
                  <button type="button" className="flex min-w-0 flex-1 items-center gap-2 text-left" onClick={() => { if (renamingDocId === d.id) return; void loadDocument(d.id); }}>
                    {IconFn("h-3.5 w-3.5 shrink-0 text-slate-500")}
                    <span className="flex min-w-0 flex-col">
                      {renamingDocId === d.id ? (
                        <input autoFocus value={renameDocName} onChange={e => setRenameDocName(e.target.value)}
                          onClick={e => e.stopPropagation()} onDoubleClick={e => e.stopPropagation()}
                          onKeyDown={e => { e.stopPropagation(); if (e.key === "Enter") void saveRenameDoc(d); else if (e.key === "Escape") setRenamingDocId(null); }}
                          onBlur={() => void saveRenameDoc(d)}
                          className="w-full rounded border border-cyan-400/50 bg-slate-900 px-1.5 py-0.5 text-caption text-white outline-none" />
                      ) : (
                        <span className="truncate text-caption text-slate-200">{d.name}</span>
                      )}
                      <span className="text-micro text-slate-400">{formatTime(d.updatedAt)}</span>
                    </span>
                  </button>
                  {activeFolderId && <button type="button" className="shrink-0 text-[8px] text-slate-300 opacity-0 group-hover:opacity-100 pointer-events-none group-hover:pointer-events-auto hover:text-white" onClick={() => void moveDoc(d.id, null)}>{t("mindmap.moveOut")}</button>}
                  <button type="button" className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-slate-400 opacity-0 group-hover:opacity-100 pointer-events-none group-hover:pointer-events-auto hover:bg-white/10 hover:text-white" onClick={() => startRenameDoc(d)} title={t("mindmap.rename")}><Pencil className="h-3 w-3" /></button>
                  <button type="button" className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-red-300/60 opacity-0 group-hover:opacity-100 pointer-events-none group-hover:pointer-events-auto hover:bg-red-400/10 hover:text-red-200" onClick={() => removeDoc(d.id, d.name)} title={t("mindmap.delete")}><Trash2 className="h-3 w-3" /></button>
                </div>
              );})}
            </div>
            {/* 底部工具区：新建文件夹/文档 + AI 导入入口 */}
            {/* 底部工具区：只留「文档 / 文件夹维度」的操作。
                AI 入口与「导出 Markdown」都是针对**当前打开的那份文档**的功能 ——
                前者收进第二栏（画布侧），后者一并移到第二栏，这里不再承载单文档操作；
                「N 个节点」是文档级计数，也不再占这一栏的位置。 */}
            <div className="shrink-0 space-y-1.5 border-t border-white/10 p-1.5">
              <div className="grid grid-cols-2 gap-1.5">
                <button type="button" className={button} onClick={() => { setShowFolderCreate(true); setFolderName(""); }} title={t("mindmap.newFolder")}><FolderPlus className="h-3 w-3" />{t("mindmap.folder")}</button>
                <button type="button" className={button} onClick={() => setShowCreate(true)} title={t("mindmap.newDoc")}><Plus className="h-3 w-3" />{t("mindmap.doc")}</button>
              </div>
              {/* 从 JSON 文件导入：别的 Agent / 脚本按 .agents/skills/mindmap-import 的
                  格式生成文件后，这里一键进导图（不需要 AI） */}
              <button type="button" className={`${button} w-full justify-center`} onClick={() => void importJsonFile()} title={t("mindmap.importJsonTip")}>
                <FileDown className="h-3 w-3" />{t("mindmap.importJson")}
              </button>
              {/* 模块专属设置（热键 / 外部编辑器 / AI 探索参数） */}
              <ModuleSettingsButton
                title={t("mindmap.settingsTitle")}
                label={t("mindmap.settingsTitle")}
                buttonClassName="border border-white/10 bg-white/[0.05] w-full justify-center hover:bg-white/[0.1]"
              >
                <MindmapModuleSettings />
              </ModuleSettingsButton>
            </div>
            {/* 宽度拖拽把手 + 收起按钮（侧边栏右侧） */}
            <div className="absolute -right-1 top-0 z-10 flex h-full w-2.5 cursor-col-resize items-center justify-center hover:bg-white/[0.06]" title={t("mindmap.dragResize")}
              onMouseDown={(e) => {
                if (e.button !== 0) return;
                e.preventDefault();
                sbResizeRef.current.moved = false;
                const startX = e.clientX; const startW = sidebarW;
                // 区间夹紧与持久化由 usePaneWidth 负责
                const onMove = (ev: MouseEvent) => { if (Math.abs(ev.clientX - startX) > 2) sbResizeRef.current.moved = true; setSidebarW(startW + (ev.clientX - startX)); };
                const onUp = () => { window.removeEventListener("mousemove", onMove); window.removeEventListener("mouseup", onUp); };
                window.addEventListener("mousemove", onMove);
                window.addEventListener("mouseup", onUp);
              }}>
              <button type="button" className="flex h-6 w-2.5 items-center justify-center rounded-l bg-slate-800/80 text-slate-400 opacity-0 transition group-hover/sb:opacity-100 pointer-events-none group-hover/sb:pointer-events-auto hover:text-white" title={t("mindmap.collapseSidebar")}
                onClick={(e) => { e.stopPropagation(); if (!sbResizeRef.current.moved) setSidePanesOpen(false); }}>
                <ChevronLeft className="h-3 w-3" />
              </button>
            </div>
          </aside>
        )}
        {sidebarCollapsed && (
          <button type="button" className="absolute left-2 top-2 z-20 inline-flex h-8 w-8 items-center justify-center rounded-md border border-white/10 bg-slate-900/90 text-slate-300 shadow-lg transition hover:text-white" onClick={() => setSidePanesOpen(true)} title={t("mindmap.expandSidebar")}>
            <ChevronRight className="h-4 w-4" />
          </button>
        )}
        <main className="relative min-w-0 flex-1">
          {/* 画布常驻：右栏对话展开/折叠时也保持挂载，边生成边绘制。
              右栏不再是弹窗，画布全程可交互 —— 这正是三栏布局的意义。 */}
          {(full ? (
            <Canvas full={full} accent={ACCENT} onDocumentUpdate={onDocumentUpdated} onHistoryPush={commitHistory} historyVersion={historyVersion} onAiProject={() => void openAiImport()} onExportMd={() => void exportMd()} onError={setError}
              treeOpen={treeOpen} onTreeOpenChange={setSidePanesOpen}
              aiPill={aiMinimized && showAi ? <AiRunningPill running={aiLoading} onRestore={() => setAiMinimized(false)} onStop={() => void stopAi()} /> : null} />
          ) : (
            <VexEmptyState
              title={t("mindmap.emptyTitle")}
              desc={t("mindmap.emptyDesc")}
              tick={t("mindmap.emptyTick")}
              avatarSize={56}
              className="h-full"
            />
          ))}
          {error && !showAi && <div className="absolute bottom-8 left-1/2 z-40 -translate-x-1/2 max-w-md rounded-md border border-red-400/20 bg-slate-900 px-3 py-2 text-caption text-red-300 shadow-xl">{error}<button type="button" className="ml-2 text-slate-400 hover:text-white" onClick={() => setError("")}>✕</button></div>}
        </main>
        {/* 右栏：AI 对话常驻（替代原弹窗）。showAi=展开；最小化时画布只留恢复胶囊。
            与左栏同构：可拖宽、可收起，收起后经左栏「AI 分析」按钮或画布胶囊恢复。 */}
        {showAi && !aiMinimized && (
          <aside className="relative flex-shrink-0 border-l border-white/5 bg-slate-950/40" style={{ width: aiPanelW }}>
            <div className="flex h-full min-h-0 flex-col">
              <div className="flex flex-shrink-0 items-center gap-2 border-b border-white/5 px-3 py-2">
                <Brain className="h-3.5 w-3.5" style={{ color: ACCENT }} />
                <span className="truncate text-caption font-bold text-slate-200">{t("mindmap.aiUnifiedTitle")}</span>
                <div className="ml-auto flex items-center gap-1">
                  <button type="button" onClick={() => setAiMinimized(true)} title={t("aiMinimized.minimize")}
                    className="flex h-6 w-6 items-center justify-center rounded-md text-slate-400 transition hover:bg-white/10 hover:text-white">
                    <Minimize2 className="w-3.5 h-3.5" />
                  </button>
                  <button type="button" onClick={requestCloseAi} title={t("mindmap.aiPanelClose")}
                    className="flex h-6 w-6 items-center justify-center rounded-md px-1.5 text-slate-400 transition hover:bg-white/10 hover:text-white">✕</button>
                </div>
              </div>
              {/* 说明：Agent 的改动直接落到画布，不再有待确认清单（回退用 Ctrl+Z） */}
              {/* AI 智能体工作台（项目/文档共用）：会话式多轮 + 阶段计划 + 工具透明 + 流式反馈 */}
              <div className="min-h-0 flex-1 overflow-y-auto">
                <AgentWorkbench
                  mode={aiMode}
                  onModeChange={setAiMode}
                  documents={docs.map(d => ({ id: d.id, name: d.name, sourceType: d.sourceType }))}
                  targetDocumentId={targetDocumentId}
                  onTargetDocumentChange={(id) => void selectAiTarget(id)}
                  providers={providers}
                  providerId={providerId}
                  modelId={modelId}
                  onSelectModel={(pid, mid) => {
                    setProviderId(pid);
                    setModelId(mid);
                    void rememberModel(pid, mid);
                  }}
                  projectPath={projectPath}
                  onPickProject={() => void (async () => { const d = await openDialog({ directory: true, multiple: false, title: t("mindmap.pickDir") }); if (typeof d === "string") setProjectPath(d); })()}
                  aiDepth={aiDepth}
                  onDepthChange={setAiDepth}
                  aiViews={aiViews}
                  onViewsChange={setAiViews}
                  textTitle={textTitle}
                  onTextTitleChange={setTextTitle}
                  loading={aiLoading}
                  onRun={(instruction) => { if (aiMode === "chat") void runAgentChat(instruction); else if (aiMode === "project") void runAiProject(instruction); else void runAiText(instruction); }}
                  onStop={() => void stopAi()}
                  onAnswer={(answer) => { const rid = aiRunIdRef.current; if (rid) void mmApi.aiAnswer(rid, answer).catch((e) => console.error("回答询问失败:", e)); }}
                  result={lastAiResult}
                  runError={error}
                  onShowReport={() => { if (lastAiResult) setAiReport(lastAiResult); }}
                  onNewSession={() => void newAgentSession()}
                  sessions={agentSessions}
                  sessionId={agentSessionId}
                  onSelectSession={(id) => void selectAgentSession(id)}
                  onRenameSession={(id, title) => void renameAgentSession(id, title)}
                  onDeleteSession={(id) => void deleteAgentSession(id)}
                  onForkSession={(messageId) => void forkAgentSession(messageId)}
                  projectRoot={aiMode === "project" ? projectPath : null}
                  projectDir={full?.document.projectDir ?? null}
                  projectFiles={projectFiles}
                  onBindProjectDir={() => void pickAndBindDir()}
                />
              </div>
            </div>
            {/* 左缘拖宽把手：与左栏同一套逻辑，拖动方向相反 */}
            <div className="absolute -left-1 top-0 z-10 flex h-full w-2.5 cursor-col-resize items-center justify-center hover:bg-white/[0.06]" title={t("mindmap.dragResize")}
              onMouseDown={(e) => {
                if (e.button !== 0) return;
                e.preventDefault();
                const startX = e.clientX; const startW = aiPanelW;
                const onMove = (ev: MouseEvent) => { setAiPanelW(startW - (ev.clientX - startX)); };
                const onUp = () => { window.removeEventListener("mousemove", onMove); window.removeEventListener("mouseup", onUp); };
                window.addEventListener("mousemove", onMove);
                window.addEventListener("mouseup", onUp);
              }} />
          </aside>
        )}
      </div>
      {showCreate && <CreateDocModal onClose={() => setShowCreate(false)} onCreate={(n,d,fid) => { void createDoc(n,d,fid); }} folderId={activeFolderId} />}
      {error && showAi && <div className="absolute bottom-8 left-1/2 z-40 -translate-x-1/2 max-w-md rounded-md border border-red-400/20 bg-slate-900 px-3 py-2 text-caption text-red-300 shadow-xl">{error}<button type="button" className="ml-2 text-slate-400 hover:text-white" onClick={() => setError("")}>✕</button></div>}
      {notice && <div className="absolute bottom-8 left-1/2 z-40 -translate-x-1/2 rounded-md border border-white/10 bg-slate-900 px-3 py-2 text-caption text-slate-200 shadow-xl">{notice}</div>}
      {/* AI 弹框关闭确认：任务进行中询问，确认后停止任务再关闭 */}
      <ConfirmDialog
        open={aiCloseGuard}
        onCancel={() => setAiCloseGuard(false)}
        onConfirm={() => void confirmCloseAi()}
        title={t("aiCloseGuard.title")}
        desc={t("aiCloseGuard.desc")}
        confirmText={t("aiCloseGuard.confirm")}
        cancelText={t("aiCloseGuard.cancel")}
        danger
      />
      {confirmState && <ConfirmModal title={confirmState.title} message={confirmState.message} accent={ACCENT} onConfirm={confirmState.action} onClose={() => setConfirmState(null)} />}
      {aiReport && <AiImportReportModal result={aiReport} onClose={() => setAiReport(null)} onOpenDoc={(id) => void openReportDoc(id)} />}
      {/* 拖拽浮层：跟随鼠标，提示当前落点是否有效 */}
      {ghost && (
        <div className="pointer-events-none fixed z-[9998] flex items-center gap-1.5 rounded-md border border-cyan-400/50 bg-slate-900/95 px-2 py-1 text-tiny text-slate-200 shadow-2xl"
          style={{ left: ghost.x + 14, top: ghost.y + 16 }}>
          {ghost.kind === "folder" ? <Folder className="h-3 w-3 shrink-0 text-amber-400" /> : <FileText className="h-3 w-3 shrink-0 text-slate-400" />}
          <span className="max-w-[150px] truncate font-medium">{ghost.name}</span>
          <span className={`${ghost.ok ? "text-cyan-300" : "text-slate-500"}`}>{ghost.ok ? t("mindmap.dropOk") : t("mindmap.dropTarget")}</span>
        </div>
      )}
      {/* 导入目标选择：新建 / 追加到当前文档 / 替换当前文档内容 */}
      {importPick && createPortal(
        <div className="fixed inset-0 z-[200] modal-mask flex items-center justify-center bg-black/70 p-6 backdrop-blur-[3px]">
          <div className="w-[380px] rounded-card border border-white/10 bg-surface-panel p-5 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <h3 className="mb-1 text-sm font-semibold text-white">{t("mindmap.importTargetTitle")}</h3>
            <p className="mb-4 truncate text-tiny text-slate-500" title={importPick.path}>{t("mindmap.importTargetDesc", { file: importPick.path.split(/[\\/]/).pop() ?? importPick.path })}</p>
            <div className="space-y-1.5">
              <button type="button"
                className="w-full rounded-ctl border border-white/10 bg-white/[0.04] px-3 py-2 text-left text-caption text-slate-200 transition hover:border-cyan-400/50 hover:text-white"
                onClick={() => { const p = importPick; setImportPick(null); void runImportJson(p.path, null).catch((e) => setError(String(e))); }}>
                <Plus className="mr-1.5 inline h-3 w-3" />{t("mindmap.importTargetNew")}
              </button>
              <button type="button"
                className="w-full rounded-ctl border border-white/10 bg-white/[0.04] px-3 py-2 text-left text-caption text-slate-200 transition hover:border-cyan-400/50 hover:text-white"
                onClick={() => confirmImportTarget(false)}>
                <FileDown className="mr-1.5 inline h-3 w-3" />{t("mindmap.importTargetAppend", { name: importPick.docName })}
              </button>
              <button type="button"
                className="w-full rounded-ctl border border-rose-400/25 bg-rose-500/[0.06] px-3 py-2 text-left text-caption text-rose-300 transition hover:border-rose-400/60 hover:text-rose-200"
                onClick={() => confirmImportTarget(true)}>
                <Trash2 className="mr-1.5 inline h-3 w-3" />{t("mindmap.importTargetReplace", { name: importPick.docName })}
              </button>
            </div>
            <div className="mt-4 flex justify-end">
              <button type="button" className="rounded-md px-4 py-1.5 text-caption text-slate-400 hover:text-white" onClick={() => setImportPick(null)}>{t("mindmap.cancel")}</button>
            </div>
          </div>
        </div>, document.body)}
      {/* Folder create/edit modal */}
      {showFolderCreate && createPortal(
        <div className="fixed inset-0 z-[200] modal-mask flex items-center justify-center bg-black/70 p-6 backdrop-blur-[3px]">
          <div className="w-[340px] rounded-card border border-white/10 bg-surface-panel p-5 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <h3 className="mb-4 text-sm font-semibold text-white">{t("mindmap.newFolderTitle")}</h3>
            <input className="w-full h-9 rounded-ctl ui-input px-3 text-body text-white outline-none mb-4" value={folderName} onChange={(e) => setFolderName(e.target.value)} placeholder={t("mindmap.folderNamePh")} autoFocus onKeyDown={(e) => e.key === "Enter" && createFolder()} />
            <div className="flex justify-end gap-2">
              <button type="button" className="rounded-md px-4 py-1.5 text-caption text-slate-400 hover:text-white" onClick={() => setShowFolderCreate(false)}>{t("mindmap.cancel")}</button>
              <button type="button" className="rounded-md px-4 py-1.5 text-caption font-semibold text-white disabled:opacity-40" style={{ backgroundColor: ACCENT }} disabled={!folderName.trim()} onClick={createFolder}>{t("mindmap.create")}</button>
            </div>
          </div>
        </div>, document.body)}
      {editingFolder && createPortal(
        <div className="fixed inset-0 z-[200] modal-mask flex items-center justify-center bg-black/70 p-6 backdrop-blur-[3px]">
          <div className="w-[340px] rounded-card border border-white/10 bg-surface-panel p-5 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <h3 className="mb-4 text-sm font-semibold text-white">{t("mindmap.renameFolderTitle")}</h3>
            <input className="w-full h-9 rounded-ctl ui-input px-3 text-body text-white outline-none mb-4" value={folderName} onChange={(e) => setFolderName(e.target.value)} autoFocus onKeyDown={(e) => e.key === "Enter" && updateFolder()} />
            <div className="flex justify-end gap-2">
              <button type="button" className="rounded-md px-4 py-1.5 text-caption text-slate-400 hover:text-white" onClick={() => setEditingFolder(null)}>{t("mindmap.cancel")}</button>
              <button type="button" className="rounded-md px-4 py-1.5 text-caption font-semibold text-white disabled:opacity-40" style={{ backgroundColor: ACCENT }} disabled={!folderName.trim()} onClick={updateFolder}>{t("mindmap.save")}</button>
            </div>
          </div>
        </div>, document.body)}
    </div>
  );
}
