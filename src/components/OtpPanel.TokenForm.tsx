import { btnPrimary, btnSecondary } from "./shared/Button";
import { useState } from "react";
import { theamedAlert } from "./shared/ThemedAlert";
import { Check, X, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { type OtpCategory, type OtpToken } from "./OtpPanel.model";
// 添加/编辑表单
export function TokenForm({
  token,
  categories,
  onSave,
  onClose,
  busy,
}: {
  token: OtpToken | null;
  categories: OtpCategory[];
  onSave: (t: OtpToken, categoryIds: number[]) => void;
  onClose: () => void;
  busy: boolean;
}) {
  const { t } = useTranslation();
  const [form, setForm] = useState<OtpToken>(
    token || {
      id: 0,
      issuer: "",
      account: "",
      secret: "",
      tokenType: "TOTP",
      algorithm: "SHA1",
      digits: 6,
      period: 30,
      counter: 0,
      pin: "",
      pinned: false,
      createdAt: 0,
      description: "",
      tags: "",
      copyTimes: 0,
      lastCopyTime: 0,
      customIcon: "",
    }
  );

  const set = (k: keyof OtpToken, v: unknown) => setForm((f) => ({ ...f, [k]: v }));

  // 该令牌当前所属分类（编辑时回填；新增为空）
  const [selectedCatIds, setSelectedCatIds] = useState<number[]>(() =>
    token
      ? categories.filter((c) => c.tokenIds.includes(token.id)).map((c) => c.id)
      : []
  );

  const toggleCat = (id: number) =>
    setSelectedCatIds((prev) =>
      prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]
    );

  const submit = () => {
    if (busy) return;
    if (!form.secret.trim()) {
      theamedAlert(t("otp.needSecret"));
      return;
    }
    onSave(form, selectedCatIds);
  };

  const isSteam = form.tokenType === "Steam";
  const isYandex = form.tokenType === "Yandex";
  const isMotp = form.tokenType === "MOTP";
  const isHotp = form.tokenType === "HOTP";

  return (
    <div className="fixed inset-0 z-[110] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
      <div role="dialog" aria-modal="true" aria-label={token ? t("otp.editToken") : t("otp.addToken")} className="w-[460px] max-w-[95vw] max-h-[90vh] overflow-y-auto rounded-panel border border-white/10 bg-surface-modal shadow-2xl p-4" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-title font-semibold text-white">{token ? t("otp.editToken") : t("otp.addToken")}</h3>
          <button onClick={onClose} disabled={busy} aria-label={t("common.dialogClose")} className="ui-icon-btn w-7 h-7">
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="space-y-3">
          <div>
            <label className="text-tiny text-slate-400 mb-1 block">{t("otp.type")}</label>
            <select
              value={form.tokenType}
              onChange={(e) => {
                const t = e.target.value;
                setForm((f) => ({
                  ...f,
                  tokenType: t,
                  digits: t === "Steam" ? 5 : t === "Yandex" ? 8 : 6,
                  algorithm: t === "Steam" || t === "Yandex" || t === "MOTP" ? "SHA1" : f.algorithm,
                }));
              }}
              disabled={busy} className="ui-input w-full h-9 px-2.5"
            >
              <option value="TOTP">{t("otp.totp")}</option>
              <option value="HOTP">{t("otp.hotp")}</option>
              <option value="MOTP">{t("otp.motp")}</option>
              <option value="Steam">Steam</option>
              <option value="Yandex">Yandex</option>
            </select>
          </div>

          <div className="grid grid-cols-2 gap-2">
            <div>
              <label className="text-tiny text-slate-400 mb-1 block">{t("otp.issuer")}</label>
              <input
                value={form.issuer}
                onChange={(e) => set("issuer", e.target.value)}
                placeholder={t("otp.issuerPh")}
                disabled={busy} className="ui-input w-full h-9 px-2.5"
              />
            </div>
            <div>
              <label className="text-tiny text-slate-400 mb-1 block">{t("otp.account")}</label>
              <input
                value={form.account}
                onChange={(e) => set("account", e.target.value)}
                placeholder={t("otp.accountPh")}
                disabled={busy} className="ui-input w-full h-9 px-2.5"
              />
            </div>
          </div>

          <div>
            <label className="text-tiny text-slate-400 mb-1 block">{t("otp.secret")}</label>
            <input
              value={form.secret}
              onChange={(e) => set("secret", e.target.value)}
              placeholder={t("otp.secretPh")}
              className="w-full glass-input px-3 py-2 text-body font-mono bg-black/30 border border-white/10 rounded-ctl focus:outline-none"
            />
          </div>

          <div>
            <label className="text-tiny text-slate-400 mb-1 block">{t("otp.desc")}</label>
            <input
              value={form.description}
              onChange={(e) => set("description", e.target.value)}
              placeholder={t("otp.descPh")}
              disabled={busy} className="ui-input w-full h-9 px-2.5"
            />
          </div>

          {/* 分类多选 */}
          <div>
            <label className="text-tiny text-slate-400 mb-1 block">
              {t("otp.category", { count: selectedCatIds.length })} {selectedCatIds.length > 0 && <span className="text-slate-500">({t("otp.selected", { count: selectedCatIds.length })})</span>}
            </label>
            {categories.length === 0 ? (
              <p className="text-tiny text-slate-500">{t("otp.categoryHint")}</p>
            ) : (
              <div className="flex flex-wrap gap-1.5">
                {categories.map((c) => {
                  const on = selectedCatIds.includes(c.id);
                  return (
                    <button
                      key={c.id}
                      type="button"
                      onClick={() => toggleCat(c.id)}
                      className={`vex-chip px-2.5 py-1 rounded-full text-tiny border transition-colors cursor-pointer ${
                        on
                          ? "vex-chip-active bg-[var(--module-accent)]/20 border-[var(--module-accent)] text-[var(--module-accent)]"
                          : "bg-white/[0.03] border-white/10 text-slate-400 hover:border-white/25"
                      }`}
                    >
                      {on ? "✓ " : ""}{c.title}
                    </button>
                  );
                })}
              </div>
            )}
          </div>

          <div className="grid grid-cols-2 gap-2">
            <div>
              <label className="text-tiny text-slate-400 mb-1 block">{t("otp.tags")}</label>
              <input
                value={form.tags}
                onChange={(e) => set("tags", e.target.value)}
                placeholder={t("otp.tagsPh")}
                disabled={busy} className="ui-input w-full h-9 px-2.5"
              />
            </div>
            <div>
              <label className="text-tiny text-slate-400 mb-1 block">{t("otp.iconChar")}</label>
              <input
                value={form.customIcon}
                onChange={(e) => set("customIcon", e.target.value)}
                placeholder={t("otp.iconPh")}
                disabled={busy} className="ui-input w-full h-9 px-2.5"
              />
            </div>
          </div>

          {!isSteam && !isYandex && !isMotp && (
            <div className="grid grid-cols-2 gap-2">
              <div>
                <label className="text-tiny text-slate-400 mb-1 block">{t("otp.algorithm")}</label>
                <select
                  value={form.algorithm}
                  onChange={(e) => set("algorithm", e.target.value)}
                  disabled={busy} className="ui-input w-full h-9 px-2.5"
                >
                  <option value="SHA1">SHA1</option>
                  <option value="SHA256">SHA256</option>
                  <option value="SHA512">SHA512</option>
                </select>
              </div>
              <div>
                <label className="text-tiny text-slate-400 mb-1 block">{t("otp.digits")}</label>
                <select
                  value={form.digits}
                  onChange={(e) => set("digits", Number(e.target.value))}
                  disabled={busy} className="ui-input w-full h-9 px-2.5"
                >
                  {[5, 6, 7, 8].map((d) => (
                    <option key={d} value={d}>{t("otp.digitsCount", { count: d })}</option>
                  ))}
                </select>
              </div>
            </div>
          )}

          {isHotp ? (
            <div>
              <label className="text-tiny text-slate-400 mb-1 block">{t("otp.counter")}</label>
              <input
                type="number"
                value={form.counter}
                onChange={(e) => set("counter", Number(e.target.value))}
                disabled={busy} className="ui-input w-full h-9 px-2.5"
              />
            </div>
          ) : !isSteam && !isYandex ? (
            <div>
              <label className="text-tiny text-slate-400 mb-1 block">{t("otp.period")}</label>
              <input
                type="number"
                value={form.period}
                onChange={(e) => set("period", Number(e.target.value))}
                disabled={busy} className="ui-input w-full h-9 px-2.5"
              />
            </div>
          ) : null}

          {(isMotp || isYandex) && (
            <div>
              <label className="text-tiny text-slate-400 mb-1 block">PIN</label>
              <input
                value={form.pin}
                onChange={(e) => set("pin", e.target.value)}
                placeholder={isMotp ? t("otp.pinPh") : t("otp.pinPhShort")}
                disabled={busy} className="ui-input w-full h-9 px-2.5"
              />
            </div>
          )}
        </div>

        <div className="flex justify-end gap-2 mt-4">
          <button onClick={onClose} disabled={busy} className={btnSecondary}>{t("otp.cancel")}</button>
          <button
            onClick={submit}
            disabled={busy}
            className={btnPrimary}
          >
            {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />} {t("otp.save")}
          </button>
        </div>
      </div>
    </div>
  );
}
