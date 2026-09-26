// 全局「主题弹窗警告 / 确认」：模块内直接 theamedAlert(msg) / await theamedConfirm(msg) 即可，
// 无需 Provider / 挂载。
//
// 用途：替换原生 `alert()` 与 `window.confirm()`。原生弹窗是浏览器 chrome，
// 不跟随应用主题、不受全局快捷键屏蔽影响，也没有统一的按钮与图标 —— 与全 app 的
// 视觉语言是割裂的。这里复用 SharedModal（统一主题色 + .modal-mask 让 main.tsx
// 屏蔽全局快捷键），并按 kind 给出对应图标与配色。
//
// 挂载方式与 Toast 一致：首次调用时惰性 createRoot，之后复用同一个根节点。
import { createRoot } from "react-dom/client";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Info, XCircle } from "lucide-react";

import { SharedButton } from "./Button";
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

/* ─── 命令式确认框：替代 window.confirm ─── */

interface ConfirmState {
  open: boolean;
  message: string;
  title: string;
  confirmText: string;
  danger: boolean;
  resolve?: (ok: boolean) => void;
}

let confirmState: ConfirmState | null = null;
let confirmRender: (() => void) | undefined;
let confirmSeq = 0;

/**
 * 弹出主题风格确认框：`if (!(await theamedConfirm(t("x.confirm")))) return;`
 *
 * 与 `ConfirmDialog`（声明式组件）的分工：这里给**命令式**调用点用
 * （回调里临时问一句、不想为此加 state 的场合），外观与 ConfirmDialog 一致。
 * 危险操作传 `{ danger: true }`，确认按钮变红。
 */
export function theamedConfirm(
  message: string,
  opts: { title?: string; confirmText?: string; danger?: boolean } = {},
): Promise<boolean> {
  return new Promise<boolean>((resolve) => {
    confirmSeq += 1;
    confirmState = {
      open: true,
      message,
      title: opts.title ?? "",
      confirmText: opts.confirmText ?? "",
      danger: opts.danger ?? false,
      resolve,
    };
    if (!confirmRender) {
      const host = document.createElement("div");
      host.id = "global-theamed-confirm-root";
      document.body.appendChild(host);
      const root = createRoot(host);
      confirmRender = () => root.render(<ConfirmView key={confirmSeq} />);
    }
    confirmRender();
  });
}

function ConfirmView() {
  const { t } = useTranslation();
  const settle = (ok: boolean) => {
    const done = confirmState?.resolve;
    confirmState = confirmState ? { ...confirmState, open: false, resolve: undefined } : null;
    confirmRender?.();
    done?.(ok);
  };

  if (!confirmState) return null;
  const { open, message, title, confirmText, danger } = confirmState;
  return (
    <SharedModal
      open={open}
      onClose={() => settle(false)}
      title={title || t("dialog.confirmTitle")}
      width={380}
      footer={
        <>
          <SharedButton onClick={() => settle(false)} variant="secondary">
            {t("common.cancel")}
          </SharedButton>
          <SharedButton onClick={() => settle(true)} variant={danger ? "danger" : "primary"} autoFocus>
            {confirmText || t("common.confirm")}
          </SharedButton>
        </>
      }
    >
      <div className="text-body text-slate-400 leading-relaxed">{message}</div>
    </SharedModal>
  );
}

function AlertView() {
  const { t } = useTranslation();
  const close = () => {
    state = { ...state, open: false };
    render?.();
  };

  const kind = state.kind;
  const title = kind === "err" ? t("common.error") : kind === "info" ? t("common.info") : t("common.warning");
  const Icon = kind === "err" ? XCircle : kind === "info" ? Info : AlertTriangle;
  // 提示框外观走通用层 `ui-note*`（与 Note 组件同一套），不在这里另配一套颜色
  const tone = kind === "err"
    ? { box: "ui-note-error", text: "text-rose-200", icon: "text-rose-400" }
    : kind === "info"
      ? { box: "ui-note-info", text: "text-slate-200", icon: "text-[var(--module-accent)]" }
      : { box: "ui-note-warn", text: "text-amber-100", icon: "text-amber-400" };

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
            className="ui-btn ui-btn-primary cursor-pointer px-4 py-1.5"
          >
            {t("common.ok")}
          </button>
        </div>
      }
    >
      <div className={`ui-note ${tone.box} p-3`}>
        <p className={`text-body leading-relaxed whitespace-pre-wrap break-words ${tone.text}`}>
          {state.message}
        </p>
      </div>
    </SharedModal>
  );
}
