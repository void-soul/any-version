import { useCollabRoomActions } from "./useCollabRoomActions";
import { CollabToolPicker } from "./CollabToolPicker";
import { CollabRoomTasks } from "./CollabRoomTasks";
import { CollabRoomList } from "./CollabRoomList";
import { CollabRoomCreate } from "./CollabRoomCreate";
import React, { useEffect, useRef, useState, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { MessagesSquare, Plus, Send, Loader2, Square, RotateCcw, Settings, ChevronDown, Archive } from "lucide-react";
import type { CollabRoom as CollabRoomT, CollabMessage, CollabReference, CollabFileRef, CollabRoomPage, CollabMessagePage, CollabTask, AiProvider, DetectedAiTool, CollabDeltaPayload, CollabActivityPayload, CollabPromptPayload, CollabMsgUpdatedPayload, CollabDispatchOptions, ModelCustomParam, ContextSnapshot, CollabCompactedPayload, ProxyRequestPayload, ProxyResponseStartPayload, ProxyDeltaPayload, ProxyCompletePayload, ProxyErrorPayload, LastLaunchConfig, CollabAgentStatus } from "../../contracts/ai";
import { alertError, theamedAlert, theamedConfirm } from "../shared/ThemedAlert";
import { ModelSelector, CustomParamControls } from "../shared/ai/ModelSelector";
import { MessageView } from "./CollabRoom.MessageView";
import { ReferenceCard } from "./CollabRoom.ReferenceCard";
import { FileBadge } from "./CollabRoom.FileBadge";

export default function CollabRoom() {
  const { t } = useTranslation();
  const [rooms, setRooms] = useState<CollabRoomT[]>([]);
  const [activeRoom, setActiveRoom] = useState<CollabRoomT | null>(null);
  const [messages, setMessages] = useState<CollabMessage[]>([]);
  // E 任务流状态
  const [tasks, setTasks] = useState<CollabTask[]>([]);
  const [tasksOpen, setTasksOpen] = useState(false);
  const [newTaskTitle, setNewTaskTitle] = useState("");
  const [newTaskDesc, setNewTaskDesc] = useState("");
  // 线程缩进：按 reply_to 链计算深度
  const byId = useMemo(() => {
    const m: Record<string, CollabMessage> = {};
    messages.forEach((x) => (m[x.id] = x));
    return m;
  }, [messages]);
  const depthOf = (m: CollabMessage): number => {
    let d = 0;
    let cur = m.reply_to;
    while (cur && byId[cur] && d < 6) {
      d++;
      cur = byId[cur].reply_to;
    }
    return d;
  };
  const [tools, setTools] = useState<DetectedAiTool[]>([]);
  const [toolsLoading, setToolsLoading] = useState(false);
  const [providers, setProviders] = useState<AiProvider[]>([]);

  const [content, setContent] = useState("");
  const [selectedTool, setSelectedTool] = useState<string>("");
  const [references, setReferences] = useState<CollabReference[]>([]);
  const [files, setFiles] = useState<CollabFileRef[]>([]);
  const [providerId, setProviderId] = useState<string>("");
  const [modelId, setModelId] = useState<string>("");
  // fallback/小模型设置（与 ToolLauncher 对齐）
  const [fallbackModelId, setFallbackModelId] = useState<string>("");
  const [fallbackProviderId, setFallbackProviderId] = useState<string>("");

  // 高级协议设置（与工具启动页对齐）
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [masqueradeModel, setMasqueradeModel] = useState<string>("");
  const [oneMContext, setOneMContext] = useState(false);
  const [optimizerEnabled, setOptimizerEnabled] = useState(true);
  const [rectifierEnabled, setRectifierEnabled] = useState(true);
  // 当前模型自定义启动参数的取值（param key → 用户选的值）
  const [customParamValues, setCustomParamValues] = useState<Record<string, string>>({});

  // 当前选中模型的自定义启动参数模板（来自该模型定义）
  const currentModelCustomParams = React.useMemo<ModelCustomParam[]>(() => {
    if (!providerId || !modelId) return [];
    return providers
      .find(p => p.id === providerId)?.models
      .find(m => m.id === modelId)?.customParams || [];
  }, [providers, providerId, modelId]);

  // 模型变化时把自定义参数取值重置为默认值
  const resetCustomParamValues = React.useCallback((params: ModelCustomParam[]) => {
    const defs: Record<string, string> = {};
    for (const cp of params) if (cp.defaultValue) defs[cp.key] = cp.defaultValue;
    setCustomParamValues(defs);
  }, []);

  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [newProject, setNewProject] = useState("");
  const [busy, setBusy] = useState(false);
  const [compacting, setCompacting] = useState(false);
  const [hasSnapshot, setHasSnapshot] = useState(false);
  const [activityMap, setActivityMap] = useState<Record<string, string>>({});
  const [promptMap, setPromptMap] = useState<Record<string, { question: string; options: string[] }>>({});
  // 代理层状态：实时显示 LLM 响应进度（即使工具 stdout 未输出）
  const [proxyMap, setProxyMap] = useState<Record<string, { text: string; status: string; elapsed?: number }>>({});
  // agent 在线状态（工具当前运行状态）
  const [agents, setAgents] = useState<Record<string, CollabAgentStatus>>({});
  const scrollRef = useRef<HTMLDivElement>(null);

  // 流式事件相关 ref
  const activeRoomIdRef = useRef<string | null>(null);
  const runningMsgIdRef = useRef<string | null>(null);
  // B: 跟踪已 finalize 的消息 id，防止 delta 事件在 msg-updated 后到达时追加到已完成的消息
  const finalizedIdsRef = useRef<Set<string>>(new Set());
  // D: 缓存当前 running 消息在数组中的索引，避免高频 delta 时 O(n) findIndex
  const runningMsgIdxRef = useRef<number>(-1);
  // 预加载更早消息时抑制自动滚到底部，保持视口位置
  const suppressAutoScrollRef = useRef(false);
  useEffect(() => {
    activeRoomIdRef.current = activeRoom?.id ?? null;
  }, [activeRoom]);

  // 接收后端流式推送（替代轮询）
  useEffect(() => {
    let cancelled = false;
    const unlistenFns: UnlistenFn[] = [];
    const setup = async () => {
      const unDelta = await listen<CollabDeltaPayload>("collab:delta", (e) => {
        const p = e.payload;
        if (p.room_id !== activeRoomIdRef.current) return;
        // B: 已 finalize 的消息不再追加 delta
        if (finalizedIdsRef.current.has(p.msg_id)) return;
        // D: 用缓存的索引做 O(1) 更新，仅在缓存失效时回退到 findIndex
        setMessages((ms) => {
          let idx = runningMsgIdxRef.current;
          if (idx < 0 || idx >= ms.length || ms[idx].id !== p.msg_id) {
            idx = ms.findIndex((x) => x.id === p.msg_id);
            if (idx === -1) return ms;
            runningMsgIdxRef.current = idx;
          }
          const next = ms.slice();
          next[idx] = { ...next[idx], content: next[idx].content + p.delta };
          return next;
        });
      });
      unlistenFns.push(unDelta);
      if (cancelled) { unDelta(); return; }
      const unActivity = await listen<CollabActivityPayload>("collab:activity", (e) => {
        const p = e.payload;
        if (p.room_id !== activeRoomIdRef.current) return;
        setActivityMap((prev) => ({ ...prev, [p.msg_id]: p.activity }));
      });
      unlistenFns.push(unActivity);
      if (cancelled) { unActivity(); return; }
      const unPrompt = await listen<CollabPromptPayload>("collab:prompt", (e) => {
        const p = e.payload;
        if (p.room_id !== activeRoomIdRef.current) return;
        setPromptMap((prev) => ({ ...prev, [p.msg_id]: { question: p.question, options: p.options } }));
      });
      unlistenFns.push(unPrompt);
      if (cancelled) { unPrompt(); return; }
      const unUpdated = await listen<CollabMsgUpdatedPayload>("collab:msg-updated", (e) => {
        const p = e.payload;
        if (p.room_id !== activeRoomIdRef.current) return;
        setMessages((ms) => ms.map((x) => (x.id === p.message.id ? p.message : x)));
        // B: 标记已 finalize，后续 delta 事件将被忽略
        if (p.message.status && p.message.status !== "running") {
          finalizedIdsRef.current.add(p.message.id);
          runningMsgIdxRef.current = -1;
        }
        // 清除活动状态和 prompt 状态
        setActivityMap((prev) => {
          const next = { ...prev };
          delete next[p.message.id];
          return next;
        });
        setPromptMap((prev) => {
          const next = { ...prev };
          delete next[p.message.id];
          return next;
        });
        setProxyMap((prev) => {
          const next = { ...prev };
          delete next[p.message.id];
          return next;
        });
        if (p.message.status && p.message.status !== "running") {
          setBusy(false);
        }
      });
      unlistenFns.push(unUpdated);
      if (cancelled) { unUpdated(); return; }
      const unCompactStarted = await listen<CollabMessage>("collab:compact-started", (e) => {
        const msg = e.payload;
        if (msg.room_id !== activeRoomIdRef.current) return;
        runningMsgIdRef.current = msg.id;
        runningMsgIdxRef.current = -1;
        finalizedIdsRef.current.delete(msg.id);
        setMessages((prev) => [...prev, msg]);
        setBusy(true);
      });
      unlistenFns.push(unCompactStarted);
      if (cancelled) { unCompactStarted(); return; }
      const unCompacted = await listen<CollabCompactedPayload>("collab:compacted", (e) => {
        const p = e.payload;
        if (p.room_id !== activeRoomIdRef.current) return;
        setHasSnapshot(!!p.snapshot);
      });
      unlistenFns.push(unCompacted);
      if (cancelled) { unCompacted(); return; }
      // ── 代理层事件 ──
      const unProxyReq = await listen<ProxyRequestPayload>("collab:proxy-request", (e) => {
        const p = e.payload;
        if (p.room_id !== activeRoomIdRef.current) return;
        setProxyMap((prev) => ({ ...prev, [p.msg_id]: { text: "", status: "requesting" } }));
        setActivityMap((prev) => ({ ...prev, [p.msg_id]: t("collab.activityReq", { model: p.model, count: p.messages, stream: p.stream ? t("collab.activityStream") : "" }) }));
      });
      unlistenFns.push(unProxyReq);
      if (cancelled) { unProxyReq(); return; }
      const unProxyStart = await listen<ProxyResponseStartPayload>("collab:proxy-response-start", (e) => {
        const p = e.payload;
        if (p.room_id !== activeRoomIdRef.current) return;
        setProxyMap((prev) => ({ ...prev, [p.msg_id]: { text: prev[p.msg_id]?.text || "", status: `UPSTREAM ${p.status}` } }));
        setActivityMap((prev) => ({ ...prev, [p.msg_id]: t("collab.activityUpstream", { status: p.status, ms: p.elapsed_ms }) }));
      });
      unlistenFns.push(unProxyStart);
      if (cancelled) { unProxyStart(); return; }
      const unProxyDelta = await listen<ProxyDeltaPayload>("collab:proxy-delta", (e) => {
        const p = e.payload;
        if (p.room_id !== activeRoomIdRef.current) return;
        setProxyMap((prev) => {
          const cur = prev[p.msg_id] || { text: "", status: "streaming" };
          const text = cur.text + p.delta;
          const preview = text.slice(-60);
          return { ...prev, [p.msg_id]: { text, status: `streaming: ${preview}` } };
        });
      });
      unlistenFns.push(unProxyDelta);
      if (cancelled) { unProxyDelta(); return; }
      const unProxyComplete = await listen<ProxyCompletePayload>("collab:proxy-complete", (e) => {
        const p = e.payload;
        if (p.room_id !== activeRoomIdRef.current) return;
        setProxyMap((prev) => ({ ...prev, [p.msg_id]: { text: p.text, status: "complete", elapsed: p.elapsed_ms } }));
        setActivityMap((prev) => ({ ...prev, [p.msg_id]: t("collab.activityComplete", { count: p.text.length, ms: p.elapsed_ms }) }));
      });
      unlistenFns.push(unProxyComplete);
      if (cancelled) { unProxyComplete(); return; }
      const unProxyError = await listen<ProxyErrorPayload>("collab:proxy-error", (e) => {
        const p = e.payload;
        if (p.room_id !== activeRoomIdRef.current) return;
        setProxyMap((prev) => ({ ...prev, [p.msg_id]: { text: "", status: `error: ${p.status}` } }));
        setActivityMap((prev) => ({ ...prev, [p.msg_id]: t("collab.activityError", { status: p.status, err: p.error.slice(0, 100) }) }));
      });
      unlistenFns.push(unProxyError);
      if (cancelled) { unProxyError(); return; }
      const unTasks = await listen<{ room_id: string }>("collab:tasks-updated", (e) => {
        if (e.payload.room_id !== activeRoomIdRef.current) return;
        loadTasks(e.payload.room_id);
      });
      unlistenFns.push(unTasks);
      if (cancelled) { unTasks(); return; }
    };
    setup();
    return () => {
      cancelled = true;
      unlistenFns.forEach((f) => f());
    };
  }, []);

  // 会话列表分页（延迟加载）
  const PAGE = 20;
  const [hasMore, setHasMore] = useState(false);
  const [roomOffset, setRoomOffset] = useState(0);
  const [loadingMore, setLoadingMore] = useState(false);
  // 消息分页（滚动到顶部加载更早消息）
  const [msgOffset, setMsgOffset] = useState(0);
  const [msgHasMore, setMsgHasMore] = useState(false);
  const [loadingEarlier, setLoadingEarlier] = useState(false);

  // 初始化：房间 / 工具 / 供应商
  const refreshRooms = () => {
    setLoadingMore(true);
    invoke<CollabRoomPage>("collab_list_rooms", { offset: 0, limit: PAGE })
      .then((page) => {
        setRooms(page.rooms);
        setRoomOffset(0);
        setHasMore(page.has_more);
      })
      .catch(() => setRooms([]))
      .finally(() => setLoadingMore(false));
  };

  // 加载上次启动配置（包含 fallback 模型设置）
  const loadLastLaunchConfig = async (toolId: string) => {
    try {
      const lc = await invoke<LastLaunchConfig | null>("get_last_launch_config", { toolId });
      if (lc) {
        // camelCase：Rust 侧 LastLaunchConfig 带 rename_all = camelCase
        if (lc.fallbackModelId) setFallbackModelId(lc.fallbackModelId);
        if (lc.fallbackProviderId) setFallbackProviderId(lc.fallbackProviderId);
      }
    } catch {
      // 忽略错误
    }
  };

  // ─── E 任务流：加载与操作 ───
  const loadTasks = (roomId: string) => {
    invoke<CollabTask[]>("collab_list_tasks", { roomId })
      .then(setTasks)
      .catch(() => setTasks([]));
  };
  const taskAction = (op: string, taskId: string) => {
    if (!activeRoom) return;
    invoke("collab_task_action", { roomId: activeRoom.id, body: { op, task_id: taskId, by: "user" } })
      .then(() => loadTasks(activeRoom.id))
      .catch((e) => console.error("[CollabRoom] 任务操作失败:", e));
  };
  const createTask = () => {
    if (!activeRoom || !newTaskTitle.trim()) return;
    invoke("collab_task_action", {
      roomId: activeRoom.id,
      body: {
        op: "create",
        title: newTaskTitle.trim(),
        description: newTaskDesc,
        created_by: "user",
        // 幂等键：双击建任务不会建出两个一样的
        request_id: `${Date.now()}-${Math.random().toString(36).slice(2, 10)}`,
      },
    })
      .then(() => {
        setNewTaskTitle("");
        setNewTaskDesc("");
        loadTasks(activeRoom.id);
      })
      .catch((e) => console.error("[CollabRoom] 创建任务失败:", e));
  };

  useEffect(() => {
    refreshRooms();
    invoke<DetectedAiTool[]>("detect_ai_tools").then(setTools).catch(() => setTools([])).finally(() => setToolsLoading(false));
    setToolsLoading(true);
    invoke<{ providers: AiProvider[] }>("get_ai_config")
      .then((c) => {
        setProviders(c.providers);
        const pid = c.providers[0]?.id || "";
        setProviderId(pid);
        const p = c.providers.find((x) => x.id === pid);
        setModelId(p?.active_model_id || p?.models[0]?.id || "");
      })
      .catch((e) => console.error("[CollabRoom] 加载模型失败:", e));
  }, []);

  // 切换房间时加载任务列表（E）
  useEffect(() => {
    if (activeRoom?.id) loadTasks(activeRoom.id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeRoom?.id]);

  // 切换房间时加载消息并重置状态
  useEffect(() => {
    setBusy(false);
    setCompacting(false);
    setActivityMap({});
    setPromptMap({});
    setProxyMap({});
    runningMsgIdRef.current = null;
    runningMsgIdxRef.current = -1;
    finalizedIdsRef.current = new Set();
    if (!activeRoom) {
      setMessages([]);
      setHasSnapshot(false);
      setMsgOffset(0);
      setMsgHasMore(false);
      return;
    }
    invoke<CollabMessagePage>("collab_get_messages", {
      roomId: activeRoom.id,
      tail: true,
      limit: PAGE,
    })
      .then((page) => {
        setMessages(page.messages);
        setMsgOffset(Math.max(0, page.total - page.messages.length));
        setMsgHasMore(page.has_more);
      })
      .catch(() => {
        setMessages([]);
        setMsgOffset(0);
        setMsgHasMore(false);
      });
  }, [activeRoom]);

  // 加载 agent 在线状态
  const loadAgents = async () => {
    try {
      const list = await invoke<CollabAgentStatus[]>("collab_get_agents");
      const map: Record<string, CollabAgentStatus> = {};
      list.forEach((a) => {
        map[a.tool_id] = a;
      });
      setAgents(map);
    } catch {
      /* ignore */
    }
  };

  // 切换房间后刷新在线状态，并每 3s 轮询
  useEffect(() => {
    if (!activeRoom) return;
    loadAgents();
    const t = setInterval(() => loadAgents(), 3000);
    return () => clearInterval(t);
  }, [activeRoom]);

  // 检查快照状态（切换房间或工具时）
  useEffect(() => {
    if (!activeRoom || !selectedTool) {
      setHasSnapshot(false);
      return;
    }
    invoke<ContextSnapshot | null>("collab_get_snapshot", {
      roomId: activeRoom.id,
      toolId: selectedTool,
    }).then((snap) => setHasSnapshot(!!snap)).catch(() => setHasSnapshot(false));
  }, [activeRoom, selectedTool]);

  // 切换工具时加载上次启动配置（包含 fallback 模型设置）
  useEffect(() => {
    if (selectedTool) {
      loadLastLaunchConfig(selectedTool);
    }
  }, [selectedTool]);

  // E: 新消息自动滚动到底（用 requestAnimationFrame 节流，避免高频 delta 时卡顿）
  const scrollRafRef = useRef<number | null>(null);
  useEffect(() => {
    if (suppressAutoScrollRef.current) {
      suppressAutoScrollRef.current = false;
      return;
    }
    if (scrollRafRef.current != null) return;
    scrollRafRef.current = requestAnimationFrame(() => {
      scrollRafRef.current = null;
      scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
    });
    return () => {
      if (scrollRafRef.current != null) {
        cancelAnimationFrame(scrollRafRef.current);
        scrollRafRef.current = null;
      }
    };
  }, [messages]);

  const {
    handlePickProject,
    createRoom,
    deleteRoom,
  } = useCollabRoomActions({
    setNewProject,
    newName,
    setNewName,
    newProject,
    refreshRooms,
    setActiveRoom,
    setCreating,
    activeRoom,
  });

  // 会话列表滚动到底自动加载下一页
  const loadMoreRooms = async () => {
    if (!hasMore || loadingMore) return;
    setLoadingMore(true);
    try {
      const next = roomOffset + PAGE;
      const page = await invoke<CollabRoomPage>("collab_list_rooms", {
        offset: next,
        limit: PAGE,
      });
      setRooms((rs) => [...rs, ...page.rooms]);
      setRoomOffset(next);
      setHasMore(page.has_more);
    } catch {
      /* ignore */
    } finally {
      setLoadingMore(false);
    }
  };

  const onRoomsScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    if (el.scrollHeight - el.scrollTop - el.clientHeight < 48) loadMoreRooms();
  };

  // 滚动到消息顶部时加载更早的消息（prepend），保持视口位置不跳动
  const loadEarlierMessages = async () => {
    if (!activeRoom || !msgHasMore || loadingEarlier) return;
    setLoadingEarlier(true);
    const prevHeight = scrollRef.current?.scrollHeight ?? 0;
    suppressAutoScrollRef.current = true;
    try {
      const next = Math.max(0, msgOffset - PAGE);
      const page = await invoke<CollabMessagePage>("collab_get_messages", {
        roomId: activeRoom.id,
        offset: next,
        limit: PAGE,
      });
      setMessages((ms) => [...page.messages, ...ms]);
      setMsgOffset(next);
      setMsgHasMore(page.has_more);
      requestAnimationFrame(() => {
        if (scrollRef.current) {
          scrollRef.current.scrollTop += scrollRef.current.scrollHeight - prevHeight;
        }
      });
    } catch {
      suppressAutoScrollRef.current = false;
    } finally {
      setLoadingEarlier(false);
    }
  };

  // 在输入框输入 @（词首/空格后）自动弹出文件选择，选中即作为附件
  const openFilePicker = async () => {
    if (!activeRoom) return;
    try {
      const selected = await open({
        multiple: true,
        defaultPath: activeRoom.project_path,
        title: t("collab.pickFilesTitle"),
      });
      if (!selected) return;
      const list = (Array.isArray(selected) ? selected : [selected]) as string[];
      setFiles((fs) => {
        const seen = new Set(fs.map((f) => f.path));
        const merged = [...fs];
        for (const p of list) if (!seen.has(p)) merged.push({ path: p });
        return merged;
      });
    } catch {
      /* ignore */
    }
  };

  const onContentKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "@") {
      const el = e.currentTarget;
      const pos = el.selectionStart ?? 0;
      const prev = pos > 0 ? el.value[pos - 1] : "";
      // 仅在词首或空格/换行后触发，避免干扰邮箱等普通 @
      if (pos === 0 || /\s/.test(prev)) {
        e.preventDefault();
        openFilePicker();
      }
      return;
    }
    // 回车发送；Ctrl/⌘+回车换行（不拦截，保留默认换行）
    if (e.key === "Enter" && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      send();
    }
  };

  const quoteMessage = (m: CollabMessage) => {
    // 避免重复引用同一消息
    if (references.some((r) => r.source_message_id === m.id)) return;
    // 截取摘要（前 300 字符），避免大响应导致巨大引用卡
    const excerpt = m.content.length > 300
      ? m.content.slice(0, 300) + "…"
      : m.content;
    setReferences((rs) => [
      ...rs,
      { source_message_id: m.id, source_sender_name: m.sender_name, excerpt },
    ]);
  };

  const buildDispatchOptions = (): CollabDispatchOptions => {
    const selectedToolObj = tools.find((t) => t.id === selectedTool);
    const supportsModel = selectedToolObj?.supports_model ?? false;
    return {
      masquerade_model: supportsModel ? (masqueradeModel || null) : null,
      fallback_model_id: supportsModel ? (fallbackModelId || null) : null,
      fallback_provider_id: supportsModel ? (fallbackProviderId || null) : null,
      fallback_masquerade_model: null,
      one_m_context: supportsModel && selectedToolObj?.support_one_m_context ? oneMContext : false,
      fallback_one_m_context: false,
      optimizer_enabled: supportsModel && selectedToolObj?.supports_optimizer ? optimizerEnabled : null,
      rectifier_enabled: supportsModel && selectedToolObj?.supports_rectifier ? rectifierEnabled : null,
      optimizer_cache_injection: null,
      optimizer_thinking: null,
      optimizer_deepseek: null,
      rectifier_thinking_signature: null,
      rectifier_thinking_budget: null,
      rectifier_media_fallback: null,
      rectifier_media_heuristic: null,
      rectifier_protocol_mismatch: null,
      rectifier_toolcall_dialect: null,
      custom_params: supportsModel ? currentModelCustomParams : [],
      custom_param_values: supportsModel ? customParamValues : {},
    };
  };

  const send = async () => {
    if (!activeRoom) return;
    if (busy || compacting) return;
    if (!content.trim() && references.length === 0 && files.length === 0) return;
    if (!selectedTool) {
      theamedAlert(t("collab.needToolFirst"));
      return;
    }
    setBusy(true);
    // 不支持模型设置的工具不传模型/供应商
    const toolObj = tools.find((t) => t.id === selectedTool);
    const supportsModel = toolObj?.supports_model ?? false;
    const effectiveModelId = supportsModel ? (modelId || null) : null;
    const effectiveProviderId = supportsModel ? (providerId || null) : null;
    // 幂等键：本次「发送」的唯一 id。UI 重发 / 崩溃重放时后端按它去重，
    // 不会真的再拉起一个 agent（抄 orca 的 mutation_receipts）。
    const requestId = `${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
    try {
      const result = await invoke<CollabMessage[]>("collab_send_message", {
        roomId: activeRoom.id,
        projectPath: activeRoom.project_path,
        content: content.trim(),
        references,
        files,
        toolId: selectedTool,
        modelId: effectiveModelId,
        providerId: effectiveProviderId,
        options: buildDispatchOptions(),
        requestId,
      });
      const placeholder = result.find((m) => m.sender !== "user");
      runningMsgIdRef.current = placeholder?.id ?? null;
      runningMsgIdxRef.current = -1; // 重置索引缓存，首个 delta 到达时自动查找
      if (placeholder) finalizedIdsRef.current.delete(placeholder.id);
      // 追加新消息，不覆盖已有历史
      setMessages((prev) => [...prev, ...result]);
      setContent("");
      setReferences([]);
      setFiles([]);
    } catch (e: unknown) {
      alertError(t("collab.sendFail", { err: String(e) }));
      setBusy(false);
    }
  };

  const stopDispatch = () => {
    if (runningMsgIdRef.current) {
      invoke("collab_cancel_dispatch", { msgId: runningMsgIdRef.current }).catch((e) =>
        console.error("[CollabRoom] 取消派发失败:", e)
      );
    }
  };

  const resetSession = async () => {
    if (!activeRoom || !selectedTool) return;
    if (!(await theamedConfirm(t("collab.resetCtxConfirm"), { danger: true }))) return;
    await invoke("collab_reset_session", { roomId: activeRoom.id, toolId: selectedTool }).catch((e) =>
      console.error("[CollabRoom] 重置会话失败:", e)
    );
    setHasSnapshot(false);
  };

  const compact = async () => {
    if (!activeRoom || !selectedTool) return;
    if (!(await theamedConfirm(t("collab.compactConfirm")))) return;
    setCompacting(true);
    // 不支持模型设置的工具不传模型/供应商
    const toolObj = tools.find((t) => t.id === selectedTool);
    const supportsModel = toolObj?.supports_model ?? false;
    const effectiveModelId = supportsModel ? (modelId || null) : null;
    const effectiveProviderId = supportsModel ? (providerId || null) : null;
    try {
      const snapshot = await invoke<ContextSnapshot | null>("collab_compact_session", {
        roomId: activeRoom.id,
        toolId: selectedTool,
        projectPath: activeRoom.project_path,
        modelId: effectiveModelId,
        providerId: effectiveProviderId,
        options: buildDispatchOptions(),
      });
      if (snapshot) {
        setHasSnapshot(true);
      }
    } catch (e: unknown) {
      alertError(t("collab.compactFail", { err: String(e) }));
    } finally {
      setCompacting(false);
    }
  };

  const selectedToolObj = tools.find((t) => t.id === selectedTool);
  const showModelSettings = !!selectedToolObj && selectedToolObj.supports_model;

  return (
    <div className="h-full flex min-h-0 select-none text-slate-100">
      {/* 房间侧栏 */}
      <div className="w-48 flex-shrink-0 border-r border-white/5 py-3 px-2 flex flex-col">
        <div className="flex items-center justify-between px-1 mb-2">
          <span className="text-caption font-bold text-slate-300">{t("collab.sessionList")}</span>
          <button
            onClick={() => setCreating(true)}
            className="p-1 rounded hover:bg-white/10 text-slate-400 hover:text-[var(--module-accent)] cursor-pointer"
            title={t("collab.newSession")}
          >
            <Plus className="w-3.5 h-3.5" />
          </button>
        </div>

        {creating && (
          <CollabRoomCreate
            newName={newName}
            setNewName={setNewName}
            handlePickProject={handlePickProject}
            newProject={newProject}
            createRoom={createRoom}
            setCreating={setCreating}
          />
        )}

        <CollabRoomList
          onRoomsScroll={onRoomsScroll}
          rooms={rooms}
          loadingMore={loadingMore}
          activeRoom={activeRoom}
          setActiveRoom={setActiveRoom}
          deleteRoom={deleteRoom}
          hasMore={hasMore}
        />
      </div>

      {/* 主区域 */}
      <div className="flex-1 min-h-0 flex flex-col">
        {!activeRoom ? (
          <div className="flex-1 flex flex-col items-center justify-center text-slate-500 gap-2">
            <MessagesSquare className="w-10 h-10 opacity-30" />
            <p className="text-body">{t("collab.pickSessionHint")}</p>
          </div>
        ) : (
          <>
            {/* 线程 */}
            <div
              ref={scrollRef}
              className="flex-1 overflow-y-auto px-4 py-3 space-y-3"
              onScroll={(e) => {
                const el = e.currentTarget;
                if (el.scrollTop < 48 && el.scrollHeight > el.clientHeight) loadEarlierMessages();
              }}
            >
              {messages.map((m) => (
                <div key={m.id} style={{ marginLeft: depthOf(m) * 16 }}>
                  <MessageView
                    m={m}
                    tools={tools}
                    onQuote={quoteMessage}
                    activity={activityMap[m.id]}
                    prompt={promptMap[m.id]}
                    proxy={proxyMap[m.id]}
                    onRespond={(response) => {
                      invoke("collab_respond_prompt", { msgId: m.id, response }).catch(() => {});
                      setPromptMap((prev) => {
                        const next = { ...prev };
                        delete next[m.id];
                        return next;
                      });
                    }}
                  />
                </div>
              ))}
              {messages.length === 0 && (
                <p className="text-caption text-slate-500 text-center mt-8">
                  {t("collab.emptyThreadHint")}
                </p>
              )}
            </div>

            {/* 任务流面板（E）：agent 通过总线认领/交接/完成 */}
            <CollabRoomTasks
              setTasksOpen={setTasksOpen}
              tasks={tasks}
              tasksOpen={tasksOpen}
              taskAction={taskAction}
              newTaskTitle={newTaskTitle}
              setNewTaskTitle={setNewTaskTitle}
              createTask={createTask}
            />

            {/* 输入区 */}
            <div className="flex-shrink-0 border-t border-white/5 p-3 space-y-2 bg-slate-900/40">
              {/* @ 工具选择（单选） */}
              <CollabToolPicker
                toolsLoading={toolsLoading}
                tools={tools}
                selectedTool={selectedTool}
                agents={agents}
                setSelectedTool={setSelectedTool}
              />

              {/* 模型/供应商（仅支持模型设置的工具显示） */}
              {(() => {
                if (!showModelSettings) return null;
                return (
                  <div className="w-full max-w-[300px]">
                    <ModelSelector
                      providers={providers}
                      providerId={providerId}
                      modelId={modelId}
                      onSelect={(pid, mid) => {
                        setProviderId(pid);
                        setModelId(mid);
                        const m = providers.find((p) => p.id === pid)?.models.find((x) => x.id === mid);
                        resetCustomParamValues(m?.customParams || []);
                      }}
                      compact
                    />
                  </div>
                );
              })()}

              {/* 高级协议设置（与工具启动页对齐） */}
              {showModelSettings && (() => {
                const tool = selectedToolObj!;
                return (
                  <div className="rounded-ctl border border-white/5 bg-slate-900/30 overflow-hidden">
                    <button
                      onClick={() => setShowAdvanced(!showAdvanced)}
                      className="w-full flex items-center justify-between px-2.5 py-1.5 text-micro text-slate-400 hover:text-slate-200 cursor-pointer transition-all"
                    >
                      <span className="flex items-center gap-1.5 font-semibold">
                        <Settings className="w-3 h-3" /> {t("collab.advancedSettings")}
                      </span>
                      <ChevronDown className={`w-3 h-3 transition-transform ${showAdvanced ? "rotate-180" : ""}`} />
                    </button>
                    {showAdvanced && (
                      <div className="px-2.5 pb-2.5 space-y-1.5">
                        {/* 模型自定义启动参数（用户定义，运行时渲染为控件） */}
                        {currentModelCustomParams.length > 0 && (
                          <div className="space-y-1.5 pt-0.5">
                            <div className="text-micro text-slate-500 font-semibold">{t("collab.customParams")}</div>
                            <CustomParamControls
                              params={currentModelCustomParams}
                              values={customParamValues}
                              onChange={(key, value) => setCustomParamValues(prev => ({ ...prev, [key]: value }))}
                              compact
                            />
                          </div>
                        )}
                        {/* 模型伪装 */}
                        {tool.builtin_models.length > 0 && (
                          <div className="flex items-center gap-2">
                            <span className="text-micro text-slate-500 w-16 flex-shrink-0">{t("collab.spoofModel")}</span>
                            <select
                              value={masqueradeModel}
                              onChange={(e) => setMasqueradeModel(e.target.value)}
                              className="flex-1 bg-slate-800 border border-white/10 rounded px-1.5 py-0.5 text-micro text-slate-200 focus:outline-none focus:border-[var(--module-accent)]"
                            >
                              <option value="">{t("collab.noSpoof")}</option>
                              {tool.builtin_models.map((m) => (
                                <option key={m} value={m}>{m}</option>
                              ))}
                            </select>
                          </div>
                        )}
                        {/* 1M 上下文 */}
                        {tool.support_one_m_context && (
                          <label className="flex items-center gap-2 cursor-pointer">
                            <input
                              type="checkbox"
                              checked={oneMContext}
                              onChange={(e) => setOneMContext(e.target.checked)}
                              className="w-3 h-3 rounded accent-[var(--module-accent)]"
                            />
                            <span className="text-micro text-slate-400">{t("collab.ctx1m")}</span>
                          </label>
                        )}
                        {/* 优化器 */}
                        {tool.supports_optimizer && (
                          <label className="flex items-center gap-2 cursor-pointer">
                            <input
                              type="checkbox"
                              checked={optimizerEnabled}
                              onChange={(e) => setOptimizerEnabled(e.target.checked)}
                              className="w-3 h-3 rounded accent-[var(--module-accent)]"
                            />
                            <span className="text-micro text-slate-400">{t("collab.optimizer")}</span>
                          </label>
                        )}
                        {/* 整流器 */}
                        {tool.supports_rectifier && (
                          <label className="flex items-center gap-2 cursor-pointer">
                            <input
                              type="checkbox"
                              checked={rectifierEnabled}
                              onChange={(e) => setRectifierEnabled(e.target.checked)}
                              className="w-3 h-3 rounded accent-[var(--module-accent)]"
                            />
                            <span className="text-micro text-slate-400">{t("collab.rectifier")}</span>
                          </label>
                        )}
                      </div>
                    )}
                  </div>
                );
              })()}

              {/* 引用卡 */}
              {references.length > 0 && (
                <div className="space-y-1">
                  {references.map((r, i) => (
                    <ReferenceCard
                      key={i}
                      senderName={r.source_sender_name}
                      excerpt={r.excerpt}
                      onRemove={() => setReferences((rs) => rs.filter((_, j) => j !== i))}
                    />
                  ))}
                </div>
              )}

              {/* 文件附件（@文件） */}
              {files.length > 0 && (
                <div className="flex flex-wrap gap-1">
                  {files.map((f, i) => (
                    <FileBadge
                      key={i}
                      path={f.path}
                      onRemove={() => setFiles((fs) => fs.filter((_, j) => j !== i))}
                    />
                  ))}
                </div>
              )}

              {/* 文本 + 发送 */}
              <div className="flex gap-2 items-end">
                <textarea
                  value={content}
                  onChange={(e) => setContent(e.target.value)}
                  onKeyDown={onContentKeyDown}
                  rows={2}
                  placeholder={t("collab.inputPh")}
                  className="flex-1 bg-slate-800 border border-white/10 rounded-ctl px-3 py-2 text-caption text-slate-200 focus:outline-none focus:border-[var(--module-accent)] resize-none"
                />
                {busy ? (
                  <button
                    onClick={stopDispatch}
                    className="px-4 py-2 rounded-ctl bg-red-600 hover:bg-red-500 text-white text-body font-semibold flex items-center gap-1.5 cursor-pointer"
                  >
                    <Square className="w-3.5 h-3.5" /> {t("collab.stop")}
                  </button>
                ) : (
                  <button
                    onClick={send}
                    className="px-4 py-2 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-body font-semibold flex items-center gap-1.5 cursor-pointer"
                  >
                    <Send className="w-3.5 h-3.5" /> {t("collab.send")}
                  </button>
                )}
                <button
                  onClick={compact}
                  disabled={!selectedTool || busy || compacting}
                  title={t("collab.compactTitle")}
                  className="px-2 py-2 rounded-ctl bg-white/5 hover:bg-white/10 disabled:opacity-40 text-slate-300 text-tiny cursor-pointer"
                >
                  {compacting ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Archive className="w-3.5 h-3.5" />}
                </button>
                <button
                  onClick={resetSession}
                  disabled={!selectedTool}
                  title={t("collab.resetCtxTitle")}
                  className="px-2 py-2 rounded-ctl bg-white/5 hover:bg-white/10 disabled:opacity-40 text-slate-300 text-tiny cursor-pointer"
                >
                  <RotateCcw className="w-3.5 h-3.5" />
                </button>
              </div>
              <p className="text-[8px] text-slate-500">
                {t("collab.continueHint")}
              </p>
              {hasSnapshot && (
                <p className="text-[8px] text-cyan-400 flex items-center gap-1">
                  <Archive className="w-2.5 h-2.5" />
                  {t("collab.snapshotHint")}
                </p>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
