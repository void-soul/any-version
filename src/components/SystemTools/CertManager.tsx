import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ShieldCheck, Server, KeyRound, History, Activity, Bell, RadioTower } from "lucide-react";
import CertReceiverView from "./CertReceiverView";
import { type SubTab } from "./CertManager.model";
import { CertList } from "./CertManager.CertList";
import { DeployNodes } from "./CertManager.DeployNodes";
import { Credentials } from "./CertManager.Credentials";
import { MonitorView } from "./CertManager.MonitorView";
import { NotifyView } from "./CertManager.NotifyView";
import { SchedulerView } from "./CertManager.SchedulerView";
// ---------------------------------------------------------------------------
// 组件
// ---------------------------------------------------------------------------
let persistedTab: SubTab = "certs";

export default function CertManager() {
  const { t } = useTranslation();
  const [tab, setTab] = useState<SubTab>(persistedTab);
  const changeTab = (k: SubTab) => {
    persistedTab = k;
    setTab(k);
  };

  return (
    <div className="flex-1 overflow-hidden flex min-h-0 select-none">
      <div className="w-40 flex-shrink-0 border-r border-white/5 py-3 px-2 space-y-0.5 overflow-y-auto">
        {([
          { k: "certs" as const, label: t("certmgr.tabCerts"), icon: ShieldCheck },
          { k: "nodes" as const, label: t("certmgr.tabNodes"), icon: Server },
          { k: "creds" as const, label: t("certmgr.tabCreds"), icon: KeyRound },
          { k: "monitors" as const, label: t("certmgr.tabMonitors"), icon: Activity },
          { k: "notify" as const, label: t("certmgr.tabNotify"), icon: Bell },
          { k: "sched" as const, label: t("certmgr.tabSched"), icon: History },
          { k: "receiver" as const, label: t("certmgr.tabReceiver"), icon: RadioTower },
        ]).map(({ k, label, icon: Icon }) => (
          <button
            key={k}
            onClick={() => changeTab(k)}
            className={`w-full px-3 py-2 rounded-ctl text-caption font-semibold flex items-center gap-2 transition-all cursor-pointer text-left ${
              tab === k
                ? "bg-[var(--module-accent)] text-white shadow-md shadow-[var(--module-accent-ring)]"
                : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
            }`}
          >
            <Icon className="w-3.5 h-3.5 flex-shrink-0" />
            {label}
          </button>
        ))}
      </div>
      <div className="flex-1 min-h-0 overflow-y-auto px-6 py-4">
        {tab === "certs" && <CertList />}
        {tab === "nodes" && <DeployNodes />}
        {tab === "creds" && <Credentials />}
        {tab === "monitors" && <MonitorView />}
        {tab === "notify" && <NotifyView />}
        {tab === "sched" && <SchedulerView />}
        {tab === "receiver" && <CertReceiverView />}
      </div>
    </div>
  );
}
