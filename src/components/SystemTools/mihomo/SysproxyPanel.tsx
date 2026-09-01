// 系统代理页 —— 1:1 复刻 clash-party src/renderer/src/pages/sysproxy.tsx
// （代理主机 / 手动·PAC 模式 / UWP 工具 / PAC 脚本编辑 / bypass 列表编辑 / 保存即生效）
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Trash2 } from "lucide-react";
import { mihomoApi } from "../mihomoApi";
import { cardCls, SettingItem, Modal, btnSec, btnPrimary, inputCls } from "./ui";

const defaultPacScript = `
function FindProxyForURL(url, host) {
  return "PROXY 127.0.0.1:%mixed-port%; SOCKS5 127.0.0.1:%mixed-port%; DIRECT;";
}
`;

// Windows 默认 bypass（复刻 clash-party defaultBypass win32 分支）
const defaultBypass = [
  "localhost", "127.*", "192.168.*", "10.*",
  "172.16.*", "172.17.*", "172.18.*", "172.19.*", "172.20.*", "172.21.*", "172.22.*", "172.23.*",
  "172.24.*", "172.25.*", "172.26.*", "172.27.*", "172.28.*", "172.29.*", "172.30.*", "172.31.*",
  "<local>",
];

interface SysProxyValues {
  enable: boolean;
  host: string;
  mode: "manual" | "auto";
  bypass: string[];
  pacScript: string;
}

export default function SysproxyPanel() {
  const { t } = useTranslation();
  const [values, originSetValues] = useState<SysProxyValues>({
    enable: false, host: "", mode: "manual", bypass: defaultBypass, pacScript: defaultPacScript,
  });
  const [changed, setChanged] = useState(false);
  const [pacOpen, setPacOpen] = useState(false);
  const [pacDraft, setPacDraft] = useState("");
  const [msg, setMsg] = useState("");

  const setValues = (v: SysProxyValues) => { originSetValues(v); setChanged(true); };

  useEffect(() => {
    (async () => {
      try {
        const cfg: any = await mihomoApi.getAppConfig();
        const sp = cfg?.sysProxy || {};
        originSetValues({
          enable: sp.enable ?? cfg?.sys_proxy_enable ?? false,
          host: sp.host ?? "",
          mode: sp.mode === "auto" ? "auto" : "manual",
          bypass: Array.isArray(sp.bypass) ? sp.bypass : defaultBypass,
          pacScript: sp.pacScript || defaultPacScript,
        });
      } catch {}
    })();
  }, []);

  // 复刻 handleBypassChange：末位输入即新增，清空即删除
  const handleBypassChange = (value: string, index: number) => {
    const bypass = [...values.bypass];
    if (index === bypass.length) {
      if (value.trim() !== "") bypass.push(value);
    } else if (value.trim() === "") {
      bypass.splice(index, 1);
    } else {
      bypass[index] = value;
    }
    setValues({ ...values, bypass });
  };

  // 复刻 onSave：先存配置 → 触发系统代理 → 成功置 enable，失败回滚
  const onSave = async () => {
    setChanged(false);
    setMsg("");
    const prev = values.enable;
    try {
      await mihomoApi.patchAppConfig({ sysProxy: values });
      await mihomoApi.setSysProxy(true);
      await mihomoApi.patchAppConfig({ sysProxy: { ...values, enable: true } });
      originSetValues({ ...values, enable: true });
      setMsg(t("sysproxy.savedApplied"));
    } catch (e: any) {
      originSetValues({ ...values, enable: prev });
      setChanged(true);
      setMsg(t("sysproxy.setProxyFail", { err: String(e) }));
      try { await mihomoApi.patchAppConfig({ sysProxy: { ...values, enable: false } }); } catch {}
    }
  };

  return (
    <div className="space-y-3">
      <div className={`${cardCls} p-4`}>
        <div className="flex items-center justify-between mb-1">
          <h3 className="text-sm font-bold text-white">{t("sysproxy.title")}</h3>
          <div className="flex items-center gap-2">
            {msg && <span className="text-[11px] text-slate-400">{msg}</span>}
            {changed && <button className={btnPrimary} onClick={onSave}>{t("common.save")}</button>}
          </div>
        </div>

        <SettingItem title={t("sysproxy.proxyHost")}>
          <input
            className={`${inputCls} !w-64`}
            placeholder={t("sysproxy.hostPh")}
            value={values.host}
            onChange={(e) => setValues({ ...values, host: e.target.value })}
          />
        </SettingItem>

        <SettingItem title={t("sysproxy.mode")}>
          <div className="flex rounded-lg bg-white/5 border border-white/10 overflow-hidden">
            {([["manual", "sysproxy.manual"], ["auto", "sysproxy.pac"]] as const).map(([k, label]) => (
              <button
                key={k}
                onClick={() => setValues({ ...values, mode: k })}
                className={`px-3 py-1.5 text-[11px] font-semibold cursor-pointer transition-all ${
                  values.mode === k ? "bg-[var(--module-accent)] text-white" : "text-slate-400 hover:text-slate-200"
                }`}
              >
                {t(label)}
              </button>
            ))}
          </div>
        </SettingItem>

        <SettingItem title={t("sysproxy.uwp")}>
          <button className={btnSec} onClick={() => mihomoApi.openUwpTool().catch((e: any) => setMsg(String(e)))}>
            {t("common.open")}
          </button>
        </SettingItem>

        {values.mode === "auto" && (
          <SettingItem title={t("sysproxy.pacScript")} divider={false}>
            <button className={btnSec} onClick={() => { setPacDraft(values.pacScript || defaultPacScript); setPacOpen(true); }}>
              {t("sysproxy.editPac")}
            </button>
          </SettingItem>
        )}

        {values.mode === "manual" && (
          <>
            <SettingItem title={t("sysproxy.addDefaultBypass")}>
              <button className={btnSec} onClick={() => setValues({ ...values, bypass: defaultBypass.concat(values.bypass) })}>
                {t("sysproxy.addDefault")}
              </button>
            </SettingItem>
            <div className="pt-3">
              <h4 className="text-[12px] text-slate-300 font-semibold mb-2">{t("sysproxy.bypassList")}</h4>
              {[...values.bypass, ""].map((domain, index) => (
                <div key={index} className="mb-1.5 flex gap-2">
                  <input
                    className={inputCls}
                    placeholder={t("sysproxy.bypassPh")}
                    value={domain}
                    onChange={(e) => handleBypassChange(e.target.value, index)}
                  />
                  {index < values.bypass.length && (
                    <button className={btnSec} onClick={() => handleBypassChange("", index)}>
                      <Trash2 className="w-3.5 h-3.5 text-amber-300" />
                    </button>
                  )}
                </div>
              ))}
            </div>
          </>
        )}
      </div>

      {pacOpen && (
        <Modal
          title={t("sysproxy.pacModalTitle")}
          wide
          onClose={() => setPacOpen(false)}
          footer={
            <>
              <button className={btnSec} onClick={() => setPacOpen(false)}>{t("common.cancel")}</button>
              <button className={btnPrimary} onClick={() => { setValues({ ...values, pacScript: pacDraft }); setPacOpen(false); }}>
                {t("common.confirm")}
              </button>
            </>
          }
        >
          <textarea
            className="w-full h-80 p-3 rounded-xl bg-black/40 border border-white/10 text-[12px] font-mono text-slate-200 focus:outline-none focus:border-emerald-500 resize-none"
            value={pacDraft}
            onChange={(e) => setPacDraft(e.target.value)}
            spellCheck={false}
          />
        </Modal>
      )}
    </div>
  );
}
