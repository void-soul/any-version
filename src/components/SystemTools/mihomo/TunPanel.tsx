// TUN 页 —— 1:1 复刻 clash-party src/renderer/src/pages/tun.tsx（win32 分支）
// （防火墙重置 / 栈 gvisor·mixed·system / 设备名 / 严格路由 / 自动路由 /
//   自动选择接口 / MTU / DNS 劫持 / 排除路由地址（CIDR 校验））
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Trash2 } from "lucide-react";
import { mihomoApi } from "../mihomoApi";
import { cardCls, SettingItem, Toggle, btnSec, btnPrimary, inputCls } from "./ui";

// 复刻 shared/appConfig DEFAULT_MIHOMO_TUN_CONFIG（win 设备名默认 AnyVersion）
const DEFAULT_TUN = {
  device: "AnyVersion",
  stack: "mixed",
  autoRoute: true,
  autoRedirect: false,
  autoDetectInterface: true,
  dnsHijack: ["any:53"],
  routeExcludeAddress: [] as string[],
  strictRoute: false,
  mtu: 1500,
};

// 复刻 ipCIDRValidator
function ipCIDRValidator(s: string): boolean {
  const v4 = /^(\d{1,3}\.){3}\d{1,3}(\/(\d|[1-2]\d|3[0-2]))?$/;
  const v6 = /^([0-9a-fA-F:]+:+[0-9a-fA-F:]*)(\/(\d{1,2}|1[0-1]\d|12[0-8]))?$/;
  return v4.test(s) || v6.test(s);
}

