// 全局 Toast：模块内直接调用 toast(msg, kind) 即可，无需 Provider/挂载。
// kind: "ok"(绿) | "err"(红) | "info"(全局主色调)。自动淡出，供全 app 统一反馈体验。
// 替代各模块手写的 showToast 状态机（LauncherPanel/ClipboardPanel 等）。
//
// 主色调来源：Toast 挂在 document.body 下，而 `--module-accent` 是 App 注入在
// `#app-content` 上的**当前模块色**，body 这一层取不到——于是气泡要么没色、
// 要么跟着某个模块变。这里显式读全局设置的主色调（`resolveThemeAccent`），
// 再用内联样式把它作为 `--module-accent` 写在气泡容器上，让子元素的既有 class 直接生效。
import { useEffect, useState, type CSSProperties } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { Check, AlertCircle, Info } from "lucide-react";

import { VEX_CYBER_ACCENT, resolveThemeAccent } from "../../utils/brand";

type ToastKind = "ok" | "err" | "info";

/** 气泡上的修复动作：让「报错」不只是告知，还带一条出路（例：端口被占用 → 结束进程）。 */
export interface ToastAction {
  label: string;
  onClick: () => void;
}

interface ToastMsg {
  id: number;
  kind: ToastKind;
  msg: string;
  action?: ToastAction;
}

/** 普通提示停留时长；带动作的要多留一会儿 —— 2.6s 根本来不及看清再点按钮。 */
const TOAST_MS = 2600;
const TOAST_MS_WITH_ACTION = 9000;

let items: ToastMsg[] = [];
let seq = 0;
let render: (() => void) | undefined;

/** 手动关闭某条气泡（点了动作按钮就该立刻消失，否则会留下一个已经处理过的报错）。 */
function dismiss(id: number): void {
  items = items.filter((t) => t.id !== id);
  render?.();
}

/** 显示一条 toast：toast("已保存") 或 toast("失败", "err")。action 可选，带一个修复按钮。 */
export function toast(msg: string, kind: ToastKind = "ok", action?: ToastAction): void {
  const item: ToastMsg = { id: ++seq, kind, msg, action };
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
  }, action ? TOAST_MS_WITH_ACTION : TOAST_MS);
}

/** 主色调的进程内缓存：读一次就够（后续气泡直接用，不必先闪一下默认签名色）。 */
let cachedAccent: string | null = null;

function ToastView({ items }: { items: ToastMsg[] }) {
  const [ready, setReady] = useState(false);
  // 全局主色调（全局设置里选的那个），而不是当前模块色
  const [accent, setAccent] = useState(cachedAccent ?? VEX_CYBER_ACCENT);
  useEffect(() => {
    setReady(true);
    let alive = true;
    (async () => {
      try {
        const ap = await invoke<{ moduleThemeColors?: Record<string, string> }>(
          "get_appearance_config",
        );
        if (!alive) return;
        const next = resolveThemeAccent(ap.moduleThemeColors);
        cachedAccent = next;
        setAccent(next);
      } catch {
        /* 读不到就用默认签名色，不影响提示本身 */
      }
    })();
    return () => {
      alive = false;
    };
  }, []);
  if (items.length === 0) return null;
  return (
    <div
      className={`fixed left-4 bottom-4 flex flex-col gap-2 pointer-events-none ${ready ? "animate-in fade-in duration-200" : ""}`}
      style={
        {
          maxWidth: 420,
          // 覆写成全局主色调：下面那些 `var(--module-accent)` 的 class 从这里取值
          "--module-accent": accent,
        } as CSSProperties
      }
    >
      {items.map((t) => {
        const Icon = t.kind === "ok" ? Check : t.kind === "err" ? AlertCircle : Info;
        const iconCls =
          t.kind === "ok"
            ? "bg-emerald-500/20 text-emerald-300"
            : t.kind === "err"
              ? "bg-rose-500/20 text-rose-300"
              : "bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] text-[var(--module-accent)]";
        // 描边取该条通知的语义色：ok 绿 / err 红 / info 全局主色。
        // 气泡用的 `.vex-neon-edge` 渐变读的是 `--vex-primary`/`--vex-cyan`，
        // 不覆盖的话永远是品牌紫→青，通知就跟不上主题色了。
        const edgeColor = t.kind === "ok" ? "#34d399" : t.kind === "err" ? "#f43f5e" : accent;
        const glowShadow =
          t.kind === "ok"
            ? "0 0 12px rgba(52,211,153,0.28), 0 0 30px rgba(52,211,153,0.16), 0 12px 26px rgba(0,0,0,0.5)"
            : t.kind === "err"
              ? "0 0 14px rgba(244,63,94,0.35), 0 0 34px rgba(244,63,94,0.20), 0 12px 26px rgba(0,0,0,0.5)"
              : "0 0 12px color-mix(in srgb, var(--module-accent) 30%, transparent), 0 0 30px color-mix(in srgb, var(--module-accent) 16%, transparent), 0 12px 26px rgba(0,0,0,0.5)";
        return (
          <div
            key={t.id}
            // 容器整体是 pointer-events-none（不挡下方界面），只有带动作的气泡才放行点击
            className={`vex-neon-edge flex items-center gap-2.5 pl-3 pr-4 py-2.5 rounded-card bg-slate-900/95 backdrop-blur-md ${t.action ? "pointer-events-auto" : ""} ${t.kind === "err" ? "vex-toast-pulse" : t.kind === "ok" ? "vex-toast-light" : ""}`}
            style={
              {
                boxShadow: glowShadow,
                // 覆写霓虹描边渐变的两端为语义色（info = 全局主色），描边即随主题走
                "--vex-primary": edgeColor,
                "--vex-cyan": edgeColor,
              } as CSSProperties
            }
          >
            <span className={`w-5 h-5 rounded-md flex items-center justify-center flex-shrink-0 ${iconCls}`}>
              <Icon className="w-3 h-3" />
            </span>
            <span className="min-w-0 text-body text-slate-100 leading-snug break-words">{t.msg}</span>
            {t.action && (
              <button
                type="button"
                onClick={() => {
                  const run = t.action!.onClick;
                  dismiss(t.id);
                  run();
                }}
                className="flex-shrink-0 rounded-ctl border border-white/15 bg-white/10 px-2 py-1 text-caption text-slate-100 transition hover:bg-white/20 cursor-pointer"
              >
                {t.action.label}
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}