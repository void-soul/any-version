// DNS 页 —— 1:1 复刻 clash-party src/renderer/src/pages/dns.tsx
// （开关 / 增强模式 fake-ip·redir-host·normal / fake-ip 范围+过滤模式+过滤列表 /
//   IPv6 / respect-rules / 4 组 nameserver 列表 / nameserver-policy /
//   系统 hosts / 自定义 hosts / fallback + fallback-filter）
import React, { useEffect, useState } from "react";
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
            value === k ? "bg-emerald-600 text-white" : "text-slate-400 hover:text-slate-200"
          }`}>
          {t}
        </button>
      ))}
    </div>
  );
}

export default function DnsPanel() {
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
      setMsg(controlDns ? "已保存并生效" : "已保存（未开启 DNS 接管，仅保存）");
    } catch (e: any) {
      setMsg(`保存失败: ${e}`);
    }
  };

  return (
    <div className="space-y-3">
      <div className={`${cardCls} p-4`}>
        <div className="flex items-center justify-between mb-1">
          <h3 className="text-sm font-bold text-white">DNS 设置</h3>
          <div className="flex items-center gap-2">
            {msg && <span className="text-[11px] text-slate-400">{msg}</span>}
            {changed && <button className={btnPrimary} onClick={onSave}>{controlDns ? "保存" : "仅保存"}</button>}
          </div>
        </div>

        <SettingItem title="启用 DNS 模块">
          <Toggle v={values.enable} onChange={(v) => setValues({ ...values, enable: v })} />
        </SettingItem>
        <SettingItem title="增强模式">
          <SegTabs
            options={[["fake-ip", "虚假 IP"], ["redir-host", "真实 IP"], ["normal", "取消映射"]]}
            value={values.enhancedMode}
            onChange={(k) => setValues({ ...values, enhancedMode: k })}
          />
        </SettingItem>

        {values.enhancedMode === "fake-ip" && (
          <>
            <SettingItem title="虚假 IP 范围">
              <input className={`${inputCls} !w-64`} placeholder="198.18.0.1/16" value={values.fakeIPRange}
                onChange={(e) => setValues({ ...values, fakeIPRange: e.target.value })} />
            </SettingItem>
            <SettingItem title="过滤模式">
              <SegTabs
                options={[["blacklist", "黑名单"], ["whitelist", "白名单"], ["rule", "规则"]]}
                value={values.fakeIPFilterMode}
                onChange={(k) => setValues({ ...values, fakeIPFilterMode: k })}
              />
            </SettingItem>
            <div className="py-2 border-b border-white/5">
              <h4 className="text-[12px] text-slate-300 font-semibold">真实 IP 回应</h4>
              {renderListInputs("fakeIPFilter", values.fakeIPFilterMode === "rule" ? "例: RULE-SET,cn" : "例: +.lan")}
            </div>
          </>
        )}

        <SettingItem title="IPv6">
          <Toggle v={values.ipv6} onChange={(v) => setValues({ ...values, ipv6: v })} />
        </SettingItem>
        <SettingItem title="连接遵守规则">
          <Toggle v={values.respectRules} onChange={(v) => setValues({ ...values, respectRules: v })} />
        </SettingItem>

        <div className="py-2 border-b border-white/5">
          <h4 className="text-[12px] text-slate-300 font-semibold">默认域名解析服务器 (default-nameserver)</h4>
          {renderListInputs("defaultNameserver", "例: tls://223.5.5.5")}
        </div>
        <div className="py-2 border-b border-white/5">
          <h4 className="text-[12px] text-slate-300 font-semibold">代理节点域名解析服务器 (proxy-server-nameserver)</h4>
          {renderListInputs("proxyServerNameserver", "例: https://doh.pub/dns-query")}
        </div>
        <div className="py-2 border-b border-white/5">
          <h4 className="text-[12px] text-slate-300 font-semibold">域名解析服务器 (nameserver)</h4>
          {renderListInputs("nameserver", "例: https://doh.pub/dns-query")}
        </div>
        <div className="py-2 border-b border-white/5">
          <h4 className="text-[12px] text-slate-300 font-semibold">直连域名解析服务器 (direct-nameserver)</h4>
          {renderListInputs("directNameserver", "例: system")}
        </div>

        <SettingItem title="覆盖 DNS 策略">
          <Toggle v={values.useNameserverPolicy} onChange={(v) => setValues({ ...values, useNameserverPolicy: v })} />
        </SettingItem>
        {values.useNameserverPolicy && (
          <div className="py-2 border-b border-white/5">
            <h4 className="text-[12px] text-slate-300 font-semibold mb-2">DNS 策略列表</h4>
            {renderKvInputs("nameserverPolicy", "域名（例: +.example.com）", "服务器，多个逗号分隔")}
          </div>
        )}

        <SettingItem title="使用系统 Hosts">
          <Toggle v={values.useSystemHosts} onChange={(v) => setValues({ ...values, useSystemHosts: v })} />
        </SettingItem>
        <SettingItem title="自定义 Hosts">
          <Toggle v={values.useHosts} onChange={(v) => setValues({ ...values, useHosts: v })} />
        </SettingItem>
        {values.useHosts && (
          <div className="py-2 border-b border-white/5">
            <h4 className="text-[12px] text-slate-300 font-semibold mb-2">Hosts 列表</h4>
            {renderKvInputs("hosts", "域名", "IP/域名，多个逗号分隔")}
          </div>
        )}

        <div className="py-2">
          <h4 className="text-[12px] text-slate-300 font-semibold">后备域名解析服务器 (fallback)</h4>
          {renderListInputs("fallback", "例: tls://8.8.4.4")}
        </div>
      </div>

      {/* fallback-filter 卡片（复刻第二个 SettingCard） */}
      <div className={`${cardCls} p-4`}>
        <h3 className="text-sm font-bold text-white mb-1">后备过滤 (fallback-filter)</h3>
        <SettingItem title="GeoIP 过滤">
          <Toggle v={!!values.fallbackGeoip} onChange={(v) => setValues({ ...values, fallbackGeoip: v })} />
        </SettingItem>
        <SettingItem title="GeoIP 代码">
          <input className={`${inputCls} !w-28`} placeholder="CN" value={values.fallbackGeoipCode}
            onChange={(e) => setValues({ ...values, fallbackGeoipCode: e.target.value })} />
        </SettingItem>
        <div className="py-2 border-b border-white/5">
          <h4 className="text-[12px] text-slate-300 font-semibold">IP 段过滤 (ipcidr)</h4>
          {renderListInputs("fallbackIpcidr", "例: 240.0.0.0/4")}
        </div>
        <div className="py-2">
          <h4 className="text-[12px] text-slate-300 font-semibold">域名过滤 (domain)</h4>
          {renderListInputs("fallbackDomain", "例: +.google.com")}
        </div>
      </div>
    </div>
  );
}
