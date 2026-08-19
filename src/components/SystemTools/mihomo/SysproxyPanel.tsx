// 系统代理页 —— 1:1 复刻 clash-party src/renderer/src/pages/sysproxy.tsx
// （代理主机 / 手动·PAC 模式 / UWP 工具 / PAC 脚本编辑 / bypass 列表编辑 / 保存即生效）
import React, { useEffect, useState } from "react";
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
      setMsg("已保存并生效");
    } catch (e: any) {
      originSetValues({ ...values, enable: prev });
      setChanged(true);
      setMsg(`设置系统代理失败: ${e}`);
      try { await mihomoApi.patchAppConfig({ sysProxy: { ...values, enable: false } }); } catch {}
    }
  };

  return (
    <div className="space-y-3">
      <div className={`${cardCls} p-4`}>
        <div className="flex items-center justify-between mb-1">
          <h3 className="text-sm font-bold text-white">系统代理设置</h3>
          <div className="flex items-center gap-2">
            {msg && <span className="text-[11px] text-slate-400">{msg}</span>}
            {changed && <button className={btnPrimary} onClick={onSave}>保存</button>}
          </div>
        </div>

        <SettingItem title="代理主机">
          <input
            className={`${inputCls} !w-64`}
            placeholder="默认 127.0.0.1"
            value={values.host}
            onChange={(e) => setValues({ ...values, host: e.target.value })}
          />
        </SettingItem>

        <SettingItem title="代理模式">
          <div className="flex rounded-lg bg-white/5 border border-white/10 overflow-hidden">
            {([["manual", "手动"], ["auto", "PAC"]] as const).map(([k, t]) => (
              <button
                key={k}
                onClick={() => setValues({ ...values, mode: k })}
                className={`px-3 py-1.5 text-[11px] font-semibold cursor-pointer transition-all ${
                  values.mode === k ? "bg-[var(--module-accent)] text-white" : "text-slate-400 hover:text-slate-200"
                }`}
              >
                {t}
              </button>
            ))}
          </div>
        </SettingItem>

        <SettingItem title="UWP 工具">
          <button className={btnSec} onClick={() => mihomoApi.openUwpTool().catch((e: any) => setMsg(String(e)))}>
            打开
          </button>
        </SettingItem>

        {values.mode === "auto" && (
          <SettingItem title="PAC 脚本" divider={false}>
            <button className={btnSec} onClick={() => { setPacDraft(values.pacScript || defaultPacScript); setPacOpen(true); }}>
              编辑 PAC 脚本
            </button>
          </SettingItem>
        )}

        {values.mode === "manual" && (
          <>
            <SettingItem title="添加默认代理绕过">
              <button className={btnSec} onClick={() => setValues({ ...values, bypass: defaultBypass.concat(values.bypass) })}>
                添加默认
              </button>
            </SettingItem>
            <div className="pt-3">
              <h4 className="text-[12px] text-slate-300 font-semibold mb-2">代理绕过列表</h4>
              {[...values.bypass, ""].map((domain, index) => (
                <div key={index} className="mb-1.5 flex gap-2">
                  <input
                    className={inputCls}
                    placeholder="例: *.example.com"
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
          title="编辑 PAC 脚本（%mixed-port% 会被替换为混合端口）"
          wide
          onClose={() => setPacOpen(false)}
          footer={
            <>
              <button className={btnSec} onClick={() => setPacOpen(false)}>取消</button>
              <button className={btnPrimary} onClick={() => { setValues({ ...values, pacScript: pacDraft }); setPacOpen(false); }}>
                确认
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
