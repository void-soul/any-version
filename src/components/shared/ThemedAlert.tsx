// 全局「主题弹窗警告」：模块内直接 theamedAlert(msg) 即可，无需 Provider / 挂载。
//
// 用途：替换原生 `alert()`。原生弹窗是浏览器 chrome，不跟随应用主题、不受
// 全局快捷键屏蔽影响，也没有统一的按钮与图标 —— 与全 app 的视觉语言是割裂的。
// 这里复用 SharedModal（统一主题色 + .modal-mask 让 main.tsx 屏蔽全局快捷键），
// 并按 kind 给出对应图标与配色。
//
// 挂载方式与 Toast 一致：首次调用时惰性 createRoot，之后复用同一个根节点。
import { createRoot } from "react-dom/client";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Info, XCircle } from "lucide-react";

import { SharedModal } from "./Modal";

export type AlertKind = "warn" | "err" | "info";

interface AlertState {
  open: boolean;
  message: string;
  kind: AlertKind;
}

let state: AlertState = { open: false, message: "", kind: "warn" };
let render: (() => void) | undefined;
let seq = 0;

/**
 * 弹出一条主题风格警告：`theamedAlert("请填写名称")` / `theamedAlert(err, "err")`。
 *
 * 与 Toast 的分工：Toast 用于「已保存 / 已删除」这类一闪而过的成功反馈；
 * 需要用户点掉、或内容较长（错误信息）时用这里。
 */
export function theamedAlert(message: string, kind: AlertKind = "warn"): void {
  seq += 1;
  state = { open: true, message, kind };
  if (!render) {
    const host = document.createElement("div");
    host.id = "global-theamed-alert-root";
    document.body.appendChild(host);
    const root = createRoot(host);
    render = () => root.render(<AlertView key={seq} />);
  }
  render();
}

/** 便捷别名：失败类提示用 `alertError(...)`，读起来更贴语义。 */
export const alertError = (message: string) => theamedAlert(message, "err");

function AlertView() {
  const { t } = useTranslation();
  const close = () => {
    state = { ...state, open: false };
    render?.();
  };

  const kind = state.kind;
  const title = kind === "err" ? t("common.error") : kind === "info" ? t("common.info") : t("common.warning");
  const Icon = kind === "err" ? XCircle : kind === "info" ? Info : AlertTriangle;
  const tone = kind === "err"
    ? { bg: "bg-red-500/10", border: "border-red-500/30", text: "text-red-300", icon: "text-red-400" }
    : kind === "info"
      ? { bg: "bg-white/[0.04]", border: "border-white/10", text: "text-slate-200", icon: "text-[var(--module-accent)]" }
      : { bg: "bg-amber-500/10", border: "border-amber-500/30", text: "text-amber-100", icon: "text-amber-400" };

  return (
    <SharedModal
      open={state.open}
      onClose={close}
      title={
        <span className="flex items-center gap-2">
          <Icon className={`w-4 h-4 ${tone.icon}`} />
          {title}
        </span>
      }
      width={440}
      footer={
        <div className="flex justify-end">
          <button
            type="button"
            onClick={close}
            className="cursor-pointer rounded-lg px-4 py-1.5 text-xs font-semibold text-white transition hover:brightness-110"
            style={{ backgroundColor: "var(--module-accent)" }}
          >
            {t("common.ok")}
          </button>
        </div>
      }
    >
      <div className={`rounded-xl border ${tone.border} ${tone.bg} p-3`}>
        <p className={`text-xs leading-relaxed whitespace-pre-wrap break-words ${tone.text}`}>
          {state.message}
        </p>
      </div>
    </SharedModal>
  );
}
