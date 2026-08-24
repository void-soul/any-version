// 资源页 —— 1:1 复刻 clash-party src/renderer/src/pages/resources.tsx
// （GeoData：4 个 geox-url 编辑+确认 / db·dat 模式 / 自动更新+间隔 / 立即更新；
//   代理集合 provider：全部更新/单个更新/订阅信息/复制链接；规则集合 provider 同理）
import React, { useEffect, useMemo, useState } from "react";
import { RefreshCw } from "lucide-react";
import { mihomoApi } from "../mihomoApi";
import { proxyProviders as apiProxyProviders, ruleProviders as apiRuleProviders, updateProxyProvider, updateRuleProvider } from "./ctrl";
import { cardCls, SettingItem, Toggle, btnSec, btnPrimary, inputCls, tagCls, calcTraffic } from "./ui";

const DEFAULT_GEOX = {
  geoip: "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip-lite.dat",
  geosite: "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geosite.dat",
  mmdb: "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.metadb",
  asn: "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/GeoLite2-ASN.mmdb",
};

function fromNow(iso?: string): string {
  if (!iso) return "";
  const s = Math.max(0, Math.floor((Date.now() - new Date(iso).getTime()) / 1000));
  if (s < 60) return `${s} 秒前`;
  if (s < 3600) return `${Math.floor(s / 60)} 分钟前`;
  if (s < 86400) return `${Math.floor(s / 3600)} 小时前`;
  return `${Math.floor(s / 86400)} 天前`;
}

