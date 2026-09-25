// Mihomo 代理管理（功能对齐 clash-party，外观沿用 SystemTools 风格：Tailwind + emerald + glass-panel）
import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Waypoints, Play, Square, RefreshCw, AlertTriangle } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { mihomoApi } from "./mihomoApi";

import { ModuleSettingsButton, SettingsGroup, SettingsRow, SettingsSwitch } from "../shared/ModuleSettings";
import { Note } from "../shared/Note";

import OverviewPanel from "./mihomo/OverviewPanel";
import ProxiesPanel from "./mihomo/ProxiesPanel";
import SubscriptionsPanel from "./mihomo/SubscriptionsPanel";
import RulesPanel from "./mihomo/RulesPanel";
import ConnectionsPanel from "./mihomo/ConnectionsPanel";
import LogsPanel from "./mihomo/LogsPanel";
import TrafficPanel from "./mihomo/TrafficPanel";
import ResourcesPanel from "./mihomo/ResourcesPanel";
import OverridesPanel from "./mihomo/OverridesPanel";
import SecondaryProxiesPanel from "./mihomo/SecondaryProxiesPanel";
import SysproxyPanel from "./mihomo/SysproxyPanel";
import NetworkPanel from "./mihomo/NetworkPanel";
import TunPanel from "./mihomo/TunPanel";
import DnsPanel from "./mihomo/DnsPanel";
import SnifferPanel from "./mihomo/SnifferPanel";
import SsidPanel from "./mihomo/SsidPanel";
import GroupsPanel from "./mihomo/GroupsPanel";
import CorePanel from "./mihomo/CorePanel";
import { startTrafficLogger, stopTrafficLogger } from "./mihomo/trafficDb";

const TABS = [
  { k: "overview", t: "tabOverview" },
  { k: "profiles", t: "tabProfiles" },
  { k: "proxies", t: "tabProxies" },
  { k: "groups", t: "tabGroups" },
  { k: "secondary", t: "tabSecondary" },
  { k: "rules", t: "tabRules" },
  { k: "connections", t: "tabConnections" },
  { k: "logs", t: "tabLogs" },
  { k: "traffic", t: "tabTraffic" },
  { k: "resources", t: "tabResources" },
  { k: "overrides", t: "tabOverrides" },
  { k: "sysproxy", t: "tabSysproxy" },
  { k: "network", t: "tabNetwork" },
  { k: "tun", t: "tabTun" },
  { k: "dns", t: "tabDns" },
  { k: "sniffer", t: "tabSniffer" },
  { k: "ssid", t: "tabSsid" },
  { k: "core", t: "tabCore" },
];

/** 代理模块专属设置：是否在托盘菜单显示 Mihomo 项、是否随应用开机自启。
 *  弹窗打开时才挂载，因此在这里按需拉取设置。 */
function MihomoModuleSettings() {
  const { t } = useTranslation();
  const [tray, setTray] = useState<{ enabled: boolean; show_mihomo: boolean } | null>(null);
  const [autoStart, setAutoStart] = useState<boolean | null>(null);

  useEffect(() => {
    invoke<{ enabled: boolean; show_mihomo: boolean }>("get_tray_menu_config")
      .then(setTray)
      .catch((e) => console.error("读取托盘配置失败:", e));
    invoke<string[]>("get_auto_start_services")
      .then((list) => setAutoStart(list.includes("mihomo")))
      .catch((e) => console.error("读取自启服务失败:", e));
  }, []);

  // 注意：必须回传完整的托盘配置（含全局 enabled 总开关），只改 show_mihomo
  const saveTrayItem = async (showMihomo: boolean) => {
    if (!tray) return;
    const next = { ...tray, show_mihomo: showMihomo };
    setTray(next);
    try {
      await invoke("set_tray_menu_config", { value: next });
    } catch (e) {
      console.error("保存托盘配置失败:", e);
    }
  };

  const saveAutoStart = async (enabled: boolean) => {
    setAutoStart(enabled);
    try {
      await invoke("set_auto_start_service", { serviceId: "mihomo", enabled });
    } catch (e) {
      console.error("保存自启设置失败:", e);
      setAutoStart(!enabled); // 失败回滚
    }
  };

  return (
    <SettingsGroup>
      <SettingsRow
        label={t("mihomo.settingsTrayItem")}
        hint={t("mihomo.settingsTrayItemHint")}
      >
        <SettingsSwitch checked={!!tray?.show_mihomo} disabled={!tray} onChange={saveTrayItem} />
      </SettingsRow>
      <SettingsRow
        label={t("mihomo.settingsAutoStart")}
        hint={t("mihomo.settingsAutoStartHint")}
      >
        <SettingsSwitch
          checked={!!autoStart}
          disabled={autoStart === null}
          onChange={saveAutoStart}
        />
      </SettingsRow>
    </SettingsGroup>
  );
}

