import React, { useEffect, useRef, useState, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { MarkdownRenderer } from "./MarkdownRenderer";
import {
  MessagesSquare,
  Plus,
  Trash2,
  Send,
  Quote,
  Loader2,
  CornerDownRight,
  Bot,
  User as UserIcon,
  Zap,
  Square,
  RotateCcw,
  Settings,
  ChevronDown,
  Archive,
  Pencil,
  Check,
  X,
} from "lucide-react";
import type {
  CollabRoom as CollabRoomT,
  CollabMessage,
  CollabReference,
  CollabFileRef,
  CollabRoomPage,
  CollabMessagePage,
  AiProvider,
  DetectedAiTool,
  CollabDeltaPayload,
  CollabActivityPayload,
  CollabPromptPayload,
  CollabMsgUpdatedPayload,
  CollabDispatchOptions,
  ContextSnapshot,
  CollabCompactedPayload,
  ProxyRequestPayload,
  ProxyResponseStartPayload,
  ProxyDeltaPayload,
  ProxyCompletePayload,
  ProxyErrorPayload,
} from "./types";

// 工具配色（与房间内身份强绑定）
const TOOL_COLORS: Record<string, string> = {
  "claude-code": "bg-orange-500",
  "codex-cli": "bg-emerald-500",
  "opencode": "bg-violet-500",
  "qwencode": "bg-cyan-500",
  "gemini-cli": "bg-amber-500",
  "mimocode": "bg-pink-500",
  "deveco": "bg-teal-500",
};
function senderColor(sender: string): string {
  if (sender === "user") return "bg-blue-500";
  return TOOL_COLORS[sender] || "bg-slate-500";
}

// ─── 人类化状态文案 ───
// 运行中（思考/工作）的随机短语池
const THINKING_PHRASES = [
  "正在思考…",
  "正在分析…",
  "让我想想…",
  "正在整理思路…",
  "正在组织语言…",
  "正在查阅资料…",
  "正在编写代码…",
  "正在检查细节…",
  "正在推理…",
  "正在构思方案…",
];
// 错误时的随机短语池
const ERROR_PHRASES = [
  "出了点小问题",
  "遇到了一些困难",
  "好像哪里不对",
  "需要一点帮助",
  "处理时遇到了障碍",
  "出了点意外",
];
/// 随机选择一个短语
function pickRandom(arr: string[]): string {
  return arr[Math.floor(Math.random() * arr.length)];
}

/// 轮换短语的自定义 Hook：每隔 intervalMs 毫秒随机切换一个短语
function useRotatingPhrase(phrases: string[], active: boolean, intervalMs = 4000): string {
  const [idx, setIdx] = useState(() => Math.floor(Math.random() * phrases.length));
  useEffect(() => {
    if (!active) return;
    const timer = setInterval(() => {
      setIdx(Math.floor(Math.random() * phrases.length));
    }, intervalMs);
    return () => clearInterval(timer);
  }, [active, phrases.length, intervalMs]);
  return phrases[idx] || phrases[0] || "";
}

// ─── 自定义昵称/头像持久化 ───
const CUSTOM_NAMES_KEY = "any-version:custom-tool-names";

type CustomToolName = { avatar?: string; nickname?: string };

function loadCustomNames(): Record<string, CustomToolName> {
  try {
    const raw = localStorage.getItem(CUSTOM_NAMES_KEY);
    return raw ? JSON.parse(raw) : {};
  } catch {
    return {};
  }
}

function saveCustomNames(map: Record<string, CustomToolName>) {
  try {
    localStorage.setItem(CUSTOM_NAMES_KEY, JSON.stringify(map));
  } catch {
    /* ignore */
  }
}

/// 获取有效头像：自定义 > 配置 > null
function getToolAvatar(tools: DetectedAiTool[], toolId: string, customMap?: Record<string, CustomToolName>): string | null {
  const custom = customMap?.[toolId];
  if (custom?.avatar) return custom.avatar;
  const t = tools.find((x) => x.id === toolId);
  return t?.avatar ?? null;
}
/// 获取有效昵称：自定义 > 配置 > display_name
function getToolNickname(tools: DetectedAiTool[], toolId: string, customMap?: Record<string, CustomToolName>): string | null {
  const custom = customMap?.[toolId];
  if (custom?.nickname) return custom.nickname;
  const t = tools.find((x) => x.id === toolId);
  return t?.nickname ?? null;
}

export default function CollabRoom() {
  const [rooms, setRooms] = useState<CollabRoomT[]>([]);
  const [activeRoom, setActiveRoom] = useState<CollabRoomT | null>(null);
  const [messages, setMessages] = useState<CollabMessage[]>([]);
  const [tools, setTools] = useState<DetectedAiTool[]>([]);
  const [providers, setProviders] = useState<AiProvider[]>([]);

  const [content, setContent] = useState("");
  const [selectedTool, setSelectedTool] = useState<string>("");
  const [references, setReferences] = useState<CollabReference[]>([]);
  const [files, setFiles] = useState<CollabFileRef[]>([]);
  const [providerId, setProviderId] = useState<string>("");
  const [modelId, setModelId] = useState<string>("");

  // 高级协议设置（与工具启动页对齐）
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [masqueradeModel, setMasqueradeModel] = useState<string>("");
  const [oneMContext, setOneMContext] = useState(false);
  const [optimizerEnabled, setOptimizerEnabled] = useState(true);
  const [rectifierEnabled, setRectifierEnabled] = useState(true);

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
  // 自定义工具昵称/头像（localStorage 持久化）
  const [customNames, setCustomNames] = useState<Record<string, CustomToolName>>({});
  // 正在编辑昵称/头像的工具 id
  const [editingToolId, setEditingToolId] = useState<string | null>(null);
  const [editAvatar, setEditAvatar] = useState("");
  const [editNickname, setEditNickname] = useState("");
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
        setActivityMap((prev) => ({ ...prev, [p.msg_id]: `代理请求: ${p.model} (${p.messages}条消息${p.stream ? ", 流式" : ""})` }));
      });
      unlistenFns.push(unProxyReq);
      if (cancelled) { unProxyReq(); return; }
      const unProxyStart = await listen<ProxyResponseStartPayload>("collab:proxy-response-start", (e) => {
        const p = e.payload;
        if (p.room_id !== activeRoomIdRef.current) return;
        setProxyMap((prev) => ({ ...prev, [p.msg_id]: { text: prev[p.msg_id]?.text || "", status: `UPSTREAM ${p.status}` } }));
        setActivityMap((prev) => ({ ...prev, [p.msg_id]: `上游响应 ${p.status} (${p.elapsed_ms}ms)` }));
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
        setActivityMap((prev) => ({ ...prev, [p.msg_id]: `代理已收到完整响应 (${p.text.length}字, ${p.elapsed_ms}ms)` }));
      });
      unlistenFns.push(unProxyComplete);
      if (cancelled) { unProxyComplete(); return; }
      const unProxyError = await listen<ProxyErrorPayload>("collab:proxy-error", (e) => {
        const p = e.payload;
        if (p.room_id !== activeRoomIdRef.current) return;
        setProxyMap((prev) => ({ ...prev, [p.msg_id]: { text: "", status: `error: ${p.status}` } }));
        setActivityMap((prev) => ({ ...prev, [p.msg_id]: `代理错误 ${p.status}: ${p.error.slice(0, 100)}` }));
      });
      unlistenFns.push(unProxyError);
      if (cancelled) { unProxyError(); return; }
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
  useEffect(() => {
    refreshRooms();
    invoke<DetectedAiTool[]>("detect_ai_tools").then(setTools).catch(() => setTools([]));
    invoke<{ providers: AiProvider[]; active_provider: string | null }>("get_ai_config")
      .then((c) => {
        setProviders(c.providers);
        const pid = c.active_provider || c.providers[0]?.id || "";
        setProviderId(pid);
        const p = c.providers.find((x) => x.id === pid);
        setModelId(p?.active_model_id || p?.models[0]?.id || "");
      })
      .catch(() => {});
    setCustomNames(loadCustomNames());
  }, []);

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

  const handlePickProject = async () => {
    try {
      const selected = await open({ directory: true, title: "选择项目目录" });
      if (selected) {
        const dir = selected as string;
        setNewProject(dir);
        if (!newName.trim()) setNewName(dir.split(/[\\/]/).pop() || "");
      }
    } catch {
      /* ignore */
    }
  };

  const createRoom = async () => {
    if (!newProject.trim()) {
      alert("请先选择项目目录");
      return;
    }
    const room = await invoke<CollabRoomT>("collab_create_room", {
      name: newName || "未命名会话",
      projectPath: newProject.trim(),
    });
    await refreshRooms();
    setActiveRoom(room);
    setCreating(false);
    setNewName("");
    setNewProject("");
  };

  const deleteRoom = async (id: string) => {
    await invoke("collab_delete_room", { roomId: id }).catch(() => {});
    await refreshRooms();
    if (activeRoom?.id === id) setActiveRoom(null);
  };

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
        title: "选择要附带的文件（@文件）",
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

  const onProviderChange = (pid: string) => {
    setProviderId(pid);
    const p = providers.find((x) => x.id === pid);
    setModelId(p?.active_model_id || p?.models[0]?.id || "");
  };

  const buildDispatchOptions = (): CollabDispatchOptions => {
    const selectedToolObj = tools.find((t) => t.id === selectedTool);
    const supportsModel = selectedToolObj?.supports_model ?? false;
    return {
      masquerade_model: supportsModel ? (masqueradeModel || null) : null,
      fallback_model_id: null,
      fallback_provider_id: null,
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
      rectifier_protocol_mismatch: null,
    };
  };

  const send = async () => {
    if (!activeRoom) return;
    if (busy || compacting) return;
    if (!content.trim() && references.length === 0 && files.length === 0) return;
    if (!selectedTool) {
      alert("请先 @ 一个工具来接手任务");
      return;
    }
    setBusy(true);
    // 不支持模型设置的工具不传模型/供应商
    const toolObj = tools.find((t) => t.id === selectedTool);
    const supportsModel = toolObj?.supports_model ?? false;
    const effectiveModelId = supportsModel ? (modelId || null) : null;
    const effectiveProviderId = supportsModel ? (providerId || null) : null;
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
      alert(`发送失败: ${e}`);
      setBusy(false);
    }
  };

  const stopDispatch = () => {
    if (runningMsgIdRef.current) {
      invoke("collab_cancel_dispatch", { msgId: runningMsgIdRef.current }).catch(() => {});
    }
  };

  const resetSession = async () => {
    if (!activeRoom || !selectedTool) return;
    if (!window.confirm("重置该工具在此会话中的续聊上下文？下次发送将开启全新会话。")) return;
    await invoke("collab_reset_session", { roomId: activeRoom.id, toolId: selectedTool }).catch(() => {});
    setHasSnapshot(false);
  };

  const compact = async () => {
    if (!activeRoom || !selectedTool) return;
    if (!window.confirm("压缩上下文？将请求 AI 总结当前会话并开启新会话。")) return;
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
      alert(`压缩失败: ${e}`);
    } finally {
      setCompacting(false);
    }
  };

  // ─── 自定义昵称/头像保存 ───
  const startEditTool = (toolId: string) => {
    const t = tools.find((x) => x.id === toolId);
    const custom = customNames[toolId];
    setEditAvatar(custom?.avatar ?? t?.avatar ?? "");
    setEditNickname(custom?.nickname ?? t?.nickname ?? t?.display_name ?? "");
    setEditingToolId(toolId);
  };
  const saveEditTool = () => {
    if (!editingToolId) return;
    const next = { ...customNames };
    const t = tools.find((x) => x.id === editingToolId);
    // 仅保存与配置不同的值
    const entry: CustomToolName = {};
    if (editAvatar.trim() && editAvatar.trim() !== (t?.avatar ?? "")) entry.avatar = editAvatar.trim();
    if (editNickname.trim() && editNickname.trim() !== (t?.nickname ?? t?.display_name ?? "")) entry.nickname = editNickname.trim();
    if (Object.keys(entry).length > 0) {
      next[editingToolId] = entry;
    } else {
      delete next[editingToolId];
    }
    setCustomNames(next);
    saveCustomNames(next);
    setEditingToolId(null);
  };
  const cancelEditTool = () => setEditingToolId(null);

  const activeProvider = providers.find((p) => p.id === providerId);
  const selectedToolObj = tools.find((t) => t.id === selectedTool);
  const showModelSettings = !!selectedToolObj && selectedToolObj.supports_model;

  return (
    <div className="h-full flex min-h-0 select-none text-slate-100">
      {/* 房间侧栏 */}
      <div className="w-48 flex-shrink-0 border-r border-white/5 py-3 px-2 flex flex-col">
        <div className="flex items-center justify-between px-1 mb-2">
          <span className="text-[11px] font-bold text-slate-300">会话列表</span>
          <button
            onClick={() => setCreating(true)}
            className="p-1 rounded hover:bg-white/10 text-slate-400 hover:text-violet-300 cursor-pointer"
            title="新建会话"
          >
            <Plus className="w-3.5 h-3.5" />
          </button>
        </div>

        {creating && (
          <div className="mb-2 p-2 rounded-lg bg-slate-900/60 border border-white/10 space-y-1.5">
            <input
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              placeholder="会话名"
              className="w-full bg-slate-800 border border-white/10 rounded px-2 py-1 text-[10px] text-slate-200 focus:outline-none focus:border-violet-500"
            />
            <button
              onClick={handlePickProject}
              className={`w-full px-2 py-1 rounded text-left text-[10px] border border-white/10 truncate cursor-pointer ${
                newProject
                  ? "bg-slate-800 text-slate-200"
                  : "bg-slate-800/60 text-slate-500 hover:text-slate-300"
              }`}
              title={newProject || "选择项目目录"}
            >
              {newProject || "选择项目目录…"}
            </button>
            <div className="flex gap-1">
              <button
                onClick={createRoom}
                className="flex-1 px-2 py-1 rounded bg-violet-600 hover:bg-violet-500 text-[10px] font-semibold cursor-pointer"
              >
                创建
              </button>
              <button
                onClick={() => setCreating(false)}
                className="px-2 py-1 rounded bg-white/5 hover:bg-white/10 text-[10px] cursor-pointer"
              >
                取消
              </button>
            </div>
          </div>
        )}

        <div className="flex-1 overflow-y-auto space-y-0.5" onScroll={onRoomsScroll}>
          {rooms.length === 0 && !loadingMore && (
            <p className="text-[10px] text-slate-500 px-1 leading-relaxed">
              还没有会话。点上方 + 新建一个，把多个 CLI 工具拉进同一个群。
            </p>
          )}
          {rooms.map((r) => (
            <div
              key={r.id}
              className={`group flex items-center gap-1 px-2 py-1.5 rounded-lg cursor-pointer text-[11px] ${
                activeRoom?.id === r.id
                  ? "bg-violet-600/30 text-violet-200"
                  : "text-slate-400 hover:bg-white/5"
              }`}
              onClick={() => setActiveRoom(r)}
            >
              <MessagesSquare className="w-3 h-3 flex-shrink-0" />
              <div className="flex-1 min-w-0">
                <div className="truncate font-medium">{r.name}</div>
                <div className="truncate text-[8px] text-slate-500">{r.project_path}</div>
              </div>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  deleteRoom(r.id);
                }}
                className="opacity-0 group-hover:opacity-100 p-0.5 rounded hover:bg-red-500/20 text-slate-500 hover:text-red-300 cursor-pointer"
              >
                <Trash2 className="w-3 h-3" />
              </button>
            </div>
          ))}
          {loadingMore && (
            <div className="flex items-center justify-center gap-1.5 py-2 text-[9px] text-slate-500">
              <Loader2 className="w-3 h-3 animate-spin" /> 加载中…
            </div>
          )}
          {!hasMore && rooms.length > 0 && (
            <p className="text-center text-[8px] text-slate-600 py-1">— 已到底 —</p>
          )}
        </div>
      </div>

      {/* 主区域 */}
      <div className="flex-1 min-h-0 flex flex-col">
        {!activeRoom ? (
          <div className="flex-1 flex flex-col items-center justify-center text-slate-500 gap-2">
            <MessagesSquare className="w-10 h-10 opacity-30" />
            <p className="text-xs">选择一个会话，或新建一个开始多工具协作</p>
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
                <MessageView
                  key={m.id}
                  m={m}
                  tools={tools}
                  customNames={customNames}
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
              ))}
              {messages.length === 0 && (
                <p className="text-[11px] text-slate-500 text-center mt-8">
                  在下方输入任务，@ 一个工具并发送，它就会接手开始工作。
                </p>
              )}
            </div>

            {/* 输入区 */}
            <div className="flex-shrink-0 border-t border-white/5 p-3 space-y-2 bg-slate-900/40">
              {/* @ 工具选择（单选） */}
              <div className="flex flex-wrap gap-1 items-center">
                <span className="text-[9px] text-slate-500 mr-1">@工具</span>
                {tools.filter((t) => t.installed).map((t) => {
                  const on = selectedTool === t.id;
                  const avatar = getToolAvatar(tools, t.id, customNames);
                  const nickname = getToolNickname(tools, t.id, customNames) || t.display_name;
                  return (
                    <div key={t.id} className="relative flex items-center">
                      <button
                        onClick={() => setSelectedTool(on ? "" : t.id)}
                        className={`px-2 py-0.5 rounded-full text-[9px] font-semibold transition-all cursor-pointer ${
                          on
                            ? "bg-violet-600 text-white"
                            : "text-slate-400 hover:text-slate-200 hover:bg-white/5 border border-white/10"
                        }`}
                      >
                        {avatar ? `${avatar} ` : ""}{nickname}
                      </button>
                      {/* 编辑昵称/头像按钮 */}
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          startEditTool(t.id);
                        }}
                        className="ml-0.5 p-0.5 rounded text-slate-500 hover:text-violet-300 hover:bg-white/10 cursor-pointer opacity-60 hover:opacity-100"
                        title="设置昵称/头像"
                      >
                        <Pencil className="w-2.5 h-2.5" />
                      </button>
                    </div>
                  );
                })}
              </div>

              {/* 编辑昵称/头像弹框 */}
              {editingToolId && (
                <div className="rounded-lg border border-violet-500/30 bg-slate-900/80 p-2.5 space-y-1.5">
                  <div className="flex items-center gap-2">
                    <span className="text-[9px] font-bold text-violet-300">设置</span>
                    <span className="text-[9px] text-slate-500">{tools.find((x) => x.id === editingToolId)?.display_name}</span>
                  </div>
                  <div className="flex gap-2 items-center">
                    <div className="flex items-center gap-1.5">
                      <span className="text-[9px] text-slate-500 w-8">头像</span>
                      <input
                        value={editAvatar}
                        onChange={(e) => setEditAvatar(e.target.value)}
                        placeholder="emoji"
                        maxLength={4}
                        className="w-12 bg-slate-800 border border-white/10 rounded px-1.5 py-0.5 text-[10px] text-center text-slate-200 focus:outline-none focus:border-violet-500"
                      />
                    </div>
                    <div className="flex items-center gap-1.5 flex-1">
                      <span className="text-[9px] text-slate-500 w-8">昵称</span>
                      <input
                        value={editNickname}
                        onChange={(e) => setEditNickname(e.target.value)}
                        placeholder="昵称"
                        className="flex-1 bg-slate-800 border border-white/10 rounded px-1.5 py-0.5 text-[10px] text-slate-200 focus:outline-none focus:border-violet-500"
                      />
                    </div>
                    <button
                      onClick={saveEditTool}
                      className="p-1 rounded bg-violet-600 hover:bg-violet-500 text-white cursor-pointer"
                      title="保存"
                    >
                      <Check className="w-3 h-3" />
                    </button>
                    <button
                      onClick={cancelEditTool}
                      className="p-1 rounded bg-white/5 hover:bg-white/10 text-slate-400 cursor-pointer"
                      title="取消"
                    >
                      <X className="w-3 h-3" />
                    </button>
                  </div>
                </div>
              )}

              {/* 模型/供应商（仅支持模型设置的工具显示） */}
              {(() => {
                if (!showModelSettings) return null;
                return (
                  <div className="flex flex-wrap gap-2 items-center">
                    <select
                      value={providerId}
                      onChange={(e) => onProviderChange(e.target.value)}
                      className="bg-slate-800 border border-white/10 rounded px-2 py-1 text-[10px] text-slate-200 focus:outline-none focus:border-violet-500"
                    >
                      {providers.map((p) => (
                        <option key={p.id} value={p.id}>
                          {p.name}
                        </option>
                      ))}
                    </select>
                    <select
                      value={modelId}
                      onChange={(e) => setModelId(e.target.value)}
                      className="bg-slate-800 border border-white/10 rounded px-2 py-1 text-[10px] text-slate-200 focus:outline-none focus:border-violet-500"
                    >
                      {activeProvider?.models.map((m) => (
                        <option key={m.id} value={m.id}>
                          {m.name}
                        </option>
                      ))}
                    </select>
                  </div>
                );
              })()}

              {/* 高级协议设置（与工具启动页对齐） */}
              {showModelSettings && (() => {
                const tool = selectedToolObj!;
                return (
                  <div className="rounded-lg border border-white/5 bg-slate-900/30 overflow-hidden">
                    <button
                      onClick={() => setShowAdvanced(!showAdvanced)}
                      className="w-full flex items-center justify-between px-2.5 py-1.5 text-[9px] text-slate-400 hover:text-slate-200 cursor-pointer transition-all"
                    >
                      <span className="flex items-center gap-1.5 font-semibold">
                        <Settings className="w-3 h-3" /> 高级设置
                      </span>
                      <ChevronDown className={`w-3 h-3 transition-transform ${showAdvanced ? "rotate-180" : ""}`} />
                    </button>
                    {showAdvanced && (
                      <div className="px-2.5 pb-2.5 space-y-1.5">
                        {/* 模型伪装 */}
                        {tool.builtin_models.length > 0 && (
                          <div className="flex items-center gap-2">
                            <span className="text-[9px] text-slate-500 w-16 flex-shrink-0">伪装模型</span>
                            <select
                              value={masqueradeModel}
                              onChange={(e) => setMasqueradeModel(e.target.value)}
                              className="flex-1 bg-slate-800 border border-white/10 rounded px-1.5 py-0.5 text-[9px] text-slate-200 focus:outline-none focus:border-violet-500"
                            >
                              <option value="">不伪装（使用实际模型名）</option>
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
                              className="w-3 h-3 rounded accent-violet-500"
                            />
                            <span className="text-[9px] text-slate-400">1M 上下文窗口</span>
                          </label>
                        )}
                        {/* 优化器 */}
                        {tool.supports_optimizer && (
                          <label className="flex items-center gap-2 cursor-pointer">
                            <input
                              type="checkbox"
                              checked={optimizerEnabled}
                              onChange={(e) => setOptimizerEnabled(e.target.checked)}
                              className="w-3 h-3 rounded accent-violet-500"
                            />
                            <span className="text-[9px] text-slate-400">优化器（缓存注入 / 思考优化 / DeepSeek 规范化）</span>
                          </label>
                        )}
                        {/* 整流器 */}
                        {tool.supports_rectifier && (
                          <label className="flex items-center gap-2 cursor-pointer">
                            <input
                              type="checkbox"
                              checked={rectifierEnabled}
                              onChange={(e) => setRectifierEnabled(e.target.checked)}
                              className="w-3 h-3 rounded accent-violet-500"
                            />
                            <span className="text-[9px] text-slate-400">整流器（抹平协议差异）</span>
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
                  placeholder="输入任务内容…（回车发送；Ctrl/⌘+回车换行；输入 @ 附带文件；可选中上方消息点「引用」带入上下文）"
                  className="flex-1 bg-slate-800 border border-white/10 rounded-lg px-3 py-2 text-[11px] text-slate-200 focus:outline-none focus:border-violet-500 resize-none"
                />
                {busy ? (
                  <button
                    onClick={stopDispatch}
                    className="px-4 py-2 rounded-lg bg-red-600 hover:bg-red-500 text-white text-xs font-semibold flex items-center gap-1.5 cursor-pointer"
                  >
                    <Square className="w-3.5 h-3.5" /> 停止
                  </button>
                ) : (
                  <button
                    onClick={send}
                    className="px-4 py-2 rounded-lg bg-violet-600 hover:bg-violet-500 text-white text-xs font-semibold flex items-center gap-1.5 cursor-pointer"
                  >
                    <Send className="w-3.5 h-3.5" /> 发送
                  </button>
                )}
                <button
                  onClick={compact}
                  disabled={!selectedTool || busy || compacting}
                  title="压缩上下文（生成摘要并开启新会话）"
                  className="px-2 py-2 rounded-lg bg-white/5 hover:bg-white/10 disabled:opacity-40 text-slate-300 text-[10px] cursor-pointer"
                >
                  {compacting ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Archive className="w-3.5 h-3.5" />}
                </button>
                <button
                  onClick={resetSession}
                  disabled={!selectedTool}
                  title="重置当前工具续聊上下文"
                  className="px-2 py-2 rounded-lg bg-white/5 hover:bg-white/10 disabled:opacity-40 text-slate-300 text-[10px] cursor-pointer"
                >
                  <RotateCcw className="w-3.5 h-3.5" />
                </button>
              </div>
              <p className="text-[8px] text-slate-500">
                同一会话内重复 @ 同一工具会绑定其原生会话 id 自动「续聊」，上下文连续；点 ⟳ 可重置。
              </p>
              {hasSnapshot && (
                <p className="text-[8px] text-cyan-400 flex items-center gap-1">
                  <Archive className="w-2.5 h-2.5" />
                  已有上下文快照，下次发送将自动注入新会话
                </p>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

// ─── 单条消息渲染 ───
function PromptResponse({
  prompt,
  onRespond,
}: {
  prompt: { question: string; options: string[] };
  onRespond: (response: string) => void;
}) {
  const [customResponse, setCustomResponse] = useState("");
  return (
    <div className="mt-2 rounded-lg border border-amber-500/30 bg-amber-500/5 p-2.5 space-y-2">
      <div className="flex items-start gap-1.5">
        <span className="text-amber-400 text-[10px] font-bold mt-0.5">⚠ 询问</span>
        <span className="text-[10px] text-slate-300 whitespace-pre-wrap break-words flex-1">
          {prompt.question}
        </span>
      </div>
      <div className="flex flex-wrap gap-1.5 items-center">
        {prompt.options.map((opt) => (
          <button
            key={opt}
            onClick={() => onRespond(opt)}
            className="px-2.5 py-1 rounded-md bg-amber-600/80 hover:bg-amber-500 text-white text-[10px] font-semibold transition-colors cursor-pointer"
          >
            {opt}
          </button>
        ))}
        <input
          value={customResponse}
          onChange={(e) => setCustomResponse(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && customResponse.trim()) {
              onRespond(customResponse.trim());
              setCustomResponse("");
            }
          }}
          placeholder="自定义回复…"
          className="flex-1 min-w-[80px] bg-slate-800 border border-white/10 rounded-md px-2 py-1 text-[10px] text-slate-200 focus:outline-none focus:border-amber-500"
        />
        {customResponse.trim() && (
          <button
            onClick={() => {
              onRespond(customResponse.trim());
              setCustomResponse("");
            }}
            className="px-2 py-1 rounded-md bg-slate-700 hover:bg-slate-600 text-slate-200 text-[10px] cursor-pointer"
          >
            发送
          </button>
        )}
      </div>
      <p className="text-[8px] text-slate-500">120 秒无响应将自动选 y</p>
    </div>
  );
}

function MessageView({
  m,
  tools,
  customNames,
  onQuote,
  activity,
  prompt,
  proxy,
  onRespond,
}: {
  m: CollabMessage;
  tools: DetectedAiTool[];
  customNames?: Record<string, CustomToolName>;
  onQuote: (m: CollabMessage) => void;
  activity?: string;
  prompt?: { question: string; options: string[] };
  proxy?: { text: string; status: string; elapsed?: number };
  onRespond: (response: string) => void;
}) {
  const isUser = m.sender === "user";
  const color = senderColor(m.sender);
  const running = m.status === "running";
  const isError = m.status === "error";
  const isTool = !isUser;

  // 轮换思维短语（仅 running 状态时轮换）
  const thinkingPhrase = useRotatingPhrase(THINKING_PHRASES, running && !m.content, 4000);
  // 错误短语（仅 error 状态时使用，不轮换）
  const errorPhrase = useMemo(() => pickRandom(ERROR_PHRASES), [m.id, isError]);

  // 头像：优先用工具配置的 emoji avatar，回退到图标
  const avatar = isTool ? getToolAvatar(tools, m.sender, customNames) : null;
  // 昵称：优先用工具配置的 nickname，回退到 sender_name
  const displayName = isTool
    ? (getToolNickname(tools, m.sender, customNames) ?? m.sender_name)
    : m.sender_name;

  return (
    <div className="flex gap-2.5">
      {/* 头像 */}
      <div
        className={`w-7 h-7 rounded-full flex items-center justify-center text-white text-[11px] font-bold flex-shrink-0 ${color}`}
      >
        {isUser ? (
          <UserIcon className="w-3.5 h-3.5" />
        ) : avatar ? (
          <span className="text-[14px] leading-none">{avatar}</span>
        ) : (
          <Bot className="w-3.5 h-3.5" />
        )}
      </div>

      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2 mb-0.5">
          <span className="text-[11px] font-semibold text-slate-200">
            {displayName}
          </span>
          <span className="text-[8px] text-slate-500">{m.created_at}</span>
          {m.dispatch && (
            <span className="px-1.5 py-0.5 rounded bg-amber-500/15 text-amber-300 text-[8px] font-medium flex items-center gap-1">
              <Zap className="w-2.5 h-2.5" />
              派发{m.dispatch.model ? ` · ${m.dispatch.model}` : ""}
            </span>
          )}
        </div>

        {/* 引用卡 */}
        {m.references.length > 0 && (
          <div className="mb-1 space-y-1">
            {m.references.map((r, i) => (
              <ReferenceCard
                key={i}
                senderName={r.source_sender_name}
                excerpt={r.excerpt}
                variant="message"
              />
            ))}
          </div>
        )}

        {/* 文件附件 */}
        {m.files.length > 0 && (
          <div className="mb-1 flex flex-wrap gap-1">
            {m.files.map((f, i) => (
              <FileBadge key={i} path={f.path} color="cyan" />
            ))}
          </div>
        )}

        {/* 内容 */}
        <div
          className={`rounded-lg px-3 py-2 text-[11px] leading-relaxed break-words ${
            isUser
              ? "bg-blue-500/10 border border-blue-500/20 whitespace-pre-wrap"
              : running
              ? "bg-slate-800/40 border border-white/5 text-slate-400"
              : isError
              ? "bg-red-500/5 border border-red-500/20"
              : "bg-slate-800/60 border border-white/5"
          }`}
        >
          {running && !m.content ? (
            <span className="flex items-center gap-1.5 text-slate-400">
              <Loader2 className="w-3 h-3 animate-spin" /> {thinkingPhrase}
            </span>
          ) : isUser ? (
            m.content || <span className="text-slate-600">（空）</span>
          ) : isError ? (
            <div className="flex items-center gap-1.5 text-red-400">
              <span className="text-[10px]">⚠ {errorPhrase}：</span>
              <span className="flex-1">
                <MarkdownRenderer content={m.content} />
              </span>
            </div>
          ) : (
            <MarkdownRenderer content={m.content} />
          )}
        </div>

        {/* 活动状态指示器：思考中/调用工具等实时状态 */}
        {running && activity && (
          <div className="mt-1.5 flex items-center gap-1.5 text-[9px] text-slate-400">
            <span className="relative flex h-2 w-2">
              <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-violet-400 opacity-60"></span>
              <span className="relative inline-flex rounded-full h-2 w-2 bg-violet-500"></span>
            </span>
            <span className="animate-pulse">{activity}</span>
          </div>
        )}

        {/* 代理层实时状态：当 stdout 无输出但代理已收到响应时显示 */}
        {running && !m.content && proxy && proxy.status !== "complete" && (
          <div className="mt-1.5 rounded-md border border-cyan-500/20 bg-cyan-500/5 px-2 py-1 text-[9px] text-cyan-300/80">
            <span className="flex items-center gap-1">
              <span className="relative flex h-1.5 w-1.5">
                <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-cyan-400 opacity-60"></span>
                <span className="relative inline-flex rounded-full h-1.5 w-1.5 bg-cyan-500"></span>
              </span>
              代理: {proxy.status}
            </span>
          </div>
        )}
        {running && !m.content && proxy?.status === "complete" && proxy.text && (
          <div className="mt-1.5 rounded-md border border-emerald-500/20 bg-emerald-500/5 px-2 py-1.5 text-[9px] text-emerald-300/80">
            <div className="flex items-center gap-1 mb-1">
              <Zap className="w-2.5 h-2.5" />
              <span>代理已收到 LLM 响应 ({proxy.text.length}字{proxy.elapsed ? `, ${proxy.elapsed}ms` : ""})，等待工具处理…</span>
            </div>
            <div className="text-[8px] text-slate-400 max-h-24 overflow-y-auto whitespace-pre-wrap break-words">
              {proxy.text.slice(0, 200)}{proxy.text.length > 200 ? "…" : ""}
            </div>
          </div>
        )}

        {/* 有内容后仍在运行时显示当前活动（工具调用等） */}
        {running && m.content && activity && (
          <div className="mt-1 flex items-center gap-1 text-[8px] text-slate-500">
            <Loader2 className="w-2.5 h-2.5 animate-spin" />
            <span>{activity}</span>
          </div>
        )}

        {/* 交互式询问：工具需要用户做出选择 */}
        {prompt && <PromptResponse prompt={prompt} onRespond={onRespond} />}


        {/* 回链 + 引用按钮 */}
        <div className="flex items-center gap-3 mt-1">
          {m.reply_to && isTool && (
            <span className="flex items-center gap-1 text-[8px] text-slate-500">
              <CornerDownRight className="w-3 h-3" /> 回应你的派发
            </span>
          )}
          <button
            onClick={() => onQuote(m)}
            className="flex items-center gap-1 text-[8px] text-slate-500 hover:text-violet-300 cursor-pointer"
          >
            <Quote className="w-3 h-3" /> 引用
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── 共享组件：引用卡 / 文件徽章（输入区 + 消息渲染共用） ───

function ReferenceCard({
  senderName,
  excerpt,
  onRemove,
  variant = "input",
}: {
  senderName: string;
  excerpt: string;
  onRemove?: () => void;
  variant?: "input" | "message";
}) {
  const isInput = variant === "input";
  return (
    <div
      className={`flex items-start gap-2 p-1.5 rounded border-l-2 border-violet-500 text-[9px] ${
        isInput ? "bg-slate-800/60 text-slate-300" : "bg-slate-800/50 text-slate-400"
      }`}
    >
      <Quote className="w-3 h-3 flex-shrink-0 mt-0.5 text-violet-400" />
      <div className="flex-1 min-w-0">
        <span className="font-semibold text-violet-300">
          来自 {senderName}：
        </span>
        <span className={`whitespace-pre-wrap ${isInput ? "line-clamp-2" : "line-clamp-3"}`}>
          {excerpt}
        </span>
      </div>
      {onRemove && (
        <button
          onClick={onRemove}
          className="text-slate-500 hover:text-red-300 cursor-pointer"
        >
          <Trash2 className="w-3 h-3" />
        </button>
      )}
    </div>
  );
}

function FileBadge({
  path,
  onRemove,
  color = "slate",
}: {
  path: string;
  onRemove?: () => void;
  color?: "slate" | "cyan";
}) {
  const colorClass = color === "cyan" ? "text-cyan-300" : "text-slate-300";
  return (
    <div
      className={`flex items-center gap-1 px-1.5 py-0.5 rounded bg-slate-800/60 border border-white/10 text-[9px] ${colorClass} max-w-[200px]`}
      title={path}
    >
      <span className="truncate">@ {path.split(/[\\/]/).pop()}</span>
      {onRemove && (
        <button
          onClick={onRemove}
          className="text-slate-500 hover:text-red-300 cursor-pointer flex-shrink-0"
        >
          <Trash2 className="w-2.5 h-2.5" />
        </button>
      )}
    </div>
  );
}
