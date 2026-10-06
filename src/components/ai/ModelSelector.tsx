// ─── 通用 AI-模型选择器 ───
// 提供给所有「选供应商 + 模型」的模块/页面复用，统一展示：
//   1. 供应商与模型分组（供应商作分组头，模型作其下选项，一眼分清归属）
//   2. 供应商活动标签（promotions：活动名称 + 倒计时/已结束）
//   3. 供应商协议类型徽标（由 openai/anthropic/google 三个 URL 非空推断）
//   4. 单个模型的特殊参数（customParams 的 label 列表，只读标记）
//
// 另有三个可独立复用的子组件（ToolLauncher 等内联场景直接 import 用）：
//   ProviderProtocolBadges / ProviderPromotionTags / CustomParamControls

import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronRight } from "lucide-react";
import type { ModelCustomParam, ProviderPromotion } from "./types";
import { promotionCountdown, promotionState, prunePromotions } from "./promotions";
import { SharedModal } from "../shared/Modal";

/// 选择器只依赖供应商/模型的这些字段 —— 用结构子集，好让完整 `AiProvider`
/// 与各处（翻译 / 收藏 / API 等）声明的简化版 `AiProvider` 都能直接传入。
export interface ProviderLike {
  id: string;
  name: string;
  openai_url: string;
  anthropic_url?: string;
  google_url?: string;
  models: { id: string; name: string; customParams?: ModelCustomParam[] }[];
  promotions?: ProviderPromotion[] | null;
}

// ─── 供应商协议徽标 ───

/// 由已配置的协议 URL 非空推断，渲染成彩色小徽标（与模型配置页一致）。
export function ProviderProtocolBadges({ provider }: { provider: ProviderLike | null | undefined }) {
  if (!provider) return null;
  const items: { key: string; label: string; cls: string }[] = [];
  if (provider.openai_url) items.push({ key: "openai", label: "OpenAI", cls: "bg-blue-500/20 text-blue-300" });
  if (provider.anthropic_url) items.push({ key: "anthropic", label: "Anthropic", cls: "bg-amber-500/20 text-amber-300" });
  if (provider.google_url) items.push({ key: "google", label: "Google", cls: "bg-green-500/20 text-green-300" });
  if (items.length === 0) return null;
  return (
    <>
      {items.map((i) => (
        <span key={i.key} className={`text-[8px] px-1.5 py-0.5 rounded ${i.cls}`}>{i.label}</span>
      ))}
    </>
  );
}

// ─── 供应商活动标签 ───

/// 渲染供应商的活动（名称 + 倒计时 / 已结束），到期置灰、紧急红、临近琥珀。
export function ProviderPromotionTags({ promotions }: { promotions?: ProviderPromotion[] | null }) {
  const { t } = useTranslation();
  const now = Date.now();
  const live = prunePromotions(promotions, now);
  if (live.length === 0) return null;
  return (
    <>
      {live.map((p) => {
        const st = promotionState(p.ends_at, now);
        const cd = promotionCountdown(p.ends_at, now);
        const cls =
          st === "ended"
            ? "bg-slate-600/20 text-slate-500"
            : st === "urgent"
              ? "bg-red-500/20 text-red-300"
              : st === "soon"
                ? "bg-amber-500/20 text-amber-300"
                : "bg-emerald-500/20 text-emerald-300";
        return (
          <span key={p.id} className={`text-[8px] px-1.5 py-0.5 rounded whitespace-nowrap ${cls}`}>
            {p.name}
            {st === "ended" ? ` · ${t("modelcfg.promotionEnded")}` : cd ? ` · ${cd}` : ""}
          </span>
        );
      })}
    </>
  );
}

// ─── 模型特殊参数控件 ───

