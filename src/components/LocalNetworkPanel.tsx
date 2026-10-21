import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Activity, Cable, Globe, Server } from "lucide-react";
import NetworkOverview from "./network/NetworkOverview";
import NetworkConnections from "./network/NetworkConnections";
import NetworkDiagnostics from "./network/NetworkDiagnostics";
import PortScanner from "./PortScanner";

type TabKey = "overview" | "conns" | "diag" | "ports";

export default function LocalNetworkPanel() {
  const { t } = useTranslation();
  const [tab, setTab] = useState<TabKey>("overview");

  const tabs: { key: TabKey; label: string; Icon: typeof Activity }[] = [
    { key: "overview", label: t("netpan.tabOverview"), Icon: Activity },
    { key: "conns", label: t("netpan.tabConns"), Icon: Cable },
    { key: "diag", label: t("netpan.tabDiag"), Icon: Globe },
    { key: "ports", label: t("netpan.tabPorts"), Icon: Server },
  ];

  return (
    <div className="flex h-full w-full select-none flex-col overflow-hidden text-slate-200">
      {/* 模块头部 + 页签 */}
      <div className="flex shrink-0 items-center gap-2 border-b border-white/5 px-4 py-3">
        <Activity className="h-4 w-4 text-[var(--module-accent)]" />
        <span className="text-sm font-bold text-white">{t("netpan.title")}</span>
        <span className="hidden text-tiny text-slate-500 sm:inline">{t("netpan.subtitle")}</span>
        <div className="flex-1" />
        <div className="flex items-center gap-0.5 rounded-ctl border border-white/5 bg-white/5 p-0.5">
          {tabs.map(({ key, label, Icon }) => (
            <button
              key={key}
              onClick={() => setTab(key)}
              className={`flex cursor-pointer items-center gap-1 rounded-md px-3 py-1.5 text-tiny font-semibold transition-all ${
                tab === key
                  ? "text-white shadow-sm"
                  : "text-slate-400 hover:bg-white/5 hover:text-slate-200"
              }`}
              style={tab === key ? { backgroundColor: "var(--module-accent)" } : undefined}
            >
              <Icon className="h-3 w-3" />
              {label}
            </button>
          ))}
        </div>
      </div>

      <div className="min-h-0 flex-1">
        {/* 概览常驻挂载：切走只停止采样（active=false），曲线与网卡数据都留着 ——
            重新挂载等于又白等一次 PowerShell 枚举（1~3s）。 */}
        <div className={tab === "overview" ? "h-full" : "hidden"}>
          <NetworkOverview active={tab === "overview"} />
        </div>
        {tab === "conns" && (
          <div className="h-full p-4">
            <NetworkConnections />
          </div>
        )}
        {tab === "diag" && <NetworkDiagnostics />}
        {tab === "ports" && (
          <div className="h-full p-4">
            <PortScanner />
          </div>
        )}
      </div>
    </div>
  );
}