export default function Mihomo() {
  const { t } = useTranslation();
  const [tab, setTab] = useState("overview");
  const [state, setState] = useState<any>(null);
  const [info, setInfo] = useState<any>(null);
  const [logLevel, setLogLevel] = useState<string>("info");
  const [busy, setBusy] = useState("");

  const refreshState = async () => {
    try { setState(await mihomoApi.getState()); } catch {}
  };
  const refreshInfo = async () => {
    try { setInfo(await mihomoApi.controllerInfo()); } catch {}
  };
  const refreshAll = async () => {
    await Promise.all([refreshState(), refreshInfo()]);
    try {
      const c = await mihomoApi.getControled();
      if (c?.["log-level"]) setLogLevel(c["log-level"]);
    } catch {}
  };

  useEffect(() => {
    refreshAll();
    const t = setInterval(refreshState, 3000);
    return () => clearInterval(t);
  }, []);

  const running = !!state?.running;

  // 全局流量采样：只要内核在跑就持续记录连接流量到 IndexedDB（供「流量」页统计）
  useEffect(() => {
    if (running && info?.port) startTrafficLogger(info);
    else stopTrafficLogger();
    return () => stopTrafficLogger();
  }, [running, info?.port, info?.secret]);

  const act = async (key: string, fn: () => Promise<any>) => {
    setBusy(key);
    try { await fn(); } catch (e: any) { alert(String(e)); }
    await refreshAll();
    setBusy("");
  };

  return (
    <div className="h-full flex flex-col select-none text-slate-200">
      {/* 固定区：头部控制栏 + 告警 + Tab 栏（不滚动） */}
      <div className="flex-shrink-0 px-6 pt-4 space-y-4">
      {/* 头部控制栏 */}
      <div className="flex flex-col md:flex-row md:items-center justify-between gap-4 border-b border-white/10 pb-4">
        <div className="flex items-center gap-3">
          <div className="p-2.5 rounded-xl bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] shadow-lg shadow-[var(--module-accent-ring)]">
            <Waypoints className="w-6 h-6" />
          </div>
          <div>
            <h3 className="text-base font-bold text-white flex items-center gap-2">
              {t("mihomo.shellTitle")}
              {running ? (
                <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-[11px] font-medium bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] text-[var(--module-accent)] border border-[var(--module-accent-ring)]">
                  <span className="w-2 h-2 rounded-full bg-[var(--module-accent)] animate-pulse" />
                  {t("mihomo.shellRunning")}
                </span>
              ) : (
                <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-[11px] font-medium bg-white/5 text-slate-400 border border-white/10">
                  ○ {t("mihomo.shellStopped")}
                </span>
              )}
              {state?.core_version && (
                <span className="text-[11px] px-2 py-0.5 rounded-md bg-white/5 text-slate-300 border border-white/10 font-mono">
                  {state.core_version}
                </span>
              )}
            </h3>
            <p className="text-xs text-slate-400 mt-0.5">
              {t("mihomo.shellDesc")}
            </p>
          </div>
        </div>

        <div className="flex items-center gap-2 self-start md:self-auto">
          {!running ? (
            <button
              onClick={() => act("start", () => mihomoApi.start())}
              disabled={busy === "start"}
              className="px-3.5 py-2 rounded-xl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-xs font-semibold flex items-center gap-1.5 transition-all shadow-md shadow-[var(--module-accent-ring)] cursor-pointer disabled:opacity-50"
            >
              <Play className="w-3.5 h-3.5 fill-current" /> {busy === "start" ? t("mihomo.shellStarting") : t("mihomo.shellStart")}
            </button>
          ) : (
            <button
              onClick={() => act("stop", () => mihomoApi.stop())}
              disabled={busy === "stop"}
              className="px-3.5 py-2 rounded-xl bg-rose-500/20 hover:bg-rose-500/30 text-rose-300 border border-rose-500/30 text-xs font-semibold flex items-center gap-1.5 transition-all cursor-pointer disabled:opacity-50"
            >
              <Square className="w-3.5 h-3.5 fill-current" /> {t("mihomo.shellStop")}
            </button>
          )}
          <button
            onClick={() => act("restart", () => mihomoApi.restart())}
            disabled={busy === "restart"}
            className="px-3.5 py-2 rounded-xl bg-white/10 hover:bg-white/20 text-slate-200 text-xs font-semibold flex items-center gap-1.5 transition-all cursor-pointer disabled:opacity-50"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${busy === "restart" ? "animate-spin" : ""}`} /> {t("mihomo.shellRestart")}
          </button>
          {/* 模块专属设置入口（托盘显示 + 开机自启） */}
          <ModuleSettingsButton title={t("mihomo.settingsTitle")} buttonClassName="p-2.5">
            <MihomoModuleSettings />
          </ModuleSettingsButton>
        </div>
      </div>

      {/* 运行告警（TUN 需要管理员、内核缺失、内核日志里的关键错误） */}
      {Array.isArray(state?.warnings) && state.warnings.length > 0 && (
        <Note
          tone="warn"
          title={t("mihomo.shellWarnings")}
          action={
            <button
              onClick={() => act("dismiss", () => mihomoApi.clearWarnings())}
              disabled={busy === "dismiss"}
              className="text-amber-300 hover:text-amber-100 text-[11px] font-semibold cursor-pointer disabled:opacity-50"
              title={t("mihomo.shellClearWarnings")}
            >
              {t("mihomo.shellClear")}
            </button>
          }
        >
          {state.warnings.map((w: string, i: number) => (
            <div key={i} className="flex items-start gap-2 text-xs text-amber-200">
              <AlertTriangle className="w-4 h-4 shrink-0 mt-px" />
              <span className="break-all">{w}</span>
            </div>
          ))}
          {state?.is_admin === false && (
            <div className="pt-1">
              <button
                onClick={() => act("elevate", () => mihomoApi.restartAsAdmin())}
                disabled={busy === "elevate"}
                className="px-3 py-1.5 rounded-lg bg-amber-500/20 hover:bg-amber-500/30 text-amber-200 border border-amber-500/40 text-[11px] font-semibold cursor-pointer disabled:opacity-50"
              >
                {busy === "elevate" ? t("mihomo.shellRestarting") : t("mihomo.shellRestartAsAdmin")}
              </button>
            </div>
          )}
        </Note>
      )}

      {/* 子 Tab 栏 */}
      <div className="flex items-center gap-1 border-b border-white/5 overflow-x-auto overflow-y-hidden">
        {TABS.map((tb) => (
          <button
            key={tb.k}
            onClick={() => setTab(tb.k)}
            className={`px-3 py-2 text-xs font-medium cursor-pointer border-b-2 -mb-px transition-all whitespace-nowrap ${
              tab === tb.k
                ? "text-[var(--module-accent)] border-[var(--module-accent)]"
                : "text-slate-400 border-transparent hover:text-slate-200"
            }`}
          >
            {t(`mihomo.${tb.t}`)}
          </button>
        ))}
      </div>
      </div>

      {/* 可滚动内容区：横向铺满，仅此区域滚动 */}
      <div className="flex-1 min-h-0 overflow-y-auto px-6 pb-6 pt-4">
      {tab === "overview" && <OverviewPanel info={info} running={running} onNavigate={setTab} />}
      {tab === "proxies" && <ProxiesPanel running={running} />}
      {tab === "groups" && <GroupsPanel />}
      {tab === "secondary" && <SecondaryProxiesPanel running={running} />}
      {tab === "profiles" && <SubscriptionsPanel running={running} onNavigate={setTab} />}
      {tab === "rules" && <RulesPanel running={running} onNavigate={setTab} />}
      {tab === "connections" && <ConnectionsPanel info={info} running={running} />}
      {tab === "logs" && <LogsPanel info={info} running={running} logLevel={logLevel} />}
      {tab === "traffic" && <TrafficPanel />}
      {tab === "resources" && <ResourcesPanel running={running} />}
      {tab === "overrides" && <OverridesPanel />}
      {tab === "sysproxy" && <SysproxyPanel />}
      {tab === "network" && <NetworkPanel />}
      {tab === "tun" && <TunPanel />}
      {tab === "dns" && <DnsPanel />}
      {tab === "sniffer" && <SnifferPanel />}
      {tab === "ssid" && <SsidPanel />}
      {tab === "core" && <CorePanel onCoreChanged={refreshAll} />}
      </div>
    </div>
  );
}
