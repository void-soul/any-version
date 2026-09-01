// 规则页 —— 1:1 复刻 clash-party src/renderer/src/pages/rules.tsx
// （过滤持久化，匹配 payload/type/proxy；含规则集 provider 更新入口）
import { useEffect, useMemo, useState } from "react";
import { Search, RefreshCw, Plus, Trash2, PencilLine } from "lucide-react";
import { IMihomoRule, getRules, ruleProviders, updateRuleProvider } from "./ctrl";
import { cardCls, tagCls, btnSec, btnPrimary, inputCls, labelCls, Modal } from "./ui";
import { mihomoApi } from "../mihomoApi";
import { useTranslation } from "react-i18next";

const RULES_FILTER_KEY = "mihomo-rules-filter";

// 规则类型（对齐 clash-party edit-rules-modal 的类型集合）
const RULE_TYPES = [
  "DOMAIN", "DOMAIN-SUFFIX", "DOMAIN-KEYWORD", "DOMAIN-REGEX", "DOMAIN-WILDCARD",
  "GEOSITE", "IP-CIDR", "IP-CIDR6", "IP-SUFFIX", "IP-ASN", "GEOIP",
  "SRC-GEOIP", "SRC-IP-ASN", "SRC-IP-CIDR", "SRC-IP-SUFFIX",
  "DST-PORT", "SRC-PORT", "IN-PORT", "IN-TYPE", "IN-USER", "IN-NAME",
  "PROCESS-PATH", "PROCESS-PATH-REGEX", "PROCESS-NAME", "PROCESS-NAME-REGEX",
  "UID", "NETWORK", "DSCP", "RULE-SET", "AND", "OR", "NOT", "SUB-RULE", "MATCH",
];
// 无需 payload 的规则类型
const NO_PAYLOAD_TYPES = ["MATCH"];
// 支持 no-resolve 的规则类型
const NO_RESOLVE_TYPES = [
  "GEOIP", "IP-ASN", "IP-CIDR", "IP-CIDR6", "IP-SUFFIX", "RULE-SET",
];

type RuleOverride = { prepend: string[]; append: string[]; delete: string[] };

