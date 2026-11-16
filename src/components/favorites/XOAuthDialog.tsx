import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

import { SharedButton } from "../shared/Button";
import type { XAccount } from "./types";

interface Props {
  open: boolean;
  onClose: () => void;
  account: XAccount | null;
  /** 上次读取时间（本地记录；官方接口不返回收藏时间） */
  lastRunAt: string | null;
  onSaved: (account: XAccount | null) => void;
}

/**
 * X 官方 API 授权弹窗。
 *
 * 与 `CredentialDialog` 分开：授权不是「粘一个字符串」，而是
 * 「填 Client ID → 拉起系统浏览器 → 本地 127.0.0.1 回调 → 存 token」。
 * Client ID 只是 App 的公开标识，不是密钥。
 */
export function XOAuthDialog({ open, onClose, account, lastRunAt, onSaved }: Props) {
  const { t } = useTranslation();
  const [clientId, setClientId] = useState("");
  /** 两种拿 token 的方式：浏览器授权（PKCE），或直接粘贴已有的 token。 */
  const [mode, setMode] = useState<"oauth" | "token">("oauth");
  const [accessToken, setAccessToken] = useState("");
  const [refreshToken, setRefreshToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setError(null);
    setClientId(account?.clientId ?? "");
    setAccessToken("");
    setRefreshToken("");
  }, [open, account]);

  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      const next = await invoke<XAccount>("fav_x_oauth_start", {
        clientId: clientId.trim(),
      });
      onSaved(next);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  /** 直接保存手上已有的 token：后端会先用 /2/users/me 验证再加密落库。 */
  const saveToken = async () => {
    setBusy(true);
    setError(null);
    try {
      const next = await invoke<XAccount>("fav_x_oauth_save_token", {
        clientId: clientId.trim(),
        accessToken: accessToken.trim(),
        refreshToken: refreshToken.trim() || null,
      });
      onSaved(next);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const logout = async () => {
    setBusy(true);
    setError(null);
    try {
      // 复用通用凭证命令：传空串即清除（token 内容不经过前端）
      await invoke("fav_set_credential", { source: "x-oauth", cookie: "" });
      onSaved(null);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-[260] flex items-center justify-center bg-black/60 backdrop-blur-sm p-4"
      onClick={onClose}
    >
      <div
        className="w-[520px] max-w-full rounded-panel border border-white/10 bg-slate-900 p-4 space-y-3"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="text-title font-bold text-white">{t("favorites.xOauthTitle")}</div>
        <p className="text-tiny text-slate-400 leading-snug">{t("favorites.xOauthHint")}</p>

        <label className="block text-tiny text-slate-300">
          {t("favorites.xOauthClientId")}
          <input
            value={clientId}
            onChange={(e) => setClientId(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") onClose();
            }}
            placeholder={t("favorites.xOauthClientIdPlaceholder")}
            spellCheck={false}
            autoComplete="off"
            disabled={busy}
            className="mt-1 w-full bg-black/30 border border-white/10 rounded-ctl px-2.5 py-2 text-body font-mono text-slate-100 outline-none focus:border-[var(--module-accent)]"
          />
        </label>

        <div className="flex gap-2">
          {(["oauth", "token"] as const).map((m) => (
            <button
              key={m}
              onClick={() => { setMode(m); setError(null); }}
              disabled={busy}
              className={`px-2.5 py-1 rounded-ctl text-tiny border ${
                mode === m
                  ? "border-[var(--module-accent)] text-white bg-white/10"
                  : "border-white/10 text-slate-400 hover:text-slate-200"
              }`}
            >
              {t(m === "oauth" ? "favorites.xOauthModeBrowser" : "favorites.xOauthModeToken")}
            </button>
          ))}
        </div>

        {mode === "token" && (
          <>
            <label className="block text-tiny text-slate-300">
              {t("favorites.xOauthAccessToken")}
              <textarea
                value={accessToken}
                onChange={(e) => setAccessToken(e.target.value)}
                placeholder={t("favorites.xOauthAccessTokenPlaceholder")}
                spellCheck={false}
                autoComplete="off"
                rows={3}
                disabled={busy}
                className="mt-1 w-full bg-black/30 border border-white/10 rounded-ctl px-2.5 py-2 text-tiny font-mono text-slate-100 outline-none focus:border-[var(--module-accent)] break-all"
              />
            </label>
            <label className="block text-tiny text-slate-300">
              {t("favorites.xOauthRefreshToken")}
              <textarea
                value={refreshToken}
                onChange={(e) => setRefreshToken(e.target.value)}
                placeholder={t("favorites.xOauthRefreshTokenPlaceholder")}
                spellCheck={false}
                autoComplete="off"
                rows={2}
                disabled={busy}
                className="mt-1 w-full bg-black/30 border border-white/10 rounded-ctl px-2.5 py-2 text-tiny font-mono text-slate-100 outline-none focus:border-[var(--module-accent)] break-all"
              />
            </label>
            <p className="text-tiny text-slate-500 leading-snug">{t("favorites.xOauthTokenHint")}</p>
          </>
        )}

        <p className="text-tiny text-slate-500">
          {lastRunAt
            ? t("favorites.xOauthLastRead", { time: lastRunAt.replace("T", " ") })
            : t("favorites.xOauthNeverRead")}
        </p>

        {account && (
          <p className="text-tiny text-emerald-400">
            {t("favorites.xOauthSignedIn", {
              name: account.screenName || account.name,
            })}
          </p>
        )}
        {busy && <p className="text-tiny text-amber-400">{t("favorites.xOauthWaiting")}</p>}
        {error && <p className="text-tiny text-rose-400 break-all">{error}</p>}

        <div className="flex justify-end gap-2">
          {account && (
            <SharedButton variant="secondary" onClick={() => void logout()} disabled={busy}>
              {t("favorites.xOauthLogout")}
            </SharedButton>
          )}
          <SharedButton variant="secondary" onClick={onClose} disabled={busy}>
            {t("common.cancel")}
          </SharedButton>
          {mode === "oauth" ? (
            <SharedButton onClick={() => void start()} disabled={busy || !clientId.trim()}>
              {t("favorites.xOauthStart")}
            </SharedButton>
          ) : (
            <SharedButton
              onClick={() => void saveToken()}
              disabled={busy || !clientId.trim() || !accessToken.trim()}
            >
              {t("favorites.xOauthSaveToken")}
            </SharedButton>
          )}
        </div>
      </div>
    </div>
  );
}
