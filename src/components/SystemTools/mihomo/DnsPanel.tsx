// DNS 页 —— 1:1 复刻 clash-party src/renderer/src/pages/dns.tsx
// （开关 / 增强模式 fake-ip·redir-host·normal / fake-ip 范围+过滤模式+过滤列表 /
//   IPv6 / respect-rules / 4 组 nameserver 列表 / nameserver-policy /
//   系统 hosts / 自定义 hosts / fallback + fallback-filter）
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Trash2 } from "lucide-react";
import { mihomoApi } from "../mihomoApi";
import { cardCls, SettingItem, Toggle, btnSec, btnPrimary, inputCls } from "./ui";

// 复刻 DEFAULT_MIHOMO_DNS_CONFIG
const DEFAULT_DNS = {
  enable: true,
  ipv6: false,
  enhancedMode: "fake-ip",
  fakeIPRange: "198.18.0.1/16",
  fakeIPFilter: ["*", "+.lan", "+.local", "time.*.com", "ntp.*.com", "+.market.xiaomi.com"],
  fakeIPFilterMode: "blacklist",
  useHosts: false,
  useSystemHosts: false,
  respectRules: false,
  defaultNameserver: ["tls://223.5.5.5"],
  nameserver: ["https://doh.pub/dns-query", "https://dns.alidns.com/dns-query"],
  proxyServerNameserver: ["https://doh.pub/dns-query", "https://dns.alidns.com/dns-query"],
  directNameserver: [] as string[],
  fallback: [] as string[],
  fallbackGeoip: true as boolean,
  fallbackGeoipCode: "CN",
  fallbackIpcidr: ["240.0.0.0/4", "0.0.0.0/32"],
  fallbackDomain: ["+.google.com", "+.facebook.com", "+.youtube.com"],
};

type KV = { domain: string; value: any };

function SegTabs({ options, value, onChange }: { options: [string, string][]; value: string; onChange: (k: string) => void }) {
  return (
    <div className="flex rounded-lg bg-white/5 border border-white/10 overflow-hidden">
      {options.map(([k, t]) => (
        <button key={k} onClick={() => onChange(k)}
          className={`px-3 py-1.5 text-[11px] font-semibold cursor-pointer transition-all ${
            value === k ? "bg-[var(--module-accent)] text-white" : "text-slate-400 hover:text-slate-200"
          }`}>
          {t}
        </button>
      ))}
    </div>
  );
}

