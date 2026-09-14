// 共享确认弹框：用 SharedModal 实现统一风格的确认对话框，替代 window.confirm/alert。
// 用法：<ConfirmDialog open onCancel onConfirm title desc confirmText danger />。
// 全 app 规则自动继承（无 Esc/遮罩关闭）。
import { SharedButton } from "./Button";
import { SharedModal } from "./Modal";
import { useTranslation } from "react-i18next";

interface ConfirmDialogProps {
  open: boolean;
  onCancel: () => void;
  onConfirm: () => void;
  title?: React.ReactNode;
  desc?: React.ReactNode;
  confirmText?: string;
  cancelText?: string;
  danger?: boolean;
  width?: number;
}

export function ConfirmDialog({
  open,
  onCancel,
  onConfirm,
  title,
  desc,
  confirmText,
  cancelText,
  danger = false,
  width = 380,
}: ConfirmDialogProps) {
  const { t } = useTranslation();
  const resolvedTitle = title ?? t("dialog.confirmTitle");
  const resolvedConfirm = confirmText ?? t("common.confirm");
  const resolvedCancel = cancelText ?? t("common.cancel");
  return (
    <SharedModal
      open={open}
      onClose={onCancel}
      title={resolvedTitle}
      width={width}
      footer={
        <>
          <SharedButton onClick={onCancel} variant="secondary">
            {resolvedCancel}
          </SharedButton>
          <SharedButton onClick={onConfirm} variant={danger ? "danger" : "primary"} autoFocus>
            {resolvedConfirm}
          </SharedButton>
        </>
      }
    >
      <div className="text-[12px] text-slate-400 leading-relaxed">{desc}</div>
    </SharedModal>
  );
}

/**
 * 一次确认请求：调用方把它 set 进 state，弹窗确认后回调 onConfirm 执行真正的动作。
 * 用于替代原生 `window.confirm`（后者样式跟随系统、无法逐条展示影响面）。
 */
export interface ConfirmRequest {
  title?: React.ReactNode;
  desc: React.ReactNode;
  confirmText?: string;
  cancelText?: string;
  danger?: boolean;
  width?: number;
  onConfirm: () => void;
}

/**
 * 把一份「按需确认」请求渲染成统一弹窗：
 *
 * ```tsx
 * const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null);
 * ...
 * {<ConfirmDialogHost request={confirmRequest} onClose={() => setConfirmRequest(null)} />}
 * ```
 */
export function ConfirmDialogHost({
  request,
  onClose,
}: {
  request: ConfirmRequest | null;
  onClose: () => void;
}) {
  if (!request) return null;
  return (
    <ConfirmDialog
      open
      danger={request.danger}
      width={request.width}
      title={request.title}
      desc={request.desc}
      confirmText={request.confirmText}
      cancelText={request.cancelText}
      onCancel={onClose}
      onConfirm={() => {
        const run = request.onConfirm;
        onClose();
        run();
      }}
    />
  );
}