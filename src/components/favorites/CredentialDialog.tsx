import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";

import { SharedButton } from "../shared/Button";

interface Props {
  open: boolean;
  onClose: () => void;
  /** 凭证键：B站 Cookie = `bilibili`，知乎 Access Secret = `zhihu`，知乎实验 Cookie = `zhihu-cookie`。
   *  **不同键各存各的**，互不覆盖 */
  source: string;
  title: string;
  hint: string;
  placeholder: string;
  /** 多行（Cookie 是长串）/ 单行（Access Secret 用密码框） */
  multiline?: boolean;
  /** 琥珀色的风险提示（实验功能才给） */
  note?: string;
  onSaved?: (configured: boolean) => void;
  /**
   * 保存成功并关闭弹窗之后要做的事（例如「保存并测试」里的测试步骤）。
   *
   * **不在保存前 await**：探测要等页面重载，几十秒不关弹窗会让人以为卡死，
   * 结果统一走 toast 汇报。也不能在保存前调用——那时后端还是旧凭证。
   */
  afterSave?: (value: string) => void;
}

/**
 * 各平台凭证配置弹窗。
 *
 * 三个平台三种凭证共用这一个组件：B站 Cookie、知乎 Access Secret、知乎实验 Cookie。
 * 区别只在 `source`（存储键）与文案——**键不同就不会互相覆盖**，
 * 这点很重要：之前实验代码复用 Access Secret 的槽位存 Cookie，会把用户已配好的
 * Access Secret 顶掉，就是因为这个槽位是按 source 唯一的。
 */
export function CredentialDialog({
  open,
  onClose,
  source,
  title,
  hint,
  placeholder,
  multiline = false,
  note,
  onSaved,
  afterSave,
}: Props) {
  const { t } = useTranslation();
  const [value, setValue] = useState("");
  const [saving, setSaving] = useState(false);

  // 打开时回显已保存的值：否则再次打开看起来像没保存过
  useEffect(() => {
    if (!open) return;
    invoke<string>("fav_get_credential", { source })
      .then((v) => setValue(v ?? ""))
      .catch(() => setValue(""));
  }, [open, source]);

  const persist = async () => {
    const trimmed = value.trim();
    if (!trimmed) return;
    setSaving(true);
    try {
      await invoke("fav_set_credential", { source, cookie: trimmed });
      setValue(trimmed);
      onSaved?.(true);
      onClose();
      afterSave?.(trimmed);
    } finally {
      setSaving(false);
    }
  };

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-[250] flex items-center justify-center bg-black/60 backdrop-blur-sm p-4"
      onClick={onClose}
    >
      <div
        className="w-[460px] max-w-full rounded-2xl border border-white/10 bg-slate-900 p-4 space-y-3"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="text-[13px] font-bold text-white">{title}</div>
        <p className="text-[10px] text-slate-400 leading-snug">{hint}</p>
        {multiline ? (
          <textarea
            value={value}
            onChange={(e) => setValue(e.target.value)}
            // 聚焦即全选：Cookie 是长串，用户重新粘一份时最怕「粘在旧值后面」，
            // 拼出来的 Cookie 会静默失效。全选后直接 Ctrl+V 就是整条替换。
            onFocus={(e) => e.currentTarget.select()}
            placeholder={placeholder}
            spellCheck={false}
            className="w-full h-24 glass-input p-2 text-[10px] font-mono resize-y"
          />
        ) : (
          <input
            type="password"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void persist();
              if (e.key === "Escape") onClose();
            }}
            placeholder={placeholder}
            spellCheck={false}
            autoComplete="off"
            className="w-full bg-black/30 border border-white/10 rounded-lg px-2.5 py-2 text-[12px] font-mono text-slate-100 outline-none focus:border-[var(--module-accent)]"
          />
        )}
        {note && <p className="text-[10px] text-amber-400/80 leading-snug">{note}</p>}
        <div className="flex justify-end gap-2">
          <SharedButton variant="secondary" onClick={onClose}>
            {t("common.cancel")}
          </SharedButton>
          <SharedButton onClick={() => void persist()} disabled={saving || !value.trim()}>
            {t("favorites.credentialSave")}
          </SharedButton>
        </div>
      </div>
    </div>
  );
}