/// 按 `ModelCustomParam.paramType` 渲染 bool/text/enum 控件，并可选显示传递目标。
/// 受控：调用方持有 `values`（key → 值），`onChange(key, value)` 单键更新。
export function CustomParamControls({
  params,
  values,
  onChange,
  showTarget = true,
  compact = false,
}: {
  params: ModelCustomParam[];
  values: Record<string, string>;
  onChange: (key: string, value: string) => void;
  showTarget?: boolean;
  compact?: boolean;
}) {
  if (params.length === 0) return null;
  const inputCls = compact
    ? "flex-1 min-w-0 bg-slate-800 border border-white/10 rounded px-1.5 py-0.5 text-micro text-slate-200 focus:outline-none focus:border-[var(--module-accent)]"
    : "flex-1 min-w-0 ui-input rounded px-2 py-1 text-tiny text-slate-200 focus:outline-none focus:border-[var(--module-accent)]";
  return (
    <div className={compact ? "space-y-1.5" : "space-y-2"}>
      {params.map((cp) => (
        <div key={cp.key} className="flex items-center gap-2">
          <label
            className={`${compact ? "text-micro w-20" : "text-tiny w-28"} text-slate-400 flex-shrink-0 truncate`}
            title={cp.key}
          >
            {cp.label || cp.key}
          </label>
          {cp.paramType === "bool" ? (
            <input
              type="checkbox"
              checked={values[cp.key] !== "false"}
              onChange={(e) => onChange(cp.key, e.target.checked ? "true" : "false")}
              className={`${compact ? "w-3.5 h-3.5" : "w-4 h-4"} accent-[var(--module-accent)]`}
            />
          ) : cp.paramType === "text" ? (
            <input
              type="text"
              value={values[cp.key] || ""}
              onChange={(e) => onChange(cp.key, e.target.value)}
              placeholder={cp.defaultValue || ""}
              className={inputCls}
            />
          ) : (
            <select
              value={values[cp.key] || cp.defaultValue || ""}
              onChange={(e) => onChange(cp.key, e.target.value)}
              className={inputCls}
            >
              {(cp.options && cp.options.length > 0 ? cp.options : [cp.defaultValue || ""])
                .filter(Boolean)
                .map((o) => (
                  <option key={o} value={o}>{o}</option>
                ))}
            </select>
          )}
          {showTarget && (
            <span className={`${compact ? "w-14" : "w-16"} text-[8px] text-slate-600 font-mono flex-shrink-0 text-right`}>
              {cp.target === "config" ? (cp.configPath || "config") : (cp.envKey || "env")}
            </span>
          )}
        </div>
      ))}
    </div>
  );
}

// ─── 模型特殊参数只读标记（下拉里的模型行） ───

/// 模型有哪些特殊参数：把 customParams 的 label 拼成一行小字（最多 3 个 + 省略）。
function modelParamTags(params?: ModelCustomParam[]): string | null {
  if (!params || params.length === 0) return null;
  const labels = params.map((p) => p.label || p.key).filter(Boolean);
  if (labels.length === 0) return null;
  if (labels.length <= 3) return labels.join(" · ");
  return `${labels.slice(0, 3).join(" · ")} 等${labels.length}项`;
}

// ─── 主选择器 ───

export interface ModelSelectorProps {
  providers: ProviderLike[];
  providerId: string;
  modelId: string;
  onSelect: (providerId: string, modelId: string) => void;
  disabled?: boolean;
  /** 紧凑模式（翻译窗 / 安装 / 收藏等小空间） */
  compact?: boolean;
  /** 未选择时的占位文案（一般用调用方的 i18n key） */
  placeholder?: string;
  /** 无可用供应商时的空态文案 */
  emptyText?: string;
  /** 供应商过滤（可选，默认仅保留有模型的供应商） */
  filterProvider?: (p: ProviderLike) => boolean;
  /** 弹框标题（默认 i18n「选择模型」） */
  title?: string;
  /** 允许取消选择：点击当前已选模型时以 modelId="" 回调（工具启动页需要） */
  allowClear?: boolean;
}