export default function ResourcesPanel({ running }: { running: boolean }) {
  const [c, setC] = useState<any>({});
  const [inputs, setInputs] = useState<Record<string, string>>({});
  const [updatingGeo, setUpdatingGeo] = useState(false);
  const [msg, setMsg] = useState("");
  const [pp, setPp] = useState<Record<string, any>>({});
  const [rp, setRp] = useState<Record<string, any>>({});
  const [updating, setUpdating] = useState<Record<string, boolean>>({});

  const geoxUrl = { ...DEFAULT_GEOX, ...(c?.["geox-url"] || {}) };
  const geoMode = !!c?.["geodata-mode"];
  const geoAutoUpdate = !!c?.["geo-auto-update"];
  const geoUpdateInterval = c?.["geo-update-interval"] ?? 24;

  const load = async () => {
    try {
      const cc = await mihomoApi.getControled();
      setC(cc || {});
      const g = { ...DEFAULT_GEOX, ...((cc || {})["geox-url"] || {}) };
      setInputs({ geoip: g.geoip, geosite: g.geosite, mmdb: g.mmdb, asn: g.asn });
    } catch {}
    if (running) {
      try {
        const [p, r] = await Promise.all([apiProxyProviders(), apiRuleProviders()]);
        setPp(p); setRp(r);
      } catch {}
    }
  };
  useEffect(() => { load(); }, [running]);

  const patchC = async (patch: any) => { await mihomoApi.patchControled(patch); load(); };

  // 复刻排序：File > Inline > HTTP，过滤 Compatible
  const ppList = useMemo(() => {
    const order: Record<string, number> = { File: 1, Inline: 2, HTTP: 3 };
    return Object.entries(pp)
      .filter(([, p]) => p.vehicleType !== "Compatible")
      .sort(([, a], [, b]) => (order[a.vehicleType] || 4) - (order[b.vehicleType] || 4));
  }, [pp]);
  const rpList = useMemo(() => Object.entries(rp), [rp]);

  const doUpdate = async (kind: "proxy" | "rule", name: string) => {
    const key = `${kind}:${name}`;
    setUpdating((s) => ({ ...s, [key]: true }));
    try {
      if (kind === "proxy") await updateProxyProvider(name);
      else await updateRuleProvider(name);
      await load();
    } catch (e: any) { setMsg(String(e)); }
    finally { setUpdating((s) => ({ ...s, [key]: false })); }
  };

  const geoRow = (key: keyof typeof DEFAULT_GEOX, title: string) => (
    <SettingItem title={title}>
      <div className="flex items-center gap-2 w-[70%]">
        {inputs[key] !== geoxUrl[key] && (
          <button className={btnPrimary} onClick={() => patchC({ "geox-url": { ...geoxUrl, [key]: inputs[key] } })}>确认</button>
        )}
        <input className={inputCls} value={inputs[key] || ""} onChange={(e) => setInputs((s) => ({ ...s, [key]: e.target.value }))} />
      </div>
    </SettingItem>
  );

  return (
    <div className="space-y-3">
      {msg && <div className="text-[11px] text-rose-300 px-1">{msg}</div>}

      {/* GeoData 卡片 */}
      <div className={`${cardCls} p-4`}>
        <h3 className="text-sm font-bold text-white mb-1">Geo 数据库</h3>
        {geoRow("geoip", "GeoIP")}
        {geoRow("geosite", "GeoSite")}
        {geoRow("mmdb", "MMDB")}
        {geoRow("asn", "ASN")}
        <SettingItem title="GeoData 模式">
          <div className="flex rounded-lg bg-white/5 border border-white/10 overflow-hidden">
            {([["db", false], ["dat", true]] as const).map(([t, v]) => (
              <button key={t} onClick={() => patchC({ "geodata-mode": v })}
                className={`px-3 py-1.5 text-[11px] font-semibold cursor-pointer transition-all ${
                  geoMode === v ? "bg-[var(--module-accent)] text-white" : "text-slate-400 hover:text-slate-200"
                }`}>{t}</button>
            ))}
          </div>
        </SettingItem>
        <SettingItem title={
          <span className="inline-flex items-center gap-1.5">自动更新
            <button className="p-0.5 rounded hover:bg-white/10 cursor-pointer" title="立即更新" onClick={async () => {
              setUpdatingGeo(true);
              try { await mihomoApi.upgradeGeo(); setMsg("Geo 数据库更新成功"); }
              catch (e: any) { setMsg(`更新失败: ${e}`); }
              finally { setUpdatingGeo(false); }
            }}>
              <RefreshCw className={`w-3 h-3 text-slate-400 ${updatingGeo ? "animate-spin" : ""}`} />
            </button>
          </span>
        } divider={geoAutoUpdate}>
          <Toggle v={geoAutoUpdate} onChange={(v) => patchC({ "geo-auto-update": v })} />
        </SettingItem>
        {geoAutoUpdate && (
          <SettingItem title="更新间隔（小时）" divider={false}>
            <input className={`${inputCls} !w-24`} type="number" value={geoUpdateInterval}
              onChange={(e) => patchC({ "geo-update-interval": parseInt(e.target.value) || 24 })} />
          </SettingItem>
        )}
      </div>

      {/* 代理集合 provider */}
      {ppList.length > 0 && (
        <div className={`${cardCls} p-4`}>
          <SettingItem title="代理集合">
            <button className={btnPrimary} onClick={() => ppList.forEach(([name]) => doUpdate("proxy", name))}>全部更新</button>
          </SettingItem>
          {ppList.map(([name, p], index) => (
            <React.Fragment key={name}>
              <SettingItem title={
                <span className="inline-flex items-center gap-1.5">{name}
                  <span className={tagCls}>{p.proxies?.length || 0}</span>
                </span>
              } divider={!p.subscriptionInfo && index !== ppList.length - 1}>
                <div className="flex items-center gap-2 text-[11px] text-slate-400">
                  <span>{fromNow(p.updatedAt)}</span>
                  <span className={tagCls}>{p.vehicleType}</span>
                  <button className={btnSec} title="更新" onClick={() => doUpdate("proxy", name)}>
                    <RefreshCw className={`w-3 h-3 ${updating[`proxy:${name}`] ? "animate-spin" : ""}`} />
                  </button>
                </div>
              </SettingItem>
              {p.subscriptionInfo && (
                <SettingItem title={
                  <span className="text-slate-400">
                    {calcTraffic((p.subscriptionInfo.Upload || 0) + (p.subscriptionInfo.Download || 0))} / {calcTraffic(p.subscriptionInfo.Total || 0)}
                  </span>
                } divider={index !== ppList.length - 1}>
                  <span className="text-[11px] text-slate-400">
                    {p.subscriptionInfo.Expire ? new Date(p.subscriptionInfo.Expire * 1000).toLocaleDateString() : "长期有效"}
                  </span>
                </SettingItem>
              )}
            </React.Fragment>
          ))}
        </div>
      )}

      {/* 规则集合 provider */}
      {rpList.length > 0 && (
        <div className={`${cardCls} p-4`}>
          <SettingItem title="规则集合">
            <button className={btnPrimary} onClick={() => rpList.forEach(([name]) => doUpdate("rule", name))}>全部更新</button>
          </SettingItem>
          {rpList.map(([name, p], index) => (
            <SettingItem key={name} title={
              <span className="inline-flex items-center gap-1.5">{name}
                <span className={tagCls}>{p.ruleCount}</span>
              </span>
            } divider={index !== rpList.length - 1}>
              <div className="flex items-center gap-2 text-[11px] text-slate-400">
                <span>{fromNow(p.updatedAt)}</span>
                <span className={tagCls}>{p.behavior}</span>
                <span className={tagCls}>{p.vehicleType}</span>
                <button className={btnSec} title="更新" onClick={() => doUpdate("rule", name)}>
                  <RefreshCw className={`w-3 h-3 ${updating[`rule:${name}`] ? "animate-spin" : ""}`} />
                </button>
              </div>
            </SettingItem>
          ))}
        </div>
      )}
      {!running && <div className={`${cardCls} p-4 text-center text-xs text-slate-400`}>核心未运行，无法读取 provider 信息</div>}
    </div>
  );
}
