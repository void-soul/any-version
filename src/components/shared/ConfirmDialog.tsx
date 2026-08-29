// 共享确认弹框：用 SharedModal 实现统一风格的确认对话框，替代 window.confirm/alert。
// 用法：<ConfirmDialog open onCancel onConfirm title desc confirmText danger />。
// 全 app 规则自动继承（无 Esc/遮罩关闭）。
import { SharedButton } from "./Button";
import { SharedModal } from "./Modal";

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
  title = "确认操作",
  desc,
  confirmText = "确认",
  cancelText = "取消",
  danger = false,
  width = 380,
}: ConfirmDialogProps) {
  return (
    <SharedModal
      open={open}
      onClose={onCancel}
      title={title}
      width={width}
      footer={
        <>
          <SharedButton onClick={onCancel} variant="secondary">
            {cancelText}
          </SharedButton>
          <SharedButton onClick={onConfirm} variant={danger ? "danger" : "primary"} autoFocus>
            {confirmText}
          </SharedButton>
        </>
      }
    >
      <div className="text-[12px] text-slate-400 leading-relaxed">{desc}</div>
    </SharedModal>
  );
}