export function ModelSelector({
  providers,
  providerId,
  modelId,
  onSelect,
  disabled = false,
  compact = false,
  placeholder,
  emptyText,
  filterProvider,
  title,
  allowClear = false,
}: ModelSelectorProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  // 弹框里展开的供应商 id 集合：默认只展开当前选中的供应商，其余折叠，点分组头切换
  const [openProviders, setOpenProviders] = useState<Set<string>>(new Set());

  const openModal = () => {
    if (disabled) return;
    // 每次打开时默认只展开当前选中的供应商
    setOpenProviders(new Set(providerId ? [providerId] : []));
    setOpen(true);
  };

  const toggleProvider = (id: string) => {
    setOpenProviders((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };

  const eligible = providers.filter((p) => (filterProvider ? filterProvider(p) : p.models.length > 0));
  const currentProvider = providers.find((p) => p.id === providerId);
  const currentModel = currentProvider?.models.find((m) => m.id === modelId);
  const label = currentProvider && currentModel
    ? `${currentProvider.name} › ${currentModel.name || currentModel.id}`
    : placeholder || "";

  return (
    <>
      <button
        type="button"
        onClick={openModal}
        disabled={disabled}
        className={`${compact ? "h-7 px-2 text-tiny" : "px-3 py-2 text-body"} w-full flex items-center justify-between gap-2 rounded-ctl bg-white/5 border border-white/10 text-slate-200 hover:border-white/20 focus:outline-none focus:border-[var(--module-accent)] cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed transition-all`}
      >
        <span className="truncate min-w-0 text-left">{label}</span>
        <ChevronDown className="w-3.5 h-3.5 flex-shrink-0 text-slate-500" />
      </button>

      <SharedModal
        open={open}
        onClose={() => setOpen(false)}
        title={title || t("modelcfg.selectModel")}
        width={460}
        bodyClass="space-y-0"
      >
        <div className="rounded-ctl border border-white/5 bg-slate-900/30 overflow-hidden">
          {eligible.length === 0 ? (
            <div className="px-3 py-4 text-tiny text-slate-500 text-center">{emptyText || placeholder || ""}</div>
          ) : (
            eligible.map((p) => {
              const expanded = openProviders.has(p.id);
              return (
                <div key={p.id}>
                  {/* 供应商分组头：可点击展开/收缩，含名称 + 协议徽标 + 活动标签 */}
                  <button
                    type="button"
                    onClick={() => toggleProvider(p.id)}
                    className="w-full flex items-center gap-1.5 px-3 py-1.5 bg-white/[0.02] border-b border-white/[0.04] hover:bg-white/[0.04] cursor-pointer transition-all"
                  >
                    <ChevronRight className={`w-3 h-3 text-slate-500 flex-shrink-0 transition-transform ${expanded ? "rotate-90" : ""}`} />
                    <span className="font-semibold text-tiny text-slate-400 truncate">{p.name}</span>
                    <ProviderProtocolBadges provider={p} />
                    <ProviderPromotionTags promotions={p.promotions} />
                    {providerId === p.id && modelId && (
                      <span className="ml-auto text-[9px] text-[var(--module-accent)] font-mono truncate">{modelId}</span>
                    )}
                  </button>
                  {expanded && p.models.map((m) => {
                    const isSel = providerId === p.id && modelId === m.id;
                    const tags = modelParamTags(m.customParams);
                    return (
                      <button
                        key={`${p.id}:${m.id}`}
                        type="button"
                        onClick={() => {
                          // allowClear 时点当前已选模型 = 取消选择（工具启动页需要「未选择」态）
                          onSelect(p.id, allowClear && isSel ? "" : m.id);
                          setOpen(false);
                        }}
                        className={`w-full text-left px-3 py-1.5 text-tiny transition-all cursor-pointer flex items-center gap-2 ${
                          isSel
                            ? "bg-[var(--module-accent-soft)] text-[var(--module-accent)] font-semibold"
                            : "text-slate-300 hover:bg-white/5 hover:text-slate-100"
                        }`}
                      >
                        <span className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${isSel ? "bg-[var(--module-accent)]" : "bg-slate-600"}`} />
                        <span className="font-mono truncate min-w-0">{m.name || m.id}</span>
                        {tags && <span className="text-[9px] text-slate-500 truncate flex-shrink-0">{tags}</span>}
                      </button>
                    );
                  })}
                </div>
              );
            })
          )}
        </div>
      </SharedModal>
    </>
  );
}
