// 概览页 —— 1:1 复刻 clash-party 侧栏卡片区功能
// （出站模式切换 rule/global/direct + 切换时可选断开全部连接；
//   系统代理开关卡片；TUN 开关卡片；连接卡片：traffic WS 速率 + 迷你趋势；
//   内核卡片：memory WS 内存占用；订阅用量卡片）
import React, { useEffect, useRef, useState } from "react";
import { ArrowUpCircle, ArrowDownCircle, Cpu, Globe, Shield, Link2 } from "lucide-react";
import { mihomoApi } from "../mihomoApi";
import { openMihomoWs, patchRuntimeConfigs, closeAllConnections, WsHandle } from "./ctrl";
import { cardCls, Toggle, calcTraffic } from "./ui";

export default function OverviewPanel({ info, running, onNavigate }: {
  info: any; running: boolean; onNavigate?: (tab: string) => void;
}) {
  const [mode, setMode] = useState<string>("");
  const [app, setApp] = useState<any>({});
  const [c, setC] = useState<any>({});
  const [up, setUp] = useState(0);
  const [down, setDown] = useState(0);
  const [series, setSeries] = useState<number[]>(Array(10).fill(0));
  const [memory, setMemory] = useState(0);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");

  const load = async () => {
    try {
      const [a, cc] = await Promise.all([mihomoApi.getAppConfig(), mihomoApi.getControled()]);
      setApp(a || {});
      setC(cc || {});
      setMode((cc as any)?.mode || "rule");
    } catch {}
  };
  useEffect(() => { load(); }, [running]);

  // traffic WS（复刻 conn-card：10 点滑动窗口）
  useEffect(() => {
    if (!running || !info?.port) return;
    const ws: WsHandle = openMihomoWs(info, "traffic", (t) => {
      setUp(t.up || 0);
      setDown(t.down || 0);
      setSeries((prev) => [...prev.slice(1), (t.up || 0) + (t.down || 0)]);
    });
    return () => ws.close();
  }, [running, info?.port, info?.secret]);

  // memory WS（复刻 mihomo-core-card）
  useEffect(() => {
    if (!running || !info?.port) return;
    const ws: WsHandle = openMihomoWs(info, "memory", (m) => setMemory(m.inuse || 0));
    return () => ws.close();
  }, [running, info?.port, info?.secret]);

  // 复刻 onChangeMode：patchControled + PATCH /configs + 可选断开全部连接
  const onChangeMode = async (m: string) => {
    setMode(m);
    try {
      await mihomoApi.patchControled({ mode: m });
      await patchRuntimeConfigs({ mode: m });
      if (app?.autoCloseConnection ?? true) await closeAllConnections();
    } catch (e: any) { setMsg(String(e)); }
    load();
  };

  // 系统代理开关（复刻 sysproxy-switcher）
  const sysProxyEnabled = !!(app?.sysProxy?.enable ?? app?.sys_proxy_enable);
  const toggleSysProxy = async (v: boolean) => {
    setBusy(true);
    try {
      await mihomoApi.setSysProxy(v);
      const sp = { ...(app?.sysProxy || {}), enable: v };
      await mihomoApi.patchAppConfig({ sysProxy: sp });
    } catch (e: any) { setMsg(String(e)); }
    finally { setBusy(false); load(); }
  };

  // TUN 开关（复刻 tun-switcher：开 TUN 时关系统代理）
  const tunEnabled = !!c?.tun?.enable;
  const toggleTun = async (v: boolean) => {
    setBusy(true);
    try {
      if (v && sysProxyEnabled) await toggleSysProxy(false);
      await mihomoApi.patchControled({ tun: { enable: v } });
      await mihomoApi.restart();
    } catch (e: any) { setMsg(String(e)); }
    finally { setBusy(false); load(); }
  };

  const maxSeries = Math.max(1, ...series);

  return (
    <div className="space-y-3">
      {msg && <div className="text-[11px] text-rose-300 px-1">{msg}</div>}

      {/* 出站模式（复刻 outbound-mode-switcher） */}
      <div className={`${cardCls} p-2 flex`}>
        {([["rule", "规则"], ["global", "全局"], ["direct", "直连"]] as const).map(([k, t]) => (
          <button key={k} onClick={() => onChangeMode(k)} disabled={!running}
            className={`flex-1 py-2 rounded-xl text-[12px] font-semibold cursor-pointer transition-all disabled:opacity-40 ${
              mode === k ? "bg-emerald-600 text-white" : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
            }`}>{t}</button>
        ))}
      </div>

      <div className="grid grid-cols-2 gap-3">
        {/* 系统代理卡片 */}
        <div className={`${cardCls} p-4 flex items-center justify-between`}>
          <div className="flex items-center gap-3">
            <Globe className="w-6 h-6 text-emerald-400" />
            <div>
              <div className="text-[13px] font-bold text-white">系统代理</div>
              <div className="text-[10px] text-slate-500">{sysProxyEnabled ? "已开启" : "未开启"}</div>
            </div>
          </div>
          <Toggle v={sysProxyEnabled} disabled={busy || !running} onChange={toggleSysProxy} />
        </div>

        {/* TUN 卡片 */}
        <div className={`${cardCls} p-4 flex items-center justify-between`}>
          <div className="flex items-center gap-3">
            <Shield className="w-6 h-6 text-sky-400" />
            <div>
              <div className="text-[13px] font-bold text-white">虚拟网卡 (TUN)</div>
              <div className="text-[10px] text-slate-500">{tunEnabled ? "已开启" : "未开启"}</div>
            </div>
          </div>
          <Toggle v={tunEnabled} disabled={busy || !running} onChange={toggleTun} />
        </div>

        {/* 连接卡片（速率 + 迷你趋势图） */}
        <div className={`${cardCls} p-4 relative overflow-hidden cursor-pointer hover:border-white/20`}
          onClick={() => onNavigate?.("connections")}>
          <div className="absolute inset-x-0 bottom-0 h-12 flex items-end gap-px opacity-40 pointer-events-none">
            {series.map((v, i) => (
              <div key={i} className="flex-1 bg-emerald-500 rounded-t-sm" style={{ height: `${(v / maxSeries) * 100}%` }} />
            ))}
          </div>
          <div className="flex items-center justify-between relative">
            <Link2 className="w-6 h-6 text-emerald-400" />
            <div className="text-right space-y-0.5">
              <div className="flex items-center justify-end gap-1.5 text-[12px] text-slate-200 font-mono">
                {calcTraffic(up)}/s <ArrowUpCircle className="w-3.5 h-3.5 text-sky-400" />
              </div>
              <div className="flex items-center justify-end gap-1.5 text-[12px] text-slate-200 font-mono">
                {calcTraffic(down)}/s <ArrowDownCircle className="w-3.5 h-3.5 text-emerald-400" />
              </div>
            </div>
          </div>
          <div className="text-[13px] font-bold text-white mt-3 relative">连接</div>
        </div>

        {/* 内核卡片（内存） */}
        <div className={`${cardCls} p-4 cursor-pointer hover:border-white/20`} onClick={() => onNavigate?.("core")}>
          <div className="flex items-center justify-between">
            <Cpu className="w-6 h-6 text-amber-400" />
            <div className="text-[12px] text-slate-200 font-mono">{running ? calcTraffic(memory) : "-"}</div>
          </div>
          <div className="text-[13px] font-bold text-white mt-3">Mihomo 内核</div>
          <div className="text-[10px] text-slate-500">{running ? "运行中 · 内存占用" : "未运行"}</div>
        </div>
      </div>
    </div>
  );
}
