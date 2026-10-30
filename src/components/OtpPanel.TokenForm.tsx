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
      <div className="w-[460px] max-w-[95vw] max-h-[90vh] overflow-y-auto rounded-panel border border-white/10 bg-slate-900/95 shadow-2xl p-5" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-sm font-bold text-white">{token ? t("otp.editToken") : t("otp.addToken")}</h3>
          <button onClick={onClose} className="p-1 rounded hover:bg-white/10 text-slate-400 cursor-pointer">
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
              className="w-full glass-input px-3 py-2 text-body bg-black/30 border border-white/10 rounded-ctl focus:outline-none"
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
                className="w-full glass-input px-3 py-2 text-body bg-black/30 border border-white/10 rounded-ctl focus:outline-none"
              />
            </div>
            <div>
              <label className="text-tiny text-slate-400 mb-1 block">{t("otp.account")}</label>
              <input
                value={form.account}
                onChange={(e) => set("account", e.target.value)}
                placeholder={t("otp.accountPh")}
                className="w-full glass-input px-3 py-2 text-body bg-black/30 border border-white/10 rounded-ctl focus:outline-none"
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
              className="w-full glass-input px-3 py-2 text-body bg-black/30 border border-white/10 rounded-ctl focus:outline-none"
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
                className="w-full glass-input px-3 py-2 text-body bg-black/30 border border-white/10 rounded-ctl focus:outline-none"
              />
            </div>
            <div>
              <label className="text-tiny text-slate-400 mb-1 block">{t("otp.iconChar")}</label>
              <input
                value={form.customIcon}
                onChange={(e) => set("customIcon", e.target.value)}
                placeholder={t("otp.iconPh")}
                className="w-full glass-input px-3 py-2 text-body bg-black/30 border border-white/10 rounded-ctl focus:outline-none"
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
                  className="w-full glass-input px-3 py-2 text-body bg-black/30 border border-white/10 rounded-ctl focus:outline-none"
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
                  className="w-full glass-input px-3 py-2 text-body bg-black/30 border border-white/10 rounded-ctl focus:outline-none"
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
                className="w-full glass-input px-3 py-2 text-body bg-black/30 border border-white/10 rounded-ctl focus:outline-none"
              />
            </div>
          ) : !isSteam && !isYandex ? (
            <div>
              <label className="text-tiny text-slate-400 mb-1 block">{t("otp.period")}</label>
              <input
                type="number"
                value={form.period}
                onChange={(e) => set("period", Number(e.target.value))}
                className="w-full glass-input px-3 py-2 text-body bg-black/30 border border-white/10 rounded-ctl focus:outline-none"
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
                className="w-full glass-input px-3 py-2 text-body bg-black/30 border border-white/10 rounded-ctl focus:outline-none"
              />
            </div>
          )}
        </div>

        <div className="flex justify-end gap-2 mt-4">
          <button onClick={onClose} className="px-3 py-1.5 rounded-ctl text-caption text-slate-400 hover:bg-white/5 cursor-pointer">{t("otp.cancel")}</button>
          <button
            onClick={submit}
            disabled={busy}
            className="px-3 py-1.5 rounded-ctl text-caption bg-[var(--module-accent)] text-white font-semibold cursor-pointer hover:opacity-85 disabled:opacity-50 flex items-center gap-1"
          >
            {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />} {t("otp.save")}
          </button>
        </div>
      </div>
    </div>
  );
}
