import { useState, useEffect, useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import {
  DndContext,
  closestCenter,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import {
  GripVertical,
  X,
  Search,
  Inbox,
  ChevronDown,
  ChevronRight,
  Route as RouteIcon,
  Loader2,
  RefreshCw,
  Play,
  Square,
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { useRef } from "react";
import AggregateLogPanel from "./AggregateLogPanel";
import type {
  RouteCandidate,
  RouteCandidateView,
  HeadroomConfig,
  HeadroomHealth,
  AggregateConfig,
  AggregateStatus,
  AggregateLog,
} from "./types";

const ckey = (c: RouteCandidate) => `${c.provider_id}::${c.model_id}`;
/// 聚合日志最多保留行数（超出丢弃最旧的）
const MAX_LOG_LINES = 500;

/// 三态勾选框：全选 / 半选（部分勾选）/ 未选。
/// React 没有 indeterminate 属性，只能通过 DOM 直接设置。
function TriCheckbox({
  checked,
  partial,
  onChange,
}: {
  checked: boolean;
  partial: boolean;
  onChange: () => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = !checked && partial;
  }, [checked, partial]);
  return (
    <input
      ref={ref}
      type="checkbox"
      checked={checked}
      onChange={onChange}
      className="w-3 h-3 accent-[var(--module-accent)] cursor-pointer flex-shrink-0"
    />
  );
}

/// 链路中的一行（可拖拽排序）
function ChainRow({
  id,
  order,
  name,
  onRemove,
}: {
  id: string;
  order: number;
  name: string;
  onRemove: () => void;
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id });
  return (
    <div
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={`flex items-center gap-2 px-2 py-1.5 rounded-lg border bg-slate-900/40 ${
        isDragging ? "border-[var(--module-accent)] bg-slate-900/80" : "border-white/5"
      }`}
    >
      <span className="w-4 text-center text-[9px] font-mono text-slate-500 flex-shrink-0">{order}</span>
      <button
        {...attributes}
        {...listeners}
        className="text-slate-600 hover:text-slate-300 cursor-grab active:cursor-grabbing flex-shrink-0"
        title={String(order)}
      >
        <GripVertical className="w-3.5 h-3.5" />
      </button>
      <span className="text-[11px] text-slate-200 truncate flex-grow min-w-0">{name}</span>
      <button
        onClick={onRemove}
        className="text-slate-600 hover:text-red-400 cursor-pointer transition-colors flex-shrink-0"
        title="remove"
      >
        <X className="w-3.5 h-3.5" />
      </button>
    </div>
  );
}

export default function RouteAggregate() {
  const { t } = useTranslation();
  const [candidates, setCandidates] = useState<RouteCandidateView[]>([]);
  const [chain, setChain] = useState<RouteCandidate[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [search, setSearch] = useState("");
  /// 左栏按供应商折叠：默认全部折叠（集合里只存已展开的 providerId）
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  // Headroom 上下文压缩（链路管线的一环：请求前压缩）
  const [headroom, setHeadroom] = useState<HeadroomConfig | null>(null);
  const [headroomHealth, setHeadroomHealth] = useState<HeadroomHealth | null>(null);
  const [checkingHeadroom, setCheckingHeadroom] = useState(false);
  // 聚合服务：配置、运行状态与日志
  const [aggregate, setAggregate] = useState<AggregateConfig | null>(null);
  const [aggStatus, setAggStatus] = useState<AggregateStatus | null>(null);
  const [aggBusy, setAggBusy] = useState(false);
  const [logs, setLogs] = useState<AggregateLog[]>([]);
  const [autoStart, setAutoStart] = useState(false);

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates })
  );

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [views, savedChain, headroomCfg, aggCfg, status, autoStartList] = await Promise.all([
        invoke<RouteCandidateView[]>("list_route_candidates"),
        invoke<RouteCandidate[]>("get_route_chain"),
        invoke<HeadroomConfig>("get_headroom_config"),
        invoke<AggregateConfig>("get_aggregate_config"),
        invoke<AggregateStatus>("get_aggregate_status"),
        invoke<string[]>("get_auto_start_services"),
      ]);
      setCandidates(views);
      setChain(savedChain);
      setHeadroom(headroomCfg);
      setAggregate(aggCfg);
      setAggStatus(status);
      setAutoStart(autoStartList.includes("aggregate"));
    } catch (e) {
      console.error("加载路由链失败", e);
    } finally {
      setLoading(false);
    }
  }, []);

  const checkHeadroom = useCallback(async (port: number) => {
    setCheckingHeadroom(true);
    try {
      const health = await invoke<HeadroomHealth>("check_headroom_health", { port });
      setHeadroomHealth(health);
    } catch (e: any) {
      setHeadroomHealth({
        alive: false,
        base_url: `http://127.0.0.1:${port}`,
        path: null,
        status: null,
        detail: String(e),
      });
    } finally {
      setCheckingHeadroom(false);
    }
  }, []);

  // 启用时自动探活一次；端口变化后重新探测
  useEffect(() => {
    if (headroom?.enabled) void checkHeadroom(headroom.port);
    else setHeadroomHealth(null);
  }, [headroom?.enabled, headroom?.port, checkHeadroom]);

  // 订阅聚合服务日志（后端 aggregate-log 事件）
  useEffect(() => {
    const unlisten = listen<AggregateLog>("aggregate-log", (event) => {
      setLogs(prev => [...prev, event.payload].slice(-MAX_LOG_LINES));
    });
    return () => { void unlisten.then(fn => fn()); };
  }, []);

  const patchAggregate = (patch: Partial<AggregateConfig>) => {
    if (!aggregate) return;
    const next = { ...aggregate, ...patch };
    setAggregate(next);
    void invoke<AggregateConfig>("save_aggregate_config", { config: next })
      .then(saved => setAggregate(saved))
      .catch((e: any) => {
        setAggregate(aggregate);
        setLogs(prev => [...prev, { phase: "config", line: String(e), level: "error" }].slice(-MAX_LOG_LINES));
      });
  };

  const toggleAutoStart = async () => {
    const next = !autoStart;
    setAutoStart(next);
    try {
      await invoke("set_auto_start_service", { serviceId: "aggregate", enabled: next });
    } catch (e) {
      setAutoStart(!next);
      setLogs(prev => [...prev, { phase: "config", line: String(e), level: "error" }].slice(-MAX_LOG_LINES));
    }
  };

  const startAggregate = async () => {
    setAggBusy(true);
    try {
      const status = await invoke<AggregateStatus>("start_aggregate_service");
      setAggStatus(status);
    } catch (e: any) {
      setLogs(prev => [...prev, { phase: "start", line: String(e), level: "error" }].slice(-MAX_LOG_LINES));
    } finally {
      setAggBusy(false);
    }
  };

  const stopAggregate = async () => {
    setAggBusy(true);
    try {
      const status = await invoke<AggregateStatus>("stop_aggregate_service");
      setAggStatus(status);
    } catch (e: any) {
      setLogs(prev => [...prev, { phase: "stop", line: String(e), level: "error" }].slice(-MAX_LOG_LINES));
    } finally {
      setAggBusy(false);
    }
  };

  const patchHeadroom = (patch: Partial<HeadroomConfig>) => {
    if (!headroom) return;
    const next = { ...headroom, ...patch };
    setHeadroom(next);
    void invoke("save_headroom_config", { config: next }).catch((e) => console.error("保存压缩配置失败", e));
  };

  useEffect(() => { void load(); }, [load]);

  // 落库：后端会清洗（丢弃已删除的供应商/模型、去重）后返回实际保存内容
  const persist = useCallback(async (next: RouteCandidate[]) => {
    setSaving(true);
    try {
      const saved = await invoke<RouteCandidate[]>("save_route_chain", { chain: next });
      setChain(saved);
      const views = await invoke<RouteCandidateView[]>("list_route_candidates");
      setCandidates(views);
    } catch (e) {
      console.error("保存路由链失败", e);
    } finally {
      setSaving(false);
    }
  }, []);

  const nameOf = useCallback(
    (c: RouteCandidate) => {
      const hit = candidates.find(v => v.provider_id === c.provider_id && v.model_id === c.model_id);
      if (hit) return `${hit.provider_name} · ${hit.model_name}`;
      return `${c.provider_id} · ${c.model_id}`;
    },
    [candidates]
  );

  const isInChain = (v: RouteCandidateView) =>
    chain.some(c => c.provider_id === v.provider_id && c.model_id === v.model_id);

  const toggle = (v: RouteCandidateView) => {
    const cand: RouteCandidate = { provider_id: v.provider_id, model_id: v.model_id };
    const exists = chain.some(c => ckey(c) === ckey(cand));
    void persist(exists ? chain.filter(c => ckey(c) !== ckey(cand)) : [...chain, cand]);
  };

  const toggleGroup = (providerId: string) => {
    setExpanded(prev => {
      const next = new Set(prev);
      if (next.has(providerId)) next.delete(providerId);
      else next.add(providerId);
      return next;
    });
  };

  /// 供应商级三态勾选：全选（把未入链的模型按仓库顺序追加）/ 清空（移除该供应商全部候选）
  const toggleProvider = (items: RouteCandidateView[], allSelected: boolean) => {
    if (allSelected) {
      const ids = new Set(items.map(v => ckey({ provider_id: v.provider_id, model_id: v.model_id })));
      void persist(chain.filter(c => !ids.has(ckey(c))));
    } else {
      const existing = new Set(chain.map(ckey));
      const additions = items
        .filter(v => !existing.has(ckey({ provider_id: v.provider_id, model_id: v.model_id })))
        .map(v => ({ provider_id: v.provider_id, model_id: v.model_id }));
      void persist([...chain, ...additions]);
    }
  };

  const removeAt = (idx: number) => {
    void persist(chain.filter((_, i) => i !== idx));
  };

  const onDragEnd = (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    const from = chain.findIndex(c => ckey(c) === String(active.id));
    const to = chain.findIndex(c => ckey(c) === String(over.id));
    if (from < 0 || to < 0) return;
    void persist(arrayMove(chain, from, to));
  };

  // 搜索时强制展开命中的分组（否则折叠状态下看不见结果）
  const searching = search.trim().length > 0;

  const filtered = useMemo(() => {
    const kw = search.trim().toLowerCase();
    if (!kw) return candidates;
    return candidates.filter(v =>
      v.provider_name.toLowerCase().includes(kw) ||
      v.model_name.toLowerCase().includes(kw) ||
      v.model_id.toLowerCase().includes(kw)
    );
  }, [candidates, search]);

  // 按供应商分组（保持仓库顺序）
  const grouped = useMemo(() => {
    const map = new Map<string, RouteCandidateView[]>();
    for (const v of filtered) {
      const list = map.get(v.provider_id) ?? [];
      list.push(v);
      map.set(v.provider_id, list);
    }
    return Array.from(map.entries()).map(([pid, items]) => ({
      providerId: pid,
      providerName: items[0]?.provider_name ?? pid,
      category: items[0]?.provider_category ?? "provider",
      items,
    }));
  }, [filtered]);

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center text-slate-500">
        <Loader2 className="w-5 h-5 animate-spin" />
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col min-h-0 space-y-3">
      {/* 说明 */}
      <div className="rounded-xl border border-white/5 bg-white/[0.02] px-4 py-2.5 flex items-start gap-2">
        <RouteIcon className="w-4 h-4 text-[var(--module-accent)] flex-shrink-0 mt-0.5" />
        <div className="min-w-0">
          <div className="text-[11px] font-bold text-slate-200">
            {t("aggregate.title")}
            {saving && <span className="ml-2 text-[9px] font-normal text-slate-500">{t("aggregate.saving")}</span>}
          </div>
          <p className="text-[10px] text-slate-500 mt-0.5 leading-relaxed">{t("aggregate.hint")}</p>
        </div>
      </div>

      <div className="flex-1 min-h-0 grid grid-cols-2 gap-3">
        {/* 左：仓库候选（勾选入链） */}
        <div className="flex flex-col min-h-0 rounded-xl border border-white/5 bg-white/[0.02] overflow-hidden">
          <div className="px-3 py-2 border-b border-white/5 flex items-center justify-between gap-2">
            <span className="text-[11px] font-bold text-slate-300">{t("aggregate.repoTitle")}</span>
            <span className="text-[9px] text-slate-600">{candidates.length}</span>
          </div>
          <div className="p-2 border-b border-white/5">
            <div className="relative">
              <Search className="w-3.5 h-3.5 text-slate-500 absolute left-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
              <input
                value={search}
                onChange={e => setSearch(e.target.value)}
                placeholder={t("aggregate.searchPh")}
                className="w-full bg-slate-900 border border-white/10 rounded-lg pl-8 pr-2.5 py-1.5 text-xs text-slate-200 focus:outline-none focus:border-[var(--module-accent)]"
              />
            </div>
          </div>
          <div className="flex-1 min-h-0 overflow-y-auto p-2 space-y-1">
            {grouped.length === 0 ? (
              <div className="h-full flex flex-col items-center justify-center text-slate-600 gap-1.5 py-8">
                <Inbox className="w-6 h-6" />
                <span className="text-[10px] text-center px-4">
                  {candidates.length === 0 ? t("aggregate.emptyRepo") : t("aggregate.noMatch")}
                </span>
              </div>
            ) : grouped.map(group => {
              // 自引用（指向聚合服务自身）：入链会造成请求递归，整组禁用
              const selfRef = group.items.some(v => v.self_referential);
              const selectable = group.items.filter(v => !v.self_referential);
              const selectedCount = selectable.filter(v => isInChain(v)).length;
              const total = selectable.length;
              const allSelected = total > 0 && selectedCount === total;
              // 搜索时自动展开命中的分组，否则按用户手动展开状态（默认折叠）
              const isOpen = searching || expanded.has(group.providerId);
              return (
                <div key={group.providerId} className="rounded-lg border border-white/5 overflow-hidden">
                  {/* 分组头：点击展开/折叠；三态勾选 = 全选/清空该供应商 */}
                  <div className="flex items-center gap-1.5 px-1.5 py-1.5 bg-white/[0.02] hover:bg-white/[0.05] transition-colors">
                    <button
                      onClick={() => toggleGroup(group.providerId)}
                      className="text-slate-500 hover:text-slate-200 cursor-pointer flex-shrink-0"
                      title={isOpen ? t("aggregate.collapse") : t("aggregate.expand")}
                    >
                      {isOpen ? <ChevronDown className="w-3.5 h-3.5" /> : <ChevronRight className="w-3.5 h-3.5" />}
                    </button>
                    {selfRef ? (
                      <span
                        className="w-3 h-3 flex-shrink-0 rounded-sm border border-red-500/50 text-[8px] leading-none flex items-center justify-center text-red-400"
                        title={t("aggregate.selfReferentialHint")}
                      >
                        !
                      </span>
                    ) : (
                      <TriCheckbox
                        checked={allSelected}
                        partial={selectedCount > 0 && !allSelected}
                        onChange={() => toggleProvider(group.items, allSelected)}
                      />
                    )}
                    <button
                      onClick={() => toggleGroup(group.providerId)}
                      className="flex-grow min-w-0 text-left flex items-center gap-1.5 cursor-pointer"
                    >
                      <span className="text-[10px] font-bold text-slate-300 truncate">{group.providerName}</span>
                      <span className="text-[8px] text-slate-600 flex-shrink-0">{group.category}</span>
                    </button>
                    <span className={`text-[9px] font-mono flex-shrink-0 ${selectedCount > 0 ? "text-[var(--module-accent)]" : "text-slate-600"}`}>
                      {selectedCount}/{total}
                    </span>
                  </div>
                  {/* 叶子节点：该供应商的模型 */}
                  {isOpen && (
                    <div className="p-1 space-y-0.5">
                      {group.items.map(v => {
                        const inChain = isInChain(v);
                        if (v.self_referential) {
                          return (
                            <div
                              key={`${v.provider_id}::${v.model_id}`}
                              title={t("aggregate.selfReferentialHint")}
                              className="w-full text-left flex items-center gap-2 pl-6 pr-2 py-1 rounded-md border border-transparent opacity-50 cursor-not-allowed"
                            >
                              <input type="checkbox" checked={false} readOnly className="w-3 h-3 pointer-events-none" />
                              <span className="text-[10px] text-slate-400 font-mono truncate flex-grow min-w-0">{v.model_name}</span>
                              <span className="text-[9px] text-red-400/80 flex-shrink-0">{t("aggregate.selfReferential")}</span>
                            </div>
                          );
                        }
                        return (
                          <button
                            key={`${v.provider_id}::${v.model_id}`}
                            onClick={() => toggle(v)}
                            className={`w-full text-left flex items-center gap-2 pl-6 pr-2 py-1 rounded-md border transition-colors cursor-pointer ${
                              inChain
                                ? "border-[var(--module-accent)]/40 bg-[var(--module-accent)]/10"
                                : "border-transparent hover:bg-white/5"
                            }`}
                          >
                            <input
                              type="checkbox"
                              checked={inChain}
                              onChange={() => {}}
                              className="w-3 h-3 accent-[var(--module-accent)] pointer-events-none"
                            />
                            <span className="text-[10px] text-slate-300 font-mono truncate flex-grow min-w-0">{v.model_name}</span>
                            {inChain && (
                              <span className="text-[9px] font-mono text-[var(--module-accent)] flex-shrink-0">#{v.order}</span>
                            )}
                          </button>
                        );
                      })}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>

        {/* 右：链路顺序（拖拽排序） */}
        <div className="flex flex-col min-h-0 rounded-xl border border-white/5 bg-white/[0.02] overflow-hidden">
          <div className="px-3 py-2 border-b border-white/5 flex items-center justify-between gap-2">
            <span className="text-[11px] font-bold text-slate-300">{t("aggregate.chainTitle")}</span>
            <span className="text-[9px] text-slate-600">{t("aggregate.count", { count: chain.length })}</span>
          </div>
          <div className="flex-1 min-h-0 overflow-y-auto p-2">
            {chain.length === 0 ? (
              <div className="h-full flex flex-col items-center justify-center text-slate-600 gap-1.5 py-8">
                <RouteIcon className="w-6 h-6" />
                <span className="text-[10px] text-center px-4">{t("aggregate.emptyChain")}</span>
              </div>
            ) : (
              <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
                <SortableContext items={chain.map(ckey)} strategy={verticalListSortingStrategy}>
                  <div className="space-y-1">
                    {chain.map((c, idx) => (
                      <ChainRow
                        key={ckey(c)}
                        id={ckey(c)}
                        order={idx + 1}
                        name={nameOf(c)}
                        onRemove={() => removeAt(idx)}
                      />
                    ))}
                  </div>
                </SortableContext>
              </DndContext>
            )}
          </div>
        </div>
      </div>

      {/* Headroom 上下文压缩（链路管线的一环：请求前压缩） */}
      {headroom && (
        <div className="rounded-xl border border-white/5 bg-white/[0.02] px-4 py-2.5 space-y-2">
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2 flex-wrap">
                <span className="text-[11px] font-bold text-slate-200">{t("aggregate.headroomTitle")}</span>
                <span className={`px-1.5 py-0.5 rounded text-[9px] font-bold ${!headroom.enabled
                  ? "bg-slate-500/15 text-slate-400"
                  : headroomHealth?.alive
                    ? "bg-emerald-500/15 text-emerald-400"
                    : "bg-amber-500/15 text-amber-400"}`}>
                  {!headroom.enabled
                    ? t("aggregate.headroomOff")
                    : headroomHealth?.alive
                      ? t("aggregate.headroomAlive")
                      : t("aggregate.headroomDown")}
                </span>
              </div>
              <p className="text-[10px] text-slate-500 mt-0.5 leading-relaxed">{t("aggregate.headroomHint")}</p>
            </div>
            <label className="flex items-center gap-2 cursor-pointer flex-shrink-0">
              <input type="checkbox" checked={headroom.enabled}
                onChange={e => patchHeadroom({ enabled: e.target.checked })}
                className="w-3.5 h-3.5 accent-[var(--module-accent)] cursor-pointer" />
            </label>
          </div>

          {headroom.enabled && (
            <div className="space-y-2 pt-2 border-t border-white/5">
              <div className="flex items-center gap-2 flex-wrap">
                <label className="text-[10px] text-slate-500 w-14 flex-shrink-0">{t("aggregate.headroomPort")}</label>
                <input type="number" min={1} max={65535} value={headroom.port}
                  onChange={e => setHeadroom({ ...headroom, port: Number(e.target.value) || 0 })}
                  onBlur={e => patchHeadroom({ port: Number(e.target.value) || 8791 })}
                  className="w-20 bg-slate-900 border border-white/10 rounded-lg px-2 py-1 text-[10px] text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]" />
                <button onClick={() => void checkHeadroom(headroom.port)} disabled={checkingHeadroom}
                  className="px-2 py-1 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-300 hover:text-white cursor-pointer transition-all flex items-center gap-1 disabled:opacity-40">
                  <RefreshCw className={`w-3 h-3 ${checkingHeadroom ? "animate-spin" : ""}`} />
                  {t("aggregate.headroomCheck")}
                </button>
                {headroomHealth && (
                  <span className={`text-[9px] truncate max-w-[320px] ${headroomHealth.alive ? "text-emerald-400/80" : "text-amber-400/90"}`}
                    title={headroomHealth.detail}>
                    {headroomHealth.alive
                      ? `${headroomHealth.base_url}${headroomHealth.path} · HTTP ${headroomHealth.status}`
                      : headroomHealth.detail}
                  </span>
                )}
              </div>

              <div className="flex items-center gap-2 flex-wrap">
                <label className="text-[10px] text-slate-500 w-14 flex-shrink-0">{t("aggregate.headroomOnUnavailable")}</label>
                <select value={headroom.on_unavailable}
                  onChange={e => patchHeadroom({ on_unavailable: e.target.value })}
                  className="bg-slate-900 border border-white/10 rounded-lg px-2 py-1 text-[10px] text-slate-200 focus:outline-none focus:border-[var(--module-accent)] cursor-pointer">
                  <option value="failOpen">{t("aggregate.headroomFailOpen")}</option>
                  <option value="failClosed">{t("aggregate.headroomFailClosed")}</option>
                </select>
                <span className="text-[9px] text-slate-600">
                  {headroom.on_unavailable === "failClosed" ? t("aggregate.headroomFailClosedHint") : t("aggregate.headroomFailOpenHint")}
                </span>
              </div>

              <label className="flex items-center gap-2 text-[10px] text-slate-400 cursor-pointer">
                <input type="checkbox" checked={headroom.disable_kompress}
                  onChange={e => patchHeadroom({ disable_kompress: e.target.checked })}
                  className="w-3.5 h-3.5 accent-[var(--module-accent)] cursor-pointer" />
                {t("aggregate.headroomDisableKompress")}
              </label>
            </div>
          )}
        </div>
      )}

      {/* 聚合服务：端口 / 上下文限制 / 启停 / 日志 */}
      {aggregate && (
        <div className="rounded-xl border border-white/5 bg-white/[0.02] px-4 py-2.5 space-y-2">
          <div className="flex items-center justify-between gap-3 flex-wrap">
            <div className="min-w-0">
              <div className="flex items-center gap-2 flex-wrap">
                <span className="text-[11px] font-bold text-slate-200">{t("aggregate.serviceTitle")}</span>
                <span className={`px-1.5 py-0.5 rounded text-[9px] font-bold ${aggStatus?.running
                  ? "bg-emerald-500/15 text-emerald-400"
                  : "bg-slate-500/15 text-slate-400"}`}>
                  {aggStatus?.running ? t("aggregate.serviceRunning") : t("aggregate.serviceStopped")}
                </span>
                {aggStatus?.running && (
                  <span className="text-[9px] font-mono text-slate-500">http://127.0.0.1:{aggStatus.port}</span>
                )}
              </div>
              <p className="text-[10px] text-slate-500 mt-0.5 leading-relaxed">{t("aggregate.serviceHint")}</p>
              <label className="flex items-center gap-1.5 text-[10px] text-slate-400 cursor-pointer mt-1">
                <input type="checkbox" checked={autoStart}
                  onChange={() => void toggleAutoStart()}
                  className="w-3 h-3 accent-[var(--module-accent)] cursor-pointer" />
                {t("aggregate.autoStart")}
              </label>
            </div>
            {aggStatus?.running ? (
              <button onClick={() => void stopAggregate()} disabled={aggBusy}
                className="px-2.5 py-1 rounded-lg bg-red-500/10 border border-red-500/20 text-[10px] font-semibold text-red-400 hover:bg-red-500/20 cursor-pointer transition-all flex items-center gap-1 disabled:opacity-40">
                <Square className="w-3 h-3" /> {t("aggregate.serviceStop")}
              </button>
            ) : (
              <button onClick={() => void startAggregate()} disabled={aggBusy || chain.length === 0}
                className="px-2.5 py-1 rounded-lg bg-emerald-500/10 border border-emerald-500/20 text-[10px] font-semibold text-emerald-400 hover:bg-emerald-500/20 cursor-pointer transition-all flex items-center gap-1 disabled:opacity-40 disabled:cursor-not-allowed"
                title={chain.length === 0 ? t("aggregate.needCandidates") : undefined}>
                <Play className="w-3 h-3" /> {t("aggregate.serviceStart")}
              </button>
            )}
          </div>

          <div className="flex items-center gap-3 flex-wrap pt-2 border-t border-white/5">
            <div className="flex items-center gap-2">
              <label className="text-[10px] text-slate-500 flex-shrink-0">{t("aggregate.servicePort")}</label>
              <input type="number" min={1} max={65535} value={aggregate.port} disabled={!!aggStatus?.running}
                onChange={e => setAggregate({ ...aggregate, port: Number(e.target.value) || 0 })}
                onBlur={e => patchAggregate({ port: Number(e.target.value) || 15888 })}
                className="w-20 bg-slate-900 border border-white/10 rounded-lg px-2 py-1 text-[10px] text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)] disabled:opacity-50" />
            </div>
            <div className="flex items-center gap-2">
              <label className="text-[10px] text-slate-500 flex-shrink-0">{t("aggregate.contextLimit")}</label>
              <input type="number" min={1000} step={1000} value={aggregate.context_limit}
                onChange={e => setAggregate({ ...aggregate, context_limit: Number(e.target.value) || 0 })}
                onBlur={e => patchAggregate({ context_limit: Number(e.target.value) || 128000 })}
                className="w-24 bg-slate-900 border border-white/10 rounded-lg px-2 py-1 text-[10px] text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]" />
              <span className="text-[9px] text-slate-600">tokens</span>
            </div>
            <div className="flex items-center gap-2">
              <label className="text-[10px] text-slate-500 flex-shrink-0">{t("aggregate.retryCount")}</label>
              <input type="number" min={1} max={5} value={aggregate.retry_count}
                onChange={e => setAggregate({ ...aggregate, retry_count: Number(e.target.value) || 1 })}
                onBlur={e => patchAggregate({ retry_count: Math.min(5, Math.max(1, Number(e.target.value) || 2)) })}
                className="w-14 bg-slate-900 border border-white/10 rounded-lg px-2 py-1 text-[10px] text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]" />
              <span className="text-[9px] text-slate-600">{t("aggregate.retryCountHint")}</span>
            </div>
          </div>

          <AggregateLogPanel logs={logs} onClear={() => setLogs([])} />
        </div>
      )}
    </div>
  );
}
