// 共享弹框组件：统一所有模块的模态弹窗样式与交互。
// - 遵循全 app 统一规则：禁止 Esc / 点击遮罩关闭，只能点关闭按钮（见 main.tsx）
// - 主题色使用 --module-accent 系列 CSS 变量，随模块动态主题色联动
// - 用 createPortal 挂到 body，避免被父容器 overflow/transform 裁剪
import { useEffect } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";

import { X } from "lucide-react";

interface SharedModalProps {
  open: boolean;
  onClose: () => void;
  title?: React.ReactNode;
  /** 尺寸预设，覆盖最大宽度（px） */
  width?: number;
  /** 自定义 class，追加到面板（如 max-w、overflow） */
  className?: string;
  /** 底部操作区 */
  footer?: React.ReactNode;
  /** 顶栏标题右侧、关闭按钮左侧的附加控件区（如切换下拉框） */
  headerActions?: React.ReactNode;
  /** 内容区是否允许溢出隐藏（适合图片/表格预览） */
  bodyClass?: string;
  children?: React.ReactNode;
}

/**
 * 通用共享弹框。约定：
 * - 只通过 onClose（右上角关闭按钮 / footer 里的取消）关闭；点击遮罩、Esc 均不关闭。
 * - 背景遮罩用 .modal-mask，使 main.tsx 的「弹框打开时全局快捷键不响应」逻辑生效。
 */
export function SharedModal({
  open,
  onClose,
  title,
  width = 520,
  className = "",
  footer,
  headerActions,
  bodyClass = "",
  children,
}: SharedModalProps) {
  const { t } = useTranslation();
  // 弹框打开期间禁止 Esc 关闭（全 app 规则）
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") e.stopPropagation();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open]);

  if (!open) return null;

  return createPortal(
    <div className="fixed inset-0 z-[300] modal-mask flex items-center justify-center bg-black/60 backdrop-blur-sm p-4">
      <div
        role="dialog"
        aria-modal="true"
        className={`relative w-full rounded-2xl border border-white/10 bg-slate-900/95 shadow-2xl shadow-black/60 ${className}`}
        style={{ maxWidth: width, maxHeight: "88vh" }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* 顶栏：标题 + 关闭按钮（主题色装饰线） */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-white/10 sticky top-0 bg-slate-900/95 z-10 rounded-t-2xl">
          <h3 className="text-sm font-bold text-white flex items-center gap-2 min-w-0">
            <span className="w-1 h-4 rounded-full bg-[var(--module-accent)] flex-shrink-0" />
            <span className="truncate">{title}</span>
          </h3>
          <div className="flex items-center gap-2 flex-shrink-0">
            {headerActions}
            <button
              className="p-1 rounded-md text-slate-400 hover:text-white hover:bg-white/10 transition-all cursor-pointer"
              onClick={onClose}
              title={t("common.dialogClose")}
            >
              <X className="w-4 h-4" />
            </button>
          </div>
        </div>
        {/* 主体 */}
        <div className={`px-4 py-4 overflow-y-auto ${bodyClass || "space-y-3"}`} style={{ maxHeight: "calc(88vh - 96px)" }}>
          {children}
        </div>
        {/* 底部操作区 */}
        {footer && (
          <div className="px-4 py-3 border-t border-white/10 flex justify-end gap-2 sticky bottom-0 bg-slate-900/95 z-10 rounded-b-2xl">
            {footer}
          </div>
        )}
      </div>
    </div>,
    document.body
  );
}