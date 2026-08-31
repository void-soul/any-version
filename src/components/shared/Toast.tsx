// 全局 Toast：模块内直接调用 toast(msg, kind) 即可，无需 Provider/挂载。
// kind: "ok"(绿) | "err"(红) | "info"(模块主题色)。自动淡出，供全 app 统一反馈体验。
// 替代各模块手写的 showToast 状态机（LauncherPanel/ClipboardPanel 等）。
import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { Check, AlertCircle, Info } from "lucide-react";

type ToastKind = "ok" | "err" | "info";

interface ToastMsg {
  id: number;
  kind: ToastKind;
  msg: string;
}

let items: ToastMsg[] = [];
let seq = 0;
let render: (() => void) | undefined;

/** 显示一条 toast：toast("已保存") 或 toast("失败", "err")。 */
export function toast(msg: string, kind: ToastKind = "ok"): void {
  const item: ToastMsg = { id: ++seq, kind, msg };
  items = [...items.slice(-3), item];
  // 首次调用惰性挂载（避免模块渲染期间立即创建根节点）
  if (!render) {
    const host = document.createElement("div");
    host.id = "global-toast-root";
    document.body.appendChild(host);
    const root = createRoot(host);
    render = () => root.render(<ToastView key={seq} items={items} />);
  }
  render();
  setTimeout(() => {
    items = items.filter((t) => t.id !== item.id);
    render?.();
  }, 2600);
}

function ToastView({ items }: { items: ToastMsg[] }) {
  const [ready, setReady] = useState(false);
  useEffect(() => setReady(true), []);
  if (items.length === 0) return null;
  return (
    <div className={`fixed left-4 bottom-4 flex flex-col gap-2 pointer-events-none ${ready ? "animate-in fade-in duration-200" : ""}`} style={{ maxWidth: 420 }}>
      {items.map((t) => {
        const Icon = t.kind === "ok" ? Check : t.kind === "err" ? AlertCircle : Info;
        const iconCls =
          t.kind === "ok"
            ? "bg-emerald-500/20 text-emerald-300"
            : t.kind === "err"
              ? "bg-rose-500/20 text-rose-300"
              : "bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] text-[var(--module-accent)]";
        const glowShadow =
          t.kind === "ok"
            ? "0 0 12px rgba(52,211,153,0.28), 0 0 30px rgba(52,211,153,0.16), 0 12px 26px rgba(0,0,0,0.5)"
            : t.kind === "err"
              ? "0 0 14px rgba(244,63,94,0.35), 0 0 34px rgba(244,63,94,0.20), 0 12px 26px rgba(0,0,0,0.5)"
              : "0 0 12px color-mix(in srgb, var(--module-accent) 30%, transparent), 0 0 30px color-mix(in srgb, var(--module-accent) 16%, transparent), 0 12px 26px rgba(0,0,0,0.5)";
        return (
          <div
            key={t.id}
            className={`vex-neon-edge flex items-center gap-2.5 pl-3 pr-4 py-2.5 rounded-xl bg-slate-900/95 backdrop-blur-md ${t.kind === "err" ? "vex-toast-pulse" : t.kind === "ok" ? "vex-toast-light" : ""}`}
            style={{ boxShadow: glowShadow }}
          >
            <span className={`w-5 h-5 rounded-md flex items-center justify-center flex-shrink-0 ${iconCls}`}>
              <Icon className="w-3 h-3" />
            </span>
            <span className="text-[12px] text-slate-100 leading-snug break-words">{t.msg}</span>
          </div>
        );
      })}
    </div>
  );
}