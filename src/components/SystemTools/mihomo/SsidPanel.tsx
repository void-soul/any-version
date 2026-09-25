// SSID 感知：按当前 Wi-Fi 名称自动切换订阅配置（抄 clash-party sys/ssid.ts）
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Wifi, RefreshCw, Plus, Trash2, Check } from "lucide-react";
import { mihomoApi } from "../mihomoApi";
import { Toggle } from "./ui";

type SsidRule = { ssid: string; profile_id: string };
type ProfileItem = { id: string; name: string };

const cardCls = "rounded-xl border border-white/5 bg-slate-900/30 p-3 space-y-3";
const inputCls =
  "px-2 py-1 rounded-md bg-slate-900 border border-white/10 text-[11px] text-slate-200 placeholder-slate-600 focus:outline-none focus:border-[var(--module-accent)]/50";
const btnSec =
  "px-2 py-1 rounded-md bg-white/5 hover:bg-white/10 text-[10px] text-slate-300 cursor-pointer flex items-center gap-1 disabled:opacity-50";

export default function SsidPanel() {
  const { t } = useTranslation();
  const [profiles, setProfiles] = useState<ProfileItem[]>([]);
  const [ssid, setSsid] = useState<string | null>(null);
  const [rules, setRules] = useState<SsidRule[]>([]);
  const [enabled, setEnabled] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [c, p] = await Promise.all([
        mihomoApi.getAppConfig().catch(() => null),
        mihomoApi.getProfileConfig().catch(() => null),
      ]);
      setProfiles((p?.items || []).map((i: ProfileItem) => ({ id: i.id, name: i.name })));
      setEnabled(!!c?.ssid_switch_enabled);
      setRules(c?.ssid_rules || []);
    } catch { /* ignore */ }
    try {
      const s = await invoke<string | null>("mihomo_get_ssid");
      setSsid(s ?? null);
    } catch {
      setSsid(null);
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const save = async (nextEnabled: boolean, nextRules: SsidRule[]) => {
    setBusy(true);
    setMsg(null);
    try {
      await invoke("mihomo_set_ssid_rules", { enabled: nextEnabled, rules: nextRules });
      setEnabled(nextEnabled);
      setRules(nextRules);
      setMsg({ ok: true, text: t("ssid.saved") });
    } catch (e: any) {
      setMsg({ ok: false, text: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const addRule = () => {
    if (!profiles.length) return;
    setRules((r) => [...r, { ssid: ssid || "", profile_id: profiles[0].id }]);
  };

  return (
    <div className="space-y-3">
      <div className={cardCls}>
        <div className="flex items-center gap-2">
          <Wifi className="w-4 h-4 text-[var(--module-accent)]" />
          <span className="text-xs font-semibold text-slate-200">{t("ssid.title")}</span>
          <div className="flex-1" />
          <button className={btnSec} onClick={() => void refresh()} disabled={busy}>
            <RefreshCw className="w-3 h-3" /> {t("ssid.refresh")}
          </button>
        </div>
        <div className="text-[11px] text-slate-400">
          {t("ssid.current")}:{" "}
          <span className="font-mono text-slate-200">{ssid || t("ssid.notConnected")}</span>
        </div>
        <div className="text-[10px] text-slate-500">{t("ssid.hint")}</div>
        <Toggle
          label={t("ssid.enable")}
          v={enabled}
          onChange={(v) => void save(v, rules)}
        />
      </div>

      <div className={cardCls}>
        <div className="flex items-center gap-2">
          <span className="text-xs font-semibold text-slate-200">{t("ssid.rules")}</span>
          <div className="flex-1" />
          <button className={btnSec} onClick={addRule} disabled={!profiles.length}>
            <Plus className="w-3 h-3" /> {t("ssid.add")}
          </button>
          <button className={btnSec} onClick={() => void save(enabled, rules)} disabled={busy}>
            <Check className="w-3 h-3" /> {t("ssid.save")}
          </button>
        </div>

        {rules.length === 0 && (
          <div className="text-[11px] text-slate-500">{t("ssid.noRules")}</div>
        )}

        {rules.map((r, i) => (
          <div key={i} className="flex items-center gap-2">
            <input
              className={`${inputCls} flex-1 min-w-0`}
              value={r.ssid}
              placeholder={t("ssid.ssidPh")}
              onChange={(e) => setRules((rs) => rs.map((x, j) => (j === i ? { ...x, ssid: e.target.value } : x)))}
            />
            <span className="text-slate-600 text-[11px]">→</span>
            <select
              className={`${inputCls} max-w-[180px]`}
              value={r.profile_id}
              onChange={(e) => setRules((rs) => rs.map((x, j) => (j === i ? { ...x, profile_id: e.target.value } : x)))}
            >
              {profiles.map((p) => (
                <option key={p.id} value={p.id}>{p.name}</option>
              ))}
            </select>
            <button
              className="p-1 rounded-md hover:bg-white/10 text-slate-400 hover:text-rose-300 cursor-pointer"
              title={t("ssid.remove")}
              onClick={() => setRules((rs) => rs.filter((_, j) => j !== i))}
            >
              <Trash2 className="w-3.5 h-3.5" />
            </button>
          </div>
        ))}
      </div>

      {msg && (
        <div className={`text-[11px] ${msg.ok ? "text-emerald-400" : "text-rose-300"}`}>{msg.text}</div>
      )}
    </div>
  );
}
