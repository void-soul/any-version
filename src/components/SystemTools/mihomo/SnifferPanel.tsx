// 嗅探页 —— 1:1 复刻 clash-party src/renderer/src/pages/sniffer.tsx
// （开关 / 覆盖目标 / 强制 DNS 映射 / 纯 IP 解析 / HTTP·TLS·QUIC 端口 /
//   跳过域名 / 强制嗅探域名 / 跳过目标·来源地址）
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Trash2 } from "lucide-react";
import { mihomoApi } from "../mihomoApi";
import { cardCls, SettingItem, Toggle, btnSec, btnPrimary, inputCls } from "./ui";

// 复刻 DEFAULT_MIHOMO_SNIFFER_CONFIG
const DEFAULT_SNIFFER = {
  enable: true,
  parsePureIP: true,
  forceDNSMapping: true,
  overrideDestination: false,
  sniff: {
    HTTP: { ports: [80, 443] as (number | string)[], "override-destination": false },
    TLS: { ports: [443] as (number | string)[] },
    QUIC: { ports: [] as (number | string)[] },
  },
  skipDomain: ["+.push.apple.com"],
  forceDomain: [] as string[],
  skipDstAddress: [
    "91.105.192.0/23", "91.108.4.0/22", "91.108.8.0/21", "91.108.16.0/21", "91.108.56.0/22",
    "95.161.64.0/20", "149.154.160.0/20", "185.76.151.0/24",
    "2001:67c:4e8::/48", "2001:b28:f23c::/47", "2001:b28:f23f::/48", "2a0a:f280:203::/48",
  ],
  skipSrcAddress: [] as string[],
};