/** 规则覆写编辑器（对齐 clash-party EditRulesModal：prepend / append / delete + 位置偏移） */
function RuleOverrideEditor({
  profileId,
  proxies,
  onClose,
}: {
  profileId: string;
  proxies: string[];
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [data, setData] = useState<RuleOverride>({ prepend: [], append: [], delete: [] });
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState("");
  const [target, setTarget] = useState<"prepend" | "append">("prepend");
  const [type, setType] = useState("DOMAIN-SUFFIX");
  const [payload, setPayload] = useState("");
  const [proxy, setProxy] = useState("DIRECT");
  const [noResolve, setNoResolve] = useState(false);
  const [offset, setOffset] = useState("");

  useEffect(() => {
    (async () => {
      try {
        const d = await mihomoApi.getRuleOverride(profileId);
        setData({
          prepend: (d.prepend || []).map(String),
          append: (d.append || []).map(String),
          delete: (d.delete || []).map(String),
        });
      } catch (e: any) {
        setErr(String(e));
      } finally {
        setLoading(false);
      }
    })();
  }, [profileId]);

  const add = () => {
    const needPayload = !NO_PAYLOAD_TYPES.includes(type);
    if (needPayload && !payload.trim()) {
      setErr(t("rules.fillContent"));
      return;
    }
    let rule = needPayload ? `${type},${payload.trim()},${proxy}` : `${type},${proxy}`;
    if (noResolve && NO_RESOLVE_TYPES.includes(type)) rule += ",no-resolve";
    const off = offset.trim();
    if (off && /^\d+$/.test(off)) rule = `${off},${rule}`;
    setErr("");
    setPayload("");
    setOffset("");
    setData((d) => ({ ...d, [target]: [...d[target], rule] }));
  };

  const remove = (key: keyof RuleOverride, idx: number) =>
    setData((d) => ({ ...d, [key]: d[key].filter((_, i) => i !== idx) }));

  const save = async () => {
    setSaving(true);
    setErr("");
    try {
      await mihomoApi.setRuleOverride(profileId, data);
      onClose();
    } catch (e: any) {
      setErr(String(e));
    } finally {
      setSaving(false);
    }
  };

  const List = ({ k, title, hint }: { k: keyof RuleOverride; title: string; hint: string }) => (
    <div>
      <div className="flex items-center justify-between mb-1">
        <span className="text-[11px] text-slate-300 font-semibold">{title}</span>
        <span className="text-[10px] text-slate-500">{hint}</span>
      </div>
      <div className="rounded-xl border border-white/10 bg-white/[0.02] divide-y divide-white/5 max-h-32 overflow-y-auto">
        {data[k].length === 0 && <div className="px-3 py-2 text-[11px] text-slate-500">{t("rules.empty")}</div>}
        {data[k].map((r, i) => (
          <div key={`${r}-${i}`} className="px-3 py-1.5 flex items-center gap-2">
            <span className="flex-1 text-[11px] text-slate-200 font-mono truncate select-text">{r}</span>
            <button className="text-rose-400 hover:text-rose-300 cursor-pointer" onClick={() => remove(k, i)}>
              <Trash2 className="w-3 h-3" />
            </button>
          </div>
        ))}
      </div>
    </div>
  );

  return (
    <Modal
      title={t("rules.overwrite")}
      onClose={onClose}
      wide
      busy={saving}
      busyText={t("rules.saveReloadBusy")}
      footer={
        <>
          <button className={btnSec} disabled={saving} onClick={onClose}>{t("rules.cancel")}</button>
          <button className={btnPrimary} disabled={saving || loading} onClick={save}>
            {saving ? t("rules.saving") : t("rules.saveReload")}
          </button>
        </>
      }
    >
      {loading ? (
        <div className="text-xs text-slate-400 py-6 text-center">{t("rules.loading")}</div>
      ) : (
        <div className="space-y-3">
          <div className="grid grid-cols-12 gap-2 items-end">
            <div className="col-span-2">
              <label className={labelCls}>{t("rules.position")}</label>
              <select className={inputCls} value={target} onChange={(e) => setTarget(e.target.value as any)}>
                <option value="prepend">{t("rules.prepend")}</option>
                <option value="append">{t("rules.append")}</option>
              </select>
            </div>
            <div className="col-span-3">
              <label className={labelCls}>{t("rules.type")}</label>
              <select className={inputCls} value={type} onChange={(e) => setType(e.target.value)}>
                {RULE_TYPES.map((t) => <option key={t} value={t}>{t}</option>)}
              </select>
            </div>
            <div className="col-span-3">
              <label className={labelCls}>{t("rules.content")}</label>
              <input
                className={inputCls}
                value={payload}
                disabled={NO_PAYLOAD_TYPES.includes(type)}
                placeholder={NO_PAYLOAD_TYPES.includes(type) ? t("rules.noPayload") : t("rules.payloadPh")}
                onChange={(e) => setPayload(e.target.value)}
              />
            </div>
            <div className="col-span-2">
              <label className={labelCls}>{t("rules.strategy")}</label>
              <input
                className={inputCls}
                list="mihomo-rule-proxies"
                value={proxy}
                onChange={(e) => setProxy(e.target.value)}
              />
              <datalist id="mihomo-rule-proxies">
                {["DIRECT", "REJECT", "REJECT-DROP", "PASS", ...proxies].map((p) => (
                  <option key={p} value={p} />
                ))}
              </datalist>
            </div>
            <div className="col-span-1">
              <label className={labelCls}>{t("rules.offset")}</label>
              <input className={inputCls} value={offset} placeholder="0" onChange={(e) => setOffset(e.target.value)} />
            </div>
            <div className="col-span-1">
              <button className={`${btnPrimary} w-full h-9`} onClick={add}>
                <Plus className="w-3.5 h-3.5 mx-auto" />
              </button>
            </div>
          </div>
          {NO_RESOLVE_TYPES.includes(type) && (
            <label className="flex items-center gap-1.5 text-[11px] text-slate-400 cursor-pointer">
              <input type="checkbox" checked={noResolve} onChange={(e) => setNoResolve(e.target.checked)} />
              {t("rules.noResolve")}
            </label>
          )}
          {err && <div className="text-[11px] text-rose-400">{err}</div>}
          <List k="prepend" title={t("rules.prependTitle")} hint={t("rules.prependHint")} />
          <List k="append" title={t("rules.appendTitle")} hint={t("rules.appendHint")} />
          <List k="delete" title={t("rules.deleteTitle")} hint={t("rules.deleteHint")} />
          <div className="text-[10px] text-slate-500 leading-relaxed">
            {t("rules.offsetDesc")}
          </div>
        </div>
      )}
    </Modal>
  );
}

export default function RulesPanel({ running }: { running: boolean; onNavigate?: (t: string) => void }) {
  const { t } = useTranslation();
  const [rules, setRules] = useState<IMihomoRule[]>([]);
  const [providers, setProviders] = useState<Record<string, any>>({});
  const [filter, setFilter] = useState(() => localStorage.getItem(RULES_FILTER_KEY) || "");
  const [updating, setUpdating] = useState<Record<string, boolean>>({});
  const [current, setCurrent] = useState("");
  const [editOverride, setEditOverride] = useState(false);

  const refresh = async () => {
    try {
      const [r, p] = await Promise.all([getRules(), ruleProviders()]);
      setRules(r);
      setProviders(p);
    } catch { setRules([]); setProviders({}); }
  };

  useEffect(() => {
    if (!running) return;
    // 切换订阅后核心经 PATCH /configs 异步重载，需稍候再拉取；挂载时做「即时 + 两次延迟」刷新
    refresh();
    const t1 = setTimeout(refresh, 600);
    const t2 = setTimeout(refresh, 1500);
    const t = setInterval(refresh, 10000);
    return () => { clearTimeout(t1); clearTimeout(t2); clearInterval(t); };
  }, [running]);

  // 订阅列表中切换当前订阅后立即刷新规则（核心重载为异步，做延迟兜底）
  useEffect(() => {
    const onChanged = () => {
      refresh();
      setTimeout(refresh, 600);
      setTimeout(refresh, 1500);
    };
    window.addEventListener("mihomo:profile-changed", onChanged);
    return () => window.removeEventListener("mihomo:profile-changed", onChanged);
  }, []);

  useEffect(() => { localStorage.setItem(RULES_FILTER_KEY, filter); }, [filter]);

  // 当前订阅 id（规则覆写以订阅为维度存储）
  useEffect(() => {
    const load = () =>
      mihomoApi.getProfileConfig().then((c: any) => setCurrent(c?.current || "")).catch(() => {});
    load();
    window.addEventListener("mihomo:profile-changed", load);
    return () => window.removeEventListener("mihomo:profile-changed", load);
  }, []);

  const filtered = useMemo(() => {
    const kw = filter.trim().toLowerCase();
    if (!kw) return rules;
    return rules.filter(
      (r) =>
        r.payload.toLowerCase().includes(kw) ||
        r.type.toLowerCase().includes(kw) ||
        r.proxy.toLowerCase().includes(kw)
    );
  }, [rules, filter]);

  const onUpdateProvider = async (name: string) => {
    setUpdating((s) => ({ ...s, [name]: true }));
    try { await updateRuleProvider(name); await refresh(); } finally {
      setUpdating((s) => ({ ...s, [name]: false }));
    }
  };

  if (!running) return <div className={`${cardCls} p-6 text-center text-xs text-slate-400`}>{t("rules.coreNotRunning")}</div>;

  const providerList = Object.entries(providers);

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <div className="relative flex-1">
          <Search className="w-3.5 h-3.5 absolute left-2.5 top-1/2 -translate-y-1/2 text-slate-500" />
          <input
            className="w-full h-8 pl-8 pr-2.5 rounded-lg bg-white/5 border border-white/10 text-xs text-white placeholder-slate-500 focus:outline-none focus:border-emerald-500"
            placeholder={t("rules.filterPh")}
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
        </div>
        <button
          className={`${btnSec} flex items-center gap-1 h-8`}
          disabled={!current}
          title={current ? t("rules.editCurrent") : t("rules.selectSubFirst")}
          onClick={() => setEditOverride(true)}
        >
          <PencilLine className="w-3 h-3" /> {t("rules.overwrite")}
        </button>
      </div>

      {editOverride && current && (
        <RuleOverrideEditor
          profileId={current}
          proxies={Array.from(new Set(rules.map((r) => r.proxy).filter(Boolean)))}
          onClose={() => { setEditOverride(false); setTimeout(refresh, 800); }}
        />
      )}

      {/* 规则集 provider（对齐 clash-party resources 规则集更新能力，就近放规则页） */}
      {providerList.length > 0 && (
        <div className={`${cardCls} p-3`}>
          <div className="text-[11px] text-slate-400 mb-2 font-semibold">{t("rules.ruleSets", { count: providerList.length })}</div>
          <div className="grid grid-cols-2 gap-1.5">
            {providerList.map(([name, p]) => (
              <div key={name} className="flex items-center gap-2 px-2.5 py-1.5 rounded-lg bg-white/[0.03] border border-white/5">
                <div className="min-w-0 flex-1">
                  <div className="text-[11px] text-slate-200 truncate">{name}</div>
                  <div className="text-[10px] text-slate-500">
                    {t("rules.providerCount", { count: p.ruleCount })} · {p.behavior} · {p.vehicleType}
                  </div>
                </div>
                <button className={btnSec} disabled={!!updating[name]} onClick={() => onUpdateProvider(name)}>
                  <RefreshCw className={`w-3 h-3 ${updating[name] ? "animate-spin" : ""}`} />
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="text-[11px] text-slate-400 px-1">{t("rules.rulesCount", { count: filtered.length })}</div>
      <div className={`${cardCls} max-h-[56vh] overflow-y-auto divide-y divide-white/5`}>
        {filtered.slice(0, 2000).map((r, i) => (
          <div key={i} className="px-3 py-2 flex items-center gap-3 hover:bg-white/[0.02]">
            <span className="text-[10px] text-slate-600 w-8 flex-shrink-0 font-mono">{i + 1}</span>
            <div className="min-w-0 flex-1">
              <div className="text-[12px] text-slate-200 truncate select-text">{r.payload || "-"}</div>
              <div className="text-[10px] text-slate-500">
                {r.type}
                {typeof r.size === "number" && r.size >= 0 ? t("rules.ruleCount", { count: r.size }) : ""}
              </div>
            </div>
            <span className={tagCls}>{r.proxy}</span>
          </div>
        ))}
        {filtered.length === 0 && <div className="p-6 text-center text-xs text-slate-400">{t("rules.noMatch")}</div>}
      </div>
    </div>
  );
}