export default function DnsPanel() {
  const { t } = useTranslation();
  const [values, originSetValues] = useState<any>({
    ...DEFAULT_DNS,
    useNameserverPolicy: false,
    nameserverPolicy: [] as KV[],
    hosts: [] as KV[],
  });
  const [changed, setChanged] = useState(false);
  const [controlDns, setControlDns] = useState(false);
  const [msg, setMsg] = useState("");
  const setValues = (v: any) => { originSetValues(v); setChanged(true); };

  useEffect(() => {
    (async () => {
      try {
        const [app, c]: any[] = await Promise.all([mihomoApi.getAppConfig(), mihomoApi.getControledConfig()]);
        const dns = c?.dns || {};
        const ff = dns["fallback-filter"] || {};
        setControlDns(app?.control_dns ?? app?.controlDns ?? false);
        originSetValues({
          enable: dns.enable ?? DEFAULT_DNS.enable,
          ipv6: dns.ipv6 ?? DEFAULT_DNS.ipv6,
          enhancedMode: dns["enhanced-mode"] ?? DEFAULT_DNS.enhancedMode,
          fakeIPRange: dns["fake-ip-range"] ?? DEFAULT_DNS.fakeIPRange,
          fakeIPFilter: dns["fake-ip-filter"] ?? DEFAULT_DNS.fakeIPFilter,
          fakeIPFilterMode: dns["fake-ip-filter-mode"] ?? "blacklist",
          useHosts: dns["use-hosts"] ?? false,
          useSystemHosts: dns["use-system-hosts"] ?? false,
          respectRules: dns["respect-rules"] ?? false,
          defaultNameserver: dns["default-nameserver"] ?? DEFAULT_DNS.defaultNameserver,
          nameserver: dns.nameserver ?? DEFAULT_DNS.nameserver,
          proxyServerNameserver: dns["proxy-server-nameserver"] ?? DEFAULT_DNS.proxyServerNameserver,
          directNameserver: dns["direct-nameserver"] ?? [],
          fallback: dns.fallback ?? [],
          fallbackGeoip: ff.geoip ?? true,
          fallbackGeoipCode: ff["geoip-code"] ?? "CN",
          fallbackIpcidr: ff.ipcidr ?? DEFAULT_DNS.fallbackIpcidr,
          fallbackDomain: ff.domain ?? DEFAULT_DNS.fallbackDomain,
          useNameserverPolicy: app?.useNameserverPolicy ?? false,
          nameserverPolicy: Object.entries(app?.nameserverPolicy || {}).map(([domain, value]) => ({ domain, value })),
          hosts: Object.entries(c?.hosts || {}).map(([domain, value]) => ({ domain, value })),
        });
      } catch {}
    })();
  }, []);

  // 复刻 handleListChange
  const handleListChange = (type: string, value: string, index: number) => {
    const list = [...values[type]];
    if (value.trim()) {
      if (index < list.length) list[index] = value; else list.push(value);
    } else list.splice(index, 1);
    setValues({ ...values, [type]: list });
  };

  const renderListInputs = (type: string, placeholder: string) => {
    const items: string[] = values[type];
    const showNewLine = items.every((i) => (typeof i === "string" ? i.trim() !== "" : true));
    return [...items, ...(showNewLine ? [""] : [])].map((item, index) => (
      <div key={index} className="mt-1.5 flex gap-2">
        <input className={inputCls} placeholder={placeholder} value={item as string}
          onChange={(e) => handleListChange(type, e.target.value, index)} />
        {index < items.length && (
          <button className={btnSec} onClick={() => handleListChange(type, "", index)}>
            <Trash2 className="w-3.5 h-3.5 text-amber-300" />
          </button>
        )}
      </div>
    ));
  };

  // 复刻 handleSubkeyChange（hosts 值恒为数组；policy 多值逗号分隔）
  const handleSubkeyChange = (type: string, domain: string, value: string, index: number) => {
    const list = [...values[type]];
    const parts = value.split(",").map((s) => s.trim()).filter(Boolean);
    const processed = type === "hosts" ? parts : parts.length > 1 ? parts : value.trim();
    if (domain || parts.length > 0) list[index] = { domain: domain.trim(), value: processed };
    else list.splice(index, 1);
    setValues({ ...values, [type]: list });
  };

  const renderKvInputs = (type: "nameserverPolicy" | "hosts", dph: string, vph: string) => (
    <>
      {[...values[type], { domain: "", value: "" }].map(({ domain, value }: KV, index: number) => (
        <div key={index} className="flex mb-1.5 items-center gap-2">
          <input className={`${inputCls} !w-2/5`} placeholder={dph} value={domain}
            onChange={(e) => handleSubkeyChange(type, e.target.value, Array.isArray(value) ? value.join(",") : value, index)} />
          <span className="text-slate-500">:</span>
          <input className={inputCls} placeholder={vph} value={Array.isArray(value) ? value.join(",") : value}
            onChange={(e) => handleSubkeyChange(type, domain, e.target.value, index)} />
          {index < values[type].length && (
            <button className={btnSec} onClick={() => handleSubkeyChange(type, "", "", index)}>
              <Trash2 className="w-3.5 h-3.5 text-amber-300" />
            </button>
          )}
        </div>
      ))}
    </>
  );

  // 复刻 onSave
  const onSave = async () => {
    setChanged(false);
    const nsPolicy = values.useNameserverPolicy
      ? Object.fromEntries(
          values.nameserverPolicy.flatMap(({ domain, value }: KV) => {
            const key = domain.trim();
            const nv = Array.isArray(value) ? value.map((s: string) => s.trim()).filter(Boolean) : String(value).trim();
            if (!key || (Array.isArray(nv) ? nv.length === 0 : !nv)) return [];
            return [[key, nv]];
          })
        )
      : {};
    const dnsConfig: any = {
      enable: values.enable,
      ipv6: values.ipv6,
      "fake-ip-range": values.fakeIPRange,
      "fake-ip-filter": values.fakeIPFilter,
      "fake-ip-filter-mode": values.fakeIPFilterMode,
      "enhanced-mode": values.enhancedMode,
      "use-hosts": values.useHosts,
      "use-system-hosts": values.useSystemHosts,
      "respect-rules": values.respectRules,
      "default-nameserver": values.defaultNameserver,
      nameserver: values.nameserver,
      "proxy-server-nameserver": values.proxyServerNameserver,
      "direct-nameserver": values.directNameserver,
      fallback: values.fallback,
      "fallback-filter": {
        ...(values.fallbackGeoip ? { geoip: values.fallbackGeoip } : {}),
        "geoip-code": values.fallbackGeoipCode,
        ipcidr: values.fallbackIpcidr,
        domain: values.fallbackDomain,
      },
    };
    if (values.useNameserverPolicy) dnsConfig["nameserver-policy"] = nsPolicy;
    const patch: any = { dns: dnsConfig };
    if (values.useHosts) {
      patch.hosts = Object.fromEntries(values.hosts.map(({ domain, value }: KV) => [domain, value]));
    }
    try {
      await mihomoApi.patchAppConfig({ nameserverPolicy: nsPolicy, useNameserverPolicy: values.useNameserverPolicy });
      await mihomoApi.patchControledConfig(patch);
      setMsg(controlDns ? t("mihomo.dnsSavedEffective") : t("mihomo.dnsSavedOnly"));
    } catch (e: any) {
      setMsg(t("mihomo.dnsSaveFailed", { err: String(e) }));
    }
  };

  return (
    <div className="space-y-3">
      <div className={`${cardCls} p-4`}>
        <div className="flex items-center justify-between mb-1">
          <h3 className="text-sm font-bold text-white">{t("mihomo.dnsTitle")}</h3>
          <div className="flex items-center gap-2">
            {msg && <span className="text-[11px] text-slate-400">{msg}</span>}
            {changed && <button className={btnPrimary} onClick={onSave}>{controlDns ? t("mihomo.dnsSave") : t("mihomo.dnsSaveOnly")}</button>}
          </div>
        </div>

        <SettingItem title={t("mihomo.dnsEnable")}>
          <Toggle v={values.enable} onChange={(v) => setValues({ ...values, enable: v })} />
        </SettingItem>
        <SettingItem title={t("mihomo.dnsEnhancedMode")}>
          <SegTabs
            options={[["fake-ip", t("mihomo.dnsFakeIp")], ["redir-host", t("mihomo.dnsRedirHost")], ["normal", t("mihomo.dnsNormal")]]}
            value={values.enhancedMode}
            onChange={(k) => setValues({ ...values, enhancedMode: k })}
          />
        </SettingItem>

        {values.enhancedMode === "fake-ip" && (
          <>
            <SettingItem title={t("mihomo.dnsFakeIpRange")}>
              <input className={`${inputCls} !w-64`} placeholder="198.18.0.1/16" value={values.fakeIPRange}
                onChange={(e) => setValues({ ...values, fakeIPRange: e.target.value })} />
            </SettingItem>
            <SettingItem title={t("mihomo.dnsFilterMode")}>
              <SegTabs
                options={[["blacklist", t("mihomo.dnsBlacklist")], ["whitelist", t("mihomo.dnsWhitelist")], ["rule", t("mihomo.dnsRule")]]}
                value={values.fakeIPFilterMode}
                onChange={(k) => setValues({ ...values, fakeIPFilterMode: k })}
              />
            </SettingItem>
            <div className="py-2 border-b border-white/5">
              <h4 className="text-[12px] text-slate-300 font-semibold">{t("mihomo.dnsRealIpRespond")}</h4>
              {renderListInputs("fakeIPFilter", values.fakeIPFilterMode === "rule" ? t("mihomo.dnsExampleRuleSet") : t("mihomo.dnsExampleLan"))}
            </div>
          </>
        )}

        <SettingItem title={t("mihomo.dnsIpv6")}>
          <Toggle v={values.ipv6} onChange={(v) => setValues({ ...values, ipv6: v })} />
        </SettingItem>
        <SettingItem title={t("mihomo.dnsRespectRules")}>
          <Toggle v={values.respectRules} onChange={(v) => setValues({ ...values, respectRules: v })} />
        </SettingItem>

        <div className="py-2 border-b border-white/5">
          <h4 className="text-[12px] text-slate-300 font-semibold">{t("mihomo.dnsDefaultNs")}</h4>
          {renderListInputs("defaultNameserver", t("mihomo.dnsExampleTls"))}
        </div>
        <div className="py-2 border-b border-white/5">
          <h4 className="text-[12px] text-slate-300 font-semibold">{t("mihomo.dnsProxyNs")}</h4>
          {renderListInputs("proxyServerNameserver", t("mihomo.dnsExampleDoh"))}
        </div>
        <div className="py-2 border-b border-white/5">
          <h4 className="text-[12px] text-slate-300 font-semibold">{t("mihomo.dnsNs")}</h4>
          {renderListInputs("nameserver", t("mihomo.dnsExampleDoh"))}
        </div>
        <div className="py-2 border-b border-white/5">
          <h4 className="text-[12px] text-slate-300 font-semibold">{t("mihomo.dnsDirectNs")}</h4>
          {renderListInputs("directNameserver", t("mihomo.dnsExampleSystem"))}
        </div>

        <SettingItem title={t("mihomo.dnsUsePolicy")}>
          <Toggle v={values.useNameserverPolicy} onChange={(v) => setValues({ ...values, useNameserverPolicy: v })} />
        </SettingItem>
        {values.useNameserverPolicy && (
          <div className="py-2 border-b border-white/5">
            <h4 className="text-[12px] text-slate-300 font-semibold mb-2">{t("mihomo.dnsPolicyList")}</h4>
            {renderKvInputs("nameserverPolicy", t("mihomo.dnsPolicyDomainPh"), t("mihomo.dnsPolicyValuePh"))}
          </div>
        )}

        <SettingItem title={t("mihomo.dnsUseSystemHosts")}>
          <Toggle v={values.useSystemHosts} onChange={(v) => setValues({ ...values, useSystemHosts: v })} />
        </SettingItem>
        <SettingItem title={t("mihomo.dnsUseCustomHosts")}>
          <Toggle v={values.useHosts} onChange={(v) => setValues({ ...values, useHosts: v })} />
        </SettingItem>
        {values.useHosts && (
          <div className="py-2 border-b border-white/5">
            <h4 className="text-[12px] text-slate-300 font-semibold mb-2">{t("mihomo.dnsHostsList")}</h4>
            {renderKvInputs("hosts", t("mihomo.dnsDomain"), t("mihomo.dnsHostsValuePh"))}
          </div>
        )}

        <div className="py-2">
          <h4 className="text-[12px] text-slate-300 font-semibold">{t("mihomo.dnsFallback")}</h4>
          {renderListInputs("fallback", t("mihomo.dnsExampleTls8844"))}
        </div>
      </div>

      {/* fallback-filter 卡片（复刻第二个 SettingCard） */}
      <div className={`${cardCls} p-4`}>
        <h3 className="text-sm font-bold text-white mb-1">{t("mihomo.dnsFallbackFilter")}</h3>
        <SettingItem title={t("mihomo.dnsGeoipFilter")}>
          <Toggle v={!!values.fallbackGeoip} onChange={(v) => setValues({ ...values, fallbackGeoip: v })} />
        </SettingItem>
        <SettingItem title={t("mihomo.dnsGeoipCode")}>
          <input className={`${inputCls} !w-28`} placeholder="CN" value={values.fallbackGeoipCode}
            onChange={(e) => setValues({ ...values, fallbackGeoipCode: e.target.value })} />
        </SettingItem>
        <div className="py-2 border-b border-white/5">
          <h4 className="text-[12px] text-slate-300 font-semibold">{t("mihomo.dnsIpcidr")}</h4>
          {renderListInputs("fallbackIpcidr", t("mihomo.dnsExampleIpcidr"))}
        </div>
        <div className="py-2">
          <h4 className="text-[12px] text-slate-300 font-semibold">{t("mihomo.dnsDomainFilter")}</h4>
          {renderListInputs("fallbackDomain", t("mihomo.dnsExampleDomain"))}
        </div>
      </div>
    </div>
  );
}
