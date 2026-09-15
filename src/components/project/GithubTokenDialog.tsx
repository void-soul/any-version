// GitHub Token 设置弹框：说明为什么需要 + 三步引导（直达 Token 生成页 → 选权限 → 粘贴保存）。
// 用 SharedModal 保持全 app 统一弹框风格（不可 Esc/遮罩关闭）。
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";
import { ExternalLink, Eye, EyeOff, Trash2 } from "lucide-react";

import { SharedModal } from "../shared/Modal";
import { SharedButton } from "../shared/Button";

/** Fine-grained Token 直接新建页（推荐：Public Repositories 只读即可） */
const TOKEN_PAGE_FINE = "https://github.com/settings/personal-access-tokens/new";
/** Classic Token 直接新建页（无需勾选任何 scope） */
const TOKEN_PAGE_CLASSIC = "https://github.com/settings/tokens/new";

interface Props {
  open: boolean;
  onClose: () => void;
  /** 保存/清除成功后回调（告知父组件当前是否已设置，用于刷新入口图标颜色） */
  onSaved?: (hasToken: boolean) => void;
}

export function GithubTokenDialog({ open, onClose, onSaved }: Props) {
  const { t } = useTranslation();
  const [value, setValue] = useState("");
  const [initial, setInitial] = useState("");
  const [show, setShow] = useState(false);
  const [saving, setSaving] = useState(false);
  const [loadError, setLoadError] = useState(false);

  useEffect(() => {
    if (!open) return;
    setLoadError(false);
    invoke<string>("project_get_github_token")
      .then((v) => {
        setValue(v);
        setInitial(v);
      })
      .catch(() => setLoadError(true));
  }, [open]);

  const dirty = value !== initial;
  const hasSaved = !!initial.trim();

  const persist = async (token: string) => {
    setSaving(true);
    try {
      await invoke("project_set_github_token", { token });
      setInitial(token);
      setValue(token);
      onSaved?.(!!token.trim());
      if (!token.trim()) onClose(); // 清除后直接收起
    } finally {
      setSaving(false);
    }
  };

  return (
    <SharedModal
      open={open}
      onClose={onClose}
      title={t("projlist.githubTokenTitle")}
      width={520}
      footer={
        <>
          {hasSaved && (
            <SharedButton
              variant="danger"
              onClick={() => persist("")}
              disabled={saving}
              className="mr-auto"
            >
              <Trash2 className="w-3 h-3" />
              {t("projlist.githubTokenClear")}
            </SharedButton>
          )}
          <SharedButton variant="secondary" onClick={onClose}>
            {t("common.cancel")}
          </SharedButton>
          <SharedButton
            variant="primary"
            onClick={() => persist(value.trim())}
            disabled={saving || !dirty}
            title={!dirty ? t("projlist.githubTokenNoChange") : undefined}
          >
            {t("projlist.githubTokenSave")}
          </SharedButton>
        </>
      }
    >
      {/* 为什么要设置 */}
      <div className="text-[12px] text-slate-300 leading-relaxed">
        {t("projlist.githubTokenHint")}
      </div>

      {/* 三步引导 */}
      <div className="space-y-2 mt-1">
        {/* 步骤 1 */}
        <div className="p-2.5 rounded-xl bg-white/[0.03] border border-white/5">
          <div className="flex items-start gap-2">
            <span className="flex-shrink-0 w-4 h-4 rounded-full bg-[var(--module-accent)]/20 text-[var(--module-accent)] text-[10px] font-bold flex items-center justify-center mt-px">1</span>
            <div className="min-w-0 flex-1">
              <p className="text-[12px] text-slate-200 font-semibold">{t("projlist.githubTokenStep1")}</p>
              <div className="flex flex-wrap gap-1.5 mt-1.5">
                <SharedButton variant="secondary" className="!h-6 !px-2 !text-[10px]" onClick={() => openUrl(TOKEN_PAGE_FINE)}>
                  {t("projlist.githubTokenLinkFine")} <ExternalLink className="w-2.5 h-2.5" />
                </SharedButton>
                <SharedButton variant="ghost" className="!h-6 !px-2 !text-[10px]" onClick={() => openUrl(TOKEN_PAGE_CLASSIC)}>
                  {t("projlist.githubTokenLinkClassic")} <ExternalLink className="w-2.5 h-2.5" />
                </SharedButton>
              </div>
            </div>
          </div>
        </div>
        {/* 步骤 2 */}
        <div className="p-2.5 rounded-xl bg-white/[0.03] border border-white/5">
          <div className="flex items-start gap-2">
            <span className="flex-shrink-0 w-4 h-4 rounded-full bg-[var(--module-accent)]/20 text-[var(--module-accent)] text-[10px] font-bold flex items-center justify-center mt-px">2</span>
            <div className="min-w-0 flex-1">
              <p className="text-[12px] text-slate-200 font-semibold">{t("projlist.githubTokenStep2")}</p>
              <p className="text-[11px] text-slate-400 mt-0.5">{t("projlist.githubTokenScopeNote")}</p>
            </div>
          </div>
        </div>
        {/* 步骤 3 */}
        <div className="p-2.5 rounded-xl bg-white/[0.03] border border-white/5">
          <div className="flex items-start gap-2">
            <span className="flex-shrink-0 w-4 h-4 rounded-full bg-[var(--module-accent)]/20 text-[var(--module-accent)] text-[10px] font-bold flex items-center justify-center mt-px">3</span>
            <div className="min-w-0 flex-1">
              <p className="text-[12px] text-slate-200 font-semibold">{t("projlist.githubTokenStep3")}</p>
              <div className="flex items-center gap-1 mt-1.5">
                <input
                  type={show ? "text" : "password"}
                  value={value}
                  onChange={(e) => setValue(e.target.value)}
                  placeholder={t("projlist.githubTokenPh")}
                  spellCheck={false}
                  autoComplete="off"
                  className="flex-1 min-w-0 glass-input px-2 h-7 text-[11px] font-mono"
                />
                <button
                  onClick={() => setShow(!show)}
                  className="p-1.5 rounded text-slate-400 hover:text-slate-200 hover:bg-white/10 cursor-pointer flex-shrink-0"
                  title={show ? t("projlist.githubTokenHide") : t("projlist.githubTokenShow")}
                >
                  {show ? <EyeOff className="w-3 h-3" /> : <Eye className="w-3 h-3" />}
                </button>
              </div>
              {hasSaved && (
                <p className="text-[10px] text-emerald-400 mt-1.5 flex items-center gap-1">
                  <span className="w-1.5 h-1.5 rounded-full bg-emerald-400" />
                  {t("projlist.githubTokenSet")}
                </p>
              )}
              {loadError && (
                <p className="text-[10px] text-amber-400 mt-1.5">{t("projlist.githubTokenLoadFail")}</p>
              )}
            </div>
          </div>
        </div>
      </div>

      <p className="text-[10px] text-slate-500 leading-snug">{t("projlist.githubTokenLocalNote")}</p>
    </SharedModal>
  );
}