export default function TunPanel() {
  const { t } = useTranslation();
  const [values, originSetValues] = useState({ ...DEFAULT_TUN });
  const [changed, setChanged] = useState(false);
  const [loading, setLoading] = useState(false);
  const [msg, setMsg] = useState("");
  const setValues = (v: typeof values) => { originSetValues(v); setChanged(true); };
  // 记录加载时网卡名，保存时若发生变化需重启内核（wintun 适配器名在创建时确定）
  const loadedDeviceRef = useRef(DEFAULT_TUN.device);

  useEffect(() => {
    (async () => {
      let device = DEFAULT_TUN.device;
      try {
        const app: any = await mihomoApi.getAppConfig();
        if (app?.tun_name) device = app.tun_name;
      } catch {}
      loadedDeviceRef.current = device;
      try {
        const c: any = await mihomoApi.getControledConfig();
        const tun = c?.tun || {};
        originSetValues({
          device,
          stack: tun.stack ?? DEFAULT_TUN.stack,
          autoRoute: tun["auto-route"] ?? DEFAULT_TUN.autoRoute,
          autoRedirect: tun["auto-redirect"] ?? DEFAULT_TUN.autoRedirect,
          autoDetectInterface: tun["auto-detect-interface"] ?? DEFAULT_TUN.autoDetectInterface,
          dnsHijack: tun["dns-hijack"] ?? DEFAULT_TUN.dnsHijack,
          routeExcludeAddress: tun["route-exclude-address"] ?? [],
          strictRoute: tun["strict-route"] ?? false,
          mtu: Math.min(Math.max(tun.mtu || 1500, 1), 65535),
        });
      } catch {}
    })();
  }, []);

  const normalized = values.routeExcludeAddress.map((a) => a.trim()).filter(Boolean);
  const hasInvalid = normalized.some((a) => !ipCIDRValidator(a));
  const excludeInputs = hasInvalid ? values.routeExcludeAddress : [...values.routeExcludeAddress, ""];

  const handleExcludeChange = (value: string, index: number) => {
    const arr = [...values.routeExcludeAddress];
    if (index === arr.length) {
      if (value.trim() !== "") arr.push(value);
    } else if (value.trim() === "") {
      arr.splice(index, 1);
    } else {
      arr[index] = value;
    }
    setValues({ ...values, routeExcludeAddress: arr });
  };

  // 复刻 onSave：持久化到 AppConfig（单一真相源）+ 热更新 controled；
  // 若网卡名变化则重启内核，使 wintun 适配器重建为新名称（热更新无法改名）
  const onSave = async () => {
    const tun: any = {
      device: values.device,
      stack: values.stack,
      "auto-route": values.autoRoute,
      "auto-redirect": values.autoRedirect,
      "auto-detect-interface": values.autoDetectInterface,
      "dns-hijack": values.dnsHijack,
      "strict-route": values.strictRoute,
      mtu: values.mtu,
    };
    if (!hasInvalid) tun["route-exclude-address"] = normalized;
    const nameChanged = values.device.trim() !== loadedDeviceRef.current.trim();
    try {
      await mihomoApi.patchAppConfig({ tun_name: values.device });
      await mihomoApi.patchControledConfig({ tun });
      if (nameChanged) {
        loadedDeviceRef.current = values.device;
        await mihomoApi.restart();
        setMsg(t("tun.savedRestart"));
      } else {
        setMsg(t("tun.savedReload"));
      }
    } catch (e: any) {
      setMsg(t("tun.saveFail", { err: String(e) }));
    } finally {
      setChanged(false);
    }
  };

  return (
    <div className={`${cardCls} p-4`}>
      <div className="flex items-center justify-between mb-1">
        <h3 className="text-sm font-bold text-white">{t("tun.title")}</h3>
        <div className="flex items-center gap-2">
          {msg && <span className="text-[11px] text-slate-400">{msg}</span>}
          {changed && <button className={btnPrimary} onClick={onSave}>{t("common.save")}</button>}
        </div>
      </div>

      <SettingItem title={t("tun.resetFirewall")}>
        <button
          className={btnSec}
          disabled={loading}
          onClick={async () => {
            setLoading(true);
            try {
              await mihomoApi.setupFirewall();
              setMsg(t("tun.firewallResetOk"));
              await mihomoApi.restart();
            } catch (e: any) { setMsg(t("tun.firewallFail", { err: String(e) })); }
            finally { setLoading(false); }
          }}
        >
          {loading ? t("tun.processing") : t("tun.reset")}
        </button>
      </SettingItem>

      <SettingItem title={t("tun.stack")}>
        <div className="flex rounded-lg bg-white/5 border border-white/10 overflow-hidden">
          {(["gvisor", "mixed", "system"] as const).map((k) => (
            <button
              key={k}
              onClick={() => setValues({ ...values, stack: k })}
              className={`px-3 py-1.5 text-[11px] font-semibold cursor-pointer transition-all ${
                values.stack === k ? "bg-[var(--module-accent)] text-white" : "text-slate-400 hover:text-slate-200"
              }`}
            >
              {k === "gvisor" ? "gVisor" : k === "mixed" ? "Mixed" : "System"}
            </button>
          ))}
        </div>
      </SettingItem>

      <SettingItem title={t("tun.device")}>
        <input className={`${inputCls} !w-40`} placeholder="Mihomo" value={values.device}
          onChange={(e) => setValues({ ...values, device: e.target.value })} />
      </SettingItem>

      <SettingItem title={t("tun.strictRoute")}>
        <Toggle v={values.strictRoute} onChange={(v) => setValues({ ...values, strictRoute: v })} />
      </SettingItem>
      <SettingItem title={t("tun.autoRoute")}>
        <Toggle v={values.autoRoute} onChange={(v) => setValues({ ...values, autoRoute: v })} />
      </SettingItem>
      <SettingItem title={t("tun.autoDetect")}>
        <Toggle v={values.autoDetectInterface} onChange={(v) => setValues({ ...values, autoDetectInterface: v })} />
      </SettingItem>

      <SettingItem title="MTU">
        <input
          className={`${inputCls} !w-28`} type="number" min={1} value={values.mtu}
          onChange={(e) => setValues({ ...values, mtu: Math.min(Math.max(parseInt(e.target.value) || 1500, 1), 65535) })}
        />
      </SettingItem>

      <SettingItem title={t("tun.dnsHijack")}>
        <input
          className={`${inputCls} !w-64`} placeholder="any:53"
          value={values.dnsHijack.join(",")}
          onChange={(e) => setValues({ ...values, dnsHijack: e.target.value !== "" ? e.target.value.split(",") : [] })}
        />
      </SettingItem>

      <div className="pt-3">
        <h4 className="text-[12px] text-slate-300 font-semibold mb-2">{t("tun.excludeCidrs")}</h4>
        {excludeInputs.map((address, index) => {
          const invalid = address.trim() !== "" && !ipCIDRValidator(address.trim());
          return (
            <div key={index} className="mb-1.5">
              <div className="flex gap-2">
                <input
                  className={`${inputCls} ${invalid ? "!border-rose-500" : ""}`}
                  placeholder={t("tun.cidrPlaceholder")}
                  value={address}
                  onChange={(e) => handleExcludeChange(e.target.value, index)}
                />
                {index < values.routeExcludeAddress.length && (
                  <button className={btnSec} onClick={() => handleExcludeChange("", index)}>
                    <Trash2 className="w-3.5 h-3.5 text-amber-300" />
                  </button>
                )}
              </div>
              {invalid && <div className="text-[10px] text-rose-400 mt-0.5 px-1">{t("tun.invalidCidr")}</div>}
            </div>
          );
        })}
        {hasInvalid && <div className="text-[10px] text-rose-400 px-1">{t("tun.invalidEntries")}</div>}
      </div>
    </div>
  );
}
