import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { Download, Check, Loader2, KeyRound, ExternalLink, FolderOpen, LogIn, X, Package } from "lucide-react";
import { type BuddyAccount, type OAuthStartResponse } from "./BuddyPanel.model";
import type { Dispatch, SetStateAction } from "react";

/** Owns the BuddyAccountImport feature, independently of page rendering. */
export function useBuddyAccountImport({ setBusy, setMessage, showMsg, displayName, load, platformLabel, busy, platform }: {
  setBusy: Dispatch<SetStateAction<boolean>>;
  setMessage: Dispatch<SetStateAction<{ ok: boolean; text: string; } | null>>;
  showMsg: (ok: boolean, text: string) => void;
  displayName: (acc: BuddyAccount) => string;
  load: () => Promise<void>;
  platformLabel: string;
  busy: boolean;
  platform: string;
}) {
  const { t } = useTranslation();

  // 新增账号
  const [showAdd, setShowAdd] = useState(false);

  const [addMode, setAddMode] = useState<"oauth" | "token" | "local">("oauth");

  const [oauth, setOauth] = useState<OAuthStartResponse | null>(null);

  const [oauthBusy, setOauthBusy] = useState(false);

  const [tokenInput, setTokenInput] = useState("");

  // WorkDaddy「账号导出」的密码（只有加密导出包需要，明文 JSON 留空即可）
  const [workdaddyPassword, setWorkdaddyPassword] = useState("");

  // ─── 账号操作 ───

  const importFromLocal = async () => {
    setBusy(true);
    setMessage(null);
    try {
      const account = await invoke<BuddyAccount | null>("buddy_import_from_local", { platform });
      if (account) {
        showMsg(true, t("buddy.imported", { name: displayName(account) }));
        setShowAdd(false);
      } else {
        showMsg(false, t("buddy.noLocalLogin"));
      }
      await load();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setBusy(false);
    }
  };

  /** 第三方工具导出文件导入（WorkDaddy 加密包 / cockpit-tools 明文 JSON，均不读取对方数据目录） */
  const importThirdParty = async (tool: "workdaddy" | "cockpit-tools") => {
    if (tool === "workdaddy" && !workdaddyPassword.trim()) {
      showMsg(false, t("buddy.importWorkdaddyPasswordRequired"));
      return;
    }
    try {
      const selected = await openDialog({
        title: tool === "workdaddy" ? t("buddy.importWorkdaddyPick") : t("buddy.importCockpitPick"),
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!selected) return;
      const path = Array.isArray(selected) ? selected[0] : selected;
      setBusy(true);
      setMessage(null);
      const content = await invoke<string>("read_text_file", { path });
      const imported = await invoke<BuddyAccount[]>("buddy_import_third_party", {
        platform,
        tool,
        jsonContent: content,
        password: tool === "workdaddy" ? workdaddyPassword : null,
      });
      showMsg(true, t("buddy.importedThirdParty", { count: imported.length }));
      setShowAdd(false);
      await load();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setBusy(false);
    }
  };

  const importFromJson = async () => {
    try {
      const selected = await openDialog({
        title: t("buddy.importJsonPick"),
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!selected) return;
      const path = Array.isArray(selected) ? selected[0] : selected;
      setBusy(true);
      setMessage(null);
      const json = await invoke<string>("read_text_file", { path });
      const imported = await invoke<BuddyAccount[]>("buddy_import_accounts", {
        platform,
        jsonContent: json,
      });
      showMsg(true, t("buddy.importedJson", { count: imported.length }));
      await load();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setBusy(false);
    }
  };

  // ─── 新增账号 ───

  const startOAuth = async () => {
    setOauthBusy(true);
    setMessage(null);
    try {
      const res = await invoke<OAuthStartResponse>("buddy_oauth_start", { platform });
      setOauth(res);
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setOauthBusy(false);
    }
  };

  const completeOAuth = async () => {
    if (!oauth) return;
    setOauthBusy(true);
    setMessage(null);
    try {
      const account = await invoke<BuddyAccount>("buddy_oauth_complete", {
        platform,
        loginId: oauth.loginId,
      });
      setShowAdd(false);
      setOauth(null);
      showMsg(true, t("buddy.added", { name: displayName(account) }));
      await load();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setOauthBusy(false);
    }
  };

  const cancelOAuth = async () => {
    if (oauth) {
      try {
        await invoke("buddy_oauth_cancel", { platform, loginId: oauth.loginId });
      } catch {
        // 忽略取消失败
      }
    }
    setOauth(null);
    setShowAdd(false);
  };

  const addWithToken = async () => {
    const token = tokenInput.trim();
    if (!token) return;
    setBusy(true);
    setMessage(null);
    try {
      const account = await invoke<BuddyAccount>("buddy_add_account_with_token", {
        platform,
        accessToken: token,
      });
      setShowAdd(false);
      setTokenInput("");
      showMsg(true, t("buddy.added", { name: displayName(account) }));
      await load();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setBusy(false);
    }
  };

const renderAddAccountDialog = () => (<div className="fixed inset-0 z-[130] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
          <div className="w-[420px] max-w-[95vw] rounded-panel border border-white/10 bg-slate-900/95 shadow-2xl p-5">
            <div className="flex items-center gap-2.5 mb-4">
              <div className="w-9 h-9 rounded-card bg-[var(--module-accent)]/15 border border-[var(--module-accent)]/30 flex items-center justify-center">
                {addMode === "oauth" ? <LogIn className="w-4 h-4 text-[var(--module-accent)]" /> : addMode === "token" ? <KeyRound className="w-4 h-4 text-[var(--module-accent)]" /> : <Download className="w-4 h-4 text-[var(--module-accent)]" />}
              </div>
              <div className="flex-1">
                <h3 className="text-sm font-bold text-white">{t("buddy.addTitle", { platform: platformLabel })}</h3>
              </div>
              <button onClick={() => void cancelOAuth()} className="p-1.5 rounded-ctl hover:bg-white/10 text-slate-400 cursor-pointer">
                <X className="w-4 h-4" />
              </button>
            </div>

            {/* 模式切换 */}
            <div className="flex items-center gap-1 bg-black/30 rounded-ctl border border-white/10 p-0.5 mb-4">
              <button
                onClick={() => setAddMode("oauth")}
                className={`flex-1 px-2 py-1.5 rounded-md text-caption transition cursor-pointer ${
                  addMode === "oauth" ? "bg-[var(--module-accent)]/25 text-white font-semibold" : "text-slate-400"
                }`}
              >
                {t("buddy.addOAuth")}
              </button>
              <button
                onClick={() => setAddMode("token")}
                className={`flex-1 px-2 py-1.5 rounded-md text-caption transition cursor-pointer ${
                  addMode === "token" ? "bg-[var(--module-accent)]/25 text-white font-semibold" : "text-slate-400"
                }`}
              >
                {t("buddy.addToken")}
              </button>
              <button
                onClick={() => setAddMode("local")}
                className={`flex-1 px-2 py-1.5 rounded-md text-caption transition cursor-pointer ${
                  addMode === "local" ? "bg-[var(--module-accent)]/25 text-white font-semibold" : "text-slate-400"
                }`}
              >
                {t("buddy.importLocal")}
              </button>
            </div>

            {addMode === "oauth" ? (
              oauth ? (
                <div className="space-y-3">
                  <p className="text-caption text-slate-400 leading-relaxed">{t("buddy.oauthWaitHint")}</p>
                  <a
                    href={oauth.verificationUri}
                    target="_blank"
                    rel="noreferrer"
                    className="flex items-center gap-2 text-caption ui-link break-all"
                  >
                    <ExternalLink className="w-3 h-3 flex-shrink-0" />
                    {oauth.verificationUri}
                  </a>
                  <p className="text-micro text-slate-600">
                    {t("buddy.oauthExpires", { minutes: Math.round(oauth.expiresIn / 60) })}
                  </p>
                  <div className="flex justify-end gap-2 pt-2">
                    <button
                      onClick={() => void cancelOAuth()}
                      className="px-3 py-1.5 rounded-ctl text-caption text-slate-400 hover:bg-white/5 cursor-pointer"
                    >
                      {t("buddy.cancel")}
                    </button>
                    <button
                      onClick={() => void completeOAuth()}
                      disabled={oauthBusy}
                      className="px-4 py-1.5 rounded-ctl text-caption ui-btn-primary text-white font-semibold flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
                    >
                      {oauthBusy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />}
                      {t("buddy.oauthComplete")}
                    </button>
                  </div>
                </div>
              ) : (
                <div className="space-y-3">
                  <p className="text-caption text-slate-400 leading-relaxed">{t("buddy.oauthStartHint")}</p>
                  <button
                    onClick={startOAuth}
                    disabled={oauthBusy}
                    className="w-full px-3 py-2 rounded-ctl text-caption ui-btn-primary text-white font-semibold flex items-center justify-center gap-1 cursor-pointer transition disabled:opacity-50"
                  >
                    {oauthBusy ? <Loader2 className="w-3 h-3 animate-spin" /> : <LogIn className="w-3 h-3" />}
                    {t("buddy.oauthStart")}
                  </button>
                </div>
              )
            ) : addMode === "token" ? (
              <div className="space-y-3">
                <textarea
                  value={tokenInput}
                  onChange={(e) => setTokenInput(e.target.value)}
                  placeholder={t("buddy.tokenPlaceholder")}
                  rows={4}
                  className="w-full bg-black/30 border border-white/10 rounded-ctl px-3 py-2 text-caption text-white outline-none focus:border-[var(--module-accent)]/50 placeholder:text-slate-600 resize-none"
                />
                <div className="flex justify-end gap-2">
                  <button
                    onClick={() => setShowAdd(false)}
                    className="px-3 py-1.5 rounded-ctl text-caption text-slate-400 hover:bg-white/5 cursor-pointer"
                  >
                    {t("buddy.cancel")}
                  </button>
                  <button
                    onClick={() => void addWithToken()}
                    disabled={busy || !tokenInput.trim()}
                    className="px-4 py-1.5 rounded-ctl text-caption ui-btn-primary text-white font-semibold flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
                  >
                    <KeyRound className="w-3 h-3" /> {t("buddy.addTokenSubmit")}
                  </button>
                </div>
              </div>
            ) : (
              <div className="space-y-3">
                <p className="text-caption text-slate-400 leading-relaxed">{t("buddy.importLocalHint")}</p>
                <button
                  onClick={() => void importFromLocal()}
                  disabled={busy}
                  className="w-full px-3 py-2 rounded-ctl text-caption ui-btn-primary text-white font-semibold flex items-center justify-center gap-1 cursor-pointer transition disabled:opacity-50"
                >
                  {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Download className="w-3 h-3" />}
                  {t("buddy.importLocal")}
                </button>

                {/* 第三方工具导出文件（不读取对方数据目录，仅解析导出的 JSON） */}
                <p className="text-tiny text-slate-500 pt-3 border-t border-white/10">{t("buddy.importSourceTitle")}</p>

                {platform === "workbuddy" && (
                  <div className="space-y-1.5">
                    <input
                      type="password"
                      value={workdaddyPassword}
                      onChange={(e) => setWorkdaddyPassword(e.target.value)}
                      placeholder={t("buddy.importWorkdaddyPassword")}
                      className="w-full bg-black/30 border border-white/10 rounded-ctl px-3 py-2 text-caption text-white outline-none focus:border-[var(--module-accent)]/50 placeholder:text-slate-600"
                    />
                    <button
                      onClick={() => void importThirdParty("workdaddy")}
                      disabled={busy}
                      className="w-full px-3 py-2 rounded-ctl text-caption border border-white/10 hover:border-[var(--module-accent)]/40 hover:bg-white/5 text-slate-200 flex items-center justify-center gap-1 cursor-pointer transition disabled:opacity-50"
                    >
                      {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Package className="w-3 h-3" />}
                      {t("buddy.importWorkdaddy")}
                    </button>
                    <p className="text-micro text-slate-600 leading-relaxed">{t("buddy.importWorkdaddyHint")}</p>
                  </div>
                )}

                <div className="space-y-1.5">
                  <button
                    onClick={() => void importThirdParty("cockpit-tools")}
                    disabled={busy}
                    className="w-full px-3 py-2 rounded-ctl text-caption border border-white/10 hover:border-[var(--module-accent)]/40 hover:bg-white/5 text-slate-200 flex items-center justify-center gap-1 cursor-pointer transition disabled:opacity-50"
                  >
                    {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <FolderOpen className="w-3 h-3" />}
                    {t("buddy.importCockpit")}
                  </button>
                  <p className="text-micro text-slate-600 leading-relaxed">{t("buddy.importCockpitHint")}</p>
                </div>
              </div>
            )}
          </div>
        </div>);
  return {
    showAdd,
    setShowAdd,
    setAddMode,
    setOauth,
    setTokenInput,
    importFromJson,
    renderAddAccountDialog,
  };
}
