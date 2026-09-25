import { useEffect, useRef, useState, type ReactNode } from "react";
import { ChevronDown, RefreshCw } from "lucide-react";

/** 可点的菜单条目（`type` 缺省即条目，显式写出来是为了让 TS 能收窄联合类型）。 */
export interface MenuAction {
  key: string;
  type?: "item";
  label: ReactNode;
  /** 右侧灰色补充说明（如「已配置」「正在导入」） */
  hint?: ReactNode;
  disabled?: boolean;
  danger?: boolean;
  /** 当前生效项（如选中的模型），左侧会有高亮点 */
  active?: boolean;
  onSelect?: () => void;
}

/** 菜单里的一项：可点条目 / 分隔线 / 分组标题。 */
export type MenuEntry =
  | MenuAction
  | { key: string; type: "separator" }
  | { key: string; type: "header"; label: ReactNode };

export interface MenuProps {
  /** 触发器文案 */
  label: ReactNode;
  items: MenuEntry[];
  /** 触发按钮禁用（如任务冲突时） */
  disabled?: boolean;
  busy?: boolean;
  title?: string;
  /** 面板对齐方式，靠右的菜单建议 right，避免超出窗口 */
  align?: "left" | "right";
  className?: string;
}

/**
 * 轻量下拉菜单（无第三方依赖）。
 *
 * 存在的理由：收藏模块顶部的动作按来源平铺时会**自动换行成两排**，
 * 看着乱且每加一个来源就多挤一格。把「同一条主线上的多个动作」收进一个菜单，
 * 顶栏就永远只有固定几个入口。
 *
 * 行为：点外部或按 Esc 关闭；条目点击后自动关闭（除非 disabled）。
 */
export function Menu({
  label,
  items,
  disabled,
  busy,
  title,
  align = "left",
  className = "",
}: MenuProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div ref={rootRef} className={`relative inline-flex ${className}`}>
      <button
        type="button"
        disabled={disabled}
        title={title}
        onClick={() => setOpen((v) => !v)}
        className={`text-[11px] cursor-pointer transition-colors flex items-center gap-0.5 ${
          disabled
            ? "text-slate-600 cursor-not-allowed"
            : open
              ? "text-[var(--module-accent)]"
              : "text-slate-400 hover:text-[var(--module-accent)]"
        }`}
        aria-haspopup="menu"
        aria-expanded={open}
      >
        {/* busy：菜单里的事情正在跑（如导入中），触发器上直接转圈告知 */}
        {busy && <RefreshCw className="w-2.5 h-2.5 animate-spin" />}
        {label}
        <ChevronDown className={`w-3 h-3 transition-transform ${open ? "rotate-180" : ""}`} />
      </button>

      {open && (
        <div
          role="menu"
          className={`absolute top-full mt-1 z-[260] min-w-[180px] max-w-[300px] py-1 rounded-lg border border-white/10 bg-surface-panel shadow-2xl ${
            align === "right" ? "right-0" : "left-0"
          }`}
        >
          {items.map((it) => {
            if ("type" in it && it.type === "separator") {
              return <div key={it.key} className="my-1 h-px bg-white/10" />;
            }
            if ("type" in it && it.type === "header") {
              return (
                <div key={it.key} className="px-2 py-1 text-[9px] uppercase tracking-wide text-slate-600">
                  {it.label}
                </div>
              );
            }
            const item = it as MenuAction;
            return (
              <button
                key={item.key}
                role="menuitem"
                type="button"
                disabled={item.disabled}
                onClick={() => {
                  if (item.disabled) return;
                  item.onSelect?.();
                  setOpen(false);
                }}
                className={`w-full flex items-center gap-2 px-2 py-1 text-left text-[11px] transition-colors ${
                  item.disabled
                    ? "text-slate-600 cursor-not-allowed"
                    : item.danger
                      ? "text-rose-300 hover:bg-rose-500/15 cursor-pointer"
                      : "text-slate-300 hover:bg-white/10 cursor-pointer"
                } ${item.active ? "text-[var(--module-accent)]" : ""}`}
              >
                <span className="w-2 shrink-0">{item.active ? "•" : ""}</span>
                <span className="flex-1 min-w-0 truncate">{item.label}</span>
                {item.hint && <span className="shrink-0 text-[9px] text-slate-500">{item.hint}</span>}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
