// 导出弹框：导出 SQL 的所有选项都在这里。
//
// 为什么要有这个弹框：导出本身有两类选择 —— **目标**（写文件 / 剪贴板 / 另存为）
// 与**产物形态**（带不带外键约束）。原先只有一个「导出 SQL」按钮：目标靠「有没有落盘过」
// 隐式决定，形态完全没有入口 —— 想导一份不带外键的 SQL，只能去改设计文件本身。
// 现在两者都摆到台面上。
//
// 每次打开都用默认值（面板侧靠「打开时才挂载」实现）：**不记忆上次选择** ——
// 上一份设计要外键、这一份不要，凭记忆很容易忘改，而忘改的后果是导出一份带外键的 SQL。
//
// 结构上刻意做成「一个选项 = 一项配置」：以后加工选项就是往下面再加一块，骨架不动。
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { FileDown, ClipboardCopy, Save } from "lucide-react";

import { SharedModal } from "../shared/Modal";
import { predictSqlPath, defaultExportChoice, type ExportChoice, type ExportTarget } from "./exportOptions";

// 纯逻辑（默认值 / 路径预测）在 exportOptions.ts，那儿才跑测试；这里只管展示
export type { ExportChoice, ExportTarget } from "./exportOptions";

export default function ExportDialog({
  open,
  onClose,
  designPath,
  busy,
  onExport,
}: {
  open: boolean;
  onClose: () => void;
  /** 当前设计文件路径（决定「写入文件」可不可用、以及预测文件名） */
  designPath: string | null;
  busy: boolean;
  onExport: (choice: ExportChoice) => void;
}) {
  const { t } = useTranslation();
  // 面板侧只在打开时挂载本组件，所以这里的初始值就是「本次打开的默认值」
  const [choice, setChoice] = useState<ExportChoice>(() => defaultExportChoice(!!designPath));
  if (!open) return null;
  const set = (patch: Partial<ExportChoice>) => setChoice((prev) => ({ ...prev, ...patch }));

  const TARGETS: { id: ExportTarget; icon: typeof FileDown; label: string; hint: string; disabled?: boolean }[] = [
    {
      id: "file",
      icon: FileDown,
      label: t("dbd.exportTargetFile"),
      hint: designPath ? predictSqlPath(designPath) : t("dbd.exportTargetFileNoPath"),
      disabled: !designPath,
    },
    {
      id: "clipboard",
      icon: ClipboardCopy,
      label: t("dbd.exportTargetClipboard"),
      hint: t("dbd.exportTargetClipboardHint"),
    },
    {
      id: "saveAs",
      icon: Save,
      label: t("dbd.exportTargetSaveAs"),
      hint: t("dbd.exportTargetSaveAsHint"),
    },
  ];

  return (
    <SharedModal
      open={open}
      onClose={onClose}
      title={t("dbd.exportDialogTitle")}
      width={520}
      footer={
        <div className="flex items-center justify-end gap-2">
          <button onClick={onClose} className="rounded-ctl px-3 py-1.5 text-body text-slate-300 cursor-pointer">
            {t("dbd.exportCancel")}
          </button>
          <button
            onClick={() => onExport(choice)}
            disabled={busy || (choice.target === "file" && !designPath)}
            className="flex items-center gap-1.5 rounded-ctl bg-[var(--module-accent)] px-3 py-1.5 text-body font-semibold text-white disabled:opacity-50 cursor-pointer"
          >
            <FileDown className="h-3.5 w-3.5" />
            {t("dbd.exportConfirm")}
          </button>
        </div>
      }
    >
      <div className="space-y-4">
        {/* 导出目标 */}
        <div>
          <div className="mb-1.5 text-tiny font-semibold text-slate-300">{t("dbd.exportTargetLabel")}</div>
          <div className="space-y-1">
            {TARGETS.map((x) => {
              const Icon = x.icon;
              const on = choice.target === x.id;
              return (
                <label
                  key={x.id}
                  className={`flex cursor-pointer items-start gap-2 rounded-ctl border px-2.5 py-2 transition ${
                    on ? "border-[var(--module-accent)] bg-white/[0.04]" : "border-white/10 hover:bg-white/[0.02]"
                  } ${x.disabled ? "cursor-not-allowed opacity-45" : ""}`}
                >
                  <input
                    type="radio"
                    name="dbd-export-target"
                    className="mt-0.5"
                    checked={on}
                    disabled={x.disabled}
                    onChange={() => set({ target: x.id })}
                  />
                  <Icon className="mt-0.5 h-3.5 w-3.5 shrink-0 text-slate-400" />
                  <span className="min-w-0 flex-1">
                    <span className="block text-body text-slate-200">{x.label}</span>
                    <span className="block truncate text-micro text-slate-500" title={x.hint}>
                      {x.hint}
                    </span>
                  </span>
                </label>
              );
            })}
          </div>
        </div>

        {/* 产物形态。以后加别的选项（如「含注释」）就是在这里再加一块 */}
        <label
          className={`flex cursor-pointer items-start gap-2 rounded-ctl border px-2.5 py-2 transition ${
            choice.foreignKeys
              ? "border-[var(--module-accent)] bg-white/[0.04]"
              : "border-white/10 hover:bg-white/[0.02]"
          }`}
        >
          <input
            type="checkbox"
            className="mt-0.5"
            checked={choice.foreignKeys}
            onChange={(e) => set({ foreignKeys: e.target.checked })}
          />
          <span className="min-w-0 flex-1">
            <span className="block text-body text-slate-200">{t("dbd.exportFkLabel")}</span>
            <span className="block text-micro text-slate-500">{t("dbd.exportFkHint")}</span>
            {/* 关掉外键时的连带提醒：校验仍要求外键字段有索引（校验只看设计模型，
                不看导出选项），不提前说会以为漏了索引导致导出被拦 */}
            {!choice.foreignKeys ? (
              <span className="mt-1 block text-micro text-amber-400/90">{t("dbd.exportFkIndexNote")}</span>
            ) : null}
          </span>
        </label>
      </div>
    </SharedModal>
  );
}