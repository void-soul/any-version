import { useState, useEffect, useRef } from "react";
import { SharedModal } from "../shared/Modal";
import { useTranslation } from "react-i18next";
/**
 * 申请日志弹窗：live 模式逐行追加并自动滚到底，历史模式直接展示落盘内容。
 *
 * 两种模式共用同一个组件，是为了让「正在跑」和「回看」看到的东西完全一致 ——
 * 申请途中关掉面板，再点行内「日志」看到的就是同一份内容（后端同时落盘）。
 */
export function IssueLogModal({
  open,
  title,
  lines,
  summary,
  onClose,
}: {
  open: boolean;
  title: string;
  lines: string[];
  summary?: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const boxRef = useRef<HTMLDivElement | null>(null);
  const [copied, setCopied] = useState(false);

  // 新行进来就贴底：签发过程几十秒，用户不该边看边手动滚
  useEffect(() => {
    const el = boxRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [lines.length, open]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(lines.join("\n"));
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // 复制失败不影响查看
    }
  };

  return (
    <SharedModal
      open={open}
      onClose={onClose}
      title={title}
      width={780}
      footer={
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={copy}
            className="px-3 py-1.5 rounded-ctl bg-white/10 text-slate-300 text-caption cursor-pointer"
          >
            {copied ? t("certmgr.copied") : t("certmgr.copy")}
          </button>
          <button
            type="button"
            onClick={onClose}
            className="px-4 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold cursor-pointer"
          >
            {t("certmgr.close")}
          </button>
        </div>
      }
    >
      {summary && <div className="mb-2 text-caption text-slate-300">{summary}</div>}
      <div
        ref={boxRef}
        className="max-h-[55vh] overflow-y-auto rounded-ctl border border-white/10 bg-black/40 p-2"
      >
        {lines.length === 0 ? (
          <div className="text-caption text-slate-500">{t("certmgr.issueLogEmpty")}</div>
        ) : (
          <pre className="text-micro font-mono whitespace-pre-wrap break-all text-slate-300">
            {lines.join("\n")}
          </pre>
        )}
      </div>
    </SharedModal>
  );
}