export default function SnifferPanel() {
  const { t } = useTranslation();
  const [values, originSetValues] = useState<any>(structuredClone(DEFAULT_SNIFFER));
  const [changed, setChanged] = useState(false);
  const [controlSniff, setControlSniff] = useState(false);
  const [msg, setMsg] = useState("");
  const setValues = (v: any) => { originSetValues(v); setChanged(true); };

  useEffect(() => {
    (async () => {
      try {
        const [app, c]: any[] = await Promise.all([mihomoApi.getAppConfig(), mihomoApi.getControledConfig()]);
        const s = c?.sniffer || {};
        setControlSniff(app?.control_sniff ?? app?.controlSniff ?? false);
        originSetValues({
          enable: s.enable ?? DEFAULT_SNIFFER.enable,
          parsePureIP: s["parse-pure-ip"] ?? DEFAULT_SNIFFER.parsePureIP,
          forceDNSMapping: s["force-dns-mapping"] ?? DEFAULT_SNIFFER.forceDNSMapping,
          overrideDestination: s["override-destination"] ?? false,
          sniff: {
            HTTP: { ports: s.sniff?.HTTP?.ports ?? [80, 443], "override-destination": s.sniff?.HTTP?.["override-destination"] ?? false },
            TLS: { ports: s.sniff?.TLS?.ports ?? [443] },
            QUIC: { ports: s.sniff?.QUIC?.ports ?? [] },
          },
          skipDomain: s["skip-domain"] ?? DEFAULT_SNIFFER.skipDomain,
          forceDomain: s["force-domain"] ?? [],
          skipDstAddress: s["skip-dst-address"] ?? DEFAULT_SNIFFER.skipDstAddress,
          skipSrcAddress: s["skip-src-address"] ?? [],
        });
      } catch {}
    })();
  }, []);

  const handleSniffPortChange = (protocol: "HTTP" | "TLS" | "QUIC", value: string) => {
    setValues({
      ...values,
      sniff: {
        ...values.sniff,
        [protocol]: { ...values.sniff[protocol], ports: value.split(",").map((p) => p.trim()).filter(Boolean) },
      },
    });
  };

  const handleListChange = (type: string, value: string, index: number) => {
    const list = [...values[type]];
    if (value.trim()) {
      if (index < list.length) list[index] = value; else list.push(value);
    } else list.splice(index, 1);
    setValues({ ...values, [type]: list });
  };

  const renderListInputs = (type: string, placeholder: string) => {
    const items: string[] = values[type];
    const showNewLine = items.every((i) => i.trim() !== "");
    return [...items, ...(showNewLine ? [""] : [])].map((item, index) => (
      <div key={index} className="mt-1.5 flex gap-2">
        <input className={inputCls} placeholder={placeholder} value={item}
          onChange={(e) => handleListChange(type, e.target.value, index)} />
        {index < items.length && (
          <button className={btnSec} onClick={() => handleListChange(type, "", index)}>
            <Trash2 className="w-3.5 h-3.5 text-amber-300" />
          </button>
        )}
      </div>
    ));
  };

  const onSave = async () => {
    setChanged(false);
    try {
      await mihomoApi.patchControledConfig({
        sniffer: {
          enable: values.enable,
          "parse-pure-ip": values.parsePureIP,
          "force-dns-mapping": values.forceDNSMapping,
          "override-destination": values.overrideDestination,
          sniff: values.sniff,
          "skip-domain": values.skipDomain,
          "force-domain": values.forceDomain,
          "skip-dst-address": values.skipDstAddress,
          "skip-src-address": values.skipSrcAddress,
        },
      });
      setMsg(controlSniff ? t("mihomo.sniffSavedEffective") : t("mihomo.sniffSavedOnly"));
    } catch (e: any) {
      setMsg(t("mihomo.sniffSaveFailed", { err: String(e) }));
    }
  };

  return (
    <div className={`${cardCls} p-4`}>
      <div className="flex items-center justify-between mb-1">
        <h3 className="text-sm font-bold text-white">{t("mihomo.sniffTitle")}</h3>
        <div className="flex items-center gap-2">
          {msg && <span className="text-[11px] text-slate-400">{msg}</span>}
          {changed && <button className={btnPrimary} onClick={onSave}>{controlSniff ? t("mihomo.sniffSave") : t("mihomo.sniffSaveOnly")}</button>}
        </div>
      </div>

      <SettingItem title={t("mihomo.sniffEnable")}>
        <Toggle v={values.enable} onChange={(v) => setValues({ ...values, enable: v })} />
      </SettingItem>
      <SettingItem title={t("mihomo.sniffOverrideDest")}>
        <Toggle v={values.overrideDestination} onChange={(v) =>
          setValues({
            ...values,
            overrideDestination: v,
            sniff: {
              ...values.sniff,
              HTTP: { ...values.sniff.HTTP, "override-destination": v, ports: values.sniff.HTTP?.ports || [80, 443] },
            },
          })
        } />
      </SettingItem>
      <SettingItem title={t("mihomo.sniffForceDns")}>
        <Toggle v={values.forceDNSMapping} onChange={(v) => setValues({ ...values, forceDNSMapping: v })} />
      </SettingItem>
      <SettingItem title={t("mihomo.sniffParsePureIp")}>
        <Toggle v={values.parsePureIP} onChange={(v) => setValues({ ...values, parsePureIP: v })} />
      </SettingItem>

      <SettingItem title={t("mihomo.sniffHttpPort")}>
        <input className={`${inputCls} !w-64`} placeholder="80,443"
          value={values.sniff.HTTP?.ports.join(",")} onChange={(e) => handleSniffPortChange("HTTP", e.target.value)} />
      </SettingItem>
      <SettingItem title={t("mihomo.sniffTlsPort")}>
        <input className={`${inputCls} !w-64`} placeholder="443"
          value={values.sniff.TLS?.ports.join(",")} onChange={(e) => handleSniffPortChange("TLS", e.target.value)} />
      </SettingItem>
      <SettingItem title={t("mihomo.sniffQuicPort")}>
        <input className={`${inputCls} !w-64`} placeholder="443"
          value={values.sniff.QUIC?.ports.join(",")} onChange={(e) => handleSniffPortChange("QUIC", e.target.value)} />
      </SettingItem>

      <div className="py-2 border-b border-white/5">
        <h4 className="text-[12px] text-slate-300 font-semibold">{t("mihomo.sniffSkipDomain")}</h4>
        {renderListInputs("skipDomain", t("mihomo.sniffEg", { example: "+.push.apple.com" }))}
      </div>
      <div className="py-2 border-b border-white/5">
        <h4 className="text-[12px] text-slate-300 font-semibold">{t("mihomo.sniffForceDomain")}</h4>
        {renderListInputs("forceDomain", t("mihomo.sniffEg", { example: "+.v2ex.com" }))}
      </div>
      <div className="py-2 border-b border-white/5">
        <h4 className="text-[12px] text-slate-300 font-semibold">{t("mihomo.sniffSkipDst")}</h4>
        {renderListInputs("skipDstAddress", t("mihomo.sniffEg", { example: "91.105.192.0/23" }))}
      </div>
      <div className="py-2">
        <h4 className="text-[12px] text-slate-300 font-semibold">{t("mihomo.sniffSkipSrc")}</h4>
        {renderListInputs("skipSrcAddress", t("mihomo.sniffEgSrc", { example: "192.168.1.0/24" }))}
      </div>
    </div>
  );
}
